//! SNES system: S-CPU bus, WRAM, cartridge access and the peripheral clock.
//!
//! The S-CPU, S-SMP/S-DSP, S-PPU and DMA/HDMA engines are clocked here. Every
//! S-CPU bus access advances the peripherals by its wait-state count, so the
//! S-SMP and S-PPU observe the same master-clock instants as ares' cooperative
//! threads. Coprocessors (SuperFX, SA-1, CX4, S-DD1, SPC7110, DSP1-4) are not
//! implemented yet.

use crate::apu::Apu;
use crate::cpu::{Cpu, CpuBus};
use crate::dma::{Dma, DmaBus};
use crate::io::CpuIo;
use crate::ppu::Ppu;
use crate::rom::Cartridge;

/// Size of the SNES work RAM.
pub const WRAM_SIZE: usize = 0x2_0000;

/// Master clocks the 5A22 spends refreshing WRAM on every scanline
/// (ares `CPU::step`: five stalls of 6 + 2 clocks).
const DRAM_REFRESH_CLOCKS: u32 = 40;
/// Dot (ares hcounter 536 / 4) at which that refresh completes.
const DRAM_REFRESH_DOT: u16 = 134;

/// A SNES system with a CPU and cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snes {
    /// The S-CPU.
    pub cpu: Cpu,
    /// The inserted cartridge.
    pub cart: Cartridge,
    /// The S-SMP/S-DSP audio unit.
    pub apu: Apu,
    /// The S-PPU pair.
    pub ppu: Ppu,
    /// General-purpose DMA channels.
    pub dma: Dma,
    /// Controller ports, auto-poll and the multiply/divide unit.
    pub io: CpuIo,
    wram: Vec<u8>,
    apu_credit: i64,
    ppu_credit: i64,
    nmi_enable: bool,
    nmi_flag: bool,
    fast_rom: bool,
    /// `$2180-$2183` WRAM data-port address (17 bits).
    wram_port: u32,
    /// Master clocks the CPU still owes the peripherals (HDMA bytes already
    /// transferred and the current scanline's DRAM refresh).
    cpu_stall: u32,
    /// Whether the current scanline's DRAM refresh has been applied.
    refresh_done: bool,
    /// Set when this peripheral advance crossed the refresh position; the
    /// wrapper then runs the 40 refresh clocks and reports them to the CPU.
    refresh_pending: bool,
    /// Whether this frame's joypad auto-poll has started.
    auto_poll_started: bool,
    /// IRQ line state latched by the H/V comparator, applied at the next
    /// instruction boundary.
    irq_line: bool,
    /// NMI edge seen while the CPU was mid-instruction, applied at the next
    /// instruction boundary.
    nmi_line: bool,
    /// Number of NMI edges delivered (debug).
    pub nmi_count: u64,
    /// Master clock elapsed since reset (debug).
    pub clock: u64,
}

impl Snes {
    /// Create a system around a parsed cartridge.
    pub fn new(cart: Cartridge) -> Self {
        Self {
            cpu: Cpu::new(),
            cart,
            apu: Apu::new(),
            ppu: Ppu::new(),
            dma: Dma::new(),
            io: CpuIo::new(),
            wram: vec![0; WRAM_SIZE],
            apu_credit: 0,
            ppu_credit: 0,
            nmi_enable: false,
            nmi_flag: false,
            fast_rom: false,
            wram_port: 0,
            cpu_stall: 0,
            refresh_done: false,
            refresh_pending: false,
            auto_poll_started: false,
            irq_line: false,
            nmi_line: false,
            nmi_count: 0,
            clock: 0,
        }
    }

    /// Work RAM.
    pub fn wram(&self) -> &[u8] {
        &self.wram
    }

    /// Mutable work RAM, for save states and tests.
    pub fn wram_mut(&mut self) -> &mut [u8] {
        &mut self.wram
    }

    /// Set the buttons held on a controller port (`0` or `1`).
    ///
    /// Bits use [`crate::io::button`], which matches the `$4218-$421F` layout.
    pub fn set_joypad(&mut self, port: usize, state: u16) {
        self.io.set_joypad(port, state);
    }

    /// Buttons held on a controller port (`0` or `1`).
    pub fn joypad(&self, port: usize) -> u16 {
        self.io.joypad(port)
    }

    /// Whether NMI is enabled by the last `$4200` write (debug).
    pub const fn nmi_enabled(&self) -> bool {
        self.nmi_enable
    }

    /// Install a user-supplied 64-byte SPC700 IPL ROM.
    pub fn set_ipl(&mut self, ipl: [u8; crate::apu::IPL_SIZE]) {
        self.apu.set_ipl(ipl);
    }

