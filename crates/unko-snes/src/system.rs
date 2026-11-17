//! SNES system: S-CPU bus, WRAM, cartridge access and the peripheral clock.
//!
//! The S-CPU, S-SMP/S-DSP, S-PPU and DMA/HDMA engines are clocked here. Every
//! S-CPU bus access advances the peripherals by its wait-state count, so the
//! S-SMP and S-PPU observe the same master-clock instants as ares' cooperative
//! threads. Coprocessors (SuperFX, SA-1, CX4, S-DD1, SPC7110, DSP1-4) are not
//! implemented yet.

use crate::apu::Apu;
use crate::cpu::{Cpu, CpuBus};
use crate::cx4::{Cx4, Cx4Error};
use crate::dma::{Dma, DmaBus};
use crate::gsu::Gsu;
use crate::io::CpuIo;
use crate::necdsp::{DspError, DspWindow, NecDsp};
use crate::ppu::Ppu;
use crate::rom::{Cartridge, Region};
use crate::sa1::Sa1;
use crate::sdd1::Sdd1;
use crate::spc7110::Spc7110;

/// Magic number at the start of a serialized save state (`"SNES"`).
pub const SAVE_STATE_MAGIC: u32 = 0x534E_4553;
/// Save-state format version. Bump whenever the serialized shape changes.
pub const SAVE_STATE_VERSION: u32 = 7;

/// A complete `unko-snes` snapshot.
///
/// This is the typed form of [`Snes::save_state`]. It owns every piece of
/// mutable state that affects future emulation: the S-CPU, S-SMP/S-DSP, PPU,
/// DMA/HDMA engine, CPU I/O, WRAM, cartridge SRAM and the scheduler credits and
/// latches. The ROM itself (and the derived region) are not stored because they
/// come from the inserted cartridge.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnesState {
    /// [`SAVE_STATE_MAGIC`].
    pub magic: u32,
    /// [`SAVE_STATE_VERSION`].
    pub version: u32,
    /// S-CPU register file and counters.
    pub cpu: Cpu,
    /// S-SMP + S-DSP.
    pub apu: Apu,
    /// S-PPU pair.
    pub ppu: Ppu,
    /// SuperFX coprocessor, when present.
    pub gsu: Option<Gsu>,
    /// Carried GSU master-clock credit.
    pub gsu_credit: i64,
    /// NEC DSP-1 coprocessor, when firmware is installed.
    pub dsp1: Option<NecDsp>,
    /// Carried DSP-1 clock credit.
    pub dsp_credit: i64,
    /// Capcom CX4 coprocessor, when firmware is installed.
    pub cx4: Option<Cx4>,
    /// Carried CX4 clock credit.
    pub cx4_credit: i64,
    /// S-DD1 decompression coprocessor, when the cartridge has one.
    pub sdd1: Option<Sdd1>,
    /// SA-1 coprocessor, when the cartridge has one.
    pub sa1: Option<Sa1>,
    /// SPC7110 coprocessor, when the cartridge has one.
    pub spc7110: Option<Spc7110>,
    /// Carried SA-1 instruction credit (master clocks).
    pub sa1_credit: i64,
    /// Last S-CPU bus address, for SA-1 bus-conflict penalties.
    pub snes_mar: u32,
    /// DMA/HDMA channels.
    pub dma: Dma,
    /// Controller ports and CPU I/O registers.
    pub io: CpuIo,
    /// 128 KiB work RAM.
    pub wram: Vec<u8>,
    /// Cartridge save RAM (empty when the cart has none).
    pub sram: Vec<u8>,
    /// Carried APU master-clock credit.
    pub apu_credit: i64,
    /// Carried PPU dot-clock credit.
    pub ppu_credit: i64,
    /// `$4200` bit 7.
    pub nmi_enable: bool,
    /// `$4210` bit 7.
    pub nmi_flag: bool,
    /// `$420D` MEMSEL.
    pub fast_rom: bool,
    /// `$2180-$2183` data-port address.
    pub wram_port: u32,
    /// Master clocks still owed to the peripherals.
    pub cpu_stall: u32,
    /// Current line's DRAM refresh latch.
    pub refresh_done: bool,
    /// A refresh crossed this step and must be charged.
    pub refresh_pending: bool,
    /// Joypad auto-poll started this frame.
    pub auto_poll_started: bool,
    /// This line's HDMA run completed.
    pub hdma_run_done: bool,
    /// Latched IRQ output.
    pub irq_line: bool,
    /// NMI edge waiting for an instruction boundary.
    pub nmi_line: bool,
    /// NMI edges delivered (debug).
    pub nmi_count: u64,
    /// Master clocks since reset (debug).
    pub clock: u64,
}

/// Errors from loading a save state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// The blob is too short, or its magic/version does not match.
    InvalidHeader,
    /// The blob could not be decoded.
    Decode(String),
    /// The saved WRAM is the wrong size.
    WramSize {
        /// Expected size in bytes.
        expected: usize,
        /// Saved size in bytes.
        found: usize,
    },
    /// The saved SRAM does not match the inserted cartridge.
    SramSize {
        /// Size the cartridge expects.
        expected: usize,
        /// Size in the save state.
        found: usize,
    },
    /// A memory region in the save state has the wrong length.
    Corrupt(&'static str),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader => write!(f, "not a valid unko-snes save state"),
            Self::Decode(error) => write!(f, "could not decode save state: {error}"),
            Self::WramSize { expected, found } => {
                write!(f, "save state WRAM is {found} bytes, expected {expected}")
            }
            Self::SramSize { expected, found } => {
                write!(f, "save state SRAM is {found} bytes, expected {expected}")
            }
            Self::Corrupt(what) => write!(f, "save state has an invalid {what} length"),
        }
    }
}

