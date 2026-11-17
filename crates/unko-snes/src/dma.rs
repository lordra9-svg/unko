//! S-CPU DMA channels.
//!
//! General-purpose DMA is executed one byte at a time; HDMA walks a table once
//! per scanline and supports direct and indirect entries.

/// One DMA channel's register state.
///
/// The registers power up zeroed (ares `channels[id] = {}`). Games such as
/// Super Mario World rely on that: they program only the A-bus side of the
/// indirect HDMA channel and leave A2A at zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DmaChannel {
    /// `$43x0` mode, direction, fixed and reverse bits.
    pub control: u8,
    /// `$43x1` B-bus address.
    pub target: u8,
    /// `$43x2-$43x3` A-bus address.
    pub source: u16,
    /// `$43x4` A-bus bank.
    pub bank: u8,
    /// `$43x5-$43x6` transfer size.
    pub size: u16,
    /// `$43x7` HDMA indirect bank.
    pub indirect_bank: u8,
    /// `$43x8-$43x9` HDMA table address.
    pub hdma_address: u16,
    /// `$43xA` HDMA line counter.
    pub line_counter: u8,
}

/// Per-channel HDMA table cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
struct HdmaState {
    address: u16,
    indirect_address: u16,
    line_counter: u8,
    completed: bool,
    do_transfer: bool,
}

/// The eight DMA channels and their enable latches.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Dma {
    channels: [DmaChannel; 8],
    pending: u8,
    /// Channels enabled by a write to `$420C` (HDMA).
    pub hdma_enable: u8,
    hdma: [HdmaState; 8],
}

/// Bus operations used by DMA.
pub trait DmaBus {
    /// Read the A bus. Invalid A-bus addresses must return `0`.
    fn read_a(&mut self, address: u32) -> u8;
    /// Write the A bus. Invalid addresses must be ignored.
    fn write_a(&mut self, address: u32, value: u8);
    /// Read a B-bus register in `$2100-$21FF`.
    fn read_b(&mut self, address: u8) -> u8;
    /// Write a B-bus register in `$2100-$21FF`.
    fn write_b(&mut self, address: u8, value: u8);
}

impl Dma {
    /// One channel's register state (debug and tests).
    pub const fn channel(&self, index: usize) -> DmaChannel {
        self.channels[index]
    }

    /// Create eight idle channels.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a DMA register in `$4300-$437F`.
    pub fn read(&self, address: u16) -> u8 {
        let index = usize::from((address >> 4) & 0x07);
        let channel = &self.channels[index];
        match address & 0x0F {
            0x0 => channel.control,
            0x1 => channel.target,
            0x2 => channel.source as u8,
            0x3 => (channel.source >> 8) as u8,
            0x4 => channel.bank,
            0x5 => channel.size as u8,
            0x6 => (channel.size >> 8) as u8,
            0x7 => channel.indirect_bank,
            0x8 => channel.hdma_address as u8,
            0x9 => (channel.hdma_address >> 8) as u8,
            0xA => channel.line_counter,
            _ => 0,
        }
    }

    /// Write a DMA register in `$4300-$437F`.
    pub fn write(&mut self, address: u16, value: u8) {
        let index = usize::from((address >> 4) & 0x07);
        let channel = &mut self.channels[index];
        match address & 0x0F {
            0x0 => channel.control = value,
            0x1 => channel.target = value,
            0x2 => channel.source = (channel.source & 0xFF00) | u16::from(value),
            0x3 => channel.source = (channel.source & 0x00FF) | (u16::from(value) << 8),
            0x4 => channel.bank = value,
            0x5 => channel.size = (channel.size & 0xFF00) | u16::from(value),
            0x6 => channel.size = (channel.size & 0x00FF) | (u16::from(value) << 8),
            0x7 => channel.indirect_bank = value,
            0x8 => channel.hdma_address = (channel.hdma_address & 0xFF00) | u16::from(value),
            0x9 => {
                channel.hdma_address = (channel.hdma_address & 0x00FF) | (u16::from(value) << 8);
            }
            0xA => channel.line_counter = value,
            _ => {}
        }
    }