    /// Reset the CPU and fetch the reset vector through the system bus.
    pub fn reset(&mut self) -> u32 {
        self.apu.reset();
        self.ppu.reset();
        self.dma.hdma_reset();
        self.io.reset();
        self.apu_credit = 0;
        self.ppu_credit = 0;
        self.nmi_enable = false;
        self.nmi_flag = false;
        self.fast_rom = false;
        self.wram_port = 0;
        self.irq_line = false;
        self.nmi_line = false;
        self.nmi_count = 0;
        self.auto_poll_started = false;
        self.clock = 0;
        self.apu.debug.set_clock(0);
        let mut bus = SnesBus {
            wram: &mut self.wram,
            cart: &mut self.cart,
            apu: &mut self.apu,
            ppu: &mut self.ppu,
            dma: &mut self.dma,
            io: &mut self.io,
            nmi_enable: &mut self.nmi_enable,
            nmi_flag: &mut self.nmi_flag,
            fast_rom: &mut self.fast_rom,
            wram_port: &mut self.wram_port,
            vblank: false,
            hblank: false,
        };
        self.cpu.reset(&mut bus)
    }

    /// Execute one CPU instruction or interrupt.
    ///
    /// The instruction runs against [`Snes`] itself, so every bus access can
    /// advance the S-SMP, S-PPU and DMA engines to the exact master clock of
    /// that access (ares' cooperative-thread ordering). The CPU is swapped out
    /// for the duration of the call so the system bus can borrow the rest of
    /// the machine.
    pub fn step(&mut self) -> u32 {
        let mut cpu = std::mem::take(&mut self.cpu);
        let cpu_cycles = cpu.step(self);
        self.cpu = cpu;
        self.apply_interrupts();
        let mut cycles = cpu_cycles;
        let mut dma_clocks = 0u32;
        while let Some(mut transfer) = self.dma.begin_transfer() {
            while transfer.busy() {
                let clocks = {
                    let vblank = self.ppu.in_vblank();
                    let hblank = self.ppu.in_hblank();
                    let mut bus = SnesBus {
                        wram: &mut self.wram,
                        cart: &mut self.cart,
                        apu: &mut self.apu,
                        ppu: &mut self.ppu,
                        dma: &mut self.dma,
                        io: &mut self.io,
                        nmi_enable: &mut self.nmi_enable,
                        nmi_flag: &mut self.nmi_flag,
                        fast_rom: &mut self.fast_rom,
                        wram_port: &mut self.wram_port,
                        vblank,
                        hblank,
                    };
                    transfer.step(&mut bus)
                };
                let extra = self.advance_peripherals(clocks);
                cycles += clocks + extra;
                // DMA time is master time: keep the CPU's cumulative clock
                // equal to the system clock, otherwise any timing analysis
                // based on `cpu.cycles` is short by the DMA/stall time.
                dma_clocks += clocks + extra;
            }
        }
        self.cpu.cycles += u64::from(dma_clocks);
        self.apply_interrupts();
        cycles
    }

    /// Feed the latched IRQ/NMI edges to the CPU at an instruction boundary.
    fn apply_interrupts(&mut self) {
        self.cpu.set_irq(self.irq_line);
        if self.nmi_line {
            self.nmi_line = false;
            self.cpu.trigger_nmi();
        }
    }

    /// Advance the APU, PPU, HDMA and IRQ comparators by `clocks` master
    /// clocks. NMI/IRQ edges are latched for the next instruction boundary.
    ///
    /// Returns the extra master clocks the CPU must account for, which is
    /// `DRAM_REFRESH_CLOCKS` whenever this step crossed the scanline's refresh
    /// point (ares `CPU::step` runs the five 6+2-clock stalls at the end of the
    /// step call that reaches `dramRefreshPosition`).
    fn advance_peripherals(&mut self, clocks: u32) -> u32 {
        self.advance_peripherals_inner(clocks);
        if self.refresh_pending {
            self.refresh_pending = false;
            self.advance_peripherals_inner(DRAM_REFRESH_CLOCKS);
            DRAM_REFRESH_CLOCKS
        } else {
            0
        }
    }

