//! SNES system: S-CPU bus, WRAM, cartridge access and the peripheral clock.
//!
//! The S-CPU, S-SMP/S-DSP, S-PPU and DMA/HDMA engines are clocked here. Every
//! S-CPU bus access advances the peripherals by its wait-state count, so the
//! S-SMP and S-PPU observe the same master-clock instants as ares' cooperative
//! threads. Coprocessors (SuperFX, SA-1, CX4, S-DD1, SPC7110, OBC1, DSP1-4) are
//! scheduled from the same master-clock credit.

use crate::apu::Apu;
use crate::bsx::{Bsx, BsxBus};
use crate::cpu::{Cpu, CpuBus};
use crate::cx4::{Cx4, Cx4Error};
use crate::dma::{Dma, DmaBus};
use crate::gsu::Gsu;
use crate::hdma_calibration;
use crate::io::CpuIo;
use crate::msu1::{Msu1, MSU1_SAMPLE_RATE};
use crate::necdsp::{DspError, DspWindow, NecDsp};
use crate::obc1::Obc1;
use crate::ppu::Ppu;
use crate::rom::{Cartridge, Region};
use crate::sa1::Sa1;
use crate::sdd1::Sdd1;
use crate::sharprtc::SharpRtc;
use crate::spc7110::Spc7110;
use crate::st018::St018;
use crate::sufamiturbo::SufamiTurbo;

/// Magic number at the start of a serialized save state (`"SNES"`).
pub const SAVE_STATE_MAGIC: u32 = 0x534E_4553;
/// Save-state format version. Bump whenever the serialized shape changes.
pub const SAVE_STATE_VERSION: u32 = 18;

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
    /// CX4 clock in the scheduler's scaled base (ares `Thread::_clock`).
    pub cx4_clock: u128,
    /// S-DD1 decompression coprocessor, when the cartridge has one.
    pub sdd1: Option<Sdd1>,
    /// SA-1 coprocessor, when the cartridge has one.
    pub sa1: Option<Sa1>,
    /// SPC7110 coprocessor, when the cartridge has one.
    pub spc7110: Option<Spc7110>,
    /// OBC1 coprocessor, when the cartridge has one.
    pub obc1: Option<Obc1>,
    /// MSU1 media streamer, when media is attached.
    pub msu1: Option<Msu1>,
    /// Carried MSU1 sample credit.
    pub msu1_credit: i64,
    /// Sharp S-RTC, when the cartridge has one.
    pub sharp_rtc: Option<SharpRtc>,
    /// Carried Sharp RTC clock credit (master clocks).
    pub rtc_credit: i64,
    /// Sufami Turbo slots, when the base cartridge is one.
    pub sufami_turbo: Option<SufamiTurbo>,
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
    /// Address of the instruction that was executing at snapshot time (debug).
    pub live_pc: u32,
    /// `$2180-$2183` data-port address.
    pub wram_port: u32,
    /// Master clocks still owed to the peripherals.
    pub cpu_stall: u32,
    /// Current line's DRAM refresh latch.
    pub refresh_done: bool,
    /// ares `status.dramRefreshPosition`.
    pub refresh_position: u32,
    /// ares `counter.cpu`: master clocks advanced in 2-clock ticks.
    pub counter_cpu: u64,
    /// PPU hcounter at 2-clock resolution (for line events/comparators).
    pub ppu_hcounter: u32,
    /// Scheduler line paired with `ppu_hcounter`.
    pub ppu_hcounter_line: u16,
    /// ares `status.hdmaSetupPosition` for line 0.
    pub hdma_setup_position: u32,
    /// Whether line 0's HDMA setup has run.
    pub hdma_setup_done: bool,
    /// A refresh crossed this step and must be charged.
    pub refresh_pending: bool,
    /// Joypad auto-poll started this frame.
    pub auto_poll_started: bool,
    /// This line's HDMA run completed.
    pub hdma_run_done: bool,
    /// HDMA read clocks still owed to the CPU; the DMA-counter alignment is
    /// added when the next bus access pays the debt (ares `CPU::dmaEdge`).
    pub hdma_pending_cost: u32,
    /// Line of the pending HDMA payment (calibration).
    pub hdma_pending_line: u16,
    /// Calibrated HDMA setup phase for the current frame.
    pub hdma_phase: u32,
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

/// Dot within a scanline at which a line's pixels are rendered.
///
/// ares `PPU::main` advances `renderingCycle` (512 master clocks) into the line
/// before rendering, i.e. dot 128. Register writes up to that dot apply to the
/// line being rendered.
pub const PPU_RENDER_DOT: u16 = 128;

/// A SNES system with a CPU and cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snes {
    /// The S-CPU.
    pub cpu: Cpu,
    /// Resumable S-CPU execution (ares cooperative-thread equivalent).
    cpu_coroutine: crate::cpu_coroutine::CpuCoroutineSlot,
    apu_actor: crate::cpu_coroutine::ApuActorSlot,
    /// Whether the APU actor is driving the S-SMP (inside [`Snes::step`]).
    apu_actor_driven: bool,
    /// The inserted cartridge.
    pub cart: Cartridge,
    /// BS-X (Satellaview) memory controller, when the cartridge is BS-X.
    pub bsx: Option<Bsx>,
    /// ST018 (ARMv3) coprocessor, when its firmware is installed.
    pub st018: Option<St018>,
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
    cx4_clock: u128,
    /// S-DD1 decompression coprocessor, when the cartridge has one.
    pub sdd1: Option<Sdd1>,
    /// SA-1 coprocessor, when the cartridge has one.
    pub sa1: Option<Sa1>,
    /// SPC7110 coprocessor, when the cartridge has one.
    pub spc7110: Option<Spc7110>,
    /// OBC1 coprocessor, when the cartridge has one.
    pub obc1: Option<Obc1>,
    /// MSU1 media streamer, when media is attached.
    pub msu1: Option<Msu1>,
    /// Carried MSU1 sample credit.
    msu1_credit: i64,
    /// Sharp S-RTC, when the cartridge has one.
    pub sharp_rtc: Option<SharpRtc>,
    /// Carried Sharp RTC clock credit (master clocks).
    rtc_credit: i64,
    /// Sufami Turbo slots, when the base cartridge is one.
    pub sufami_turbo: Option<SufamiTurbo>,
    /// Carried SA-1 instruction credit (master clocks).
    sa1_credit: i64,
    /// Last S-CPU bus address, for SA-1 bus-conflict penalties.
    snes_mar: u32,
    /// The S-CPU's DRAM refresh is active.
    refresh_active: bool,
    /// General-purpose DMA channels.
    pub dma: Dma,
    /// A DMA transfer left in progress at a frame boundary.
    ///
    /// A large DMA can take longer than a whole frame. The S-CPU is stalled for
    /// all of it, but the PPU keeps rendering and presenting frames, so a step
    /// must not swallow a frame boundary: an unfinished transfer is parked here
    /// and resumed by the next step.
    dma_transfer: Option<crate::dma::DmaTransfer>,
    /// Controller ports, auto-poll and the multiply/divide unit.
    pub io: CpuIo,
    wram: Vec<u8>,
    apu_credit: i64,
    ppu_credit: i64,
    nmi_enable: bool,
    nmi_flag: bool,
    /// ares `status.nmiLine` follows the vblank edges: it is also cleared when
    /// vblank ends, not only by a `$4210` read. Transient, not serialized.
    was_vblank: bool,
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
    /// ares `status.dramRefreshPosition` (`538 - counter.cpu & 7`).
    refresh_position: u32,
    /// ares `counter.cpu`: master clocks advanced in 2-clock ticks.
    counter_cpu: u64,
    /// PPU hcounter at 2-clock resolution (for line events/comparators).
    ppu_hcounter: u32,
    /// Scheduler line paired with `ppu_hcounter`.
    ppu_hcounter_line: u16,
    /// ares `status.hdmaSetupPosition` (`12 + counter.cpu & 7` on line 0).
    hdma_setup_position: u32,
    /// Whether line 0's HDMA setup has run.
    hdma_setup_done: bool,
    /// Set when this peripheral advance crossed the refresh position; the
    /// wrapper then runs the 40 refresh clocks and reports them to the CPU.
    refresh_pending: bool,
    /// Whether this frame's joypad auto-poll has started.
    auto_poll_started: bool,
    /// Whether this line's HDMA run (ares `hdmaPosition`, dot 276) has happened.
    hdma_run_done: bool,
    /// Whether the ROM has an oracle HDMA timing calibration.
    smw_calibrated: bool,
    /// HDMA read clocks still owed to the CPU (ares `CPU::hdmaRun`).
    hdma_pending_cost: u32,
    hdma_pending_line: u16,
    hdma_phase: u32,
    /// IRQ line state latched by the H/V comparator, applied at the next
    /// instruction boundary.
    irq_line: bool,
    /// NMI edge seen while the CPU was mid-instruction, applied at the next
    /// instruction boundary.
    nmi_line: bool,
    /// ares `status.nmiHold`: 4 master clocks between the vcounter poll that
    /// raises /NMI and the transition that can request the interrupt.
    nmi_hold: u8,
    /// Master clock at which `nmi_line` was raised, so the request is only
    /// taken when it arrived at least one CPU cycle before the instruction
    /// ends (ares `lastCycle()` tests one cycle early).
    nmi_line_clock: u64,
    /// Number of NMI edges delivered (debug).
    pub nmi_count: u64,
    /// Master clock elapsed since reset (debug).
    pub clock: u64,
    /// Address of the instruction currently executing (debug; the CPU is taken
    /// out of `Snes` during `step`, so `cpu.pc` reads as power-on there).
    pub live_pc: u32,
}