    /// Start any channels whose bit is set and return a copy of the channel
    /// configurations for the transfer engine.
    pub fn take_pending(&mut self) -> Option<([DmaChannel; 8], u8)> {
        if self.pending == 0 {
            return None;
        }
        let mask = self.pending;
        self.pending = 0;
        Some((self.channels, mask))
    }

    /// Latch a write to `$420B`.
    pub fn start(&mut self, mask: u8) {
        self.pending = mask;
    }

    /// Take the latched `$420B` write as an in-progress transfer.
    pub fn begin_transfer(&mut self) -> Option<DmaTransfer> {
        self.take_pending()
            .map(|(channels, mask)| DmaTransfer::new(channels, mask))
    }

    /// Enable HDMA channels and reset their table cursors.
    pub fn set_hdma_enable(&mut self, mask: u8) {
        self.hdma_enable = mask;
        self.hdma_reset();
    }

    /// Reset every HDMA table cursor to the channel's table address.
    ///
    /// The HDMA table lives at `A1T`/`A1B` (`$4302-$4304`), not at `A2A`
    /// (`$4308`); ares `Channel::hdmaSetup` sets `hdmaAddress = sourceAddress`.
    pub fn hdma_reset(&mut self) {
        // ares `Channel::hdmaReset`: only the completion/transfer latches; the
        // address and counter are set by the per-frame setup.
        for state in &mut self.hdma {
            state.completed = false;
            state.do_transfer = false;
        }
    }

    /// Read the next HDMA table entry when the current block has run out
    /// (ares `Channel::hdmaReload`).
    fn hdma_reload(&mut self, bus: &mut impl DmaBus, index: usize, channel: DmaChannel) -> u32 {
        let state = &mut self.hdma[index];
        if state.line_counter & 0x7F != 0 {
            return 0;
        }
        let mut reads = 1;
        let address = (u32::from(channel.bank) << 16) | u32::from(state.address);
        let count = bus.read_a(address);
        state.address = state.address.wrapping_add(1);
        state.line_counter = count;
        state.completed = count == 0;
        state.do_transfer = count != 0;
        if channel.control & 0x40 != 0 {
            let low = bus.read_a((u32::from(channel.bank) << 16) | u32::from(state.address));
            state.address = state.address.wrapping_add(1);
            let high = bus.read_a((u32::from(channel.bank) << 16) | u32::from(state.address));
            state.address = state.address.wrapping_add(1);
            state.indirect_address = u16::from_le_bytes([low, high]);
            reads += 2;
        }
        reads
    }

    /// Per-frame HDMA setup at V=0 (ares `Channel::hdmaSetup`): restart every
    /// enabled channel at its table and load the first block's counter.
    pub fn hdma_setup(&mut self, bus: &mut impl DmaBus) -> u32 {
        if self.hdma_enable == 0 {
            return 0;
        }
        // ares `CPU::hdmaSetup`: one 8-clock cycle plus 8 clocks per table read.
        let mut cost = 8;
        for index in 0..8 {
            self.hdma[index].do_transfer = true;
            if self.hdma_enable & (1 << index) == 0 {
                continue;
            }
            let channel = self.channels[index];
            let state = &mut self.hdma[index];
            state.address = channel.source;
            state.line_counter = 0;
            state.completed = false;
            cost += 8 * self.hdma_reload(bus, index, channel);
        }
        cost
    }