    fn advance_peripherals_inner(&mut self, clocks: u32) {
        // NTSC master clock is 21.477272 MHz; the S-SMP runs at 2,050,560 Hz,
        // so one SMP clock is 21,477,272 / 2,050,560 master clocks. The credit
        // is carried across instructions so long-term APU timing stays in step.
        // Draining *one SMP clock* at a time (rather than one two-clock access
        // unit) is what makes an S-SMP port write become visible at the exact
        // master clock it lands on: ares/bsnes synchronize the S-SMP on every
        // S-CPU step, so an S-CPU poll must observe every SMP write up to that
        // instant and no further.
        const APU_UNIT_NUM: i64 = 2_050_560;
        const APU_UNIT_DEN: i64 = 21_477_272;
        if self.apu.port_logging_enabled() {
            // Resolve every access unit to the master clock it starts on, so the
            // port handshake log can be compared event-for-event with a
            // reference. The tick order is unchanged: the extra arithmetic only
            // moves the recorded timestamp.
            let mut credit = self.apu_credit;
            let mut spent = 0u32;
            loop {
                // Clocks from `credit` to the next unit boundary (both positive).
                let need = u32::try_from((APU_UNIT_DEN - credit + APU_UNIT_NUM - 1) / APU_UNIT_NUM)
                    .unwrap_or(0);
                if spent + need > clocks {
                    break;
                }
                spent += need;
                credit += i64::from(need) * APU_UNIT_NUM - APU_UNIT_DEN;
                self.apu.debug.set_clock(self.clock + u64::from(spent));
                self.apu.tick();
            }
            credit += i64::from(clocks - spent) * APU_UNIT_NUM;
            self.apu_credit = credit;
        } else {
            self.apu_credit += i64::from(clocks) * APU_UNIT_NUM;
            while self.apu_credit >= APU_UNIT_DEN {
                self.apu.tick();
                self.apu_credit -= APU_UNIT_DEN;
            }
        }
        self.clock += u64::from(clocks);
        // The PPU dot clock is the master clock divided by four on NTSC.
        self.ppu_credit += i64::from(clocks);
        while self.ppu_credit >= 4 {
            let pre_dot = self.ppu.dot();
            self.ppu.tick();
            self.ppu_credit -= 4;
            // ares `CPU::step`: "DRAM refresh occurs once every scanline" as five
            // 6+2 master-clock stalls at hcounter 536 (dot 134). The CPU loses
            // those 40 clocks; the rest of the machine keeps running.
            if pre_dot == DRAM_REFRESH_DOT - 1 && !self.refresh_done {
                // ares `CPU::step` applies the five 6+2-clock refresh stalls at
                // the end of the step that reaches the refresh position, so the
                // caller adds them to the current access.
                self.refresh_done = true;
                self.refresh_pending = true;
            }
            self.irq_line = self.io.poll_irq(self.ppu.line(), self.ppu.dot());
            if pre_dot == 32 && self.ppu.line() == 225 && !self.auto_poll_started {
                // ares `CPU::joypadEdge` starts the sequence on line 225 at
                // hcounter 130-384, not at the start of the line.
                self.auto_poll_started = true;
                self.io.begin_auto_poll();
            }
            if let Some(line) = self.ppu.take_line_event() {
                self.refresh_done = false;
                if line == 0 {
                    self.dma.hdma_reset();
                    self.auto_poll_started = false;
                }
                let mut bus = HdmaBus {
                    wram: &mut self.wram,
                    cart: &mut self.cart,
                    apu: &mut self.apu,
                    ppu: &mut self.ppu,
                };
                let bytes = self.dma.hdma_line(&mut bus);
                // ares: an HDMA byte costs the CPU eight master clocks.
                self.cpu_stall += bytes * 8;
                self.ppu.render_current_line();
            }
        }
        self.io.step(clocks);
        self.irq_line = self.io.poll_irq(self.ppu.line(), self.ppu.dot());
        // The $4210 flag latches at vblank regardless of $4200 bit 7; only the
        // CPU interrupt output is gated by NMI enable.
        if self.ppu.take_nmi() {
            self.nmi_flag = true;
            if self.nmi_enable {
                self.nmi_line = true;
                self.nmi_count += 1;
            }
        }
    }

    /// Run until `max_cycles` master clocks have elapsed, the CPU stops, or an
    /// instruction limit is reached.
    pub fn run_cycles(&mut self, max_cycles: u64, max_instructions: u32) -> (u64, u32) {
        let mut cycles = 0;
        let mut instructions = 0;
        while cycles < max_cycles && instructions < max_instructions && !self.cpu.stp {
            cycles += u64::from(self.step());
            instructions += 1;
        }
        (cycles, instructions)
    }
}

impl Snes {
    /// Build a bus view over the machine's fields for one access.
    fn bus(&mut self) -> SnesBus<'_> {
        let vblank = self.ppu.in_vblank();
        let hblank = self.ppu.in_hblank();
        SnesBus {
            wram: &mut self.wram,
            cart: &mut self.cart,
            apu: &mut self.apu,
            ppu: &mut self.ppu,
            dma: &mut self.dma,
            io: &mut self.io,
            nmi_enable: &mut self.nmi_enable,
            nmi_flag: &mut self.nmi_flag,
            fast_rom: &mut self.fast_rom,
            wram_port: &mut self.wram_port,
            vblank,
            hblank,
        }
    }
}

impl CpuBus for Snes {
    fn read(&mut self, address: u32, mdr: u8) -> u8 {
        let value = self.bus().read(address, mdr);
        if address & 0x40_FF00 == 0x2100 && (0x2140..=0x2143).contains(&(address & 0xFFFF)) {
            let port = ((address & 0xFFFF) - 0x2140) as usize;
            self.apu.debug.record_cpu_read(self.clock, port, value);
        }
        value
    }

    fn write(&mut self, address: u32, value: u8) {
        self.bus().write(address, value);
        if address & 0x40_FF00 == 0x2100 && (0x2140..=0x2143).contains(&(address & 0xFFFF)) {
            let port = ((address & 0xFFFF) - 0x2140) as usize;
            self.apu.debug.record_cpu_write(self.clock, port, value);
        }
    }

    fn access_cycles(&mut self, address: u32) -> u32 {
        let base = self.bus().access_cycles(address);
        // HDMA bytes transferred inside hblank and the scanline's DRAM refresh
        // both steal master clocks from the CPU (ares); the debt is paid on the
        // next access, which also advances the peripherals by the same amount.
        base + std::mem::take(&mut self.cpu_stall)
    }