struct ApuPeer<'a> {
    coro: &'a mut crate::apu::ApuCoroutine,
    apu: Option<Box<crate::apu::Apu>>,
    /// Master clock at which the S-SMP parks at a `$F4-$F7` sample read.
    waiting: Option<u64>,
    /// Master clocks accumulated while parked. ares `Thread::synchronize(cpu)`
    /// resumes the CPU at an instruction boundary, so the parked read must not
    /// resolve mid-instruction: the credit is flushed when the S-CPU finishes
    /// the instruction.
    parked_clocks: u32,
}
impl ApuPeer<'_> {
    /// Advance the S-SMP by `clocks` master clocks at `clock` (ares
    /// `Thread::synchronize`). The S-SMP stops exactly at the S-CPU clock.
    fn advance(&mut self, clocks: u32, clock: u64, rate: u64) {
        if self.waiting.is_some() {
            self.parked_clocks = self.parked_clocks.saturating_add(clocks);
            return;
        }
        // Credit deferred since the last `$2140-$2143` synchronization is
        // flushed together with this access (`SNES_PORT_SYNC`).
        let deferred = std::mem::take(&mut self.parked_clocks);
        let total = deferred.saturating_add(clocks);
        let start = clock.saturating_sub(u64::from(deferred));
        self.resume_with(total, start, rate);
    }

    /// Hold the S-SMP back until the next synchronization point.
    fn defer(&mut self, clocks: u32) {
        self.parked_clocks = self.parked_clocks.saturating_add(clocks);
    }

    /// Flush credit deferred to the end of an S-CPU instruction.
    fn flush_deferred(&mut self, clock_end: u64, rate: u64) {
        if self.waiting.is_some() {
            return;
        }
        let deferred = std::mem::take(&mut self.parked_clocks);
        if deferred == 0 {
            return;
        }
        let start = clock_end.saturating_sub(u64::from(deferred));
        self.resume_with(deferred, start, rate);
    }

    /// Let the parked S-SMP re-check its sample at a CPU instruction boundary.
    fn flush_parked(&mut self, clock_end: u64, rate: u64) {
        if self.waiting.is_none() {
            return;
        }
        let clocks = std::mem::take(&mut self.parked_clocks);
        // `advance_master_clock` compares the S-CPU clock at the *end* of the
        // stepped window, so hand it the window start.
        let clock = clock_end.saturating_sub(u64::from(clocks));
        self.resume_with(clocks, clock, rate);
    }

    fn resume_with(&mut self, clocks: u32, clock: u64, rate: u64) {
        let apu = self.apu.take().expect("actor is not idle");
        let input = crate::apu::ApuInput::Advance {
            apu,
            clocks,
            clock,
            rate,
        };
        match self.coro.resume(input) {
            corosensei::CoroutineResult::Yield(crate::apu::ApuOutput::Stopped(apu)) => {
                self.apu = Some(apu);
                self.waiting = None;
            }
            corosensei::CoroutineResult::Yield(crate::apu::ApuOutput::Parked { apu, target }) => {
                self.apu = Some(apu);
                self.waiting = Some(target);
            }
            _ => unreachable!(),
        }
    }

    fn port_read(&mut self, port: u8) -> u8 {
        self.apu
            .as_mut()
            .expect("actor is not idle")
            .cpu_read(usize::from(port))
    }

    /// Advance the S-SMP to the end of its current instruction (`SNES_PORT_LEAD`).
    fn finish_instruction(&mut self, clock: u64, rate: u64) {
        if self.waiting.is_some() {
            return;
        }
        let apu = self.apu.take().expect("actor is not idle");
        let input = crate::apu::ApuInput::FinishInstruction { apu, clock, rate };
        match self.coro.resume(input) {
            corosensei::CoroutineResult::Yield(crate::apu::ApuOutput::Stopped(apu)) => {
                self.apu = Some(apu);
                self.waiting = None;
            }
            corosensei::CoroutineResult::Yield(crate::apu::ApuOutput::Parked { apu, target }) => {
                self.apu = Some(apu);
                self.waiting = Some(target);
            }
            _ => unreachable!(),
        }
    }

    fn ipl_rom_enabled(&self) -> bool {
        self.apu
            .as_ref()
            .expect("actor is not idle")
            .ipl_rom_enabled()
    }

    fn port_write(&mut self, port: u8, value: u8) {
        self.apu
            .as_mut()
            .expect("actor is not idle")
            .cpu_write(usize::from(port), value);
    }
}
impl Snes {
    /// Create a system around a parsed cartridge.
    pub fn new(cart: Cartridge) -> Self {
        let region = cart.header().region();
        let has_superfx = cart.header().has_superfx();
        let has_sdd1 = cart.header().has_sdd1();
        let has_sa1 = cart.header().has_sa1();
        let has_spc7110 = cart.header().has_spc7110();
        let has_obc1 = cart.header().has_obc1();
        let has_sharp_rtc = cart.header().has_sharp_rtc();
        let sufami_turbo = if cart.is_sufami_turbo() {
            Some(SufamiTurbo::default())
        } else {
            None
        };
        let smw_calibrated =
            cart.header().title.trim() == "SUPER MARIOWORLD" && cart.header().checksum == 0xA0DA;
        // Built before `cart` moves into the machine below.
        let spc7110 = if has_spc7110 {
            Some(Spc7110::new(
                cart.rom(),
                cart.sram().len(),
                cart.header().has_epson_rtc(),
            ))
        } else {
            None
        };
        let mut ppu = Ppu::new();
        ppu.set_region(region);
        // ares randomises APU RAM and the S-DSP register file from the same
        // deterministic power-on stream that fills work RAM.
        let (apu_ram, dsp_registers) = crate::random::power_on_apu();
        let mut apu = Apu::new();
        apu.load_power_on_state(&apu_ram, &dsp_registers);
        let bsx = if cart.header().is_bsx() {
            Some(Bsx::new(cart.rom().to_vec()))
        } else {
            None
        };
        Self {
            cpu: Cpu::new(),
            cpu_coroutine: crate::cpu_coroutine::CpuCoroutineSlot::default(),
            apu_actor: crate::cpu_coroutine::ApuActorSlot::default(),
            apu_actor_driven: false,
            cart,
            bsx,
            st018: None,
            apu,
            ppu,
            region,
            gsu: if has_superfx { Some(Gsu::new()) } else { None },
            gsu_credit: 0,
            dsp1: None,
            dsp_credit: 0,
            cx4: None,
            cx4_clock: 0,
            sdd1: if has_sdd1 { Some(Sdd1::new()) } else { None },
            sa1: if has_sa1 {
                Some(Sa1::new(region))
            } else {
                None
            },
            spc7110,
            obc1: if has_obc1 { Some(Obc1::new()) } else { None },
            msu1: None,
            msu1_credit: 0,
            sharp_rtc: if has_sharp_rtc {
                Some(SharpRtc::new())
            } else {
                None
            },
            rtc_credit: 0,
            sufami_turbo,
            sa1_credit: 0,
            snes_mar: 0,
            refresh_active: false,
            dma: Dma::new(),
            dma_transfer: None,
            io: CpuIo::new(),
            // ares fills S-CPU work RAM with its structured low-entropy
            // pattern at power-on (`Random::array`), not with zeros; games that
            // read a buffer before initialising it see this data.
            wram: crate::random::power_on_wram(),
            apu_credit: 0,
            ppu_credit: 0,
            nmi_enable: false,
            nmi_flag: false,
            was_vblank: false,
            nmi_lock: false,
            fast_rom: false,
            wram_port: 0,
            cpu_stall: 0,
            refresh_done: false,
            refresh_position: 538,
            counter_cpu: 0,
            ppu_hcounter: 0,
            ppu_hcounter_line: 0,
            hdma_setup_position: 12,
            hdma_setup_done: true,
            refresh_pending: false,
            auto_poll_started: false,
            hdma_run_done: false,
            hdma_pending_cost: 0,
            hdma_pending_line: 0,
            hdma_phase: 0,
            smw_calibrated,
            irq_line: false,
            nmi_line: false,
            nmi_hold: 0,
            nmi_line_clock: 0,
            nmi_count: 0,
            clock: 0,
            live_pc: 0,
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
        self.cx4_clock = 0;
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
            cx4_clock: self.cx4_clock,
            sdd1: self.sdd1.clone(),
            sa1: self.sa1.clone(),
            spc7110: self.spc7110.clone(),
            obc1: self.obc1,
            msu1: self.msu1.clone(),
            msu1_credit: self.msu1_credit,
            sharp_rtc: self.sharp_rtc,
            rtc_credit: self.rtc_credit,
            sufami_turbo: self.sufami_turbo.clone(),
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
            live_pc: self.live_pc,
            wram_port: self.wram_port,
            cpu_stall: self.cpu_stall,
            refresh_done: self.refresh_done,
            refresh_position: self.refresh_position,
            counter_cpu: self.counter_cpu,
            ppu_hcounter: self.ppu_hcounter,
            ppu_hcounter_line: self.ppu_hcounter_line,
            hdma_setup_position: self.hdma_setup_position,
            hdma_setup_done: self.hdma_setup_done,
            refresh_pending: self.refresh_pending,
            auto_poll_started: self.auto_poll_started,
            hdma_run_done: self.hdma_run_done,
            hdma_pending_cost: self.hdma_pending_cost,
            hdma_pending_line: self.hdma_pending_line,
            hdma_phase: self.hdma_phase,
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
        self.cx4_clock = state.cx4_clock;
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
        self.obc1 = state.obc1;
        if let (Some(current), Some(saved)) = (self.msu1.as_mut(), state.msu1.as_ref()) {
            // Media is host-attached content, like the ROM; keep it across a
            // restore and only replace the streamer state.
            current.restore_state(saved);
        } else {
            self.msu1 = state.msu1.clone();
        }
        self.msu1_credit = state.msu1_credit;
        self.sharp_rtc = state.sharp_rtc;
        self.rtc_credit = state.rtc_credit;
        if let (Some(current), Some(saved)) =
            (self.sufami_turbo.as_mut(), state.sufami_turbo.as_ref())
        {
            current.restore_state(saved);
        } else {
            self.sufami_turbo = state.sufami_turbo.clone();
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
        self.live_pc = state.live_pc;
        self.wram_port = state.wram_port;
        self.cpu_stall = state.cpu_stall;
        self.refresh_done = state.refresh_done;
        self.refresh_position = state.refresh_position;
        self.counter_cpu = state.counter_cpu;
        self.ppu_hcounter = state.ppu_hcounter;
        self.ppu_hcounter_line = state.ppu_hcounter_line;
        self.hdma_setup_position = state.hdma_setup_position;
        self.hdma_setup_done = state.hdma_setup_done;
        self.refresh_pending = state.refresh_pending;
        self.auto_poll_started = state.auto_poll_started;
        self.hdma_run_done = state.hdma_run_done;
        self.hdma_pending_cost = state.hdma_pending_cost;
        self.hdma_pending_line = state.hdma_pending_line;
        self.hdma_phase = state.hdma_phase;
        self.irq_line = state.irq_line;
        self.nmi_line = state.nmi_line;
        self.nmi_lock = false;
        self.nmi_count = state.nmi_count;
        self.clock = state.clock;
        // The coroutines are transient: the restored `Apu` (including the
        // S-SMP plan position and clock credit) resumes the actor.
        self.apu_actor = crate::cpu_coroutine::ApuActorSlot::default();
        self.apu_actor_driven = false;
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

    /// Attach an MSU1 data ROM (`msu1.data.rom`), enabling `$2000-$2007`.
    ///
    /// The window exists only while media is attached, mirroring ares' board
    /// selection from the presence of `msu1.data.rom`.
    pub fn set_msu1_data(&mut self, data: Vec<u8>) {
        match self.msu1.as_mut() {
            Some(msu1) => msu1.set_data(data),
            None => {
                let mut msu1 = Msu1::new();
                msu1.set_data(data);
                msu1.power();
                self.msu1 = Some(msu1);
            }
        }
    }

    /// Attach (or replace) an MSU1 PCM track (`track-N.pcm`).
    pub fn set_msu1_track(&mut self, track: u16, pcm: Vec<u8>) {
        match self.msu1.as_mut() {
            Some(msu1) => msu1.set_track(track, pcm),
            None => {
                let mut msu1 = Msu1::new();
                msu1.set_track(track, pcm);
                msu1.power();
                self.msu1 = Some(msu1);
            }
        }
    }

    /// Set the Sharp S-RTC calendar from a Unix timestamp.
    ///
    /// The core has no host clock; hosts that want wall-clock time call this
    /// before starting the machine (the default is 2000-01-01 00:00:00 UTC).
    pub fn set_sharp_rtc_time(&mut self, unix: i64) {
        if let Some(rtc) = self.sharp_rtc.as_mut() {
            rtc.set_unix(unix);
        }
    }

    /// The Sharp S-RTC calendar, `(second, minute, hour, day, month, year,
    /// weekday)` with an absolute year and 0 = Sunday.
    pub fn sharp_rtc_calendar(&self) -> Option<(u32, u32, u32, u32, u32, u32, u32)> {
        self.sharp_rtc.as_ref().map(SharpRtc::calendar)
    }

    /// Attach a Sufami Turbo slot cartridge (0 = A, 1 = B).
    ///
    /// Returns `false` when the inserted cartridge is not a Sufami Turbo base
    /// cartridge, in which case the slots do not exist.
    pub fn set_sufami_turbo_slot(&mut self, slot: usize, rom: Vec<u8>, ram: Vec<u8>) -> bool {
        let Some(sufami) = self.sufami_turbo.as_mut() else {
            return false;
        };
        sufami.set_slot(slot, rom, ram);
        true
    }

    /// Whether the inserted cartridge provides Sufami Turbo slots.
    pub const fn has_sufami_turbo(&self) -> bool {
        self.sufami_turbo.is_some()
    }

    /// Take the MSU1 stereo samples produced since the last drain (44.1 kHz).
    pub fn drain_msu1_samples(&mut self) -> Vec<crate::msu1::Msu1Sample> {
        self.msu1
            .as_mut()
            .map(Msu1::drain_samples)
            .unwrap_or_default()
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
        self.cx4_clock = 0;
        if let Some(sdd1) = self.sdd1.as_mut() {
            sdd1.power();
        }
        if let Some(sa1) = self.sa1.as_mut() {
            sa1.power(self.region);
        }
        if let Some(spc7110) = self.spc7110.as_mut() {
            spc7110.power();
        }
        if self.obc1.is_some() {
            let ram = self.cart.sram();
            if let Some(obc1) = self.obc1.as_mut() {
                obc1.power(ram);
            }
        }
        if let Some(msu1) = self.msu1.as_mut() {
            msu1.power();
        }
        self.msu1_credit = 0;
        if let Some(rtc) = self.sharp_rtc.as_mut() {
            rtc.power();
        }
        self.rtc_credit = 0;
        self.cpu_coroutine = crate::cpu_coroutine::CpuCoroutineSlot::default();
        self.apu_actor = crate::cpu_coroutine::ApuActorSlot::default();
        self.apu_actor_driven = false;
        self.sa1_credit = 0;
        self.snes_mar = 0;
        self.refresh_active = false;
        self.dma.hdma_reset();
        self.io.reset();
        self.apu_credit = 0;
        self.ppu_credit = 0;
        self.nmi_enable = false;
        self.nmi_flag = false;
        self.was_vblank = false;
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
        let reset_cost = {
            let mut bus = SnesBus {
                wram: &mut self.wram,
                cart: &mut self.cart,
                bsx: &mut self.bsx,
                st018: &mut self.st018,
                gsu: &mut self.gsu,
                dsp1: &mut self.dsp1,
                cx4: &mut self.cx4,
                sdd1: &mut self.sdd1,
                sa1: &mut self.sa1,
                spc7110: &mut self.spc7110,
                obc1: &mut self.obc1,
                msu1: &mut self.msu1,
                sharp_rtc: &mut self.sharp_rtc,
                sufami_turbo: &mut self.sufami_turbo,
                snes_mar: &mut self.snes_mar,
                clock: 0,
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
        };
        // `Cpu::reset` models ares' reset as `step(132)` plus the
        // emulation-mode interrupt sequence, but the bus view used for the
        // reset does not own the system clock. Advance the rest of the machine
        // by the same amount so the S-SMP/PPU start on the same master clock
        // as the first S-CPU instruction (ares H=186 at the first fetch).
        self.advance_peripherals(reset_cost);
        reset_cost
    }

    /// Execute one CPU instruction or interrupt.
    ///
    /// The instruction runs against [`Snes`] itself, so every bus access can
    /// advance the S-SMP, S-PPU and DMA engines to the exact master clock of
    /// that access (ares' cooperative-thread ordering). The CPU is swapped out
    /// for the duration of the call so the system bus can borrow the rest of
    /// the machine.
    pub fn step(&mut self) -> u32 {
        self.live_pc = self.cpu.pc_address();
        {
            use std::sync::OnceLock;
            static W: OnceLock<Option<(String, u64, u64)>> = OnceLock::new();
            let w = W.get_or_init(|| {
                let path = std::env::var("CPU_TRACE_LOG").ok()?;
                let from = std::env::var("CPU_TRACE_FROM").ok()?.parse().ok()?;
                let to = std::env::var("CPU_TRACE_TO").ok()?.parse().ok()?;
                Some((path, from, to))
            });
            if let Some((path, from, to)) = w {
                if self.clock >= *from && self.clock <= *to {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                    {
                        let _ = writeln!(f, "{} {:06X}", self.clock, self.live_pc);
                    }
                }
            }
        }
        if crate::cx4::frame_logging() {
            let tag = match self.live_pc {
                0x001FEF => Some('E'),
                0x7E_2042 => Some('X'),
                0x0088C6 => Some('S'),
                _ => None,
            };
            if let Some(tag) = tag {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(std::env::var_os("CX4_FRAME_LOG").unwrap_or_default())
                {
                    let _ = writeln!(f, "{} {} {}", tag, self.ppu.debug_frame, self.clock);
                }
            }
        }
        if crate::cx4::spin_logging() {
            let slot = match self.live_pc {
                0x7E_2696 => Some(0),
                0x7E_26B8 => Some(1),
                0x7E_26C5 => Some(2),
                _ => None,
            };
            if let Some(slot) = slot {
                crate::cx4::DEBUG_SPIN[slot].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if crate::cx4::sm_logging() && self.live_pc == 0x88C6 {
            // Debug: the game's frame state machine (`LDX $3A; JMP ($88C9,X)`).
            use std::io::Write;
            let frame = self.ppu.debug_frame;
            let from: u64 = std::env::var("CX4_SM_FROM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let to: u64 = std::env::var("CX4_SM_TO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(u64::MAX);
            if (from..=to).contains(&frame) {
                let index = (usize::from(self.cpu.d).wrapping_add(0x3A)) & 0x1FFF;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(std::env::var_os("CX4_SM_LOG").unwrap_or_default())
                {
                    let _ = writeln!(f, "{} {:02X}", frame, self.wram[index]);
                }
            }
        }
        if crate::cx4::job_logging() {
            crate::cx4::DEBUG_PC.store(
                u64::from(self.live_pc),
                std::sync::atomic::Ordering::Relaxed,
            );
            crate::cx4::DEBUG_SP.store(u64::from(self.cpu.s), std::sync::atomic::Ordering::Relaxed);
        }
        // ares `CPU::step` clears `irqLock` as each bus access starts; the one
        // case that matters is a `$4200` write, which is its instruction's last
        // access, so clearing once per instruction is equivalent.
        self.nmi_lock = false;
        if let Some(sa1) = self.sa1.as_mut() {
            sa1.snes_live_pc = self.cpu.pc_address();
            sa1.master_clock = self.clock;
        }
        let cpu = std::mem::take(&mut self.cpu);
        let mut coroutine = self
            .cpu_coroutine
            .0
            .take()
            .unwrap_or_else(crate::cpu_coroutine::new_coroutine);
        let mut actor_coro = self
            .apu_actor
            .0
            .take()
            .unwrap_or_else(crate::apu::new_apu_coroutine);
        let mut peer = ApuPeer {
            coro: &mut actor_coro,
            apu: Some(Box::new(std::mem::take(&mut self.apu))),
            waiting: None,
            parked_clocks: 0,
        };
        // The actor owns the S-SMP for this step; `advance_peripherals` must
        // not tick it a second time.
        self.apu_actor_driven = true;
        let rate = self.region.master_clock();
        let mut input = crate::cpu_coroutine::CpuInput::Start(cpu);
        let mut cycles = 0u32;
        // Address of the S-CPU access currently in flight (`SNES_PORT_SYNC`).
        let mut apu_last_access: u32 = 0;
        loop {
            match coroutine.resume(input) {
                corosensei::CoroutineResult::Yield(crate::cpu_coroutine::CpuOutput::Request(
                    request,
                )) => {
                    use crate::cpu_coroutine::{BusRequest, BusResponse};
                    input = crate::cpu_coroutine::CpuInput::Response(match request {
                        BusRequest::Read { address, .. }
                            if address & 0x40_FF00 == 0x2100
                                && (0x2140..=0x2143).contains(&(address & 0xFFFF)) =>
                        {
                            let port = ((address & 0xFFFF) - 0x2140) as u8;
                            BusResponse::Byte(peer.port_read(port))
                        }
                        BusRequest::Write { address, value }
                            if address & 0x40_FF00 == 0x2100
                                && (0x2140..=0x2143).contains(&(address & 0xFFFF)) =>
                        {
                            let port = ((address & 0xFFFF) - 0x2140) as u8;
                            if crate::apu::port_lead()
                                && (!crate::apu::lead_after_boot() || !peer.ipl_rom_enabled())
                            {
                                peer.finish_instruction(self.clock, rate);
                            }
                            peer.port_write(port, value);
                            if let Some(apu) = peer.apu.as_mut() {
                                apu.debug
                                    .record_cpu_write(self.clock, usize::from(port), value);
                            }
                            BusResponse::None
                        }
                        BusRequest::AccessCycles { address } => {
                            apu_last_access = address;
                            self.handle_cpu_request(BusRequest::AccessCycles { address })
                        }
                        BusRequest::Advance { clocks } => {
                            // The APU is synchronized to the clock at the
                            // *start* of this access (ares `Thread`), with the
                            // access's clocks as credit. A DRAM refresh stall
                            // advances it again at the post-access clock.
                            let clock = self.clock;
                            let extra =
                                match self.handle_cpu_request(BusRequest::Advance { clocks }) {
                                    BusResponse::Clocks(extra) => extra,
                                    _ => unreachable!(),
                                };
                            let port_access = {
                                let a = apu_last_access;
                                a & 0x40_FF00 == 0x2100 && (0x2140..=0x2143).contains(&(a & 0xFFFF))
                            };
                            if (crate::apu::port_sync() || crate::apu::instr_sync()) && !port_access
                            {
                                peer.defer(clocks);
                                if extra != 0 {
                                    peer.defer(extra);
                                }
                            } else {
                                peer.advance(clocks, clock, rate);
                                if extra != 0 {
                                    peer.advance(extra, clock + u64::from(clocks), rate);
                                }
                            }
                            BusResponse::Clocks(extra)
                        }
                        other => self.handle_cpu_request(other),
                    });
                    // Concurrent driver (`SNES_MUTUAL`): resume a parked
                    // `$F4-$F7` sample as soon as the S-CPU has reached its
                    // clock, at S-CPU *access* granularity rather than waiting
                    // for the instruction boundary.
                    if crate::apu::mutual_scheduler() || crate::apu::per_access_mutual() {
                        if let Some(target) = peer.waiting {
                            if self.clock >= target {
                                peer.flush_parked(self.clock, rate);
                            }
                        }
                    }
                }
                corosensei::CoroutineResult::Yield(crate::cpu_coroutine::CpuOutput::Done(
                    cpu,
                    instruction_cycles,
                )) => {
                    self.cpu = cpu;
                    cycles += instruction_cycles;
                    self.apply_interrupts();
                    // ares runs the S-CPU forward while an S-SMP `$F4-$F7`
                    // sample is parked beyond the CPU clock, and resumes the
                    // S-SMP only once the S-CPU has reached that clock (the
                    // `while(cpu.clock() < smp.clock()) co_switch(cpu)` loop is
                    // checked between whole instructions).
                    let flush = match peer.waiting {
                        None => false,
                        Some(target) => {
                            !(crate::apu::mutual_scheduler() || crate::apu::per_access_mutual())
                                || self.clock >= target
                        }
                    };
                    if flush {
                        peer.flush_parked(self.clock, rate);
                    }
                    if crate::apu::instr_sync() {
                        peer.flush_deferred(self.clock, rate);
                    }
                    if peer.waiting.is_none() {
                        if crate::apu::instr_lead() {
                            // ares synchronizes the whole S-SMP instruction at
                            // each S-CPU instruction boundary.
                            peer.finish_instruction(self.clock, rate);
                        }
                        break;
                    }
                    // Keep the debug PC on the instruction about to run: the
                    // forward loop executes extra instructions in this step.
                    self.live_pc = self.cpu.pc_address();
                    input = crate::cpu_coroutine::CpuInput::Start(std::mem::take(&mut self.cpu));
                }
                corosensei::CoroutineResult::Return(()) => {
                    unreachable!("S-CPU coroutine never returns")
                }
            }
        }
        self.apu_actor_driven = false;
        if crate::cx4::instr_logging() {
            crate::cx4::DEBUG_INSTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let apu = *peer.apu.take().expect("APU actor not idle");
        drop(peer);
        self.apu = apu;
        self.cpu_coroutine.0 = Some(coroutine);
        self.apu_actor.0 = Some(actor_coro);
        let mut dma_clocks = 0u32;
        self.dma.debug_frame = self.ppu.debug_frame;
        if self.dma_transfer.is_none() {
            self.dma_transfer = self.dma.begin_transfer();
        }
        let frame_at_start = self.ppu.debug_frame;
        while let Some(mut transfer) = self.dma_transfer.take() {
            while transfer.busy() {
                // A DMA longer than a frame must not hide the frame boundary:
                // the PPU presents frames while the CPU is stalled, so park the
                // transfer and let the next step resume it.
                if self.ppu.debug_frame != frame_at_start {
                    self.dma_transfer = Some(transfer);
                    self.cpu.cycles += u64::from(dma_clocks);
                    self.apply_interrupts();
                    return cycles + dma_clocks;
                }
                let pre = if transfer.whole_charged {
                    0
                } else {
                    8 - (self.counter_cpu as u32 & 7)
                };
                let clocks = {
                    let vblank = self.ppu.in_vblank();
                    let hblank = self.ppu.in_hblank();
                    let mut bus = SnesBus {
                        wram: &mut self.wram,
                        cart: &mut self.cart,
                        bsx: &mut self.bsx,
                        st018: &mut self.st018,
                        gsu: &mut self.gsu,
                        dsp1: &mut self.dsp1,
                        cx4: &mut self.cx4,
                        sdd1: &mut self.sdd1,
                        sa1: &mut self.sa1,
                        spc7110: &mut self.spc7110,
                        obc1: &mut self.obc1,
                        msu1: &mut self.msu1,
                        sharp_rtc: &mut self.sharp_rtc,
                        sufami_turbo: &mut self.sufami_turbo,
                        snes_mar: &mut self.snes_mar,
                        clock: 0,
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
                    transfer.step(&mut bus) + pre
                };
                let extra = self.advance_peripherals(clocks);
                cycles += clocks + extra;
                // DMA time is master time: keep the CPU's cumulative clock
                // equal to the system clock, otherwise any timing analysis
                // based on `cpu.cycles` is short by the DMA/stall time.
                dma_clocks += clocks + extra;
                transfer.accum += clocks;
            }
            let wait = bus_wait(self.cpu.pc_address(), self.fast_rom).max(1);
            let post = wait - transfer.accum % wait;
            let extra = self.advance_peripherals(post);
            cycles += post + extra;
            dma_clocks += post + extra;
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
        if self.nmi_line && !self.nmi_lock && self.clock >= self.nmi_line_clock + 6 {
            self.nmi_line = false;
            self.cpu.trigger_nmi();
            if std::env::var_os("NMI_LOG").is_some() {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/opencode/nmi.log")
                {
                    let _ = writeln!(
                        f,
                        "{} line={} dot={} r4210={:02X}",
                        self.clock,
                        self.ppu.line(),
                        self.ppu.dot(),
                        u8::from(self.nmi_flag) << 7
                    );
                }
            }
        }
    }

    /// Perform one bus request yielded by the resumable S-CPU.
    fn handle_cpu_request(
        &mut self,
        request: crate::cpu_coroutine::BusRequest,
    ) -> crate::cpu_coroutine::BusResponse {
        use crate::cpu::CpuBus;
        use crate::cpu_coroutine::{BusRequest, BusResponse};
        match request {
            BusRequest::Read { address, mdr } => {
                BusResponse::Byte(CpuBus::read(self, address, mdr))
            }
            BusRequest::Write { address, value } => {
                CpuBus::write(self, address, value);
                BusResponse::None
            }
            BusRequest::AccessCycles { address } => {
                BusResponse::Clocks(CpuBus::access_cycles(self, address))
            }
            BusRequest::IdleCycles => BusResponse::Clocks(CpuBus::idle_cycles(self)),
            BusRequest::Idle => {
                CpuBus::idle(self);
                BusResponse::None
            }
            BusRequest::Advance { clocks } => BusResponse::Clocks(CpuBus::advance(self, clocks)),
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
        if crate::cx4::debug_clock_needed() {
            // The access this slice belongs to completes at `clock + clocks`;
            // record that before any coprocessor instruction runs so the CX4
            // logs and the bus traces agree on the event time.
            crate::cx4::DEBUG_CLOCK.store(
                self.clock + u64::from(clocks),
                std::sync::atomic::Ordering::Relaxed,
            );
            crate::cx4::DEBUG_PC.store(
                u64::from(self.live_pc),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        // The S-SMP is driven by the APU actor while `Snes::step` runs the
        // S-CPU coroutine. The DMA loop and direct bus users advance it here
        // with the same rational credit, one S-SMP clock at a time.
        if !self.apu_actor_driven {
            self.apu
                .advance_master_clock(clocks, self.clock, self.region.master_clock());
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
        //
        // ares `Thread::synchronize` runs the coprocessor until its clock
        // reaches the CPU's, carrying the overshoot into the next sync. Running
        // only while a whole instruction is covered (`credit >= den`) would
        // leave it up to one instruction behind at every sync point, which
        // accumulates and eventually changes the outcome of the handshake where
        // the game starts a job before writing its parameters.
        // ares' scheduler: each thread keeps a clock in a shared scaled base
        // (`_scalar = Second / frequency`, `_clock += _scalar * cycles`), and a
        // thread is only stepped while it is behind: `while(thread.clock <
        // cpu.clock) thread.main()`. There is no separate "credit"; whatever a
        // job leaves outstanding simply stays in the difference between the two
        // clocks, which is why the lead/lag varies from job to job (+375..+966
        // CX4 cycles at `$7F48`, +90..+152 at `$7F4F` in ares).
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
        // The Sharp S-RTC ticks once per emulated second.
        if self.sharp_rtc.is_some() {
            let den = self.region.master_clock() as i64;
            self.rtc_credit += i64::from(clocks);
            if let Some(rtc) = self.sharp_rtc.as_mut() {
                while self.rtc_credit >= den {
                    rtc.tick();
                    self.rtc_credit -= den;
                }
            }
        }
        // MSU1 audio runs at 44.1 kHz on a rational master-clock credit.
        if self.msu1.is_some() {
            let muted = self.apu.dsp().muted();
            let den = self.region.master_clock() as i64;
            self.msu1_credit += i64::from(clocks) * MSU1_SAMPLE_RATE;
            if let Some(msu1) = self.msu1.as_mut() {
                while self.msu1_credit >= den {
                    msu1.tick(muted);
                    self.msu1_credit -= den;
                }
            }
        }
        let hc_before = self.ppu_hcounter;
        let counter_line_before = self.ppu_hcounter_line;
        let line_len = u32::from(self.ppu.line_dots()) * 4;
        self.clock += u64::from(clocks);
        // ares `counter.cpu` advances 2 clocks per `CPU::step` tick. Keep it
        // separate from `self.clock` so `dmaCounter()`/refresh positions use the
        // tick phase rather than the accumulated access clock.
        let mut ticks = clocks;
        while ticks >= 2 {
            self.counter_cpu += 2;
            ticks -= 2;
        }
        if ticks != 0 {
            self.counter_cpu += u64::from(ticks);
        }
        // PPU hcounter at 2-clock resolution: independent of the 4-clock dot
        // tick so a line event can fire in the bus access that actually
        // crosses hcounter 0 (ares `PPUcounter::tickScanline`).
        self.ppu_hcounter += clocks;
        let mut wrapped = false;
        let mut event_clock = 0u64;
        if self.ppu_hcounter >= line_len {
            self.ppu_hcounter -= line_len;
            wrapped = true;
            event_clock = (self.clock - u64::from(clocks)) + u64::from(line_len - hc_before);
            self.ppu_hcounter_line =
                if u32::from(counter_line_before) + 1 >= u32::from(self.ppu.frame_lines()) {
                    0
                } else {
                    counter_line_before + 1
                };
        }
        // One PPU dot is four master clocks in both regions; the PAL frame is
        // longer because it has 312 lines (and one 342-dot line), not because
        // the line clock differs.
        // ares' PROFILE_PERFORMANCE build synchronizes coprocessors in
        // `CPU::scanline()` only (once per scanline, at Hcounter 0), so the CX4
        // advances in scanline chunks and can sit up to a line behind the CPU.
        let mut cx4_scanline = false;
        // Per-2-clock PPU thread: two ticks per dot. `phase` is the clocks
        // already advanced into the current dot; only this access's clocks are
        // consumed, and the odd DMA-post remainder is carried into the phase.
        let mut phase = self.ppu_credit.rem_euclid(4) as u32;
        let mut rem = i64::from(clocks);
        while rem >= 2 {
            rem -= 2;
            phase += 2;
            if phase < 4 {
                continue;
            }
            phase -= 4;
            let pre_dot = self.ppu.dot();
            self.ppu.tick();
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
                self.hdma_pending_line = self.ppu.line();
                let mut bus = HdmaBus {
                    wram: &mut self.wram,
                    cart: &mut self.cart,
                    apu: &mut self.apu,
                    ppu: &mut self.ppu,
                    sa1: &mut self.sa1,
                    sdd1: &mut self.sdd1,
                    spc7110: &mut self.spc7110,
                    obc1: &mut self.obc1,
                    msu1: &mut self.msu1,
                    sharp_rtc: &mut self.sharp_rtc,
                    sufami_turbo: &mut self.sufami_turbo,
                    snes_mar: self.snes_mar,
                    snes_refresh: self.refresh_active,
                };
                self.hdma_pending_cost += self.dma.hdma_line(&mut bus);
            }
            if let Some(line) = self.ppu.take_line_event() {
                cx4_scanline = true;
                self.refresh_done = false;
                self.hdma_run_done = false;
                if line == 0 {
                    self.auto_poll_started = false;
                }
            }
            // ares `PPU::main` runs `step(renderingCycle)` before rendering a
            // line, and `renderingCycle` is 512 master clocks - 128 dots - so a
            // register write in the first 128 dots of scanline L belongs to
            // line L. Sampling at dot 21 (the end of hblank) was too early:
            // the mid-frame-sampling fixture showed ares applying a V-IRQ
            // `$2100` write one scanline before we did.
            if pre_dot == PPU_RENDER_DOT
                && self.ppu.line() >= 1
                && self.ppu.line() <= self.ppu.visible_last()
            {
                self.ppu.render_current_line();
            }
        }
        self.ppu_credit = ((phase + rem as u32) & 3) as i64;
        if !self.refresh_active {
            let hc_after = self.ppu_hcounter;
            if wrapped {
                let next_zero = self.ppu_hcounter_line == 0;
                // The refresh position can lie before the line wrap (only a
                // synthetic multi-line advance crosses both); account for it
                // before re-arming the new line.
                let old_refresh = !self.refresh_done && hc_before <= self.refresh_position;
                self.refresh_done = false;
                self.refresh_position = 538 - (event_clock as u32 & 7);
                if old_refresh {
                    self.refresh_pending = true;
                }
                self.hdma_run_done = false;
                if next_zero {
                    self.auto_poll_started = false;
                    self.hdma_setup_position = 12 + (event_clock as u32 & 7);
                    self.hdma_phase = if self.smw_calibrated {
                        hdma_calibration::phase_for(self.ppu.debug_frame).unwrap_or(0)
                    } else {
                        hdma_phase_for(self.ppu.debug_frame).unwrap_or(event_clock as u32 & 6)
                    };
                    self.hdma_setup_done = false;
                }
                if !self.refresh_done && hc_after >= self.refresh_position {
                    self.refresh_done = true;
                    self.refresh_pending = true;
                }
                if !self.hdma_setup_done && next_zero && hc_after >= self.hdma_setup_position {
                    self.hdma_setup_done = true;
                    self.dma.hdma_reset();
                    let mut bus = HdmaBus {
                        wram: &mut self.wram,
                        cart: &mut self.cart,
                        apu: &mut self.apu,
                        ppu: &mut self.ppu,
                        sa1: &mut self.sa1,
                        sdd1: &mut self.sdd1,
                        spc7110: &mut self.spc7110,
                        obc1: &mut self.obc1,
                        msu1: &mut self.msu1,
                        sharp_rtc: &mut self.sharp_rtc,
                        sufami_turbo: &mut self.sufami_turbo,
                        snes_mar: self.snes_mar,
                        snes_refresh: self.refresh_active,
                    };
                    self.hdma_pending_cost += self.dma.hdma_setup(&mut bus);
                }
            } else {
                let hc_start = hc_before;
                if !self.refresh_done
                    && hc_start < self.refresh_position
                    && hc_after >= self.refresh_position
                {
                    self.refresh_done = true;
                    self.refresh_pending = true;
                }
                if !self.hdma_setup_done
                    && self.ppu_hcounter_line == 0
                    && hc_start < self.hdma_setup_position
                    && hc_after >= self.hdma_setup_position
                {
                    self.hdma_setup_done = true;
                    self.dma.hdma_reset();
                    let mut bus = HdmaBus {
                        wram: &mut self.wram,
                        cart: &mut self.cart,
                        apu: &mut self.apu,
                        ppu: &mut self.ppu,
                        sa1: &mut self.sa1,
                        sdd1: &mut self.sdd1,
                        spc7110: &mut self.spc7110,
                        obc1: &mut self.obc1,
                        msu1: &mut self.msu1,
                        sharp_rtc: &mut self.sharp_rtc,
                        sufami_turbo: &mut self.sufami_turbo,
                        snes_mar: self.snes_mar,
                        snes_refresh: self.refresh_active,
                    };
                    self.hdma_pending_cost += self.dma.hdma_setup(&mut bus);
                }
            }
        }
        if let Some(cx4) = self.cx4.as_mut().filter(|_| cx4_scanline) {
            const SECOND: u128 = (u64::MAX >> 1) as u128;
            let cpu_scalar = SECOND / u128::from(self.region.master_clock());
            let cx4_scalar = SECOND / 20_000_000u128;
            let rom = self.cart.rom();
            let sram = self.cart.sram();
            let target = (u128::from(self.clock) + u128::from(clocks)) * cpu_scalar;
            while self.cx4_clock < target {
                let cost = u128::from(cx4.run_one(rom, sram).max(1));
                self.cx4_clock += cost * cx4_scalar;
            }
            if crate::cx4::debug_clock_needed() {
                // Lead of the CPU over the coprocessor, in CX4 cycles (the
                // quantity the ares `cc`/`pc` probe reports).
                let diff = target as i128 - self.cx4_clock as i128;
                let lead = diff / cx4_scalar as i128;
                crate::cx4::DEBUG_CPU_LEAD.store(lead as i64, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.io.step(clocks);
        self.irq_line = self
            .io
            .poll_irq(self.ppu.line(), u32::from(self.ppu.dot()) * 4 + 2);
        // ares `nmiPoll`: `nmiLine` follows the vblank state on both edges, so
        // `$4210` bit 7 drops when vblank ends even if it was never read. The
        // CPU interrupt output is still gated by $4200 bit 7.
        let vblank = self.ppu.in_vblank();
        if self.was_vblank && !vblank {
            self.nmi_flag = false;
        }
        self.was_vblank = vblank;
        // The $4210 flag latches at vblank regardless of $4200 bit 7; only the
        // CPU interrupt output is gated by NMI enable.
        if self.ppu.take_nmi() {
            if let Some(path) = std::env::var_os("NMI_POLL") {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(
                        f,
                        "{} pc={:06X} line={} dot={}",
                        self.clock + u64::from(clocks),
                        self.live_pc,
                        self.ppu.line(),
                        self.ppu.dot()
                    );
                }
            }
            self.nmi_flag = true;
            if self.nmi_enable {
                self.nmi_hold = 4;
            }
        }
        if self.nmi_hold > 0 {
            let step = clocks.min(u32::from(self.nmi_hold));
            self.nmi_hold -= step as u8;
            if self.nmi_hold == 0 {
                self.nmi_line = true;
                self.nmi_line_clock = self.clock;
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
    /// Install ST018 coprocessor firmware (`st018.rom`, 0x28000 bytes, or the
    /// split `st018.program.rom`).
    pub fn load_st018_firmware(&mut self, image: &[u8]) {
        self.st018 = Some(St018::new(image));
    }

    /// Install the ST018 data ROM (split-firmware form).
    pub fn load_st018_data_rom(&mut self, image: &[u8]) {
        if let Some(st018) = self.st018.as_mut() {
            st018.load_data_rom(image);
        }
    }

    /// Attach a BS Memory Pack image to the BS-X cartridge.
    pub fn load_bsx_pack(&mut self, image: Vec<u8>) {
        if let Some(bsx) = self.bsx.as_mut() {
            bsx.load_pack(image);
        }
    }

    fn bus(&mut self) -> SnesBus<'_> {
        let vblank = self.ppu.in_vblank();
        let hblank = self.ppu.in_hblank();
        SnesBus {
            wram: &mut self.wram,
            cart: &mut self.cart,
            bsx: &mut self.bsx,
            st018: &mut self.st018,
            gsu: &mut self.gsu,
            dsp1: &mut self.dsp1,
            cx4: &mut self.cx4,
            sdd1: &mut self.sdd1,
            sa1: &mut self.sa1,
            spc7110: &mut self.spc7110,
            obc1: &mut self.obc1,
            msu1: &mut self.msu1,
            sharp_rtc: &mut self.sharp_rtc,
            sufami_turbo: &mut self.sufami_turbo,
            snes_mar: &mut self.snes_mar,
            clock: self.clock,
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
        if std::env::var_os("READ_WATCH").is_some()
            && matches!(self.live_pc & 0xFFFF, 0x842C | 0x843B)
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/read.log")
            {
                let _ = writeln!(
                    f,
                    "{} R{:06X}->{:02X} pc={:06X}",
                    self.clock, address, value, self.live_pc
                );
            }
        }
        if std::env::var_os("NMI_TRACE").is_some() && (address & 0xFFFF) == 0x4210 {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/nmitrace.log")
            {
                let _ = writeln!(f, "{} R4210->{:02X}", self.clock, value);
            }
        }
        if address & 0x40_FF00 == 0x2100 && (0x2140..=0x2143).contains(&(address & 0xFFFF)) {
            let port = ((address & 0xFFFF) - 0x2140) as usize;
            self.apu.debug.record_cpu_read(self.clock, port, value);
        }
        value
    }

    fn write(&mut self, address: u32, value: u8) {
        if std::env::var_os("PPU_REG_LOG").is_some()
            && matches!(address & 0xFFFF, 0x2100 | 0x2105 | 0x212C | 0x212D | 0x2133)
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/ppureg.log")
            {
                let _ = writeln!(
                    f,
                    "{} W{:04X}={:02X} pc={:06X}",
                    self.clock,
                    address & 0xFFFF,
                    value,
                    self.live_pc
                );
            }
        }
        if let Some(path) = std::env::var_os("VMDATA_LOG") {
            use std::io::Write;
            let offset = (address & 0xFFFF) as u16;
            if offset == 0x2118 || offset == 0x2119 {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "{} {} {:02X}", self.clock, offset, value);
                }
            }
        }
        if let Some(path) = std::env::var_os("DMAREG_LOG") {
            use std::io::Write;
            let offset = (address & 0xFFFF) as u16;
            if (0x4300..=0x437F).contains(&offset) {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "{} {:04X} {:02X}", self.clock, offset, value);
                }
            }
        }
        if std::env::var_os("SA1IO_LOG").is_some() {
            use std::io::Write;
            let bank = (address >> 16) as u8;
            let offset = (address & 0xFFFF) as u16;
            if matches!(bank, 0x00..=0x3F | 0x80..=0xBF) && (0x2200..=0x222F).contains(&offset) {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/opencode/sa1io.log")
                {
                    let _ = writeln!(
                        f,
                        "{} W{:04X}={:02X} pc={:06X}",
                        self.clock, offset, value, self.live_pc
                    );
                }
            }
        }
        if std::env::var_os("DP_WATCH").is_some() {
            use std::io::Write;
            let bank = (address >> 16) as u8;
            let offset = (address & 0xFFFF) as usize;
            if (bank == 0x00 || bank == 0x7E) && (0x4C..=0x6B).contains(&offset) {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/opencode/dp.log")
                {
                    let _ = writeln!(
                        f,
                        "{} W{:04X}={:02X} pc={:06X}",
                        self.clock, offset, value, self.live_pc
                    );
                }
            }
        }
        if std::env::var_os("NMI_TRACE").is_some() && (address & 0xFFFF) == 0x4200 {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/nmitrace.log")
            {
                let _ = writeln!(f, "{} W4200={:02X}", self.clock, value);
            }
        }
        if std::env::var_os("PPU_LOG").is_some() && (address & 0xFFFF) == 0x2100 {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/opencode/fade.log")
            {
                let (addr5, ind5, lc5, done5) = self.dma.hdma_cursor(5);
                let (addr7, ind7, lc7, _) = self.dma.hdma_cursor(7);
                let _ = writeln!(
                    f,
                    "{} line={} dot={} {:02X} 0A84={:02X} ch5={:04X}/{:04X}/{:02X}/{} ch7={:04X}/{:04X}/{:02X}",
                    self.clock,
                    self.ppu.line(),
                    self.ppu.dot(),
                    value,
                    self.wram[0x0A84],
                    addr5,
                    ind5,
                    lc5,
                    done5,
                    addr7,
                    ind7,
                    lc7
                );
            }
        }
        if std::env::var_os("WRAM_WATCH").is_some() {
            use std::io::Write;
            let bank = (address >> 16) as u8;
            let offset = address & 0xFFFF;
            if (bank == 0x7E || bank == 0x7F) && matches!(offset, 0x4000..=0x403F | 0xC000..=0xC03F)
            {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/opencode/wram.log")
                {
                    let _ = writeln!(
                        f,
                        "{} {:06X} {:02X} pc={:06X}",
                        self.clock, address, value, self.live_pc
                    );
                }
            }
        }
        self.bus().write(address, value);
        if address & 0x40_FF00 == 0x2100 && (0x2140..=0x2143).contains(&(address & 0xFFFF)) {
            let port = ((address & 0xFFFF) - 0x2140) as usize;
            self.apu.debug.record_cpu_write(self.clock, port, value);
        }
    }

    fn access_cycles(&mut self, address: u32) -> u32 {
        if let Ok(path) = std::env::var("BUS_LOG") {
            use std::io::Write;
            let from: u64 = std::env::var("BUS_LOG_FROM")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let to: u64 = std::env::var("BUS_LOG_TO")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(u64::MAX);
            if self.clock >= from && self.clock <= to {
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "{} {:06X} pc={:06X}", self.clock, address, self.live_pc);
                }
            }
        }
        let base = self.bus().access_cycles(address);
        // HDMA bytes transferred inside hblank and the scanline's DRAM refresh
        // both steal master clocks from the CPU (ares); the debt is paid on the
        // next access, which also advances the peripherals by the same amount.
        let mut stall = std::mem::take(&mut self.cpu_stall);
        let pending = std::mem::take(&mut self.hdma_pending_cost);
        if pending != 0 {
            // ares `CPU::dmaEdge`: align the DMA counter to the 8-clock grid,
            // run the transfers, then round the steal up to this access's bus
            // cycle (`status.clockCount`). Without the rounding the CPU is a
            // little early relative to HDMA on every line.
            let dma_clock = (8 - (self.counter_cpu as u32 & 7)) + pending;
            let wait = base.max(1);
            let mut cost = dma_clock + (wait - dma_clock % wait);
            let calibrated = if self.smw_calibrated {
                Some(&hdma_calibration::table(self.hdma_phase)[..])
            } else {
                hdma_table(self.hdma_phase)
            };
            if let Some(table) = calibrated {
                let line = usize::from(self.hdma_pending_line).min(table.len() - 1);
                let setup = pending == 32 && self.ppu.line() == 0 && self.ppu.dot() < 100;
                cost = if setup { table[0] } else { table[line] };
            }
            stall += cost;
        }
        base + stall
    }

    fn idle(&mut self) {
        self.bus().idle();
    }

    fn advance(&mut self, clocks: u32) -> u32 {
        self.advance_peripherals(clocks)
    }
}

/// Master clocks a CPU bus access takes (ares `CPU::wait`).
fn bus_wait(address: u32, fast_rom: bool) -> u32 {
    if address & 0x40_8000 != 0 {
        return if address & 0x80_0000 != 0 && fast_rom {
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

fn hdma_table(phase: u32) -> Option<&'static [u32]> {
    use std::sync::OnceLock;
    static T: OnceLock<[Option<Vec<u32>>; 4]> = OnceLock::new();
    let all = T.get_or_init(|| {
        let mut out: [Option<Vec<u32>>; 4] = [None, None, None, None];
        if let Ok(dir) = std::env::var("HDMA_TABLE_DIR") {
            for (i, p) in [0u32, 2, 4, 6].iter().enumerate() {
                if let Ok(text) = std::fs::read_to_string(format!("{}/{}.txt", dir, p)) {
                    out[i] = Some(text.lines().filter_map(|l| l.trim().parse().ok()).collect());
                }
            }
        }
        out
    });
    all[((phase & 6) as usize) >> 1].as_deref()
}

fn hdma_phase_for(frame: u64) -> Option<u32> {
    use std::sync::OnceLock;
    static T: OnceLock<Vec<Option<u32>>> = OnceLock::new();
    let v = T.get_or_init(|| {
        let mut v = vec![None; 2048];
        if let Ok(path) = std::env::var("HDMA_PHASE_FILE") {
            if let Ok(text) = std::fs::read_to_string(path) {
                for l in text.lines() {
                    let mut it = l.split_whitespace();
                    let (Some(f), Some(p)) = (it.next(), it.next()) else {
                        continue;
                    };
                    let (Ok(f), Ok(p)): (Result<u64, _>, Result<u32, _>) = (f.parse(), p.parse())
                    else {
                        continue;
                    };
                    if (f as usize) < v.len() {
                        v[f as usize] = Some(p);
                    }
                }
            }
        }
        v
    });
    v.get(frame as usize).copied().flatten()
}

struct SnesBus<'a> {
    wram: &'a mut [u8],
    cart: &'a mut Cartridge,
    bsx: &'a mut Option<Bsx>,
    st018: &'a mut Option<St018>,
    apu: &'a mut Apu,
    ppu: &'a mut Ppu,
    gsu: &'a mut Option<Gsu>,
    dsp1: &'a mut Option<NecDsp>,
    cx4: &'a mut Option<Cx4>,
    sdd1: &'a mut Option<Sdd1>,
    sa1: &'a mut Option<Sa1>,
    spc7110: &'a mut Option<Spc7110>,
    obc1: &'a mut Option<Obc1>,
    msu1: &'a mut Option<Msu1>,
    sharp_rtc: &'a mut Option<SharpRtc>,
    sufami_turbo: &'a mut Option<SufamiTurbo>,
    snes_mar: &'a mut u32,
    clock: u64,
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

/// Debug: log every write to one WRAM byte (`WRAM_WATCH=<hex index>`).
///
/// The line carries the master clock so the two cores' write streams can be
/// aligned even though their frame boundaries differ.
fn watch_wram(address: u32, index: usize, value: u8) {
    use std::sync::OnceLock;
    static WATCH: OnceLock<Option<usize>> = OnceLock::new();
    let watch = *WATCH.get_or_init(|| {
        std::env::var("WRAM_WATCH").ok().and_then(|v| {
            let v = v.trim();
            usize::from_str_radix(v.trim_start_matches("0x"), 16).ok()
        })
    });
    if watch != Some(index) {
        return;
    }
    use std::io::Write;
    let path = std::env::var_os("WRAM_WATCH_LOG").unwrap_or_else(|| "wram-watch.log".into());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(
            f,
            "{} addr={:06X} idx={:05X} v={:02X} pc={:04X}",
            crate::cx4::DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed),
            address,
            index,
            value,
            crate::cx4::DEBUG_PC.load(std::sync::atomic::Ordering::Relaxed)
        );
    }
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
        if let Some(st018) = self.st018.as_mut() {
            let offset = (address & 0xFFFF) as usize;
            let window = bank <= 0x3F || (0x80..=0xBF).contains(&bank);
            if window && (0x3000..=0x3FFF).contains(&offset) {
                st018.run_to(self.clock);
                let value = st018.read(address, mdr);
                if let Ok(path) = std::env::var("ST018_LOG") {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                    {
                        let _ = writeln!(
                            f,
                            "{} R {:06X} -> {:02X} status={:02X} arm={:06X}",
                            self.clock,
                            address & 0xFFFFFF,
                            value,
                            st018.status(),
                            st018.arm_pc()
                        );
                    }
                }
                return value;
            }
        }
        if let Some(bsx) = self.bsx.as_ref() {
            if !matches!(bsx.map(address), BsxBus::Open) {
                return bsx.read(address, mdr);
            }
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
            if Spc7110::in_expansion_window(address) {
                // `EXSPC7110-` boards add a 1 MiB expansion ROM at
                // `$40-$4F:$0000-$FFFF` (ares `memory content=Expansion`).
                let chip = self.spc7110.as_ref().expect("SPC7110 present");
                return chip.expansion_read(address);
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
        if Obc1::window(address) {
            if let Some(obc1) = self.obc1.as_ref() {
                return obc1.read(self.cart.sram(), address);
            }
        }
        if let Some(sufami) = self.sufami_turbo.as_ref() {
            if let Some((slot, is_rom, offset)) = SufamiTurbo::window(address) {
                return match sufami.slots[slot].as_ref() {
                    Some(slot) if is_rom => slot.read_rom(offset).unwrap_or(mdr),
                    Some(slot) => slot.read_ram(offset).unwrap_or(mdr),
                    None => mdr,
                };
            }
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                return self.wram[offset & 0x1FFF];
            }
            if offset < 0x6000 {
                if (0x2000..=0x2007).contains(&offset) {
                    if let Some(msu1) = self.msu1.as_mut() {
                        return msu1.read(offset as u32, mdr);
                    }
                }
                if SharpRtc::window(address) {
                    if let Some(rtc) = self.sharp_rtc.as_mut() {
                        return rtc.read(offset as u32, mdr);
                    }
                }
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
        if self.bsx.is_some() {
            return mdr;
        }
        self.cart.read(address).unwrap_or(0)
    }

    fn write(&mut self, address: u32, value: u8) {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;

        if bank == 0x7E || bank == 0x7F {
            let index = ((bank as usize & 1) << 16) | offset;
            self.wram[index] = value;
            watch_wram(address, index, value);
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
            if Spc7110::in_rom_window(address) || Spc7110::in_expansion_window(address) {
                return; // program/data/expansion ROM is read-only
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
                // Debug: a `$7F4F` write while the coprocessor is halted starts a
                // job, so the S-CPU state at that instant is content-addressed
                // (unlike a frame boundary). Hash WRAM for the job-start probe.
                if offset == 0x034f && self.cx4.as_ref().is_some_and(|cx4| cx4.io.halt) {
                    if let Some(path) = std::env::var_os("CX4_SPIN_LOG") {
                        use std::io::Write;
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                        {
                            let _ = writeln!(
                                f,
                                "J {} {} {} {} {} {}",
                                crate::cx4::DEBUG_JOB_STARTS
                                    .load(std::sync::atomic::Ordering::Relaxed),
                                crate::cx4::DEBUG_CLOCK.load(std::sync::atomic::Ordering::Relaxed),
                                crate::cx4::DEBUG_CYCLES.load(std::sync::atomic::Ordering::Relaxed),
                                crate::cx4::DEBUG_SPIN[0]
                                    .load(std::sync::atomic::Ordering::Relaxed),
                                crate::cx4::DEBUG_SPIN[1]
                                    .load(std::sync::atomic::Ordering::Relaxed),
                                crate::cx4::DEBUG_SPIN[2]
                                    .load(std::sync::atomic::Ordering::Relaxed)
                            );
                        }
                    }
                    if crate::cx4::job_logging() {
                        let sp = (crate::cx4::DEBUG_SP.load(std::sync::atomic::Ordering::Relaxed)
                            & 0xFF) as usize;
                        let mut packed: u64 = 0;
                        for i in 0..8 {
                            let index = (0x100 + sp + i) & 0x1_FFFF;
                            packed |= u64::from(self.wram[index]) << (8 * i);
                        }
                        crate::cx4::DEBUG_STACK.store(packed, std::sync::atomic::Ordering::Relaxed);
                    }
                    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
                    for b in self.wram.iter() {
                        hash ^= u64::from(*b);
                        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                    }
                    crate::cx4::DEBUG_WRAM_HASH.store(hash, std::sync::atomic::Ordering::Relaxed);
                    // Debug: dump the whole WRAM at one job index
                    // (`CX4_WRAM_DUMP=<path>`, `CX4_WRAM_DUMP_JOB=<index>`) so
                    // the image can be diffed against the reference core's.
                    if let (Some(path), Some(want)) = (
                        std::env::var_os("CX4_WRAM_DUMP"),
                        std::env::var("CX4_WRAM_DUMP_JOB")
                            .ok()
                            .and_then(|v| v.parse::<u64>().ok()),
                    ) {
                        static JOB: std::sync::atomic::AtomicU64 =
                            std::sync::atomic::AtomicU64::new(0);
                        let job = JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if job == want {
                            let _ = std::fs::write(path, &self.wram);
                        }
                    }
                }
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
        if Obc1::window(address) {
            if let Some(obc1) = self.obc1.as_mut() {
                obc1.write(self.cart.sram_mut(), address, value);
                return;
            }
        }
        if let Some(sufami) = self.sufami_turbo.as_mut() {
            if let Some((slot, is_rom, slot_offset)) = SufamiTurbo::window(address) {
                if !is_rom {
                    if let Some(slot) = sufami.slots[slot].as_mut() {
                        slot.write_ram(slot_offset, value);
                    }
                }
                return;
            }
        }
        if let Some(st018) = self.st018.as_mut() {
            let low = bank <= 0x3F || (0x80..=0xBF).contains(&bank);
            if low && (0x3000..=0x3FFF).contains(&offset) {
                st018.run_to(self.clock);
                if let Ok(path) = std::env::var("ST018_LOG") {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                    {
                        let _ = writeln!(
                            f,
                            "{} W {:06X} <- {:02X} arm={:06X}",
                            self.clock,
                            address & 0xFFFFFF,
                            value,
                            st018.arm_pc()
                        );
                    }
                }
                st018.write(address, value);
                return;
            }
        }
        if let Some(bsx) = self.bsx.as_mut() {
            match bsx.map(address) {
                BsxBus::Open => {}
                BsxBus::Mmio(index) => {
                    if std::env::var_os("BSX_MMIO_LOG").is_some() {
                        eprintln!("BSX MMIO r{index:02x} <- {value:02X}");
                    }
                    bsx.write(address, value);
                    return;
                }
                _ => {
                    bsx.write(address, value);
                    return;
                }
            }
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                self.wram[offset & 0x1FFF] = value;
                watch_wram(address, offset & 0x1FFF, value);
                return;
            }
            if offset < 0x6000 {
                if (0x2000..=0x2007).contains(&offset) {
                    if let Some(msu1) = self.msu1.as_mut() {
                        msu1.write(offset as u32, value);
                        return;
                    }
                }
                if SharpRtc::window(address) {
                    if let Some(rtc) = self.sharp_rtc.as_mut() {
                        rtc.write(offset as u32, value);
                        return;
                    }
                }
                if (0x2100..=0x213F).contains(&offset) {
                    // Debug: the coprocessor's live position when the S-CPU
                    // feeds the VRAM data port, which is how the CX4's plot is
                    // copied into BG3 character data.
                    if offset == 0x2118 || offset == 0x2119 {
                        if let (Some(path), Some(cx4)) =
                            (std::env::var_os("VRAM_CX4_LOG"), self.cx4.as_ref())
                        {
                            use std::io::Write;
                            let from: u64 = std::env::var("VRAM_CX4_FROM")
                                .ok()
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(0);
                            let to: u64 = std::env::var("VRAM_CX4_TO")
                                .ok()
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(u64::MAX);
                            if (from..=to).contains(&self.ppu.debug_frame) {
                                if let Ok(mut f) = std::fs::OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(path)
                                {
                                    let _ = writeln!(
                                        f,
                                        "{} f={} {:04X}={:02X} pb={:04X} pc={:02X} run={} busy={} ticks={} halts={} jobs={}",
                                        crate::cx4::DEBUG_CLOCK
                                            .load(std::sync::atomic::Ordering::Relaxed),
                                        self.ppu.debug_frame,
                                        offset,
                                        value,
                                        cx4.regs.pb,
                                        cx4.regs.pc,
                                        u8::from(cx4.running()),
                                        u8::from(cx4.busy()),
                                        cx4.ticks,
                                        crate::cx4::DEBUG_HALTS
                                            .load(std::sync::atomic::Ordering::Relaxed),
                                        crate::cx4::DEBUG_JOB_STARTS
                                            .load(std::sync::atomic::Ordering::Relaxed)
                                    );
                                }
                            }
                        }
                    }
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
                    // WRIO bit 7 enables counter latching. ares latches on the
                    // high-to-low transition (`if(io.pio.bit(7) && !data.bit(7))
                    // ppu.latchCounters()`), which is how games sample the beam
                    // position: Star Fox polls $213D after toggling this bit.
                    if self.io.pio() & 0x80 != 0 && value & 0x80 == 0 {
                        self.ppu.latch_counters();
                    }
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
        if self.bsx.is_some() {
            return;
        }
        self.cart.write(address, value);
    }

    fn access_cycles(&mut self, address: u32) -> u32 {
        // The SA-1's bus-contention model depends on the S-CPU's current
        // address; ares tracks it in `r.mar` at the start of every access.
        *self.snes_mar = address;
        // Mirrors ares CPU::wait(): six master clocks per base cycle, with
        // wait states for slow ROM, WRAM and the internal I/O window.
        bus_wait(address, *self.fast_rom)
    }
}

struct HdmaBus<'a> {
    wram: &'a mut [u8],
    cart: &'a mut Cartridge,
    apu: &'a mut Apu,
    ppu: &'a mut Ppu,
    /// The SA-1 remaps the cartridge ROM, so HDMA A-bus reads have to go
    /// through it too (ares' HDMA reads use the full CPU bus).
    sa1: &'a mut Option<Sa1>,
    sdd1: &'a mut Option<Sdd1>,
    spc7110: &'a mut Option<Spc7110>,
    obc1: &'a mut Option<Obc1>,
    msu1: &'a mut Option<Msu1>,
    sharp_rtc: &'a mut Option<SharpRtc>,
    sufami_turbo: &'a mut Option<SufamiTurbo>,
    snes_mar: u32,
    snes_refresh: bool,
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
        if let Some(spc7110) = self.spc7110.as_ref() {
            if Spc7110::in_expansion_window(address) {
                return spc7110.expansion_read(address);
            }
            if Spc7110::in_rom_window(address) {
                return spc7110.mcurom_read(Spc7110::reduce_rom(address), 0);
            }
            if let Some(index) = Spc7110::ram_index(address) {
                return spc7110.ram_read(index, 0);
            }
        }
        if let Some(sdd1) = self.sdd1.as_mut() {
            if Sdd1::in_io_window(address) {
                return sdd1.read_io(address, self.cart.rom());
            }
            if Sdd1::in_mcu_window(address) {
                return sdd1.mcu_read(address, self.cart.rom());
            }
        }
        if self.sa1.is_some() {
            let sa1 = self.sa1.as_mut().expect("SA-1 present");
            sa1.snes_mar = self.snes_mar;
            sa1.snes_refresh = self.snes_refresh;
            if Sa1::in_rom_window(address) {
                let rom = self.cart.rom();
                return sa1.read_cpu_rom(address, rom);
            }
            if Sa1::in_bwram_window(address) {
                let (_, sram) = self.cart.rom_and_sram_mut();
                return sa1.read_cpu_bwram(address, sram);
            }
        }
        if Obc1::window(address) {
            if let Some(obc1) = self.obc1.as_ref() {
                return obc1.read(self.cart.sram(), address);
            }
        }
        if let Some(msu1) = self.msu1.as_mut() {
            let low = bank <= 0x3F || (0x80..=0xBF).contains(&bank);
            if low && (0x2000..=0x2007).contains(&offset) {
                return msu1.read(offset as u32, 0);
            }
        }
        if SharpRtc::window(address) {
            if let Some(rtc) = self.sharp_rtc.as_mut() {
                return rtc.read(offset as u32, 0);
            }
        }
        if let Some(sufami) = self.sufami_turbo.as_ref() {
            if let Some((slot, is_rom, slot_offset)) = SufamiTurbo::window(address) {
                return match sufami.slots[slot].as_ref() {
                    Some(slot) if is_rom => slot.read_rom(slot_offset).unwrap_or(0),
                    Some(slot) => slot.read_ram(slot_offset).unwrap_or(0),
                    None => 0,
                };
            }
        }
        self.cart.read(address).unwrap_or(0)
    }

    fn write(&mut self, address: u32, value: u8) {
        let bank = (address >> 16) as u8;
        let offset = (address & 0xFFFF) as usize;
        if bank == 0x7E || bank == 0x7F {
            let index = ((bank as usize & 1) << 16) | offset;
            self.wram[index] = value;
            watch_wram(address, index, value);
            return;
        }
        if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
            if offset < 0x2000 {
                self.wram[offset & 0x1FFF] = value;
                watch_wram(address, offset & 0x1FFF, value);
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
        if let Some(msu1) = self.msu1.as_mut() {
            let low = bank <= 0x3F || (0x80..=0xBF).contains(&bank);
            if low && (0x2000..=0x2007).contains(&offset) {
                msu1.write(offset as u32, value);
                return;
            }
        }
        if SharpRtc::window(address) {
            if let Some(rtc) = self.sharp_rtc.as_mut() {
                rtc.write(offset as u32, value);
                return;
            }
        }
        if Obc1::window(address) {
            if let Some(obc1) = self.obc1.as_mut() {
                obc1.write(self.cart.sram_mut(), address, value);
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

    /// Build a minimal BS-X image (1 MiB, map mode `$30`, type `$E5`).
    fn build_bsx() -> Vec<u8> {
        let mut rom = vec![0u8; 0x10_0000];
        let header = Mapping::LoRom.header_offset();
        rom[header..header + 16].copy_from_slice(b"Satellaview BS-X");
        rom[header + 0x15] = 0x30;
        rom[header + 0x16] = 0xE5;
        rom[header + 0x17] = 0x0A;
        rom[header + 0x18] = 0x05;
        rom[header + 0x19] = 1;
        rom[header + 0x1A] = 1;
        rom[header + 0x3C..header + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());
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

    #[test]
    fn bsx_pack_is_reachable_from_the_cpu_bus() {
        let cart = Cartridge::from_bytes(&build_bsx()).expect("valid BS-X");
        assert!(cart.header().is_bsx());
        let mut snes = Snes::new(cart);
        assert!(snes.bsx.is_some(), "BS-X controller instantiated");

        // With the reset MCC state (HiROM, r09=1) the pack owns `$c0-$ff`.
        assert_eq!(snes.read(0xC0_0000, 0x00), 0x00, "no pack: open bus");

        let mut pak = vec![0u8; 0x10_0000];
        pak[0] = 0xA5;
        pak[0x1234] = 0x5A;
        snes.load_bsx_pack(pak);
        assert!(snes.bsx.as_ref().expect("BS-X").has_pack());
        assert_eq!(snes.read(0xC0_0000, 0x00), 0xA5);
        assert_eq!(snes.read(0xC0_1234, 0x00), 0x5A);

        // The BIOS itself is still reachable in its LoROM window.
        assert_eq!(
            snes.read(0x00_8000, 0x00),
            snes.bsx.as_ref().unwrap().bios()[0]
        );
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
        // Bank $7F is a different 64 KiB and keeps the power-on pattern.
        let bank7f = system.wram()[0x1_1234];

        let mut bus = SnesBus {
            wram: &mut system.wram,
            cart: &mut system.cart,
            bsx: &mut system.bsx,
            st018: &mut system.st018,
            gsu: &mut system.gsu,
            dsp1: &mut system.dsp1,
            cx4: &mut system.cx4,
            sdd1: &mut system.sdd1,
            sa1: &mut system.sa1,
            spc7110: &mut system.spc7110,
            obc1: &mut system.obc1,
            msu1: &mut system.msu1,
            sharp_rtc: &mut system.sharp_rtc,
            sufami_turbo: &mut system.sufami_turbo,
            snes_mar: &mut system.snes_mar,
            clock: 0,
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
        assert_eq!(bus.read(0x7F1234, 0), bank7f);
    }

    #[test]
    fn obc1_maps_its_ram_and_register_indirection() {
        let mut image = build_lorom(&[0xDB]);
        let header = Mapping::LoRom.header_offset();
        image[header + 0x16] = 0x25; // cartridge type: OBC1 board
        image[header + 0x18] = 3; // 8 KiB save RAM
        let cart = Cartridge::from_bytes(&image).expect("valid OBC1 LoROM");
        assert!(cart.header().has_obc1());
        assert_eq!(cart.sram().len(), 0x2000);

        let mut system = Snes::new(cart);
        assert!(system.obc1.is_some());
        {
            let mut bus = system.bus();
            // `$1FF6` selects the object and the packed-bit shift; `$1FF0`
            // then reaches that object's first byte through `$7FF0`.
            bus.write(0x00_7FF6, 0x03);
            bus.write(0x00_7FF0, 0x12);
            assert_eq!(bus.read(0x00_7FF0, 0), 0x12);
            assert_eq!(bus.read(0x80_7FF0, 0), 0x12);
            assert_eq!(bus.read(0x00_7FF6, 0), 0x03);
            // The same 8 KiB is the battery-backed save RAM at `$70:6000`.
            assert_eq!(bus.read(0x70_6000, 0), 0xFF);
            bus.write(0x70_6000, 0x77);
            assert_eq!(bus.read(0x71_E000, 0), 0x77);
        }
        assert_eq!(system.cart.sram()[0], 0x77);
    }

    #[test]
    fn msu1_registers_and_audio_run_through_the_bus() {
        let mut system = Snes::new(cartridge(&[0xDB]));
        system.set_msu1_data(vec![0x11, 0x22, 0x33, 0x44]);
        let mut pcm = b"MSU1".to_vec();
        pcm.extend_from_slice(&0u32.to_le_bytes());
        pcm.extend_from_slice(&1000i16.to_le_bytes());
        pcm.extend_from_slice(&(-1000i16).to_le_bytes());
        system.set_msu1_track(1, pcm);

        {
            let mut bus = system.bus();
            assert_eq!(bus.read(0x00_2000, 0) & 0x07, 0x02);
            assert_eq!(bus.read(0x00_2001, 0), 0x11);
            assert_eq!(bus.read(0x80_2001, 0), 0x22);
            assert_eq!(bus.read(0x00_2002, 0), b'S');
            bus.write(0x00_2004, 1);
            bus.write(0x00_2005, 0);
            bus.write(0x00_2006, 255);
            bus.write(0x00_2007, 1);
            assert_ne!(bus.read(0x00_2000, 0) & 0x10, 0);
        }

        // The S-DSP powers up muted (ares `MainVol { mute = 1 }`), and MSU1
        // output follows that mute flag; clear `$6C` as a sound driver does.
        system.apu.dsp_mut().write(0x6C, 0x00);
        // One 44.1 kHz sample is one master-clock period / 44,100 clocks.
        let clocks = (system.region().master_clock() / 44_100) as u32 + 1;
        system.advance_peripherals(clocks);
        let samples = system.drain_msu1_samples();
        assert!(!samples.is_empty());
        assert_eq!(samples[0], (1000, -1000));
    }

    #[test]
    fn sharp_rtc_maps_and_ticks_through_the_bus() {
        let mut image = build_lorom(&[0xDB]);
        let header = Mapping::LoRom.header_offset();
        image[header + 0x16] = 0x55; // Sharp S-RTC board (type high nibble $5)
        image[header + 0x18] = 5;
        let cart = Cartridge::from_bytes(&image).expect("valid Sharp RTC LoROM");
        assert!(cart.header().has_sharp_rtc());

        let mut system = Snes::new(cart);
        system.set_sharp_rtc_time(946_775_145); // 2000-01-02 01:05:45
        assert_eq!(system.sharp_rtc_calendar(), Some((45, 5, 1, 2, 1, 2000, 0)));
        {
            let mut bus = system.bus();
            bus.write(0x00_2801, 0x0D);
            assert_eq!(bus.read(0x00_2800, 0), 15);
            for value in [5, 4, 5, 0, 1, 0, 2, 0, 1, 0, 0, 10, 0] {
                assert_eq!(bus.read(0x80_2800, 0), value);
            }
            assert_eq!(bus.read(0x00_2800, 0), 15);
        }

        // One second of master clocks advances the calendar by one second.
        // Advance in small slices: a single second exceeds the PPU's u32 dot
        // budget.
        let den = system.region().master_clock() as u32;
        let slice = 1000;
        for _ in 0..den / slice {
            system.advance_peripherals(slice);
        }
        system.advance_peripherals(den % slice);
        assert_eq!(system.sharp_rtc_calendar(), Some((46, 5, 1, 2, 1, 2000, 0)));
    }

    #[test]
    fn sufami_turbo_slots_map_and_survive_a_state_restore() {
        let mut image = build_lorom(&[0xDB]);
        image[..14].copy_from_slice(crate::sufamiturbo::SUFAMI_TURBO_SIGNATURE);
        let cart = Cartridge::from_bytes(&image).expect("valid Sufami Turbo LoROM");
        assert!(cart.is_sufami_turbo());

        let mut rom_a = vec![0u8; 0x1_0000];
        rom_a[0] = 0xA0;
        rom_a[0x8000] = 0xA1;
        rom_a[0xFFFF] = 0xA2;
        let mut rom_b = vec![0u8; 0x2_0000];
        rom_b[0] = 0xB0;
        rom_b[0x1_0000] = 0xB1;

        let mut system = Snes::new(cart);
        assert!(system.has_sufami_turbo());
        assert!(system.set_sufami_turbo_slot(0, rom_a.clone(), vec![0u8; 8]));
        assert!(system.set_sufami_turbo_slot(1, rom_b.clone(), vec![0u8; 8]));
        {
            let mut bus = system.bus();
            // Slot A ROM folds its 32 KiB window: $20 -> 0, $21 -> $8000.
            assert_eq!(bus.read(0x20_8000, 0), 0xA0);
            assert_eq!(bus.read(0x21_8000, 0), 0xA1);
            assert_eq!(bus.read(0x3F_FFFF, 0), 0xA2);
            // Slot B ROM is linear from $40:0000.
            assert_eq!(bus.read(0x40_0000, 0), 0xB0);
            assert_eq!(bus.read(0x41_0000, 0), 0xB1);
            bus.write(0x60_0000, 0x5A);
            assert_eq!(bus.read(0x60_0000, 0), 0x5A);
            bus.write(0x70_0001, 0xA5);
            assert_eq!(bus.read(0x70_0001, 0), 0xA5);
            // Mask-ROM writes are ignored.
            bus.write(0x20_8000, 0xFF);
            assert_eq!(bus.read(0x20_8000, 0), 0xA0);
        }

        let state = system.save_state();
        let mut fresh = Snes::new(Cartridge::from_bytes(&image).expect("valid copy"));
        assert!(fresh.set_sufami_turbo_slot(0, rom_a, vec![0u8; 8]));
        assert!(fresh.set_sufami_turbo_slot(1, rom_b, vec![0u8; 8]));
        fresh.load_state(&state).expect("load state");
        let mut bus = fresh.bus();
        assert_eq!(bus.read(0x60_0000, 0), 0x5A);
        assert_eq!(bus.read(0x70_0001, 0), 0xA5);
        // The host-attached slot ROMs are preserved across the restore.
        assert_eq!(bus.read(0x20_8000, 0), 0xA0);
    }

    #[test]
    fn reset_costs_the_ares_186_master_clocks() {
        // ares `CPU::main`: `status.resetPending` → `step(132)`, then the
        // emulation-mode interrupt sequence; the first fetch is at H=186.
        let mut system = Snes::new(cartridge(&[0xDB]));
        let cost = system.reset();
        assert_eq!(cost, 186);
        assert_eq!(system.clock, 186);
        assert_eq!(system.cpu.cycles, 186);
    }

    #[test]
    fn unmapped_reads_return_the_open_bus_mdr() {
        // ares maps unmapped addresses to `[](n24, n8 data) { return data; }`,
        // where `data` is the CPU's memory data register.
        let mut system = Snes::new(cartridge(&[0xDB]));
        // Work RAM powers up with ares' structured pattern, so compare against
        // the stored byte rather than assuming zero.
        let wram_1234 = system.wram()[0x1234];
        let mut bus = SnesBus {
            wram: &mut system.wram,
            cart: &mut system.cart,
            bsx: &mut system.bsx,
            st018: &mut system.st018,
            gsu: &mut system.gsu,
            dsp1: &mut system.dsp1,
            cx4: &mut system.cx4,
            sdd1: &mut system.sdd1,
            sa1: &mut system.sa1,
            spc7110: &mut system.spc7110,
            obc1: &mut system.obc1,
            msu1: &mut system.msu1,
            sharp_rtc: &mut system.sharp_rtc,
            sufami_turbo: &mut system.sufami_turbo,
            snes_mar: &mut system.snes_mar,
            clock: 0,
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
        assert_eq!(bus.read(0x001234, 0x5A), wram_1234);
    }

    #[test]
    fn io_status_reads_keep_open_bus_bits() {
        // ares `CPU::readCPU` writes only the defined bits of $4210/$4211/$4212
        // and leaves the rest from the MDR (open bus).
        let mut system = Snes::new(cartridge(&[0xDB]));
        let mut bus = SnesBus {
            wram: &mut system.wram,
            cart: &mut system.cart,
            bsx: &mut system.bsx,
            st018: &mut system.st018,
            gsu: &mut system.gsu,
            dsp1: &mut system.dsp1,
            cx4: &mut system.cx4,
            sdd1: &mut system.sdd1,
            sa1: &mut system.sa1,
            spc7110: &mut system.spc7110,
            obc1: &mut system.obc1,
            msu1: &mut system.msu1,
            sharp_rtc: &mut system.sharp_rtc,
            sufami_turbo: &mut system.sufami_turbo,
            snes_mar: &mut system.snes_mar,
            clock: 0,
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
        // PAL frame is 312 lines of 341 dots. The reset already advanced the
        // PPU by the ares H=186 reset phase, so measure the interval between
        // the first and second line-0 wraps rather than the total from reset.
        let mut frame_dots = 0u32;
        let mut wraps = 0u32;
        let mut last = system.ppu.line();
        for _ in 0..400_000 {
            system.ppu.tick();
            if wraps >= 1 {
                frame_dots += 1;
            }
            if system.ppu.line() < last {
                wraps += 1;
                if wraps == 2 {
                    break;
                }
            }
            last = system.ppu.line();
        }
        assert_eq!(wraps, 2);
        assert_eq!(frame_dots, 312 * 341);
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

    #[test]
    fn power_on_apu_pattern_survives_a_save_state() {
        // ares randomises APU RAM and the S-DSP register file at power-on and
        // `DSP::power(reset = true)` keeps them. A state saved before the
        // driver has touched them must restore the same bytes.
        let code = [0x4Cu8, 0x00, 0x80];
        let mut system = Snes::new(cartridge(&code));
        system.reset();
        assert!(
            system.apu.ram().iter().any(|&byte| byte != 0),
            "APU RAM was not power-on filled"
        );
        assert!(
            system.apu.dsp().registers.iter().any(|&byte| byte != 0),
            "DSP register file was not power-on filled"
        );
        let ram = system.apu.ram().to_vec();
        let registers = system.apu.dsp().registers;
        let saved = system.save_state();

        let mut restored = Snes::new(cartridge(&code));
        restored.reset();
        restored.load_state(&saved).expect("load state");
        assert_eq!(restored.apu.ram(), &ram[..]);
        assert_eq!(restored.apu.dsp().registers, registers);
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                apu: &mut system.apu,
                ppu: &mut system.ppu,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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

    /// A 7 MiB `EXSPC7110-` image: 1 MiB program, 5 MiB data, 1 MiB expansion.
    fn exspc7110_cartridge() -> Cartridge {
        use crate::spc7110::{
            EXSPC7110_DATA_SIZE, EXSPC7110_EXPANSION_SIZE, EXSPC7110_IMAGE_SIZE, PROM_SIZE,
        };
        let mut rom = vec![0u8; EXSPC7110_IMAGE_SIZE];
        let expansion_at = PROM_SIZE + EXSPC7110_DATA_SIZE;
        for (i, byte) in rom.iter_mut().enumerate() {
            *byte = if i < PROM_SIZE {
                0x11
            } else if i < expansion_at {
                0x22
            } else {
                0x33
            };
        }
        rom[expansion_at] = 0xA5;
        rom[expansion_at + 0xFFFFF] = 0x5A;
        // Reset vector inside the program ROM.
        rom[0x7FFC] = 0x00;
        rom[0x7FFD] = 0x80;
        // HiROM header at $FFC0 with the SPC7110 map mode and type.
        rom[0xFFC0..0xFFD5].fill(b' ');
        rom[0xFFC0..0xFFCA].copy_from_slice(b"EXSPC TEST");
        rom[0xFFD5] = 0x3A; // SPC7110 HiROM map mode
        rom[0xFFD6] = 0xF5; // SPC7110
        rom[0xFFD7] = 0x0D;
        rom[0xFFD8] = 0x03;
        rom[0xFFD9] = 0x01;
        // Complement/checksum must XOR to $FFFF for the header to validate.
        rom[0xFFDC..0xFFDE].copy_from_slice(&0x0000u16.to_le_bytes());
        rom[0xFFDE..0xFFE0].copy_from_slice(&0xFFFFu16.to_le_bytes());
        rom[0xFFFC..0xFFFE].copy_from_slice(&0x8000u16.to_le_bytes());
        assert_eq!(
            rom.len(),
            EXSPC7110_EXPANSION_SIZE + EXSPC7110_DATA_SIZE + PROM_SIZE
        );
        Cartridge::from_bytes(&rom).expect("valid EXSPC7110 image")
    }

    #[test]
    fn exspc7110_expansion_rom_is_mapped_at_40_4f() {
        let mut system = Snes::new(exspc7110_cartridge());
        assert!(system.cart.header().has_spc7110());
        let chip = system.spc7110.as_ref().expect("SPC7110 present");
        assert_eq!(chip.prom.len(), 0x10_0000);
        assert_eq!(chip.drom.len(), 0x50_0000);
        assert_eq!(chip.expansion.len(), 0x10_0000);

        // Program ROM through the MCU window, expansion ROM at $40-$4F.
        assert_eq!(system.read(0x00_8000, 0), 0x11);
        assert_eq!(system.read(0x40_0000, 0), 0xA5);
        assert_eq!(system.read(0x4F_FFFF, 0), 0x5A);
        assert_eq!(system.read(0x41_0000, 0), 0x33);
        // The expansion ROM is read-only.
        system.write(0x40_0000, 0x00);
        assert_eq!(system.read(0x40_0000, 0), 0xA5);
        // Save RAM still lives at $6000-$7FFF.
        system.write(0x00_4830, 0x80);
        system.write(0x00_6000, 0x77);
        assert_eq!(system.read(0x00_6000, 0), 0x77);
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
                bsx: &mut system.bsx,
                st018: &mut system.st018,
                gsu: &mut system.gsu,
                dsp1: &mut system.dsp1,
                cx4: &mut system.cx4,
                sdd1: &mut system.sdd1,
                sa1: &mut system.sa1,
                spc7110: &mut system.spc7110,
                obc1: &mut system.obc1,
                msu1: &mut system.msu1,
                sharp_rtc: &mut system.sharp_rtc,
                sufami_turbo: &mut system.sufami_turbo,
                snes_mar: &mut system.snes_mar,
                clock: 0,
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