    /// Run one scanline of HDMA transfers (ares `hdmaRun`): transfer the
    /// current block's bytes, then decrement the line counter and reload the
    /// next entry when the block is exhausted.
    ///
    /// Returns the master clocks the transfer steals from the CPU (ares
    /// `CPU::hdmaRun`: 8 for the run plus 8 per table read and 8 per byte
    /// read + write).
    pub fn hdma_line(&mut self, bus: &mut impl DmaBus) -> u32 {
        if self.hdma_enable == 0 {
            return 0;
        }
        let mut cost = 8u32;
        for index in 0..8 {
            if self.hdma_enable & (1 << index) == 0 || self.hdma[index].completed {
                continue;
            }
            let channel = self.channels[index];
            if self.hdma[index].do_transfer {
                let length = match channel.control & 0x07 {
                    0 => 1,
                    1 | 2 | 6 => 2,
                    _ => 4,
                };
                for offset in 0..length {
                    let target_offset = match channel.control & 0x07 {
                        1 | 5 => (offset & 1) as u8,
                        3 | 7 => ((offset >> 1) & 1) as u8,
                        4 => offset as u8,
                        _ => 0,
                    };
                    let target = channel.target.wrapping_add(target_offset);
                    let indirect = channel.control & 0x40 != 0;
                    let state = &mut self.hdma[index];
                    let address = if indirect {
                        (u32::from(channel.indirect_bank) << 16) | u32::from(state.indirect_address)
                    } else {
                        (u32::from(channel.bank) << 16) | u32::from(state.address)
                    };
                    let value = bus.read_a(address);
                    if std::env::var_os("HDMA_LOG").is_some() && (0x21..=0x22).contains(&target) {
                        use std::io::Write;
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("/tmp/opencode/hdma.log")
                        {
                            let _ = writeln!(
                                f,
                                "ch{} target={:02X} value={:02X} addr={:06X} indirect={} ctrl={:02X} bank={:02X} iaddr={:04X} ibank={:02X}",
                                index,
                                target,
                                value,
                                address,
                                indirect,
                                channel.control,
                                channel.bank,
                                state.indirect_address,
                                channel.indirect_bank
                            );
                        }
                    }
                    bus.write_b(target, value);
                    cost += 16;
                    if indirect {
                        state.indirect_address = state.indirect_address.wrapping_add(1);
                    } else {
                        state.address = state.address.wrapping_add(1);
                    }
                }
            }
            let state = &mut self.hdma[index];
            state.line_counter = state.line_counter.wrapping_sub(1);
            state.do_transfer = state.line_counter & 0x80 != 0;
            cost += 8 * self.hdma_reload(bus, index, channel);
        }
        cost
    }
}

/// A general-purpose DMA transfer in progress.
///
/// The transfer is driven one A/B byte at a time so the system can advance the
/// PPU, APU and IRQ comparators for each eight-clock DMA cycle. ares runs the
/// same way, which keeps a sprite or VRAM upload aligned with the scanline it
/// lands on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DmaTransfer {
    channels: [DmaChannel; 8],
    mask: u8,
    channel: usize,
    /// ares `CPU::dmaRun` charges one eight-clock cycle before any channel.
    whole_charged: bool,
    setup: bool,
    transferred: u32,
    count: u32,
}

impl DmaTransfer {
    fn new(channels: [DmaChannel; 8], mask: u8) -> Self {
        Self {
            channels,
            mask,
            channel: 0,
            whole_charged: false,
            setup: true,
            transferred: 0,
            count: 0,
        }
    }

    /// Whether any channel still has bytes or setup left.
    pub const fn busy(&self) -> bool {
        self.mask != 0
    }