    fn idle(&mut self) {
        self.bus().idle();
    }

    fn advance(&mut self, clocks: u32) -> u32 {
        self.advance_peripherals(clocks)
    }
}

struct SnesBus<'a> {
    wram: &'a mut [u8],
    cart: &'a mut Cartridge,
    apu: &'a mut Apu,
    ppu: &'a mut Ppu,
    dma: &'a mut Dma,
    io: &'a mut CpuIo,
    nmi_enable: &'a mut bool,
    nmi_flag: &'a mut bool,
    fast_rom: &'a mut bool,
    wram_port: &'a mut u32,
    vblank: bool,
    hblank: bool,
}

impl SnesBus<'_> {
    fn read_wram_port(&mut self, offset: usize) -> u8 {
        match offset {
            0x2180 => {
                let address = 0x7E_0000 | *self.wram_port;
                *self.wram_port = (*self.wram_port + 1) & 0x1_FFFF;
                let index = ((address & 0x1_0000) | (address & 0xFFFF)) as usize & 0x1FFFF;
                self.wram[index]
            }
            0x2181..=0x2183 => 0,
            _ => 0,
        }
    }

    fn write_wram_port(&mut self, offset: usize, value: u8) {
        match offset {
            0x2180 => {
                let address = 0x7E_0000 | *self.wram_port;
                let index = ((address & 0x1_0000) | (address & 0xFFFF)) as usize & 0x1FFFF;
                self.wram[index] = value;
                *self.wram_port = (*self.wram_port + 1) & 0x1_FFFF;
            }
            0x2181 => *self.wram_port = (*self.wram_port & 0x1_FF00) | u32::from(value),
            0x2182 => *self.wram_port = (*self.wram_port & 0x1_00FF) | (u32::from(value) << 8),
            0x2183 => {
                *self.wram_port = (*self.wram_port & 0x0_FFFF) | (u32::from(value & 0x01) << 16)
            }
            _ => {}
        }
    }
}

impl CpuBus for SnesBus<'_> {
    fn read(&mut self, address: u32, mdr: u8) -> u8 {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;

        if bank == 0x7E || bank == 0x7F {
            return self.wram[((bank as usize & 1) << 16) | offset];
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                return self.wram[offset & 0x1FFF];
            }
            if offset < 0x6000 {
                if (0x2100..=0x213F).contains(&offset) {
                    return self.ppu.read(offset as u16);
                }
                if (0x2140..=0x2143).contains(&offset) {
                    return self.apu.cpu_read(offset - 0x2140);
                }
                if (0x2180..=0x2183).contains(&offset) {
                    return self.read_wram_port(offset);
                }
                if offset == 0x4210 {
                    // ares `CPU::readCPU`: `data.bit(0,3) = io.version` and
                    // `data.bit(7) = rdnmi()`, leaving bits 4-6 from the open
                    // bus (the MDR).
                    let nmi = if *self.nmi_flag { 0x80 } else { 0 };
                    *self.nmi_flag = false;
                    return (mdr & 0x70) | 0x02 | nmi;
                }
                if offset == 0x4211 {
                    // ares sets only bit 7; bits 0-6 are open bus.
                    let value = self.io.read(0x4211).unwrap_or(0);
                    return (mdr & 0x7F) | (value & 0x80);
                }
                if offset == 0x4212 {
                    // ares sets bits 0, 6 and 7; bits 1-5 are open bus.
                    let busy = u8::from(self.io.auto_poll_busy());
                    return (mdr & 0x3E)
                        | (u8::from(self.vblank) << 7)
                        | (u8::from(self.hblank) << 6)
                        | busy;
                }
                if offset == 0x4016 || offset == 0x4017 {
                    return self.io.read(offset as u16).unwrap_or(0);
                }
                if (0x4211..=0x4217).contains(&offset) {
                    return self.io.read(offset as u16).unwrap_or(0);
                }
                if (0x4218..=0x421F).contains(&offset) {
                    return self.io.read(offset as u16).unwrap_or(0);
                }
                if (0x4300..=0x437F).contains(&offset) {
                    return self.dma.read(offset as u16);
                }
                return mdr;
            }
            if offset < 0x8000 {
                return mdr;
            }
        }
        self.cart.read(address).unwrap_or(0)
    }

    fn write(&mut self, address: u32, value: u8) {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;

        if bank == 0x7E || bank == 0x7F {
            self.wram[((bank as usize & 1) << 16) | offset] = value;
            return;
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                self.wram[offset & 0x1FFF] = value;
                return;
            }
            if offset < 0x6000 {
                if (0x2100..=0x213F).contains(&offset) {
                    self.ppu.write(offset as u16, value);
                } else if (0x2140..=0x2143).contains(&offset) {
                    self.apu.cpu_write(offset - 0x2140, value);
                } else if (0x2180..=0x2183).contains(&offset) {
                    self.write_wram_port(offset, value);
                } else if offset == 0x4200 {
                    *self.nmi_enable = value & 0x80 != 0;
                    self.io.write(0x4200, value);
                } else if (0x4202..=0x4206).contains(&offset) || (0x4207..=0x420A).contains(&offset)
                {
                    self.io.write(offset as u16, value);
                } else if offset == 0x420B {
                    self.dma.start(value);
                } else if offset == 0x420C {
                    self.dma.set_hdma_enable(value);
                } else if offset == 0x420D {
                    *self.fast_rom = value & 0x01 != 0;
                } else if (0x4300..=0x437F).contains(&offset) {
                    self.dma.write(offset as u16, value);
                }
                if offset == 0x4016 {
                    self.io.write(0x4016, value);
                }
                return;
            }
            if offset < 0x8000 {
                return;
            }
        }
        self.cart.write(address, value);
    }

    fn access_cycles(&mut self, address: u32) -> u32 {
        // Mirrors ares CPU::wait(): six master clocks per base cycle, with
        // wait states for slow ROM, WRAM and the internal I/O window.
        if address & 0x40_8000 != 0 {
            return if address & 0x80_0000 != 0 && *self.fast_rom {
                6
            } else {
                8
            };
        }
        if address.wrapping_add(0x6000) & 0x4000 != 0 {
            return 8;
        }
        if address.wrapping_sub(0x4000) & 0x7E00 != 0 {
            return 6;
        }
        12
    }
}