impl std::error::Error for StateError {}

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
    /// Console video region, taken from the cartridge header.
    region: Region,
    /// SuperFX coprocessor, when the cartridge has one.
    pub gsu: Option<Gsu>,
    /// Carried GSU master-clock credit.
    gsu_credit: i64,
    /// NEC DSP-1 coprocessor, when firmware is installed.
    pub dsp1: Option<NecDsp>,
    /// Carried DSP-1 clock credit.
    dsp_credit: i64,
    /// Capcom CX4 coprocessor, when firmware is installed.
    pub cx4: Option<Cx4>,
    /// Carried CX4 clock credit.
    cx4_credit: i64,
    /// S-DD1 decompression coprocessor, when the cartridge has one.
    pub sdd1: Option<Sdd1>,
    /// SA-1 coprocessor, when the cartridge has one.
    pub sa1: Option<Sa1>,
    /// SPC7110 coprocessor, when the cartridge has one.
    pub spc7110: Option<Spc7110>,
    /// Carried SA-1 instruction credit (master clocks).
    sa1_credit: i64,
    /// Last S-CPU bus address, for SA-1 bus-conflict penalties.
    snes_mar: u32,
    /// The S-CPU's DRAM refresh is active.
    refresh_active: bool,
    /// General-purpose DMA channels.
    pub dma: Dma,
    /// Controller ports, auto-poll and the multiply/divide unit.
    pub io: CpuIo,
    wram: Vec<u8>,
    apu_credit: i64,
    ppu_credit: i64,
    nmi_enable: bool,
    nmi_flag: bool,
    /// ares `status.irqLock`: a `$4200` write suppresses the interrupt test
    /// for the rest of that instruction, so the NMI lands one instruction
    /// later.
    nmi_lock: bool,
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
    /// Whether this line's HDMA run (ares `hdmaPosition`, dot 276) has happened.
    hdma_run_done: bool,
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
        let region = cart.header().region();
        let has_superfx = cart.header().has_superfx();
        let has_sdd1 = cart.header().has_sdd1();
        let has_sa1 = cart.header().has_sa1();
        let has_spc7110 = cart.header().has_spc7110();
        // Built before `cart` moves into the machine below.
        let spc7110 = if has_spc7110 {
            Some(Spc7110::new(cart.rom(), cart.sram().len()))
        } else {
            None
        };
        let mut ppu = Ppu::new();
        ppu.set_region(region);
        Self {
            cpu: Cpu::new(),
            cart,
            apu: Apu::new(),
            ppu,
            region,
            gsu: if has_superfx { Some(Gsu::new()) } else { None },
            gsu_credit: 0,
            dsp1: None,
            dsp_credit: 0,
            cx4: None,
            cx4_credit: 0,
            sdd1: if has_sdd1 { Some(Sdd1::new()) } else { None },
            sa1: if has_sa1 {
                Some(Sa1::new(region))
            } else {
                None
            },
            spc7110,
            sa1_credit: 0,
            snes_mar: 0,
            refresh_active: false,
            dma: Dma::new(),
            io: CpuIo::new(),
            wram: vec![0; WRAM_SIZE],
            apu_credit: 0,
            ppu_credit: 0,
            nmi_enable: false,
            nmi_flag: false,
            nmi_lock: false,
            fast_rom: false,
            wram_port: 0,
            cpu_stall: 0,
            refresh_done: false,
            refresh_pending: false,
            auto_poll_started: false,
            hdma_run_done: false,
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

    /// Console video region, from the cartridge header.
    pub const fn region(&self) -> Region {
        self.region
    }

    /// Install the DSP-1 firmware and attach the coprocessor.
    ///
    /// The firmware is the 8 KiB `dsp1.rom`/`dsp1b.rom` dump (2K x 3-byte
    /// program ROM followed by 1K x 2-byte data ROM). The board window is
    /// selected from the cartridge mapping and ROM size; passing a different
    /// length is rejected. Firmware is copyrighted chip data, so it is supplied
    /// by the host rather than embedded in this crate.
    pub fn set_dsp1_firmware(&mut self, firmware: &[u8]) -> Result<(), DspError> {
        let window = DspWindow::select(
            self.cart.header().mapping,
            self.cart.rom().len(),
            self.cart.sram().len(),
        );
        let dsp = NecDsp::from_firmware(firmware, window)?;
        self.dsp1 = Some(dsp);
        self.dsp_credit = 0;
        Ok(())
    }

    /// Install the CX4 firmware and attach the coprocessor.
    ///
    /// The firmware is the 3 KiB `cx4.rom` dump (1K x 3-byte data ROM). The
    /// program itself lives in the game ROM and is loaded through the HG51B
    /// instruction cache. The firmware is copyrighted chip data supplied by the
    /// host.
    pub fn set_cx4_firmware(&mut self, firmware: &[u8]) -> Result<(), Cx4Error> {
        self.cx4 = Some(Cx4::from_firmware(firmware)?);
        self.cx4_credit = 0;
        Ok(())
    }

    /// Take a typed snapshot of the full mutable state.
    ///
    /// The ROM and the derived region are not included: a snapshot is meant to
    /// be restored into a system with the same cartridge inserted.
    pub fn snapshot(&self) -> SnesState {
        SnesState {
            magic: SAVE_STATE_MAGIC,
            version: SAVE_STATE_VERSION,
            cpu: self.cpu.clone(),
            apu: self.apu.clone(),
            ppu: self.ppu.clone(),
            gsu: self.gsu.clone(),
            gsu_credit: self.gsu_credit,
            dsp1: self.dsp1.clone(),
            dsp_credit: self.dsp_credit,
            cx4: self.cx4.clone(),
            cx4_credit: self.cx4_credit,
            sdd1: self.sdd1.clone(),
            sa1: self.sa1.clone(),
            spc7110: self.spc7110.clone(),
            sa1_credit: self.sa1_credit,
            snes_mar: self.snes_mar,
            dma: self.dma.clone(),
            io: self.io.clone(),
            wram: self.wram.clone(),
            sram: self.cart.sram().to_vec(),
            apu_credit: self.apu_credit,
            ppu_credit: self.ppu_credit,
            nmi_enable: self.nmi_enable,
            nmi_flag: self.nmi_flag,
            fast_rom: self.fast_rom,
            wram_port: self.wram_port,
            cpu_stall: self.cpu_stall,
            refresh_done: self.refresh_done,
            refresh_pending: self.refresh_pending,
            auto_poll_started: self.auto_poll_started,
            hdma_run_done: self.hdma_run_done,
            irq_line: self.irq_line,
            nmi_line: self.nmi_line,
            nmi_count: self.nmi_count,
            clock: self.clock,
        }
    }

    /// Restore a snapshot taken by [`Self::snapshot`].
    ///
    /// Fails if the snapshot was made for a different cartridge size (WRAM or
    /// SRAM length mismatch) or is from an incompatible format.
    pub fn restore(&mut self, state: &SnesState) -> Result<(), StateError> {
        if state.magic != SAVE_STATE_MAGIC || state.version != SAVE_STATE_VERSION {
            return Err(StateError::InvalidHeader);
        }
        if state.wram.len() != WRAM_SIZE {
            return Err(StateError::WramSize {
                expected: WRAM_SIZE,
                found: state.wram.len(),
            });
        }
        if state.sram.len() != self.cart.sram().len() {
            return Err(StateError::SramSize {
                expected: self.cart.sram().len(),
                found: state.sram.len(),
            });
        }
        if !state.ppu.state_lengths_valid() {
            return Err(StateError::Corrupt("PPU memory"));
        }
        if !state.apu.state_lengths_valid() {
            return Err(StateError::Corrupt("APU RAM"));
        }

        self.cpu = state.cpu.clone();
        self.apu = state.apu.clone();
        self.ppu = state.ppu.clone();
        self.gsu = state.gsu.clone();
        self.gsu_credit = state.gsu_credit;
        self.dsp1 = state.dsp1.clone();
        self.dsp_credit = state.dsp_credit;
        self.cx4 = state.cx4.clone();
        self.cx4_credit = state.cx4_credit;
        self.sdd1 = state.sdd1.clone();
        self.sa1 = state.sa1.clone();
        self.spc7110 = state.spc7110.clone();
        if self.spc7110.is_some() {
            // The program/data ROM is not serialized; re-attach it.
            let rom = self.cart.rom().to_vec();
            if let Some(spc7110) = self.spc7110.as_mut() {
                spc7110.attach_rom(&rom);
            }
        }
        self.sa1_credit = state.sa1_credit;
        self.snes_mar = state.snes_mar;
        self.dma = state.dma.clone();
        self.io = state.io.clone();
        self.wram.copy_from_slice(&state.wram);
        self.cart.sram_mut().copy_from_slice(&state.sram);
        self.apu_credit = state.apu_credit;
        self.ppu_credit = state.ppu_credit;
        self.nmi_enable = state.nmi_enable;
        self.nmi_flag = state.nmi_flag;
        self.fast_rom = state.fast_rom;
        self.wram_port = state.wram_port;
        self.cpu_stall = state.cpu_stall;
        self.refresh_done = state.refresh_done;
        self.refresh_pending = state.refresh_pending;
        self.auto_poll_started = state.auto_poll_started;
        self.hdma_run_done = state.hdma_run_done;
        self.irq_line = state.irq_line;
        self.nmi_line = state.nmi_line;
        self.nmi_lock = false;
        self.nmi_count = state.nmi_count;
        self.clock = state.clock;
        Ok(())
    }

    /// Serialize the full state to bytes.
    ///
    /// The layout is `bincode` over [`SnesState`]; it is versioned by the
    /// `magic`/`version` fields and is not a stable cross-version file format.
    pub fn save_state(&self) -> Vec<u8> {
        bincode::serialize(&self.snapshot()).expect("save state serialization cannot fail")
    }

    /// Deserialize and restore a state produced by [`Self::save_state`].
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        let state: SnesState =
            bincode::deserialize(bytes).map_err(|error| StateError::Decode(error.to_string()))?;
        self.restore(&state)
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
        // `Ppu::reset` preserves the region, but re-assert it in case the field
        // order changes; the master clock ratio also depends on it.
        self.ppu.set_region(self.region);
        if let Some(gsu) = self.gsu.as_mut() {
            gsu.power();
        }
        self.gsu_credit = 0;
        if let Some(dsp) = self.dsp1.as_mut() {
            dsp.power();
        }
        self.dsp_credit = 0;
        if let Some(cx4) = self.cx4.as_mut() {
            cx4.power();
        }
        self.cx4_credit = 0;
        if let Some(sdd1) = self.sdd1.as_mut() {
            sdd1.power();
        }
        if let Some(sa1) = self.sa1.as_mut() {
            sa1.power(self.region);
        }
        if let Some(spc7110) = self.spc7110.as_mut() {
            spc7110.power();
        }
        self.sa1_credit = 0;
        self.snes_mar = 0;
        self.refresh_active = false;
        self.dma.hdma_reset();
        self.io.reset();
        self.apu_credit = 0;
        self.ppu_credit = 0;
        self.nmi_enable = false;
        self.nmi_flag = false;
        self.nmi_lock = false;
        self.fast_rom = false;
        self.wram_port = 0;
        self.irq_line = false;
        self.nmi_line = false;
        self.nmi_count = 0;
        self.auto_poll_started = false;
        self.hdma_run_done = false;
        self.clock = 0;
        self.apu.debug.set_clock(0);
        let mut bus = SnesBus {
            wram: &mut self.wram,
            cart: &mut self.cart,
            gsu: &mut self.gsu,
            dsp1: &mut self.dsp1,
            cx4: &mut self.cx4,
            sdd1: &mut self.sdd1,
            sa1: &mut self.sa1,
            spc7110: &mut self.spc7110,
            snes_mar: &mut self.snes_mar,
            snes_refresh: self.refresh_active,
            apu: &mut self.apu,
            ppu: &mut self.ppu,
            dma: &mut self.dma,
            io: &mut self.io,
            nmi_enable: &mut self.nmi_enable,
            nmi_flag: &mut self.nmi_flag,
            nmi_line: &mut self.nmi_line,
            nmi_lock: &mut self.nmi_lock,
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
        // ares `CPU::step` clears `irqLock` as each bus access starts; the one
        // case that matters is a `$4200` write, which is its instruction's last
        // access, so clearing once per instruction is equivalent.
        self.nmi_lock = false;
        if let Some(sa1) = self.sa1.as_mut() {
            sa1.snes_live_pc = self.cpu.pc_address();
            sa1.master_clock = self.clock;
        }
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
                        gsu: &mut self.gsu,
                        dsp1: &mut self.dsp1,
                        cx4: &mut self.cx4,
                        sdd1: &mut self.sdd1,
                        sa1: &mut self.sa1,
                        spc7110: &mut self.spc7110,
                        snes_mar: &mut self.snes_mar,
                        snes_refresh: self.refresh_active,
                        apu: &mut self.apu,
                        ppu: &mut self.ppu,
                        dma: &mut self.dma,
                        io: &mut self.io,
                        nmi_enable: &mut self.nmi_enable,
                        nmi_flag: &mut self.nmi_flag,
                        nmi_line: &mut self.nmi_line,
                        nmi_lock: &mut self.nmi_lock,
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
        let gsu_irq = self.gsu.as_ref().is_some_and(|gsu| gsu.irq_line);
        let cx4_irq = self.cx4.as_ref().is_some_and(|cx4| cx4.irq_line());
        let sa1_irq = self.sa1.as_ref().is_some_and(|sa1| sa1.cpu_irq_line());
        self.cpu
            .set_irq(self.irq_line || gsu_irq || cx4_irq || sa1_irq);
        if self.nmi_line && !self.nmi_lock {
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
        // The SPC7110's pending units stall the S-CPU for their latency
        // (20 clocks for a DCU transfer, 30/40 for multiply/divide).
        let spc7110 = if let Some(chip) = self.spc7110.as_mut() {
            chip.run(clocks)
        } else {
            0
        };
        if self.refresh_pending {
            self.refresh_pending = false;
            self.refresh_active = true;
            self.advance_peripherals_inner(DRAM_REFRESH_CLOCKS);
            self.refresh_active = false;
            DRAM_REFRESH_CLOCKS + spc7110
        } else {
            spc7110
        }
    }

    fn advance_peripherals_inner(&mut self, clocks: u32) {
        // The S-SMP runs at 2,050,560 Hz regardless of region; the master clock
        // differs (21,477,272 NTSC / 21,281,370 PAL). Draining *one SMP clock*
        // at a time (rather than one two-clock access unit) is what makes an
        // S-SMP port write become visible at the exact master clock it lands
        // on: ares/bsnes synchronize the S-SMP on every S-CPU step, so an S-CPU
        // poll must observe every SMP write up to that instant and no further.
        const APU_UNIT_NUM: i64 = 2_050_560;
        let apu_unit_den = self.region.master_clock() as i64;
        if self.apu.port_logging_enabled() {
            // Resolve every access unit to the master clock it starts on, so the
            // port handshake log can be compared event-for-event with a
            // reference. The tick order is unchanged: the extra arithmetic only
            // moves the recorded timestamp.
            let mut credit = self.apu_credit;
            let mut spent = 0u32;
            loop {
                // Clocks from `credit` to the next unit boundary (both positive).
                let need = u32::try_from((apu_unit_den - credit + APU_UNIT_NUM - 1) / APU_UNIT_NUM)
                    .unwrap_or(0);
                if spent + need > clocks {
                    break;
                }
                spent += need;
                credit += i64::from(need) * APU_UNIT_NUM - apu_unit_den;
                self.apu.debug.set_clock(self.clock + u64::from(spent));
                self.apu.tick();
            }
            credit += i64::from(clocks - spent) * APU_UNIT_NUM;
            self.apu_credit = credit;
        } else {
            self.apu_credit += i64::from(clocks) * APU_UNIT_NUM;
            while self.apu_credit >= apu_unit_den {
                self.apu.tick();
                self.apu_credit -= apu_unit_den;
            }
        }
        // The SuperFX oscillator runs at the S-CPU master clock on the boards
        // this core supports; run whole instructions against a clock credit.
        if let Some(gsu) = self.gsu.as_mut() {
            let rom = self.cart.rom();
            self.gsu_credit += i64::from(clocks);
            while self.gsu_credit > 0 {
                let cost = gsu.run_one(rom);
                self.gsu_credit -= i64::from(cost.max(1));
            }
        }
        // The DSP-1 free-runs at 7.6 MHz; tick it with a rational credit.
        if let Some(dsp) = self.dsp1.as_mut() {
            let den = self.region.master_clock() as i64;
            self.dsp_credit += i64::from(clocks) * 7_600_000;
            while self.dsp_credit >= den {
                dsp.exec();
                self.dsp_credit -= den;
            }
        }
        // The CX4 (HG51B) runs at 20 MHz; tick it with a rational credit.
        if let Some(cx4) = self.cx4.as_mut() {
            let den = self.region.master_clock() as i64;
            let rom = self.cart.rom();
            let sram = self.cart.sram();
            self.cx4_credit += i64::from(clocks) * 20_000_000;
            while self.cx4_credit >= den {
                let cost = cx4.run_one(rom, sram);
                self.cx4_credit -= den * i64::from(cost.max(1));
            }
        }
        // The SA-1 runs at half the master clock: one SA-1 cycle is two master
        // clocks. Its DMA cost is charged through `dma_clocks` on the next run.
        if let Some(sa1) = self.sa1.as_mut() {
            sa1.snes_mar = self.snes_mar;
            sa1.snes_refresh = self.refresh_active;
            sa1.master_clock = self.clock;
            self.sa1_credit += i64::from(clocks);
            while self.sa1_credit > 0 {
                let (rom, sram) = self.cart.rom_and_sram_mut();
                let cost = sa1.run_once(rom, sram);
                self.sa1_credit -= i64::from(cost.max(2));
            }
        }
        self.clock += u64::from(clocks);
        // One PPU dot is four master clocks in both regions; the PAL frame is
        // longer because it has 312 lines (and one 342-dot line), not because
        // the line clock differs.
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
            // Sample the comparator at the midpoint of the dot just completed;
            // ares polls every two master clocks with `hcounter(10)`.
            self.irq_line = self
                .io
                .poll_irq(self.ppu.line(), u32::from(pre_dot) * 4 + 2);
            if pre_dot == 32 && self.ppu.line() == 225 && !self.auto_poll_started {
                // ares `CPU::joypadEdge` starts the sequence on line 225 at
                // hcounter 130-384, not at the start of the line.
                self.auto_poll_started = true;
                self.io.begin_auto_poll();
            }
            // ares `CPU::step`: HDMA triggers at hcounter 1104 (dot 276) of
            // every visible line, so its register writes land before the next
            // line is rendered but after the current line's CPU work.
            if pre_dot == 275 && !self.ppu.in_vblank() && !self.hdma_run_done {
                self.hdma_run_done = true;
                let mut bus = HdmaBus {
                    wram: &mut self.wram,
                    cart: &mut self.cart,
                    apu: &mut self.apu,
                    ppu: &mut self.ppu,
                };
                let cost = self.dma.hdma_line(&mut bus);
                self.cpu_stall += cost;
            }
            if let Some(line) = self.ppu.take_line_event() {
                self.refresh_done = false;
                self.hdma_run_done = false;
                if line == 0 {
                    self.dma.hdma_reset();
                    self.auto_poll_started = false;
                    // ares `hdmaSetup` runs at V=0 (hcounter ~12); our
                    // line-granular renderer runs it with the line-0 event so
                    // line 0 sees it.
                    let mut bus = HdmaBus {
                        wram: &mut self.wram,
                        cart: &mut self.cart,
                        apu: &mut self.apu,
                        ppu: &mut self.ppu,
                    };
                    self.cpu_stall += self.dma.hdma_setup(&mut bus);
                }
            }
            // ares renders a line's visible pixels from dot 22 onward; render
            // after hblank so register writes on dots 0-21 (e.g. a DMA that
            // unblanks INIDISP at the line boundary) apply to this line.
            if pre_dot == 21 && self.ppu.line() >= 1 && self.ppu.line() <= self.ppu.visible_last() {
                self.ppu.render_current_line();
            }
        }
        self.io.step(clocks);
        self.irq_line = self
            .io
            .poll_irq(self.ppu.line(), u32::from(self.ppu.dot()) * 4 + 2);
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
            gsu: &mut self.gsu,
            dsp1: &mut self.dsp1,
            cx4: &mut self.cx4,
            sdd1: &mut self.sdd1,
            sa1: &mut self.sa1,
            spc7110: &mut self.spc7110,
            snes_mar: &mut self.snes_mar,
            snes_refresh: self.refresh_active,
            apu: &mut self.apu,
            ppu: &mut self.ppu,
            dma: &mut self.dma,
            io: &mut self.io,
            nmi_enable: &mut self.nmi_enable,
            nmi_flag: &mut self.nmi_flag,
            nmi_line: &mut self.nmi_line,
            nmi_lock: &mut self.nmi_lock,
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
    gsu: &'a mut Option<Gsu>,
    dsp1: &'a mut Option<NecDsp>,
    cx4: &'a mut Option<Cx4>,
    sdd1: &'a mut Option<Sdd1>,
    sa1: &'a mut Option<Sa1>,
    spc7110: &'a mut Option<Spc7110>,
    snes_mar: &'a mut u32,
    snes_refresh: bool,
    dma: &'a mut Dma,
    io: &'a mut CpuIo,
    nmi_enable: &'a mut bool,
    nmi_flag: &'a mut bool,
    nmi_line: &'a mut bool,
    nmi_lock: &'a mut bool,
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

impl SnesBus<'_> {
    /// Map an S-CPU address onto the SA-1, if the cartridge has one.
    fn sa1_read(&mut self, address: u32, mdr: u8) -> Option<u8> {
        self.sa1.as_ref()?;
        if let Some(sa1) = self.sa1.as_mut() {
            sa1.snes_mar = address;
            sa1.snes_refresh = self.snes_refresh;
        }
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        let low = bank <= 0x3F || (0x80..=0xBF).contains(&bank);

        if low && (0x2200..=0x23FF).contains(&offset) {
            let (rom, sram) = self.cart.rom_and_sram_mut();
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            return Some(sa1.read_cpu_io(address, rom, sram, mdr));
        }
        if low && (0x3000..=0x37FF).contains(&offset) {
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            return Some(sa1.read_cpu_iram(address, mdr));
        }
        if Sa1::in_bwram_window(address) {
            let (_, sram) = self.cart.rom_and_sram_mut();
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            return Some(sa1.read_cpu_bwram(address, sram));
        }
        if Sa1::in_rom_window(address) {
            let rom = self.cart.rom();
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            return Some(sa1.read_cpu_rom(address, rom));
        }
        None
    }

    /// Map an S-CPU write onto the SA-1, if the cartridge has one.
    fn sa1_write(&mut self, address: u32, value: u8) -> bool {
        if self.sa1.is_none() {
            return false;
        }
        if let Some(sa1) = self.sa1.as_mut() {
            sa1.snes_mar = address;
            sa1.snes_refresh = self.snes_refresh;
        }
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        let low = bank <= 0x3F || (0x80..=0xBF).contains(&bank);

        if low && (0x2200..=0x23FF).contains(&offset) {
            let (rom, sram) = self.cart.rom_and_sram_mut();
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            sa1.write_cpu_io(address, rom, sram, value);
            return true;
        }
        if low && (0x3000..=0x37FF).contains(&offset) {
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            sa1.write_cpu_iram(address, value);
            return true;
        }
        if Sa1::in_bwram_window(address) {
            let (_, sram) = self.cart.rom_and_sram_mut();
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            sa1.write_cpu_bwram(address, sram, value);
            return true;
        }
        // Writes to the SA-1 ROM window are ignored by the chip.
        Sa1::in_rom_window(address)
    }

    /// Map an S-CPU address onto the DSP-1 DR/SR ports, if the DSP is attached.
    fn dsp1_port(&self, address: u32) -> Option<bool> {
        self.dsp1.as_ref()?.window.classify(address)
    }

    /// Map an S-CPU address onto the SuperFX coprocessor, if present.
    fn superfx_read(&mut self, address: u32, mdr: u8) -> Option<u8> {
        let gsu = self.gsu.as_mut()?;
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        let bank7 = bank & 0x7F;
        let rom = self.cart.rom();

        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if (0x3000..=0x34FF).contains(&offset) {
                gsu.synchronize(rom);
                return Some(gsu.read_io(address));
            }
            if (0x6000..=0x7FFF).contains(&offset) {
                gsu.synchronize(rom);
                if gsu.running() && gsu.ram_enabled() {
                    return Some(mdr);
                }
                return Some(gsu.read_ram((offset - 0x6000) as u32));
            }
        }
        if (0x60..=0x7D).contains(&bank7) && (offset < 0x8000 || (0x70..=0x71).contains(&bank7)) {
            gsu.synchronize(rom);
            if gsu.running() && gsu.ram_enabled() {
                return Some(mdr);
            }
            return Some(gsu.read_ram(offset as u32));
        }

        // ares `SuperFX::CPUROM::read`: while the GSU runs with ROM enabled the
        // S-CPU sees a fixed 16-byte vector in ROM space.
        let rom_area = ((bank <= 0x3F || (0x80..=0xBF).contains(&bank)) && offset >= 0x8000)
            || (0x40..=0x5F).contains(&bank)
            || (0xC0..=0xDF).contains(&bank);
        if rom_area && gsu.running() && gsu.rom_enabled() {
            const VECTOR: [u8; 16] = [
                0x00, 0x01, 0x00, 0x01, 0x04, 0x01, 0x00, 0x01, 0x00, 0x01, 0x08, 0x01, 0x00, 0x01,
                0x0C, 0x01,
            ];
            return Some(VECTOR[address as usize & 15]);
        }
        None
    }

    /// Map an S-CPU write onto the SuperFX coprocessor, if present.
    fn superfx_write(&mut self, address: u32, value: u8) -> bool {
        let Some(gsu) = self.gsu.as_mut() else {
            return false;
        };
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        let bank7 = bank & 0x7F;
        let rom = self.cart.rom();

        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if (0x3000..=0x34FF).contains(&offset) {
                gsu.synchronize(rom);
                gsu.write_io(address, value);
                return true;
            }
            if (0x6000..=0x7FFF).contains(&offset) {
                gsu.synchronize(rom);
                gsu.write_ram((offset - 0x6000) as u32, value);
                return true;
            }
        }
        if (0x60..=0x7D).contains(&bank7) && (offset < 0x8000 || (0x70..=0x71).contains(&bank7)) {
            gsu.synchronize(rom);
            gsu.write_ram(offset as u32, value);
            return true;
        }
        false
    }
}