    /// Execute one DMA step and return the master clocks it consumed.
    ///
    /// A channel begins with an eight-clock setup step, then transfers one A/B
    /// byte per step.
    pub fn step(&mut self, bus: &mut impl DmaBus) -> u32 {
        if self.mask == 0 {
            return 0;
        }
        if !self.whole_charged {
            // ares `CPU::dmaRun`: `counter.dma += 8; step(8);` once per trigger,
            // before the per-channel `Channel::dmaRun` setup cycle.
            self.whole_charged = true;
            return 8;
        }
        if self.setup {
            let index = self.mask.trailing_zeros() as usize;
            self.channel = index;
            let channel = self.channels[index];
            self.count = if channel.size == 0 {
                0x1_0000
            } else {
                u32::from(channel.size)
            };
            self.transferred = 0;
            self.setup = false;
            if self.count == 0 {
                self.mask &= !(1 << index);
                self.setup = true;
            }
            return 8;
        }

        let channel = &mut self.channels[self.channel];
        let mode = channel.control & 0x07;
        let fixed = channel.control & 0x08 != 0;
        let reverse = channel.control & 0x10 != 0;
        let direction = channel.control & 0x80 != 0;
        let index_in_transfer = self.transferred;
        let target = channel.target.wrapping_add(match mode {
            1 | 5 => (index_in_transfer & 1) as u8,
            3 | 7 => ((index_in_transfer >> 1) & 1) as u8,
            4 => index_in_transfer as u8,
            _ => 0,
        });
        let address = (u32::from(channel.bank) << 16) | u32::from(channel.source);
        let valid_target =
            !(target == 0x80 && ((address & 0xFE_0000) == 0x7E_0000 || (address & 0x40_E000) == 0));
        if direction {
            let value = if valid_target { bus.read_b(target) } else { 0 };
            if valid_a(address) {
                bus.write_a(address, value);
            }
        } else {
            let value = if valid_a(address) {
                bus.read_a(address)
            } else {
                0
            };
            if valid_target {
                bus.write_b(target, value);
            }
        }
        if !fixed {
            channel.source = if reverse {
                channel.source.wrapping_sub(1)
            } else {
                channel.source.wrapping_add(1)
            };
        }
        self.transferred += 1;
        if self.transferred >= self.count {
            self.mask &= !(1 << self.channel);
            self.setup = true;
        }
        8
    }
}

/// Run the general-purpose DMA channels selected by `mask` to completion.
///
/// Returns the number of master clocks the CPU was stalled.
pub fn run(channels: &[DmaChannel; 8], mask: u8, bus: &mut impl DmaBus) -> u32 {
    let mut transfer = DmaTransfer::new(*channels, mask);
    let mut cycles = 0;
    while transfer.busy() {
        cycles += transfer.step(bus);
    }
    cycles
}

fn valid_a(address: u32) -> bool {
    if (address & 0x40_FF00) == 0x2100 {
        return false;
    }
    if (address & 0x40_FE00) == 0x4000 {
        return false;
    }
    if (address & 0x40_FFE0) == 0x4200 {
        return false;
    }
    if (address & 0x40_FF80) == 0x4300 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bus {
        ram: Vec<u8>,
        writes: Vec<(u8, u8)>,
    }

    impl DmaBus for Bus {
        fn read_a(&mut self, address: u32) -> u8 {
            self.ram[(address & 0xFFFF) as usize]
        }

        fn write_a(&mut self, address: u32, value: u8) {
            self.ram[(address & 0xFFFF) as usize] = value;
        }

        fn read_b(&mut self, _address: u8) -> u8 {
            0
        }

        fn write_b(&mut self, address: u8, value: u8) {
            self.writes.push((address, value));
        }
    }

    #[test]
    fn hdma_transfers_table_bytes() {
        let mut dma = Dma::new();
        dma.write(0x4300, 0x00); // direct, one byte
        dma.write(0x4301, 0x00); // B-bus target
        dma.write(0x4304, 0x7E); // table bank
        dma.write(0x4302, 0x00); // A1T low
        dma.write(0x4303, 0x10); // A1T high: table at $7E:1000
        dma.set_hdma_enable(0x01);

        let mut bus = Bus {
            ram: vec![0; 0x10000],
            writes: Vec::new(),
        };
        bus.ram[0x1000] = 0x82; // transfer on lines 1 and 2
        bus.ram[0x1001] = 0x11;
        bus.ram[0x1002] = 0x22;
        bus.ram[0x1003] = 0x00; // end

        dma.hdma_setup(&mut bus);
        dma.hdma_line(&mut bus);
        dma.hdma_line(&mut bus);
        dma.hdma_line(&mut bus);
        assert_eq!(bus.writes, vec![(0x00, 0x11), (0x00, 0x22)]);
    }
}