struct HdmaBus<'a> {
    wram: &'a mut [u8],
    cart: &'a mut Cartridge,
    apu: &'a mut Apu,
    ppu: &'a mut Ppu,
}

impl HdmaBus<'_> {
    fn read(&mut self, address: u32) -> u8 {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        if bank == 0x7E || bank == 0x7F {
            return self.wram[((bank as usize & 1) << 16) | offset];
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                return self.wram[offset & 0x1FFF];
            }
            if offset < 0x6000 {
                if (0x2100..=0x213F).contains(&offset) {
                    return self.ppu.read(offset as u16);
                }
                if (0x2140..=0x2143).contains(&offset) {
                    return self.apu.cpu_read(offset - 0x2140);
                }
                return 0;
            }
        }
        self.cart.read(address).unwrap_or(0)
    }

    fn write(&mut self, address: u32, value: u8) {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        if bank == 0x7E || bank == 0x7F {
            self.wram[((bank as usize & 1) << 16) | offset] = value;
            return;
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                self.wram[offset & 0x1FFF] = value;
                return;
            }
            if offset < 0x6000 {
                if (0x2100..=0x213F).contains(&offset) {
                    self.ppu.write(offset as u16, value);
                } else if (0x2140..=0x2143).contains(&offset) {
                    self.apu.cpu_write(offset - 0x2140, value);
                }
                return;
            }
        }
        self.cart.write(address, value);
    }
}

impl DmaBus for HdmaBus<'_> {
    fn read_a(&mut self, address: u32) -> u8 {
        if !valid_dma_a(address) {
            return 0;
        }
        self.read(address)
    }

    fn write_a(&mut self, address: u32, value: u8) {
        if valid_dma_a(address) {
            self.write(address, value);
        }
    }

    fn read_b(&mut self, address: u8) -> u8 {
        self.read(0x2100 | u32::from(address))
    }

    fn write_b(&mut self, address: u8, value: u8) {
        self.write(0x2100 | u32::from(address), value);
    }
}

impl DmaBus for SnesBus<'_> {
    fn read_a(&mut self, address: u32) -> u8 {
        if !valid_dma_a(address) {
            return 0;
        }
        // ares' DMA reads pass the CPU's MDR; we do not model that here, so an
        // unmapped DMA source reads 0.
        self.read(address, 0)
    }

    fn write_a(&mut self, address: u32, value: u8) {
        if valid_dma_a(address) {
            self.write(address, value);
        }
    }

    fn read_b(&mut self, address: u8) -> u8 {
        self.read(0x2100 | u32::from(address), 0)
    }

    fn write_b(&mut self, address: u8, value: u8) {
        self.write(0x2100 | u32::from(address), value);
    }
}

