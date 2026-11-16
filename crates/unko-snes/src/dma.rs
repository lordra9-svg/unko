//! S-CPU DMA channels.
//!
//! General-purpose DMA is implemented here; HDMA is represented by the same
//! channel registers but its per-scanline table engine is not enabled yet.

/// One DMA channel's register state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Default for DmaChannel {
    fn default() -> Self {
        Self {
            control: 0xFF,
            target: 0xFF,
            source: 0xFFFF,
            bank: 0xFF,
            size: 0xFFFF,
            indirect_bank: 0xFF,
            hdma_address: 0xFFFF,
            line_counter: 0xFF,
        }
    }
}

/// The eight DMA channels and their enable latches.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dma {
    channels: [DmaChannel; 8],
    pending: u8,
    /// Channels enabled by a write to `$420C` (HDMA).
    pub hdma_enable: u8,
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
}

/// Run the general-purpose DMA channels selected by `mask`.
///
/// Returns an approximate S-CPU stall in bus cycles.
pub fn run(channels: &[DmaChannel; 8], mask: u8, bus: &mut impl DmaBus) -> u32 {
    let mut cycles = 0;
    for (index, channel) in channels.iter().enumerate() {
        if mask & (1 << index) == 0 {
            continue;
        }
        let mut channel = *channel;
        let mode = channel.control & 0x07;
        let fixed = channel.control & 0x08 != 0;
        let reverse = channel.control & 0x10 != 0;
        let direction = channel.control & 0x80 != 0;
        let count = if channel.size == 0 {
            0x1_0000u32
        } else {
            u32::from(channel.size)
        };
        for index_in_transfer in 0..count {
            let target = channel.target.wrapping_add(match mode {
                1 | 5 => (index_in_transfer & 1) as u8,
                3 | 7 => ((index_in_transfer >> 1) & 1) as u8,
                4 => index_in_transfer as u8,
                _ => 0,
            });
            let address = (u32::from(channel.bank) << 16) | u32::from(channel.source);
            let valid_target = target != 0x80
                || ((address & 0xFE_0000) != 0x7E_0000 && (address & 0x40_E000) != 0x0000);
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
            cycles += 8;
        }
        cycles += 8;
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