impl CpuBus for SnesBus<'_> {
    fn read(&mut self, address: u32, mdr: u8) -> u8 {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;

        if bank == 0x7E || bank == 0x7F {
            return self.wram[((bank as usize & 1) << 16) | offset];
        }
        *self.snes_mar = address;
        if let Some(value) = self.sa1_read(address, mdr) {
            return value;
        }
        if self.spc7110.is_some() {
            if let Some(offset) = Spc7110::register_offset(address) {
                let chip = self.spc7110.as_mut().expect("SPC7110 present");
                return chip.read(offset, mdr);
            }
            if let Some(index) = Spc7110::ram_index(address) {
                let chip = self.spc7110.as_ref().expect("SPC7110 present");
                return chip.ram_read(index, mdr);
            }
            if Spc7110::in_rom_window(address) {
                let reduced = Spc7110::reduce_rom(address);
                let chip = self.spc7110.as_ref().expect("SPC7110 present");
                return chip.mcurom_read(reduced, mdr);
            }
        }
        if let Some(sdd1) = self.sdd1.as_mut() {
            if Sdd1::in_io_window(address) {
                return sdd1.read_io(address, self.cart.rom());
            }
            if let Some(index) = Sdd1::sram_index(address) {
                return self.cart.sram().get(index).copied().unwrap_or(mdr);
            }
            if Sdd1::in_mcu_window(address) {
                return sdd1.mcu_read(address, self.cart.rom());
            }
        }
        if self.cx4.is_some() {
            if let Some(offset) = Cx4::io_window(address) {
                return self.cx4.as_mut().expect("CX4 present").read_io(offset);
            }
            if let Some(offset) = Cx4::dram_window(address) {
                return self.cx4.as_ref().expect("CX4 present").read_dram(offset);
            }
        }
        if let Some(status) = self.dsp1_port(address) {
            return self.dsp1.as_mut().expect("DSP-1 present").read_port(status);
        }
        if let Some(value) = self.superfx_read(address, mdr) {
            return value;
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                return self.wram[offset & 0x1FFF];
            }
            if offset < 0x6000 {
                if (0x2100..=0x213F).contains(&offset) {
                    if offset == 0x2137 && self.io.pio() & 0x80 != 0 {
                        self.ppu.latch_counters();
                    }
                    return self.ppu.read(offset as u16, mdr);
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
                    // Only the driven low bits are replaced; the rest is
                    // open bus (`$4016` bits 2-7, `$4017` bits 5-7).
                    let value = self.io.read(offset as u16).unwrap_or(0);
                    let mask = if offset == 0x4016 { 0x03 } else { 0x1F };
                    return (mdr & !mask) | (value & mask);
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
        *self.snes_mar = address;
        if self.spc7110.is_some() {
            if let Some(offset) = Spc7110::register_offset(address) {
                let chip = self.spc7110.as_mut().expect("SPC7110 present");
                chip.write(offset, value);
                return;
            }
            if let Some(index) = Spc7110::ram_index(address) {
                let chip = self.spc7110.as_mut().expect("SPC7110 present");
                chip.ram_write(index, value);
                return;
            }
            if Spc7110::in_rom_window(address) {
                return; // program/data ROM is read-only
            }
        }
        if self.sa1_write(address, value) {
            return;
        }
        if self.sdd1.is_some() {
            if Sdd1::in_io_window(address) {
                if let Some(sdd1) = self.sdd1.as_mut() {
                    sdd1.write_io(address, value);
                }
                return;
            }
            if let Some(index) = Sdd1::sram_index(address) {
                if let Some(byte) = self.cart.sram_mut().get_mut(index) {
                    *byte = value;
                }
                return;
            }
            if (0x4300..=0x437F).contains(&offset) {
                if let Some(sdd1) = self.sdd1.as_mut() {
                    sdd1.note_dma_write(offset as u16, value);
                }
            }
        }
        if self.cx4.is_some() {
            if let Some(offset) = Cx4::io_window(address) {
                self.cx4
                    .as_mut()
                    .expect("CX4 present")
                    .write_io(offset, value);
                return;
            }
            if let Some(offset) = Cx4::dram_window(address) {
                self.cx4
                    .as_mut()
                    .expect("CX4 present")
                    .write_dram(offset, value);
                return;
            }
        }
        if let Some(status) = self.dsp1_port(address) {
            self.dsp1
                .as_mut()
                .expect("DSP-1 present")
                .write_port(status, value);
            return;
        }
        if self.superfx_write(address, value) {
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
                    // ares `nmitimenUpdate`: raising the NMI enable while the
                    // vblank flag is already latched delivers an NMI at once,
                    // so a game that enables NMIs partway through vblank still
                    // sees one that frame.
                    let enable = value & 0x80 != 0;
                    *self.nmi_lock = true;
                    if !*self.nmi_enable && enable && *self.nmi_flag && self.vblank {
                        *self.nmi_line = true;
                    }
                    *self.nmi_enable = enable;
                    self.io.write(0x4200, value);
                } else if offset == 0x4201 {
                    // WRIO bit 7 enables $2137 counter latching; without it
                    // $213C/$213D never change (Mesen/ares `io.pio`).
                    self.io.write(0x4201, value);
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
        // The SA-1's bus-contention model depends on the S-CPU's current
        // address; ares tracks it in `r.mar` at the start of every access.
        *self.snes_mar = address;
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
                    return self.ppu.read(offset as u16, 0);
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
            gsu: &mut system.gsu,
            dsp1: &mut system.dsp1,
            cx4: &mut system.cx4,
            sdd1: &mut system.sdd1,
            sa1: &mut system.sa1,
            spc7110: &mut system.spc7110,
            snes_mar: &mut system.snes_mar,
            snes_refresh: system.refresh_active,
            apu: &mut system.apu,
            ppu: &mut system.ppu,
            dma: &mut system.dma,
            io: &mut system.io,
            nmi_enable: &mut system.nmi_enable,
            nmi_flag: &mut system.nmi_flag,
            nmi_line: &mut system.nmi_line,
            nmi_lock: &mut system.nmi_lock,
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
            gsu: &mut system.gsu,
            dsp1: &mut system.dsp1,
            cx4: &mut system.cx4,
            sdd1: &mut system.sdd1,
            sa1: &mut system.sa1,
            spc7110: &mut system.spc7110,
            snes_mar: &mut system.snes_mar,
            snes_refresh: system.refresh_active,
            apu: &mut system.apu,
            ppu: &mut system.ppu,
            dma: &mut system.dma,
            io: &mut system.io,
            nmi_enable: &mut system.nmi_enable,
            nmi_flag: &mut system.nmi_flag,
            nmi_line: &mut system.nmi_line,
            nmi_lock: &mut system.nmi_lock,
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
            gsu: &mut system.gsu,
            dsp1: &mut system.dsp1,
            cx4: &mut system.cx4,
            sdd1: &mut system.sdd1,
            sa1: &mut system.sa1,
            spc7110: &mut system.spc7110,
            snes_mar: &mut system.snes_mar,
            snes_refresh: system.refresh_active,
            apu: &mut system.apu,
            ppu: &mut system.ppu,
            dma: &mut system.dma,
            io: &mut system.io,
            nmi_enable: &mut system.nmi_enable,
            nmi_flag: &mut system.nmi_flag,
            nmi_line: &mut system.nmi_line,
            nmi_lock: &mut system.nmi_lock,
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
    fn region_follows_the_header_country() {
        let mut rom = build_lorom(&[0xDB]);
        let header = Mapping::LoRom.header_offset();
        rom[header + 0x19] = 0x02; // Europe
        let system = Snes::new(Cartridge::from_bytes(&rom).expect("valid LoROM"));
        assert_eq!(system.region(), crate::rom::Region::Pal);
        assert!(system.ppu.region().is_pal());

        rom[header + 0x19] = 0x01; // USA / Canada
        let system = Snes::new(Cartridge::from_bytes(&rom).expect("valid LoROM"));
        assert_eq!(system.region(), crate::rom::Region::Ntsc);
        assert!(!system.ppu.region().is_pal());
    }

    #[test]
    fn pal_frame_advances_312_scanlines() {
        let mut rom = build_lorom(&[0xDB]);
        let header = Mapping::LoRom.header_offset();
        rom[header + 0x19] = 0x02; // Europe
        let mut system = Snes::new(Cartridge::from_bytes(&rom).expect("valid LoROM"));
        system.reset();
        // Tick dots directly (one dot = four master clocks). A non-interlaced
        // PAL frame is 312 lines of 341 dots.
        let mut dots = 0u32;
        let mut wraps = 0u32;
        let mut last = system.ppu.line();
        for _ in 0..400_000 {
            system.ppu.tick();
            dots += 1;
            if system.ppu.line() < last {
                wraps += 1;
                if wraps == 2 {
                    break;
                }
            }
            last = system.ppu.line();
        }
        assert_eq!(wraps, 2);
        assert_eq!(dots, 2 * 312 * 341);
    }

    #[test]
    fn save_state_round_trips_and_continues_identically() {
        // JMP $8000: keeps the CPU busy so the peripherals advance too.
        let code = [0x4Cu8, 0x00, 0x80];
        let mut system = Snes::new(cartridge(&code));
        system.reset();

        // Touch every subsystem before saving.
        system.ppu.write(0x2100, 0x0F); // display on
        system.ppu.write(0x2105, 0x01); // mode 1
        system.ppu.write(0x2107, 0x04);
        system.ppu.write(0x210B, 0x00);
        system.ppu.write(0x2121, 0x02);
        system.ppu.write(0x2122, 0x1F);
        system.ppu.write(0x2122, 0x00);
        system.ppu.write(0x2115, 0x80); // VRAM increment after the high byte
        system.ppu.write(0x2116, 0x00);
        system.ppu.write(0x2117, 0x00);
        system.ppu.write(0x2118, 0x34);
        system.ppu.write(0x2119, 0x12);
        system.ppu.write(0x2102, 0x00);
        system.ppu.write(0x2103, 0x00);
        for byte in [0x00, 0x10, 0x01, 0x20] {
            system.ppu.write(0x2104, byte);
        }
        system.apu.cpu_write(0, 0x11);
        system.apu.cpu_write(1, 0x22);
        system.set_joypad(0, crate::io::button::A | crate::io::button::START);
        system.write(0x42_07, 50); // HTIMEL
        system.write(0x42_09, 100); // VTIMEL
        system.write(0x42_00, 0x30); // H+V IRQ
        system.cart.sram_mut()[0] = 0x5A;
        system.wram_mut()[0x1234] = 0xA5;

        system.run_cycles(120_000, 200_000);

        let saved = system.save_state();
        let snapshot = system.snapshot();

        let mut restored = Snes::new(cartridge(&code));
        restored.reset();
        restored.load_state(&saved).expect("load state");
        assert_eq!(restored.snapshot(), snapshot);
        // `Snes` compares the whole machine, including cartridge SRAM.
        assert_eq!(restored, system);

        // The restored machine must evolve exactly like the original.
        system.run_cycles(40_000, 200_000);
        restored.run_cycles(40_000, 200_000);
        assert_eq!(restored.snapshot(), system.snapshot());
    }

    fn superfx_cartridge(code: &[u8]) -> Cartridge {
        let mut rom = build_lorom(code);
        let header = Mapping::LoRom.header_offset();
        rom[header + 0x16] = 0x14; // SuperFX + RAM + battery
        Cartridge::from_bytes(&rom).expect("valid SuperFX LoROM")
    }

    #[test]
    fn superfx_cartridges_expose_io_and_work_ram() {
        let mut system = Snes::new(superfx_cartridge(&[0x4C, 0x00, 0x80]));
        assert!(system.gsu.is_some());
        // The 32 KiB work RAM is visible at $70-$71 and at $6000-$7FFF.
        system.write(0x70_0000, 0xA5);
        assert_eq!(system.read(0x70_0000, 0), 0xA5);
        assert_eq!(system.read(0x00_6000, 0), 0xA5);
        // Registers are visible at $3000-$301F.
        system.write(0x00_3002, 0x34);
        system.write(0x00_3003, 0x12);
        assert_eq!(system.read(0x00_3002, 0), 0x34);
        assert_eq!(system.read(0x00_3003, 0), 0x12);
        // The G flag is clear before the CPU starts it.
        assert_eq!(system.read(0x00_3030, 0) & 0x20, 0);
    }

    #[test]
    fn save_state_round_trips_a_superfx_machine() {
        let mut system = Snes::new(superfx_cartridge(&[0x4C, 0x00, 0x80]));
        system.reset();
        system.write(0x70_0000, 0x5A);
        system.write(0x00_3002, 0x34);
        system.write(0x00_3003, 0x12);
        system.run_cycles(50_000, 200_000);
        let saved = system.save_state();

        let mut restored = Snes::new(superfx_cartridge(&[0x4C, 0x00, 0x80]));
        restored.reset();
        restored.load_state(&saved).expect("load SuperFX state");
        assert_eq!(restored, system);
    }

    #[test]
    fn save_state_continues_a_running_superfx_identically() {
        // S-CPU: JMP $8000. GSU program at $8100: many INC r0, then STOP.
        let mut code = vec![0x4C, 0x00, 0x80];
        code.resize(0x100, 0xEA);
        code.extend(std::iter::repeat_n(0xD0u8, 4000)); // INC r0
        code.push(0x00); // STOP
        let mut system = Snes::new(superfx_cartridge(&code));
        system.reset();
        {
            let gsu = system.gsu.as_mut().expect("SuperFX");
            gsu.write_io(0x3034, 0x00); // PBR
            gsu.write_io(0x303A, 0x18); // RON | RAN
            gsu.write_io(0x3039, 0x01); // fast clock
            gsu.write_io(0x301E, 0x00);
            gsu.write_io(0x301F, 0x81); // PC = $8100, G = 1
        }
        system.run_cycles(20_000, 200_000);
        let saved = system.save_state();

        let mut restored = Snes::new(superfx_cartridge(&code));
        restored.reset();
        restored.load_state(&saved).expect("load SuperFX state");
        assert_eq!(restored, system);

        // Both machines must continue the GSU program identically.
        system.run_cycles(80_000, 400_000);
        restored.run_cycles(80_000, 400_000);
        assert_eq!(restored.snapshot(), system.snapshot());
    }

    #[test]
    fn dsp1_firmware_attaches_ports_and_round_trips() {
        let mut rom = build_lorom(&[0x4C, 0x00, 0x80]);
        let header = Mapping::LoRom.header_offset();
        rom[header + 0x16] = 0x03; // ROM + coprocessor
        let firmware = vec![0u8; 8192];

        let cartridge = Cartridge::from_bytes(&rom).expect("valid LoROM");
        let mut system = Snes::new(cartridge);
        system.set_dsp1_firmware(&firmware).expect("DSP-1 firmware");
        assert!(system.dsp1.is_some());
        // LoROM <= 1 MiB: DR at $30-$3F:$8000-$BFFF, SR at $C000-$FFFF.
        system.write(0x30_8000, 0x34);
        system.write(0x30_8000, 0x12);
        assert_eq!(system.dsp1.as_ref().unwrap().regs.dr, 0x1234);
        // Reading SR must be routed to the status register, not ROM.
        let status = system.read(0x30_C000, 0);
        assert_eq!(status, 0x00);

        system.run_cycles(20_000, 200_000);
        let saved = system.save_state();

        let mut restored = Snes::new(Cartridge::from_bytes(&rom).expect("valid LoROM"));
        restored
            .set_dsp1_firmware(&firmware)
            .expect("DSP-1 firmware");
        restored.reset();
        restored.load_state(&saved).expect("load DSP-1 state");
        assert_eq!(restored, system);
    }

    fn cx4_cartridge(code: &[u8]) -> Cartridge {
        let mut rom = build_lorom(code);
        let header = Mapping::LoRom.header_offset();
        rom[header + 0x16] = 0xF3; // CX4 (Hitachi) coprocessor
        Cartridge::from_bytes(&rom).expect("valid CX4 LoROM")
    }

    fn cx4_firmware() -> Vec<u8> {
        let mut bytes = vec![0u8; 1024 * 3];
        // Data-ROM word 0 = 0x123456.
        bytes[0] = 0x56;
        bytes[1] = 0x34;
        bytes[2] = 0x12;
        bytes
    }

    #[test]
    fn cx4_header_and_windows_are_wired() {
        let mut system = Snes::new(cx4_cartridge(&[0x4C, 0x00, 0x80]));
        assert!(system.cart.header().has_cx4());
        assert!(system.cx4.is_none());

        system
            .set_cx4_firmware(&cx4_firmware())
            .expect("CX4 firmware");
        assert!(system.cx4.is_some());

        // GPR window: $7F80-$7F82 is register 0.
        system.write(0x00_7F80, 0x12);
        system.write(0x00_7F81, 0x34);
        system.write(0x00_7F82, 0x56);
        assert_eq!(system.read(0x00_7F80, 0), 0x12);
        assert_eq!(system.read(0x00_7F82, 0), 0x56);
        // Data RAM window.
        system.write(0x00_6000, 0xA5);
        assert_eq!(system.read(0x00_6000, 0), 0xA5);
        // The CX4 powers up running (ares `io = {}`).
        assert_eq!(system.read(0x00_7F5E, 0) & 0x40, 0x40);
    }

    #[test]
    fn save_state_round_trips_a_cx4_machine() {
        let firmware = cx4_firmware();
        let code = [0x4C, 0x00, 0x80];
        let mut system = Snes::new(cx4_cartridge(&code));
        system.set_cx4_firmware(&firmware).expect("CX4 firmware");
        system.reset();
        system.write(0x00_7F80, 0xAB);
        system.write(0x00_6000, 0xCD);
        system.run_cycles(50_000, 200_000);
        let saved = system.save_state();

        let mut restored = Snes::new(cx4_cartridge(&code));
        restored.set_cx4_firmware(&firmware).expect("CX4 firmware");
        restored.reset();
        restored.load_state(&saved).expect("load CX4 state");
        assert_eq!(restored, system);
    }

    fn sdd1_cartridge(code: &[u8]) -> Cartridge {
        let mut rom = build_lorom(code);
        let header = Mapping::LoRom.header_offset();
        rom[header + 0x15] = 0x22; // S-DD1 extended LoROM
        Cartridge::from_bytes(&rom).expect("valid S-DD1 LoROM")
    }

    #[test]
    fn sdd1_cartridges_attach_the_decompressor() {
        let mut system = Snes::new(sdd1_cartridge(&[0x4C, 0x00, 0x80]));
        assert!(system.cart.header().has_sdd1());
        assert!(system.sdd1.is_some());
        // The register window mirrors onto $4800-$480F.
        system.write(0x00_4804, 0x05);
        assert_eq!(system.read(0x00_4804, 0), 0x05);
        system.write(0x00_4800, 0x03);
        assert_eq!(system.read(0x00_4800, 0), 0x03);
        // A plain LoROM cartridge has no S-DD1.
        let plain = Snes::new(cartridge(&[0x4C, 0x00, 0x80]));
        assert!(plain.sdd1.is_none());
    }

    #[test]
    fn save_state_round_trips_an_sdd1_machine() {
        let code = [0x4C, 0x00, 0x80];
        let mut system = Snes::new(sdd1_cartridge(&code));
        system.reset();
        system.write(0x00_4804, 0x07);
        system.write(0x00_4800, 0x01);
        system.run_cycles(50_000, 200_000);
        let saved = system.save_state();

        let mut restored = Snes::new(sdd1_cartridge(&code));
        restored.reset();
        restored.load_state(&saved).expect("load S-DD1 state");
        assert_eq!(restored, system);
    }

    #[test]
    fn serial_ports_report_ares_driven_bits() {
        let mut system = Snes::new(cartridge(&[0x4C, 0x00, 0x80]));
        // B is the first bit of a standard gamepad.
        system.set_joypad(0, crate::io::button::B);
        system.set_joypad(1, 0);
        system.write(0x00_4016, 0x01);
        system.write(0x00_4016, 0x00);

        // JOYSER0: bit 0 is port 1 data, bit 1 is port 1's second data line
        // (low for a standard gamepad). ares `CPU::readCPU`.
        let pad1 = system.read(0x00_4016, 0);
        assert_eq!(pad1 & 0x01, 0x01, "B should be the first serial bit");
        assert_eq!(pad1 & 0x02, 0x00, "$4016.d1 is port 1 data2, low for a pad");

        // JOYSER1: bits 2-4 are tied to ground and bit 1 is port 2 data2, so a
        // standard gamepad always reads 0b0001_11x0 here. KDL3's multitap
        // probe shifts bit 1; forcing it high made the game think a multitap
        // was attached.
        let pad2 = system.read(0x00_4017, 0);
        assert_eq!(pad2 & 0x02, 0x00, "$4017.d1 is port 2 data2");
        assert_eq!(pad2 & 0x1C, 0x1C, "$4017.d2-d4 are tied to ground");
    }

    #[test]
    fn enabling_nmi_during_vblank_delivers_one_immediately() {
        // ares `nmitimenUpdate`: `nmiEnable.raise()` while the vblank flag is
        // latched sets `nmiTransition`, so a game that turns NMIs on partway
        // through vblank still gets an NMI that frame (KDL3's first NMI).
        let mut system = Snes::new(cartridge(&[0x4C, 0x00, 0x80]));
        assert!(!system.nmi_enabled());
        system.nmi_flag = true;
        {
            let mut bus = SnesBus {
                wram: &mut system.wram,
                cart: &mut system.cart,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
                fast_rom: &mut system.fast_rom,
                wram_port: &mut system.wram_port,
                vblank: true,
                hblank: false,
            };
            bus.write(0x4200, 0x80);
        }
        assert!(system.nmi_line, "raising NMI enable mid-vblank must latch");
        // ares `status.irqLock`: the `$4200` write suppresses the interrupt
        // test for the rest of its instruction, so delivery slips one
        // instruction (KDL3 matches ares only with this).
        assert!(system.nmi_lock, "$4200 writes must lock the interrupt test");
    }

    fn spc7110_cartridge(code: &[u8]) -> Cartridge {
        let mut rom = build_lorom(code);
        let header = Mapping::LoRom.header_offset();
        rom[header + 0x15] = 0x3A; // SPC7110 HiROM map mode
        rom[header + 0x16] = 0xF5; // SPC7110
        Cartridge::from_bytes(&rom).expect("valid SPC7110 image")
    }

    #[test]
    fn spc7110_cartridges_attach_the_chip() {
        let mut system = Snes::new(spc7110_cartridge(&[0x4C, 0x00, 0x80]));
        assert!(system.cart.header().has_spc7110());
        assert!(system.spc7110.is_some());
        // Register window round-trip.
        system.write(0x00_4801, 0x5A);
        assert_eq!(system.read(0x00_4801, 0), 0x5A);
        // Save RAM is gated by $4830 bit 7.
        system.write(0x00_6000, 0x11);
        assert_eq!(system.read(0x00_6000, 0), 0x00);
        system.write(0x00_4830, 0x80);
        system.write(0x00_6000, 0x11);
        assert_eq!(system.read(0x00_6000, 0), 0x11);
        // A plain LoROM cartridge has no SPC7110.
        let plain = Snes::new(cartridge(&[0x4C, 0x00, 0x80]));
        assert!(plain.spc7110.is_none());
    }

    #[test]
    fn save_state_round_trips_an_spc7110_machine() {
        let code = [0x4C, 0x00, 0x80];
        let mut system = Snes::new(spc7110_cartridge(&code));
        system.reset();
        system.write(0x00_4830, 0x80);
        system.write(0x00_4812, 0x01);
        system.write(0x00_4820, 6);
        system.write(0x00_4824, 7);
        system.write(0x00_4825, 0);
        system.run_cycles(50_000, 200_000);
        let saved = system.save_state();

        let mut restored = Snes::new(spc7110_cartridge(&code));
        restored.reset();
        restored.load_state(&saved).expect("load SPC7110 state");
        assert_eq!(restored, system);
    }

    #[test]
    fn save_state_rejects_bad_input() {
        let mut system = Snes::new(cartridge(&[0xDB]));

        assert!(matches!(
            system.load_state(b"not a save state"),
            Err(StateError::Decode(_))
        ));

        let mut state = system.snapshot();
        state.magic = 0;
        assert_eq!(system.restore(&state), Err(StateError::InvalidHeader));

        let mut state = system.snapshot();
        state.sram.push(0);
        assert!(matches!(
            system.restore(&state),
            Err(StateError::SramSize { .. })
        ));

        let mut state = system.snapshot();
        state.wram.push(0);
        assert!(matches!(
            system.restore(&state),
            Err(StateError::WramSize { .. })
        ));
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                snes_mar: &mut system.snes_mar,
                snes_refresh: system.refresh_active,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                dma: &mut system.dma,
                io: &mut system.io,
                nmi_enable: &mut system.nmi_enable,
                nmi_flag: &mut system.nmi_flag,
                nmi_line: &mut system.nmi_line,
                nmi_lock: &mut system.nmi_lock,
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