fn valid_dma_a(address: u32) -> bool {
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
    use crate::rom::{Mapping, RomHeader};

    fn build_lorom(code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x1_0000];
        let header = Mapping::LoRom.header_offset();
        rom[header..header + 8].copy_from_slice(b"TEST ROM");
        rom[header + 0x15] = 0x20;
        rom[header + 0x16] = 0x02;
        rom[header + 0x17] = 8;
        rom[header + 0x18] = 3;
        rom[header + 0x19] = 1;
        rom[header + 0x1A] = 1;
        rom[header + 0x3C..header + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());
        rom[0..code.len()].copy_from_slice(code);

        let mut checksum_source = rom.clone();
        checksum_source[header + 0x1C..header + 0x20].fill(0);
        let checksum = (checksum_source
            .iter()
            .map(|&byte| u32::from(byte))
            .sum::<u32>()
            & 0xFFFF) as u16;
        rom[header + 0x1C..header + 0x1E].copy_from_slice(&(!checksum).to_le_bytes());
        rom[header + 0x1E..header + 0x20].copy_from_slice(&checksum.to_le_bytes());
        rom
    }

    fn cartridge(code: &[u8]) -> Cartridge {
        Cartridge::from_bytes(&build_lorom(code)).expect("valid LoROM")
    }

    #[test]
    fn decodes_header_and_maps_wram() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        assert_eq!(system.cart.header().mapping, Mapping::LoRom);
        assert_eq!(system.cart.header().title, "TEST ROM");
        assert_eq!(system.cart.header().reset_vector, 0x8000);
        system.wram_mut()[0x1234] = 0x5A;
        assert_eq!(system.wram()[0x1234], 0x5A);

        let mut bus = SnesBus {
            wram: &mut system.wram,
            cart: &mut system.cart,
            apu: &mut system.apu,
            ppu: &mut system.ppu,
            dma: &mut system.dma,
            io: &mut system.io,
            nmi_enable: &mut system.nmi_enable,
            nmi_flag: &mut system.nmi_flag,
            fast_rom: &mut system.fast_rom,
            wram_port: &mut system.wram_port,
            vblank: false,
            hblank: false,
        };
        assert_eq!(bus.read(0x001234, 0), 0x5A);
        assert_eq!(bus.read(0x801234, 0), 0x5A);
        assert_eq!(bus.read(0x7E1234, 0), 0x5A);
        assert_eq!(bus.read(0x7F1234, 0), 0x00);
    }

    #[test]
    fn unmapped_reads_return_the_open_bus_mdr() {
        // ares maps unmapped addresses to `[](n24, n8 data) { return data; }`,
        // where `data` is the CPU's memory data register.
        let mut system = Snes::new(cartridge(&[0xDB]));
        let mut bus = SnesBus {
            wram: &mut system.wram,
            cart: &mut system.cart,
            apu: &mut system.apu,
            ppu: &mut system.ppu,
            dma: &mut system.dma,
            io: &mut system.io,
            nmi_enable: &mut system.nmi_enable,
            nmi_flag: &mut system.nmi_flag,
            fast_rom: &mut system.fast_rom,
            wram_port: &mut system.wram_port,
            vblank: false,
            hblank: false,
        };
        // $00:4000-$43FF is unmapped for reads ($4200 is write-only), as is
        // $00:5000 and the unused part of the I/O page.
        assert_eq!(bus.read(0x004000, 0x5A), 0x5A);
        assert_eq!(bus.read(0x004200, 0xA5), 0xA5);
        assert_eq!(bus.read(0x005000, 0x3C), 0x3C);
        // Mapped reads still return their own value and do not leak the MDR.
        assert_eq!(bus.read(0x001234, 0x5A), 0x00);
    }

    #[test]
    fn io_status_reads_keep_open_bus_bits() {
        // ares `CPU::readCPU` writes only the defined bits of $4210/$4211/$4212
        // and leaves the rest from the MDR (open bus).
        let mut system = Snes::new(cartridge(&[0xDB]));
        let mut bus = SnesBus {
            wram: &mut system.wram,
            cart: &mut system.cart,
            apu: &mut system.apu,
            ppu: &mut system.ppu,
            dma: &mut system.dma,
            io: &mut system.io,
            nmi_enable: &mut system.nmi_enable,
            nmi_flag: &mut system.nmi_flag,
            fast_rom: &mut system.fast_rom,
            wram_port: &mut system.wram_port,
            vblank: false,
            hblank: false,
        };
        // $4210: bits 4-6 open bus, bits 0-3 = version 2, bit 7 = NMI flag.
        assert_eq!(bus.read(0x004210, 0x70), 0x72);
        // $4211: bits 0-6 open bus, bit 7 = TIMEUP.
        assert_eq!(bus.read(0x004211, 0x7F), 0x7F);
        // $4212: bits 1-5 open bus; vblank/hblank/busy are all false here.
        assert_eq!(bus.read(0x004212, 0x3E), 0x3E);
    }

    #[test]
    fn reset_vector_and_execution_from_rom() {
        // LDA #$42; STP
        let mut system = Snes::new(cartridge(&[0xA9, 0x42, 0xDB]));
        system.reset();
        assert_eq!(system.cpu.pc, 0x8000);
        assert!(system.cpu.emulation);
        let (cycles, instructions) = system.run_cycles(100, 100);
        assert!(system.cpu.stp, "CPU did not reach STP");
        assert_eq!(system.cpu.a, 0x42);
        assert_eq!(instructions, 2);
        assert!(cycles >= 5);
    }

    #[test]
    fn maps_cartridge_reads_and_sram() {
        let mut system = Snes::new(cartridge(&[0x11, 0x22, 0x33]));
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            assert_eq!(bus.read(0x008000, 0), 0x11);
            assert_eq!(bus.read(0x018000, 0), 0x00);
            assert_eq!(bus.read(0x808000, 0), 0x11);
            assert!(bus.cart.write(0x700000, 0x5A));
            assert_eq!(bus.read(0x700000, 0), 0x5A);
        }
    }

    #[test]
    fn cpu_writes_reach_the_apu_ports() {
        // LDA #$5A; STA $2140; STP
        let mut system = Snes::new(cartridge(&[0xA9, 0x5A, 0x8D, 0x40, 0x21, 0xDB]));
        system.reset();
        system.run_cycles(100, 100);
        assert!(system.cpu.stp);
        assert_eq!(system.apu.apu_port(0), 0x5A);
    }

    #[test]
    fn cpu_reads_apu_output_ports() {
        // LDA $2141; STP
        let mut system = Snes::new(cartridge(&[0xAD, 0x41, 0x21, 0xDB]));
        system.reset();
        system.apu.set_cpu_port(1, 0x34);
        system.run_cycles(100, 100);
        assert!(system.cpu.stp);
        assert_eq!(system.cpu.a, 0x34);
    }

    #[test]
    fn cpu_writes_reach_ppu_registers() {
        // LDA #$0F; STA $2100; STP
        let mut system = Snes::new(cartridge(&[0xA9, 0x0F, 0x8D, 0x00, 0x21, 0xDB]));
        system.reset();
        system.run_cycles(100, 100);
        assert!(system.cpu.stp);
        assert_eq!(system.ppu.display_brightness(), 0x0F);
    }

    #[test]
    fn ppu_nmi_reaches_the_cpu() {
        let mut system = Snes::new(cartridge(&[0xEA; 64]));
        system.reset();
        // 225 scanlines * 341 dots / 1.5 dots per CPU cycle.
        let mut cycles = 0;
        while !system.ppu.in_vblank() && cycles < 1_000_000 {
            cycles += system.step();
        }
        assert!(system.ppu.in_vblank());
        // The NMI is latched and serviced on the next CPU step.
        system.step();
        assert!(system.cpu.pc != 0x8000);
    }

    #[test]
    fn cpu_bus_exposes_joypads_and_math() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        system.reset();
        system.set_joypad(0, crate::io::button::B | crate::io::button::A);
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            bus.write(0x4016, 0x01);
            bus.write(0x4016, 0x00);
            // B is the first bit shifted out.
            assert_eq!(bus.read(0x4016, 0) & 1, 1);
            bus.write(0x4202, 0x03);
            bus.write(0x4203, 0x05);
        }
        for _ in 0..8 {
            system.io.step(6);
        }
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            assert_eq!(bus.read(0x4216, 0), 0x0F);
            assert_eq!(bus.read(0x4217, 0), 0x00);
            assert_eq!(bus.read(0x4210, 0) & 0x0F, 0x02);
        }
    }

    #[test]
    fn wram_port_reads_and_writes() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        system.reset();
        let (first, second) = {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            bus.write(0x2181, 0x34);
            bus.write(0x2182, 0x12);
            bus.write(0x2183, 0x00);
            bus.write(0x2180, 0x5A);
            bus.write(0x2180, 0xA5);
            // Rewind and read both bytes back through the port.
            bus.write(0x2181, 0x34);
            bus.write(0x2182, 0x12);
            (bus.read(0x2180, 0), bus.read(0x2180, 0))
        };
        assert_eq!(system.wram[0x1234], 0x5A);
        assert_eq!(system.wram[0x1235], 0xA5);
        assert_eq!((first, second), (0x5A, 0xA5));
    }

    #[test]
    fn rdnmi_flag_sets_without_nmi_enable() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        system.reset();
        let mut guard = 0;
        while !system.ppu.in_vblank() && guard < 2_000_000 {
            system.step();
            guard += 1;
        }
        let vblank = system.ppu.in_vblank();
        let hblank = system.ppu.in_hblank();
        let (first, second) = {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank,
                hblank,
            };
            (bus.read(0x4210, 0), bus.read(0x4210, 0))
        };
        assert!(!system.nmi_enabled());
        assert_eq!(first & 0x80, 0x80);
        assert_eq!(second & 0x80, 0);
    }

    #[test]
    fn cpu_wait_states_match_the_hardware_table() {
        fn bus_read_cycles(code: &[u8], bank: u8, fast: bool) -> u32 {
            let mut system = Snes::new(cartridge(code));
            system.reset();
            system.fast_rom = fast;
            system.cpu.pbr = bank;
            system.cpu.pc = 0x8000;
            system.step()
        }

        // Slow ROM: two eight-clock accesses (opcode + operand).
        assert_eq!(bus_read_cycles(&[0xA9, 0x42], 0x00, false), 16);
        // Fast ROM needs both MEMSEL and a bank $80-$FF address.
        assert_eq!(bus_read_cycles(&[0xA9, 0x42], 0x80, true), 12);
        assert_eq!(bus_read_cycles(&[0xA9, 0x42], 0x80, false), 16);
        // Absolute WRAM read: three fetches plus one eight-clock data access.
        assert_eq!(bus_read_cycles(&[0xAD, 0x10, 0x00], 0x00, false), 32);
        // PPU register read: fetches are slow ROM, the $2100 data access is six.
        assert_eq!(bus_read_cycles(&[0xAD, 0x00, 0x21], 0x00, false), 30);
        // $4000-$41FF is the twelve-clock internal window.
        assert_eq!(bus_read_cycles(&[0xAD, 0x00, 0x40], 0x00, false), 36);
    }

    #[test]
    fn v_irq_reaches_the_cpu() {
        // CLI; NOP...  with the IRQ vector pointing back at $8000.
        let mut system = Snes::new(cartridge(&[0x58, 0xEA, 0xEA, 0xEA, 0xEA]));
        system.reset();
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            bus.write(0x4209, 10);
            bus.write(0x4200, 0x20);
        }
        // The handler starts with `CLI`, so I must be set again by the time the
        // IRQ at line 10 has been serviced.
        let mut guard = 0;
        while system.ppu.line() < 20 && guard < 2_000_000 {
            system.step();
            guard += 1;
        }
        assert!(system.cpu.flags.irq_disable, "V-IRQ was not serviced");
        assert!(system.ppu.line() >= 20);
    }

    #[test]
    fn dma_writes_vram() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        system.reset();
        system.wram_mut()[0x0100] = 0x12;
        system.wram_mut()[0x0101] = 0x34;
        system.wram_mut()[0x0102] = 0x56;
        system.wram_mut()[0x0103] = 0x78;
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            bus.write(0x2115, 0x80);
            bus.write(0x2116, 0x00);
            bus.write(0x2117, 0x00);
            bus.write(0x4300, 0x01);
            bus.write(0x4301, 0x18);
            bus.write(0x4302, 0x00);
            bus.write(0x4303, 0x01);
            bus.write(0x4304, 0x7E);
            bus.write(0x4305, 0x04);
            bus.write(0x4306, 0x00);
            bus.write(0x420B, 0x01);
        }
        let (channels, mask) = system.dma.take_pending().expect("DMA pending");
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            crate::dma::run(&channels, mask, &mut bus);
        }
        assert_eq!(system.ppu.vram_word(0), 0x3412);
        assert_eq!(system.ppu.vram_word(1), 0x7856);
    }

    #[test]
    fn dram_refresh_stalls_the_cpu_once_per_scanline() {
        // ares `CPU::step` refreshes WRAM once per scanline at the refresh
        // position as five 6+2 master-clock stalls, so the CPU loses 40 clocks
        // per line while the rest of the machine keeps running. Without it the
        // CPU executes ~2.9% more work per frame than the reference and the game
        // loop drifts (Super Mario World's title screen ran two frames ahead).
        let mut system = Snes::new(cartridge(&[0xDB]));
        // 341 dots x 4 master clocks = one NTSC scanline.
        assert_eq!(system.advance_peripherals(1364), DRAM_REFRESH_CLOCKS);
        assert_eq!(system.advance_peripherals(1364), DRAM_REFRESH_CLOCKS);
        assert_eq!(system.advance_peripherals(1364), DRAM_REFRESH_CLOCKS);
        assert_eq!(system.cpu_stall, 0);
    }

    #[test]
    fn dma_stalls_match_the_byte_rate() {
        struct CountBus {
            bytes: usize,
        }
        impl DmaBus for CountBus {
            fn read_a(&mut self, _address: u32) -> u8 {
                0x5A
            }
            fn write_a(&mut self, _address: u32, _value: u8) {}
            fn read_b(&mut self, _address: u8) -> u8 {
                0
            }
            fn write_b(&mut self, _address: u8, _value: u8) {
                self.bytes += 1;
            }
        }
        let mut dma = Dma::new();
        dma.write(0x4300, 0x00);
        dma.write(0x4301, 0x04);
        dma.write(0x4302, 0x00);
        dma.write(0x4303, 0x01);
        dma.write(0x4304, 0x7E);
        dma.write(0x4305, 0x04);
        dma.write(0x4306, 0x00);
        dma.start(0x01);
        let (channels, mask) = dma.take_pending().expect("DMA pending");
        let mut bus = CountBus { bytes: 0 };
        let cycles = crate::dma::run(&channels, mask, &mut bus);
        assert_eq!(bus.bytes, 4);
        // ares: one eight-clock cycle per trigger, one per channel setup, then
        // eight master clocks per byte.
        assert_eq!(cycles, 48);
    }

    #[test]
    fn dma_writes_cgram() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        system.reset();
        for (index, value) in [0x1F, 0x00, 0xE0, 0x03].into_iter().enumerate() {
            system.wram_mut()[0x0100 + index] = value;
        }
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            bus.write(0x2121, 0x00);
            bus.write(0x4300, 0x00);
            bus.write(0x4301, 0x22);
            bus.write(0x4302, 0x00);
            bus.write(0x4303, 0x01);
            bus.write(0x4304, 0x7E);
            bus.write(0x4305, 0x04);
            bus.write(0x4306, 0x00);
            bus.write(0x420B, 0x01);
        }
        let (channels, mask) = system.dma.take_pending().expect("DMA pending");
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: false,
                hblank: false,
            };
            crate::dma::run(&channels, mask, &mut bus);
        }
        assert_eq!(system.ppu.cgram_word(0), 0x001F);
        assert_eq!(system.ppu.cgram_word(1), 0x03E0);
    }

    #[test]
    fn header_type_is_exposed() {
        let cartridge = cartridge(&[0xDB]);
        let header: &RomHeader = cartridge.header();
        assert_eq!(header.declared_sram_size, Some(0x2000));
        assert!(header.has_battery());
    }
}
