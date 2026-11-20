//! S-SMP APU: 64 KiB APU RAM, IPL ROM, the four CPU ports and the S-DSP.
//!
//! The SPC700 CPU is implemented in [`crate::spc700`] and the S-DSP in
//! [`crate::dsp`]. This module owns the APU address space, the timers, and the
//! two independent four-byte port latches, and it records each SPC700
//! instruction as a micro-op timeline so an S-CPU access can observe an S-SMP
//! port write at its exact master clock.

use std::collections::VecDeque;

use crate::dsp::{Dsp, Sample};

/// ares `Thread::Second`: the fixed-point base of the scheduler clocks.
const SCHEDULER_SECOND: u128 = (1u128 << 62) - 1;
/// ares `Thread::setFrequency`: `_scalar = Second / frequency`.
const DSP_FREQUENCY: u128 = 24_606_720;
const SMP_FREQUENCY: u128 = 2_050_560;
const DSP_SCALAR: u128 = SCHEDULER_SECOND / DSP_FREQUENCY;
const SMP_SCALAR: u128 = SCHEDULER_SECOND / SMP_FREQUENCY;
/// One DSP `tick()` steps the thread by `3 * 8` DSP clocks.
const DSP_TICK: u128 = 24 * DSP_SCALAR;
use crate::spc700::{Spc700, SpcBus};

/// Which side produced a port event and in which direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PortEventKind {
    /// S-CPU write to `$2140-$2143`.
    CpuWrite,
    /// S-CPU read of `$2140-$2143`.
    CpuRead,
    /// S-SMP read of `$F4-$F7`.
    SmpRead,
    /// S-SMP write to `$F4-$F7` (the echo the S-CPU polls).
    SmpWrite,
}

/// One S-CPU <-> S-SMP port event, captured by [`PortDebug`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PortEvent {
    /// Master clock of the access: the commit instant for an S-CPU write, the
    /// instruction-record instant for an S-SMP read.
    pub clock: u64,
    /// Which access produced the event.
    pub kind: PortEventKind,
    /// Port index 0-3.
    pub port: u8,
    /// Value transferred.
    pub value: u8,
    /// S-SMP PC of the reading instruction (0 for S-CPU writes).
    pub spc_pc: u16,
    /// Retired S-SMP instruction index at the event.
    pub spc_instruction: u64,
}

/// Debug state for the S-CPU/S-SMP port handshake log.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PortDebug {
    enabled: bool,
    /// Master clock the APU has been advanced to.
    clock: u64,
    /// S-SMP instruction currently being recorded.
    spc_instruction: u64,
    events: Vec<PortEvent>,
    /// `(master clock at instruction start, pc, S-SMP clocks)` per instruction.
    trace: Vec<(u64, u16, u32)>,
}

impl PortDebug {
    /// Start or stop capturing port events.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.events.clear();
        }
    }

    /// Captured events in commit order.
    pub fn events(&self) -> &[PortEvent] {
        &self.events
    }

    /// Captured `(master clock, pc, clocks)` instruction trace.
    pub fn trace(&self) -> &[(u64, u16, u32)] {
        &self.trace
    }

    /// Update the master clock the APU is currently at.
    pub fn set_clock(&mut self, clock: u64) {
        self.clock = clock;
    }

    /// Record an S-CPU read of `$2140-$2143` (debug).
    pub(crate) fn record_cpu_read(&mut self, clock: u64, port: usize, value: u8) {
        let spc_instruction = self.spc_instruction;
        self.record(PortEvent {
            clock,
            kind: PortEventKind::CpuRead,
            port: u8::try_from(port).unwrap_or(0),
            value,
            spc_pc: 0,
            spc_instruction,
        });
    }

    pub(crate) fn record_cpu_write(&mut self, clock: u64, port: usize, value: u8) {
        let spc_instruction = self.spc_instruction;
        self.record(PortEvent {
            clock,
            kind: PortEventKind::CpuWrite,
            port: u8::try_from(port).unwrap_or(0),
            value,
            spc_pc: 0,
            spc_instruction,
        });
    }

    /// Record an S-SMP write to `$F4-$F7` when its access completes.
    pub(crate) fn record_smp_write(&mut self, clock: u64, port: u8, value: u8) {
        let spc_instruction = self.spc_instruction;
        self.record(PortEvent {
            clock,
            kind: PortEventKind::SmpWrite,
            port,
            value,
            spc_pc: 0,
            spc_instruction,
        });
    }

    /// Record one event.
    pub(crate) fn record(&mut self, event: PortEvent) {
        if self.enabled && self.events.len() < 4_000_000 {
            self.events.push(event);
        }
    }
}

/// Size of APU RAM.
pub const APU_RAM_SIZE: usize = 0x1_0000;

/// Size of the SPC700 IPL ROM.
pub const IPL_SIZE: usize = 64;

/// One S-SMP timer.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Timer {
    stage0: u16,
    stage1: bool,
    stage2: u8,
    stage3: u8,
    line: bool,
    enable: bool,
    target: u8,
}

impl Timer {
    /// Re-evaluate the timer line level without advancing stage 0 (ares
    /// `synchronizeStage1`), used when `$F0` changes the enable bits.
    fn synchronize(&mut self, timers_enable: bool, timers_disable: bool) {
        let level = self.stage1 && timers_enable && !timers_disable;
        if self.line && !level && self.enable {
            self.stage2 = self.stage2.wrapping_add(1);
            if self.stage2 == self.target {
                self.stage2 = 0;
                self.stage3 = (self.stage3 + 1) & 0x0F;
            }
        }
        self.line = level;
    }

    fn step(&mut self, clocks: u32, frequency: u16, timers_enable: bool, timers_disable: bool) {
        self.stage0 = self.stage0.wrapping_add(clocks as u16);
        while self.stage0 >= frequency {
            self.stage0 -= frequency;
            self.stage1 = !self.stage1;
            let level = self.stage1 && timers_enable && !timers_disable;
            if self.line && !level && self.enable {
                self.stage2 = self.stage2.wrapping_add(1);
                if self.stage2 == self.target {
                    self.stage2 = 0;
                    self.stage3 = (self.stage3 + 1) & 0x0F;
                }
            }
            self.line = level;
        }
    }
}

/// One bus access of the SPC700 instruction currently being drained.
///
/// ares runs the S-SMP as a cooperative thread: every access steps the timers
/// and the DSP by its wait-state count, and an S-SMP-to-S-CPU port write becomes
/// visible to the S-CPU at the exact clock the access completes. Recording the
/// access timeline lets us commit those port latches at the same instants while
/// the S-CPU is in the middle of its own instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PlanStep {
    /// S-SMP clocks (2.050560 MHz) this access consumes.
    clocks: u8,
    /// Latched `$F4-$F7` write, committed when the access finishes.
    port_write: Option<(u8, u8)>,
    /// This step is the deferred I/O read of the current SPC700 instruction.
    deferred: bool,
    /// The deferred read is a `$F4-$F7` port read (wait split around the fetch).
    split: bool,
}

/// The S-SMP + S-DSP APU.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Apu {
    /// SPC700 CPU state.
    pub spc: Spc700,
    ram: Box<[u8]>,
    #[serde(with = "serde_big_array::BigArray")]
    ipl: [u8; IPL_SIZE],
    ipl_loaded: bool,
    iplrom_enable: bool,
    ram_writable: bool,
    ram_disable: bool,
    timers_enable: bool,
    timers_disable: bool,
    internal_wait: u8,
    external_wait: u8,
    timers: [Timer; 3],
    dsp_address: u8,
    dsp: Dsp,
    /// Ports written by the S-CPU and read by the SPC700.
    apu_ports: [u8; 4],
    /// Ports written by the SPC700 and read by the S-CPU.
    cpu_ports: [u8; 4],
    aux4: u8,
    aux5: u8,
    timer_targets: [u8; 3],
    /// Number of SPC700 instructions executed (debug).
    pub instructions: u64,
    /// Number of S-SMP clocks retired (debug).
    pub units: u64,
    /// Micro-op timeline of the instruction being drained.
    plan: Vec<PlanStep>,
    plan_pos: usize,
    plan_sub: u32,
    /// ares `SMP::_clock` in the scheduler's fixed-point base.
    smp_clock: u128,
    /// ares `DSP::_clock` in the same base. A micro-step advances it by
    /// `DSP_TICK` (24 DSP clocks) and a step runs while it is behind.
    dsp_clock: u128,
    /// S-CPU master clock at the current S-SMP access (set by the system).
    cpu_master_clock: u64,
    /// Master clock rate paired with `cpu_master_clock`.
    master_clock_rate: u64,
    /// Whether `cpu_master_clock`/`master_clock_rate` are valid.
    master_clock_valid: bool,
    /// Carried S-SMP master-clock credit: the fractional remainder of the
    /// rational conversion from master clocks to S-SMP clocks.
    apu_credit: i64,
    samples: VecDeque<Sample>,
    /// Debug: S-CPU/S-SMP port handshake trace.
    pub debug: PortDebug,
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

impl Apu {
    /// Create an APU without an IPL ROM. A user-supplied `ipl.rom` is required
    /// for the real boot protocol.
    pub fn new() -> Self {
        Self {
            spc: Spc700::new(),
            ram: vec![0; APU_RAM_SIZE].into_boxed_slice(),
            ipl: [0; IPL_SIZE],
            ipl_loaded: false,
            iplrom_enable: true,
            ram_writable: true,
            ram_disable: false,
            timers_enable: true,
            timers_disable: false,
            internal_wait: 0,
            external_wait: 0,
            timers: std::array::from_fn(|_| Timer::default()),
            dsp_address: 0,
            dsp: Dsp::new(),
            apu_ports: [0; 4],
            cpu_ports: [0; 4],
            aux4: 0,
            aux5: 0,
            timer_targets: [0; 3],
            instructions: 0,
            units: 0,
            plan: Vec::new(),
            plan_pos: 0,
            plan_sub: 0,
            // ares `Scheduler::power`: clocks start at each thread's
            // uniqueID (CPU 0, SMP 1, DSP 2).
            smp_clock: 1,
            dsp_clock: 2,
            cpu_master_clock: 0,
            master_clock_rate: 0,
            master_clock_valid: false,
            apu_credit: 0,
            samples: VecDeque::new(),
            debug: PortDebug::default(),
        }
    }

    /// Create an APU with a 64-byte IPL ROM.
    pub fn with_ipl(ipl: [u8; IPL_SIZE]) -> Self {
        let mut apu = Self::new();
        apu.set_ipl(ipl);
        apu
    }

    /// Install ares' deterministic power-on APU RAM and DSP register pattern.
    ///
    /// ares randomises both at power-on (`DSP::power`), and the S-DSP's
    /// `registers[]` is a plain file that is *not* derived from the decoded
    /// voice state, so a driver polling an unwritten register sees these bytes.
    /// A soft reset keeps them (`DSP::power(reset = true)` does not refill).
    pub fn load_power_on_state(&mut self, ram: &[u8], dsp_registers: &[u8]) {
        if ram.len() == APU_RAM_SIZE {
            self.ram.copy_from_slice(ram);
        }
        let count = dsp_registers.len().min(crate::dsp::DSP_REGISTERS);
        self.dsp.registers[..count].copy_from_slice(&dsp_registers[..count]);
    }

    /// Install a 64-byte IPL ROM.
    pub fn set_ipl(&mut self, ipl: [u8; IPL_SIZE]) {
        self.ipl = ipl;
        self.ipl_loaded = true;
        self.reset();
    }

    /// Whether an IPL ROM is installed.
    pub const fn has_ipl(&self) -> bool {
        self.ipl_loaded
    }

    /// APU RAM.
    pub fn ram(&self) -> &[u8] {
        &self.ram[..]
    }

    /// Mutable APU RAM.
    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram[..]
    }

    /// S-DSP register file.
    pub fn dsp(&self) -> &Dsp {
        &self.dsp
    }

    /// Mutable S-DSP register file.
    pub fn dsp_mut(&mut self) -> &mut Dsp {
        &mut self.dsp
    }

    /// Reset the APU and jump to the IPL ROM reset vector.
    pub fn reset(&mut self) {
        self.spc.reset();
        self.iplrom_enable = true;
        self.ram_writable = true;
        self.ram_disable = false;
        self.timers_enable = true;
        self.timers_disable = false;
        self.internal_wait = 0;
        self.external_wait = 0;
        self.timers = std::array::from_fn(|_| Timer::default());
        self.dsp_address = 0;
        self.dsp.reset();
        self.plan.clear();
        self.plan_pos = 0;
        self.plan_sub = 0;
        self.smp_clock = 1;
        self.dsp_clock = 2;
        self.cpu_master_clock = 0;
        self.master_clock_rate = 0;
        self.master_clock_valid = false;
        self.apu_credit = 0;
        self.samples.clear();
        self.apu_ports = [0; 4];
        self.cpu_ports = [0; 4];
        self.aux4 = 0;
        self.aux5 = 0;
        self.timer_targets = [0; 3];
        if self.ipl_loaded {
            self.spc.pc = u16::from_le_bytes([self.ipl[62], self.ipl[63]]);
        } else {
            self.spc.pc = 0x0200;
        }
        self.spc.sp = 0xEF;
    }

    /// S-CPU read from `$2140-$2143`.
    pub fn cpu_read(&self, port: usize) -> u8 {
        self.cpu_ports[port & 3]
    }

    /// S-CPU write to `$2140-$2143`.
    pub fn cpu_write(&mut self, port: usize, value: u8) {
        self.apu_ports[port & 3] = value;
    }

    /// Enable or disable the port handshake log and clear it.
    pub fn set_port_logging(&mut self, enabled: bool) {
        self.debug.set_enabled(enabled);
    }

    /// Whether the port handshake log is capturing.
    pub fn port_logging_enabled(&self) -> bool {
        self.debug.enabled
    }

    /// Record the S-CPU master clock for the current access. ares
    /// `SMP::readIO($F4-$F7)` calls `synchronize(cpu)`, so the port read waits
    /// until the S-CPU has reached this clock.
    pub fn set_master_clock(&mut self, clock: u64, rate: u64) {
        self.cpu_master_clock = clock;
        self.master_clock_rate = rate;
        self.master_clock_valid = true;
    }

    /// S-SMP clock at the current plan position.
    ///
    /// `record_instruction` advances `smp_clock` by the whole instruction
    /// eagerly. The executed position is derived by subtracting the plan
    /// clocks that have not been drained yet, so the clock is step-granular
    /// (ares' S-SMP is suspended mid-instruction at a port read).
    fn smp_clock_at_position(&self) -> u128 {
        if self.plan.is_empty() {
            return self.smp_clock;
        }
        let total: u32 = self.plan.iter().map(|step| u32::from(step.clocks)).sum();
        // The clock before the plan may be less than the plan length only at
        // power-on (ares' SMP clock starts at 1 fixed-point unit); clamp there.
        let start = self
            .smp_clock
            .saturating_sub(u128::from(total) * SMP_SCALAR);
        let mut elapsed: u32 = self.plan[..self.plan_pos]
            .iter()
            .map(|step| u32::from(step.clocks))
            .sum();
        if let Some(step) = self.plan.get(self.plan_pos) {
            elapsed += self.plan_sub.min(u32::from(step.clocks));
        }
        start + u128::from(elapsed) * SMP_SCALAR
    }

    /// The S-SMP's scaled clock converted to master clocks, rounded up so the
    /// S-CPU running to this clock is not behind the S-SMP (ares compares the
    /// two thread clocks directly).
    pub fn smp_master_clock(&self, master_clock: u64) -> u64 {
        let scalar = SCHEDULER_SECOND / u128::from(master_clock);
        let fixed = self.smp_clock_at_position();
        u64::try_from(fixed.div_ceil(scalar)).unwrap_or(u64::MAX)
    }

    /// Whether the pending deferred read is a `$F4-$F7` port read whose clock
    /// is ahead of the S-CPU, i.e. ares would run the S-CPU forward first.
    pub fn port_read_waiting(&self) -> bool {
        if !self.master_clock_valid {
            return false;
        }
        let Some(crate::spc700::Pending::DirectRead { address, .. }) = self.spc.pending else {
            return false;
        };
        let effective = (u16::from(self.spc.flags.page) << 8) | u16::from(address);
        if !(0x00F4..=0x00F7).contains(&effective) {
            return false;
        }
        // ares compares the fixed-point thread clocks (`cpu.clock()` against
        // `smp.clock()`); flooring to master clocks can stop the S-CPU one
        // clock early and change which instruction's writes the read sees.
        let cpu_scalar = SCHEDULER_SECOND / u128::from(self.master_clock_rate);
        let cpu_fixed = u128::from(self.cpu_master_clock) * cpu_scalar;
        self.smp_clock_at_position() > cpu_fixed
    }

    /// Value currently visible to the SPC700 on `$F4-$F7`.
    pub fn apu_port(&self, port: usize) -> u8 {
        self.apu_ports[port & 3]
    }

    /// Set a value visible to the S-CPU on `$2140-$2143` (test/debug helper).
    pub fn set_cpu_port(&mut self, port: usize, value: u8) {
        self.cpu_ports[port & 3] = value;
    }

    /// Execute one SPC700 instruction.
    pub fn step(&mut self) -> u32 {
        if self.plan_pos >= self.plan.len() {
            self.record_instruction();
        }
        let mut total = 0;
        while self.plan_pos < self.plan.len() {
            total += self.drain_unit();
        }
        total
    }

    /// Advance the S-SMP by one clock (2.050560 MHz).
    ///
    /// The system calls this from inside an S-CPU access, so an S-SMP port write
    /// commits at the exact master clock of the S-SMP access that produced it.
    pub fn tick(&mut self) -> u32 {
        self.drain_unit()
    }

    /// Execute the next instruction into a fresh micro-op timeline.
    fn record_instruction(&mut self) {
        self.plan.clear();
        self.plan_pos = 0;
        self.plan_sub = 0;
        self.debug.spc_instruction = self.instructions;
        let entry_pc = self.spc.pc;
        let page_flag = self.spc.flags.page;
        // The S-SMP's logical time is exactly its retired access units.
        let mut plan = std::mem::take(&mut self.plan);
        {
            let mut bus = ApuBus {
                ram: &mut self.ram[..],
                ipl: &self.ipl,
                ipl_loaded: self.ipl_loaded,
                iplrom_enable: &mut self.iplrom_enable,
                ram_writable: &mut self.ram_writable,
                ram_disable: &mut self.ram_disable,
                timers_enable: &mut self.timers_enable,
                timers_disable: &mut self.timers_disable,
                internal_wait: &mut self.internal_wait,
                external_wait: &mut self.external_wait,
                timers: &mut self.timers,
                dsp_address: &mut self.dsp_address,
                dsp: &mut self.dsp,
                smp_clock: &mut self.smp_clock,
                dsp_clock: &mut self.dsp_clock,
                samples: &mut self.samples,
                apu_ports: &mut self.apu_ports,
                cpu_ports: &mut self.cpu_ports,
                aux4: &mut self.aux4,
                aux5: &mut self.aux5,
                timer_targets: &mut self.timer_targets,
                page_flag,
                spc_pc: self.spc.pc,
                debug: Some(&mut self.debug),
            };
            let mut recorder = RecordBus {
                inner: &mut bus,
                plan: &mut plan,
                units: 0,
            };
            self.spc.step(&mut recorder);
        }
        let deferred =
            if let Some(crate::spc700::Pending::DirectRead { address, .. }) = self.spc.pending {
                let effective = (u16::from(self.spc.flags.page) << 8) | u16::from(address);
                // ares `SMP::wait`: the I/O window uses the internal wait selector.
                const CYCLE_WAIT: [u8; 4] = [1, 2, 5, 10];
                let clocks = CYCLE_WAIT[usize::from(self.internal_wait.min(3))] * 2;
                let split = effective & 0xFFFC == 0x00F4;
                plan.push(PlanStep {
                    clocks,
                    port_write: None,
                    deferred: true,
                    split,
                });
                true
            } else {
                false
            };
        if plan.is_empty() && !deferred {
            plan.push(PlanStep {
                clocks: 2,
                port_write: None,
                deferred: false,
                split: false,
            });
        }
        if self.debug.enabled && self.debug.trace.len() < 6_000_000 {
            let clocks: u32 = plan.iter().map(|step| u32::from(step.clocks)).sum();
            self.debug.trace.push((self.debug.clock, entry_pc, clocks));
        }
        self.plan = plan;
        if !deferred {
            self.instructions += 1;
        }
    }

    /// Perform the deferred I/O read at its own access clock, then apply the
    /// instruction's ALU/transfer step.
    fn resume_deferred_read(&mut self) {
        if self.spc.pending.is_none() {
            return;
        }
        let page_flag = self.spc.flags.page;
        let mut bus = ApuBus {
            ram: &mut self.ram[..],
            ipl: &self.ipl,
            ipl_loaded: self.ipl_loaded,
            iplrom_enable: &mut self.iplrom_enable,
            ram_writable: &mut self.ram_writable,
            ram_disable: &mut self.ram_disable,
            timers_enable: &mut self.timers_enable,
            timers_disable: &mut self.timers_disable,
            internal_wait: &mut self.internal_wait,
            external_wait: &mut self.external_wait,
            timers: &mut self.timers,
            dsp_address: &mut self.dsp_address,
            dsp: &mut self.dsp,
            smp_clock: &mut self.smp_clock,
            dsp_clock: &mut self.dsp_clock,
            samples: &mut self.samples,
            apu_ports: &mut self.apu_ports,
            cpu_ports: &mut self.cpu_ports,
            aux4: &mut self.aux4,
            aux5: &mut self.aux5,
            timer_targets: &mut self.timer_targets,
            page_flag,
            spc_pc: self.spc.pc,
            debug: Some(&mut self.debug),
        };
        self.spc.step(&mut bus);
        self.instructions += 1;
    }

    /// Drain one SMP clock from the recorded timeline.
    ///
    /// ares runs the S-SMP as a cooperative thread synchronized on every S-CPU
    /// step, so an S-CPU port poll observes exactly the SMP writes committed up
    /// to that master instant. Advancing one clock at a time (instead of one
    /// two-clock access unit) keeps a port write's visibility window to a single
    /// SMP clock.
    fn drain_unit(&mut self) -> u32 {
        if self.plan_pos >= self.plan.len() {
            self.record_instruction();
        }
        let step = self.plan[self.plan_pos];
        if step.deferred {
            let clocks = u32::from(step.clocks);
            let resume_at = if step.split { clocks / 2 } else { clocks };
            // ares `SMP::readIO($F4-$F7)` synchronizes the S-CPU forward to
            // the S-SMP clock before sampling the port. Park the read until
            // the S-CPU catches up; the system runs CPU instructions in the
            // meantime (the S-SMP clock is frozen, so this terminates).
            if self.plan_sub + 1 == resume_at && self.port_read_waiting() {
                return 0;
            }
        }
        self.units += 1;
        self.plan_sub += 1;
        if step.deferred {
            let clocks = u32::from(step.clocks);
            let resume_at = if step.split { clocks / 2 } else { clocks };
            if self.plan_sub == resume_at {
                self.resume_deferred_read();
            }
        }
        if self.plan_sub >= u32::from(step.clocks) {
            if let Some((port, value)) = step.port_write {
                // S-SMP -> S-CPU latch ($2140-$2143), committed at the end of
                // the access, which is when the S-CPU can observe it.
                self.cpu_ports[usize::from(port)] = value;
                let clock = self.debug.clock;
                self.debug.record_smp_write(clock, port, value);
            }
            self.plan_pos += 1;
            self.plan_sub = 0;
        }
        1
    }

    /// Number of queued DSP samples.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Drain queued DSP samples.
    pub fn drain_samples(&mut self) -> impl Iterator<Item = Sample> + '_ {
        self.samples.drain(..)
    }

    /// Timer internals for diagnostics: `(stage0, stage1, stage2, stage3,
    /// target, enable)`.
    pub fn timer_state(&self, index: usize) -> (u16, bool, u8, u8, u8, bool) {
        let timer = &self.timers[index];
        (
            timer.stage0,
            timer.stage1,
            timer.stage2,
            timer.stage3,
            timer.target,
            timer.enable,
        )
    }

    /// Global timer enable/disable latches for diagnostics.
    pub const fn timer_latches(&self) -> (bool, bool) {
        (self.timers_enable, self.timers_disable)
    }

    /// Queued DSP samples without draining them.
    pub fn samples(&self) -> &VecDeque<Sample> {
        &self.samples
    }

    /// Whether the heap-backed APU RAM has its expected size.
    ///
    /// Deserialized save states carry a variable-length boxed slice; a corrupt
    /// blob must be rejected before indexing panics.
    pub(crate) fn state_lengths_valid(&self) -> bool {
        self.ram.len() == APU_RAM_SIZE
    }

    /// Run the S-SMP until at least `cycles` 2.050560 MHz clocks have elapsed.
    pub fn run_cycles(&mut self, cycles: u64) -> u64 {
        let mut total = 0;
        while total < cycles {
            total += u64::from(self.drain_unit());
        }
        total
    }

    /// Advance the S-SMP by `clocks` master clocks, one S-SMP clock at a time.
    ///
    /// This is ares `Thread::synchronize` for the S-SMP: the credit is carried
    /// as an exact rational (`clocks * 2_050_560 / rate`) so the S-SMP stops at
    /// the precise master clock rather than at an instruction boundary.
    ///
    /// Returns `true` when the S-SMP parked at a `$F4-$F7` read whose sample
    /// clock is ahead of the S-CPU. The remaining credit is carried, so the
    /// next advance (after the S-CPU has caught up) resumes at the same sample.
    pub fn advance_master_clock(&mut self, clocks: u32, clock: u64, rate: u64) -> bool {
        const APU_UNIT_NUM: i64 = 2_050_560;
        let den = rate as i64;
        self.set_master_clock(clock + u64::from(clocks), rate);
        self.apu_credit += i64::from(clocks) * APU_UNIT_NUM;
        while self.apu_credit >= den {
            if self.drain_unit() == 0 {
                return true;
            }
            self.apu_credit -= den;
        }
        false
    }

    /// Test hook: pre-load master-clock credit so a `$F4-$F7` read parks with
    /// the S-CPU behind the S-SMP (the only case ares synchronizes the S-CPU).
    #[cfg(test)]
    pub(crate) fn add_credit(&mut self, clocks: u32) {
        self.apu_credit += i64::from(clocks) * 2_050_560;
    }
}

struct ApuBus<'a> {
    ram: &'a mut [u8],
    ipl: &'a [u8; IPL_SIZE],
    ipl_loaded: bool,
    iplrom_enable: &'a mut bool,
    ram_writable: &'a mut bool,
    ram_disable: &'a mut bool,
    timers_enable: &'a mut bool,
    timers_disable: &'a mut bool,
    internal_wait: &'a mut u8,
    external_wait: &'a mut u8,
    timers: &'a mut [Timer; 3],
    dsp_address: &'a mut u8,
    dsp: &'a mut Dsp,
    smp_clock: &'a mut u128,
    dsp_clock: &'a mut u128,
    samples: &'a mut VecDeque<Sample>,
    apu_ports: &'a mut [u8; 4],
    cpu_ports: &'a mut [u8; 4],
    aux4: &'a mut u8,
    aux5: &'a mut u8,
    timer_targets: &'a mut [u8; 3],
    page_flag: bool,
    /// Debug: PC of the instruction being recorded.
    spc_pc: u16,
    /// Debug: port handshake log, absent in the unit-test buses.
    debug: Option<&'a mut PortDebug>,
}

impl ApuBus<'_> {
    /// Advance the S-DSP by `clocks` S-SMP clocks (ares `SMP::step` +
    /// `Thread::synchronize(dsp)`).
    ///
    /// Advance the S-DSP by `clocks` S-SMP clocks (ares `SMP::step` +
    /// `Thread::synchronize(dsp)`).
    ///
    /// Both thread clocks use the ares fixed-point base and integer scalars,
    /// and a micro-step runs while the DSP clock is behind the S-SMP clock.
    /// This derives the DSP phase instead of assuming two S-SMP clocks per
    /// micro-step: because `24 * DSP_SCALAR` is four fixed-point units less
    /// than two `SMP_SCALAR`s, the first synchronization runs one extra tick
    /// and the DSP stays one micro-step ahead, exactly as ares does.
    fn step_dsp_clock(&mut self, clocks: u32) {
        *self.smp_clock += u128::from(clocks) * SMP_SCALAR;
        while *self.dsp_clock < *self.smp_clock {
            if let Some(sample) = self.dsp.tick(self.ram) {
                self.samples.push_back(sample);
                if self.samples.len() > 16_384 {
                    self.samples.pop_front();
                }
            }
            *self.dsp_clock += DSP_TICK;
        }
    }

    /// Write the underlying APU RAM (ares `writeRAM`).
    fn write_ram(&mut self, address: u16, value: u8) {
        if *self.ram_writable && !*self.ram_disable {
            self.ram[usize::from(address)] = value;
        }
    }

    /// Wait-state selector for an access: RAM uses the external dividers;
    /// I/O, the IPL ROM and idle cycles use the internal dividers
    /// (ares `SMP::wait`).
    fn wait_selector(&self, address: Option<u16>) -> u8 {
        match address {
            None => *self.internal_wait,
            Some(address)
                if address & 0xFFF0 == 0x00F0 || (address >= 0xFFC0 && *self.iplrom_enable) =>
            {
                *self.internal_wait
            }
            _ => *self.external_wait,
        }
        .min(3)
    }
}

impl SpcBus for ApuBus<'_> {
    fn advance(&mut self, clocks: u32) {
        for (index, timer) in self.timers.iter_mut().enumerate() {
            let frequency = if index == 2 { 16 } else { 128 };
            timer.step(clocks, frequency, *self.timers_enable, *self.timers_disable);
        }
    }

    fn step_clock(&mut self, clocks: u32) {
        self.step_dsp_clock(clocks);
    }

    fn wait_cycles(&mut self, address: Option<u16>) -> u32 {
        // Time is accounted in access units (one unit is two S-SMP clocks).
        // ares `cycleWaitStates` is 2/4/10/20 clocks: the 8/16 settings cost
        // 10/20 on silicon, the documented hardware glitch.
        const CYCLE_WAIT: [u32; 4] = [1, 2, 5, 10];
        CYCLE_WAIT[usize::from(self.wait_selector(address))]
    }

    fn timer_cycles(&mut self, address: Option<u16>) -> u32 {
        // ares `timerWaitStates`: the timers are not affected by the glitch.
        const TIMER_WAIT: [u32; 4] = [2, 4, 8, 16];
        TIMER_WAIT[usize::from(self.wait_selector(address))]
    }

    fn split_port_read(&mut self, address: u16) -> bool {
        address & 0xFFFC == 0x00F4
    }

    fn defer_io_reads(&self) -> bool {
        true
    }

    fn read(&mut self, address: u16) -> u8 {
        match address {
            0xF0 | 0xF1 | 0xFA | 0xFB | 0xFC => 0,
            0xF2 => *self.dsp_address,
            0xF3 => self.dsp.read(*self.dsp_address),
            0xF4..=0xF7 => {
                let port = u8::try_from(address - 0xF4).unwrap_or(0);
                let value = self.apu_ports[usize::from(port)];
                if let Some(debug) = self.debug.as_deref_mut() {
                    let clock = debug.clock;
                    let spc_instruction = debug.spc_instruction;
                    let spc_pc = self.spc_pc;
                    debug.record(PortEvent {
                        clock,
                        kind: PortEventKind::SmpRead,
                        port,
                        value,
                        spc_pc,
                        spc_instruction,
                    });
                }
                value
            }
            0xF8 => *self.aux4,
            0xF9 => *self.aux5,
            0xFD..=0xFF => {
                let timer = &mut self.timers[usize::from(address - 0xFD)];
                let value = timer.stage3;
                timer.stage3 = 0;
                value
            }
            _ => {
                if self.ipl_loaded && *self.iplrom_enable && address >= 0xFFC0 {
                    self.ipl[usize::from(address & 0x3F)]
                } else if *self.ram_disable {
                    0x5A
                } else {
                    self.ram[usize::from(address)]
                }
            }
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        self.write_ram(address, value);
        match address {
            0xF0 => {
                // ares: writes are only valid when the SPC700 P flag is clear.
                if self.page_flag {
                    return;
                }
                *self.timers_disable = value & 0x01 != 0;
                *self.ram_writable = value & 0x02 != 0;
                *self.ram_disable = value & 0x04 != 0;
                *self.timers_enable = value & 0x08 != 0;
                // ares: external wait = bits 4-5, internal wait = bits 6-7.
                *self.external_wait = (value >> 4) & 0x03;
                *self.internal_wait = (value >> 6) & 0x03;
                for timer in self.timers.iter_mut() {
                    timer.synchronize(*self.timers_enable, *self.timers_disable);
                }
            }
            0xF1 => {
                for (index, timer) in self.timers.iter_mut().enumerate() {
                    let enable = value & (1 << index) != 0;
                    let rising = enable && !timer.enable;
                    timer.enable = enable;
                    // ares resets timers 0/1 on a rising enable edge, but
                    // timer 2 on every write that is not a rising edge.
                    if rising != (index == 2) {
                        timer.stage2 = 0;
                        timer.stage3 = 0;
                    }
                }
                // ares: `$F1` bit 4 clears the S-CPU`s `$2140/$2141` latches
                // and bit 5 clears `$2142/$2143` (the sound driver's port
                // reset). The SMP reads these at `$F4-$F7`.
                if value & 0x10 != 0 {
                    self.apu_ports[0] = 0;
                    self.apu_ports[1] = 0;
                }
                if value & 0x20 != 0 {
                    self.apu_ports[2] = 0;
                    self.apu_ports[3] = 0;
                }
                *self.iplrom_enable = value & 0x80 != 0;
                if value & 0x10 != 0 {
                    self.apu_ports[0] = 0;
                    self.apu_ports[1] = 0;
                }
                if value & 0x20 != 0 {
                    self.apu_ports[2] = 0;
                    self.apu_ports[3] = 0;
                }
            }
            0xF2 => *self.dsp_address = value,
            0xF3 => {
                self.dsp.set_debug_pc(self.spc_pc);
                self.dsp.set_debug_clock(*self.smp_clock);
                self.dsp.write(*self.dsp_address, value);
            }
            0xF4..=0xF7 => {
                self.cpu_ports[usize::from(address - 0xF4)] = value;
                if std::env::var_os("APU_ACTOR_LOG").is_some() {
                    eprintln!(
                        "AW {:04X} -> {:02X} smp={}",
                        address,
                        value,
                        (*self.smp_clock / SMP_SCALAR)
                    );
                }
            }
            0xF8 => *self.aux4 = value,
            0xF9 => *self.aux5 = value,
            0xFA..=0xFC => {
                self.timer_targets[usize::from(address - 0xFA)] = value;
                self.timers[usize::from(address - 0xFA)].target = value;
            }
            _ => {}
        }
    }
}

/// Recording bus: runs one SPC700 instruction, applies every side effect
/// immediately except the S-SMP-to-S-CPU port latch, and records the access
/// timeline so [`Apu`] can drain it clock by clock.
struct RecordBus<'a, 'b> {
    inner: &'a mut ApuBus<'b>,
    plan: &'a mut Vec<PlanStep>,
    /// Access units recorded so far in this instruction.
    units: u64,
}

impl SpcBus for RecordBus<'_, '_> {
    fn wait_cycles(&mut self, address: Option<u16>) -> u32 {
        self.inner.wait_cycles(address)
    }

    fn timer_cycles(&mut self, address: Option<u16>) -> u32 {
        self.inner.timer_cycles(address)
    }

    fn split_port_read(&mut self, address: u16) -> bool {
        self.inner.split_port_read(address)
    }

    fn defer_io_reads(&self) -> bool {
        self.inner.defer_io_reads()
    }

    fn advance(&mut self, clocks: u32) {
        self.inner.advance(clocks);
    }

    fn step_clock(&mut self, clocks: u32) {
        // The DSP advances while the instruction is recorded (ares runs it as a
        // cooperative thread inside `SMP::wait`), which keeps every DSP register
        // access ordered exactly one micro-step after its wait, as on hardware.
        self.inner.step_clock(clocks);
    }

    fn read(&mut self, address: u16) -> u8 {
        let clocks = (self.inner.wait_cycles(Some(address)) * 2) as u8;
        let value = self.inner.read(address);
        self.plan.push(PlanStep {
            clocks,
            port_write: None,
            deferred: false,
            split: false,
        });
        self.units += u64::from(clocks);
        value
    }

    fn write(&mut self, address: u16, value: u8) {
        let clocks = (self.inner.wait_cycles(Some(address)) * 2) as u8;
        self.units += u64::from(clocks);
        if (0xF4..=0xF7).contains(&address) {
            // ares writes the RAM mirror immediately and latches the port; the
            // latch itself is only visible to the S-CPU at the access's end.
            self.inner.write_ram(address, value);
            self.plan.push(PlanStep {
                clocks,
                port_write: Some(((address - 0xF4) as u8, value)),
                deferred: false,
                split: false,
            });
        } else {
            self.inner.write(address, value);
            self.plan.push(PlanStep {
                clocks,
                port_write: None,
                deferred: false,
                split: false,
            });
        }
    }

    fn idle(&mut self) {
        let clocks = (self.inner.wait_cycles(None) * 2) as u8;
        self.plan.push(PlanStep {
            clocks,
            port_write: None,
            deferred: false,
            split: false,
        });
        self.units += u64::from(clocks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ipl() -> [u8; IPL_SIZE] {
        let mut ipl = [0u8; IPL_SIZE];
        // reset vector -> $FFC0
        ipl[62] = 0xC0;
        ipl[63] = 0xFF;
        ipl
    }

    #[test]
    fn reset_uses_ipl_vector() {
        let apu = Apu::with_ipl(ipl());
        assert!(apu.has_ipl());
        assert_eq!(apu.spc.pc, 0xFFC0);
        assert_eq!(apu.spc.sp, 0xEF);
    }

    #[test]
    fn cpu_ports_are_bidirectional() {
        let mut apu = Apu::new();
        apu.cpu_write(0, 0x12);
        assert_eq!(apu.apu_ports[0], 0x12);
        apu.cpu_ports[1] = 0x34;
        assert_eq!(apu.cpu_read(1), 0x34);
    }

    #[test]
    fn smp_port_reads_see_only_the_cpu_latches() {
        // ares keeps two independent four-byte latches: `io.apu0..3` written by
        // the S-CPU and read by the S-SMP (`$F4-$F7`), and `io.cpu0..3` written
        // by the S-SMP and read by the S-CPU (`$2140-$2143`). A program that
        // writes `$F6` and reads it back therefore sees the *CPU's* value, not
        // its own; blargg `spc_smp` test 9 depends on exactly that.
        let mut apu = Apu::new();
        {
            let mut bus = ApuBus {
                ram: &mut apu.ram[..],
                ipl: &apu.ipl,
                ipl_loaded: apu.ipl_loaded,
                iplrom_enable: &mut apu.iplrom_enable,
                ram_writable: &mut apu.ram_writable,
                ram_disable: &mut apu.ram_disable,
                timers_enable: &mut apu.timers_enable,
                timers_disable: &mut apu.timers_disable,
                internal_wait: &mut apu.internal_wait,
                external_wait: &mut apu.external_wait,
                timers: &mut apu.timers,
                dsp_address: &mut apu.dsp_address,
                dsp: &mut apu.dsp,
                smp_clock: &mut apu.smp_clock,
                dsp_clock: &mut apu.dsp_clock,
                samples: &mut apu.samples,
                apu_ports: &mut apu.apu_ports,
                cpu_ports: &mut apu.cpu_ports,
                aux4: &mut apu.aux4,
                aux5: &mut apu.aux5,
                timer_targets: &mut apu.timer_targets,
                page_flag: false,
                spc_pc: 0,
                debug: None,
            };
            // The S-CPU has not written yet.
            assert_eq!(SpcBus::read(&mut bus, 0x00F6), 0);
            // The S-SMP writes its own latch; a read still sees the CPU side.
            SpcBus::write(&mut bus, 0x00F6, 0xFF);
            assert_eq!(SpcBus::read(&mut bus, 0x00F6), 0);
            *bus.apu_ports = [0, 0, 0x5A, 0];
            assert_eq!(SpcBus::read(&mut bus, 0x00F6), 0x5A);
        }
        // And the S-SMP write became visible to the S-CPU.
        assert_eq!(apu.cpu_read(2), 0xFF);
    }

    #[test]
    fn control_clears_the_port_latches() {
        // ares `SMP::writeIO` case 0xf1: bit 4 clears `io.apu0`/`io.apu1` and
        // bit 5 clears `io.apu2`/`io.apu3` (the latches the S-SMP reads at
        // `$F4-$F7`, i.e. whatever the S-CPU last wrote to `$2140-$2143`).
        let mut apu = Apu::new();
        apu.cpu_write(0, 0xAA);
        apu.cpu_write(1, 0xBB);
        apu.cpu_write(2, 0xCC);
        apu.cpu_write(3, 0xDD);
        {
            let mut bus = ApuBus {
                ram: &mut apu.ram[..],
                ipl: &apu.ipl,
                ipl_loaded: apu.ipl_loaded,
                iplrom_enable: &mut apu.iplrom_enable,
                ram_writable: &mut apu.ram_writable,
                ram_disable: &mut apu.ram_disable,
                timers_enable: &mut apu.timers_enable,
                timers_disable: &mut apu.timers_disable,
                internal_wait: &mut apu.internal_wait,
                external_wait: &mut apu.external_wait,
                timers: &mut apu.timers,
                dsp_address: &mut apu.dsp_address,
                dsp: &mut apu.dsp,
                smp_clock: &mut apu.smp_clock,
                dsp_clock: &mut apu.dsp_clock,
                samples: &mut apu.samples,
                apu_ports: &mut apu.apu_ports,
                cpu_ports: &mut apu.cpu_ports,
                aux4: &mut apu.aux4,
                aux5: &mut apu.aux5,
                timer_targets: &mut apu.timer_targets,
                page_flag: false,
                spc_pc: 0,
                debug: None,
            };
            SpcBus::write(&mut bus, 0x00F1, 0x10);
        }
        assert_eq!(apu.apu_ports, [0x00, 0x00, 0xCC, 0xDD]);
        {
            let mut bus = ApuBus {
                ram: &mut apu.ram[..],
                ipl: &apu.ipl,
                ipl_loaded: apu.ipl_loaded,
                iplrom_enable: &mut apu.iplrom_enable,
                ram_writable: &mut apu.ram_writable,
                ram_disable: &mut apu.ram_disable,
                timers_enable: &mut apu.timers_enable,
                timers_disable: &mut apu.timers_disable,
                internal_wait: &mut apu.internal_wait,
                external_wait: &mut apu.external_wait,
                timers: &mut apu.timers,
                dsp_address: &mut apu.dsp_address,
                dsp: &mut apu.dsp,
                smp_clock: &mut apu.smp_clock,
                dsp_clock: &mut apu.dsp_clock,
                samples: &mut apu.samples,
                apu_ports: &mut apu.apu_ports,
                cpu_ports: &mut apu.cpu_ports,
                aux4: &mut apu.aux4,
                aux5: &mut apu.aux5,
                timer_targets: &mut apu.timer_targets,
                page_flag: false,
                spc_pc: 0,
                debug: None,
            };
            SpcBus::write(&mut bus, 0x00F1, 0x20);
        }
        assert_eq!(apu.apu_ports, [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn dspdata_write_reads_back_immediately() {
        // ares `DSP::write` stores the value in the register file before the
        // per-register side effect, and `DSP::read` returns that file, so a
        // write followed by a read of `$F3` returns the new value even for a
        // read-only register such as `VxENVX` (`$x8`). blargg `spc_dsp6` spins
        // on exactly this (`str $F2=#$08`, write `$88`, read back until the
        // per-sample latch replaces it).
        let mut apu = Apu::new();
        let mut bus = ApuBus {
            ram: &mut apu.ram[..],
            ipl: &apu.ipl,
            ipl_loaded: apu.ipl_loaded,
            iplrom_enable: &mut apu.iplrom_enable,
            ram_writable: &mut apu.ram_writable,
            ram_disable: &mut apu.ram_disable,
            timers_enable: &mut apu.timers_enable,
            timers_disable: &mut apu.timers_disable,
            internal_wait: &mut apu.internal_wait,
            external_wait: &mut apu.external_wait,
            timers: &mut apu.timers,
            dsp_address: &mut apu.dsp_address,
            dsp: &mut apu.dsp,
            smp_clock: &mut apu.smp_clock,
            dsp_clock: &mut apu.dsp_clock,
            samples: &mut apu.samples,
            apu_ports: &mut apu.apu_ports,
            cpu_ports: &mut apu.cpu_ports,
            aux4: &mut apu.aux4,
            aux5: &mut apu.aux5,
            timer_targets: &mut apu.timer_targets,
            page_flag: false,
            spc_pc: 0,
            debug: None,
        };
        SpcBus::write(&mut bus, 0x00F2, 0x08); // DSPADDR = V0ENVX
        SpcBus::write(&mut bus, 0x00F3, 0x88); // DSPDATA = $88
        assert_eq!(SpcBus::read(&mut bus, 0x00F3), 0x88);
        // The register file keeps the value until the S-DSP's per-sample latch
        // update overwrites it, which is what the test loop waits for.
        assert_eq!(bus.dsp.read(0x08), 0x88);
    }

    #[test]
    fn control_disables_ipl_rom() {
        let mut apu = Apu::with_ipl(ipl());
        apu.ram_mut()[0xFFC0] = 0xAA;
        let mut bus = ApuBus {
            ram: &mut apu.ram[..],
            ipl: &apu.ipl,
            ipl_loaded: apu.ipl_loaded,
            iplrom_enable: &mut apu.iplrom_enable,
            ram_writable: &mut apu.ram_writable,
            ram_disable: &mut apu.ram_disable,
            timers_enable: &mut apu.timers_enable,
            timers_disable: &mut apu.timers_disable,
            internal_wait: &mut apu.internal_wait,
            external_wait: &mut apu.external_wait,
            timers: &mut apu.timers,
            dsp_address: &mut apu.dsp_address,
            dsp: &mut apu.dsp,
            smp_clock: &mut apu.smp_clock,
            dsp_clock: &mut apu.dsp_clock,
            samples: &mut apu.samples,
            apu_ports: &mut apu.apu_ports,
            cpu_ports: &mut apu.cpu_ports,
            aux4: &mut apu.aux4,
            aux5: &mut apu.aux5,
            timer_targets: &mut apu.timer_targets,
            page_flag: false,
            spc_pc: 0,
            debug: None,
        };
        assert_eq!(bus.read(0xFFC0), 0x00);
        bus.write(0xF1, 0x00);
        assert!(!*bus.iplrom_enable);
        assert_eq!(bus.read(0xFFC0), 0xAA);
    }

    #[test]
    fn executes_spc700_from_ram() {
        let mut apu = Apu::new();
        apu.spc.pc = 0x0200;
        apu.ram_mut()[0x0200] = 0xE8; // LDA #$42
        apu.ram_mut()[0x0201] = 0x42;
        apu.ram_mut()[0x0202] = 0x00; // NOP
        let cycles = apu.step();
        assert!(cycles >= 2);
        assert_eq!(apu.spc.a, 0x42);
    }
}

#[cfg(test)]
mod dsp_integration_tests {
    use super::*;

    #[test]
    fn apu_runs_dsp_when_spc_executes() {
        let mut apu = Apu::new();
        // NOP sled at $0500 so the SPC keeps retiring cycles.
        for byte in &mut apu.ram_mut()[0x0500..0x0580] {
            *byte = 0x00;
        }
        apu.spc.pc = 0x0500;
        // Minimal looping BRR sample at $0300.
        apu.ram_mut()[0x0200] = 0x00;
        apu.ram_mut()[0x0201] = 0x03;
        apu.ram_mut()[0x0202] = 0x00;
        apu.ram_mut()[0x0203] = 0x03;
        apu.ram_mut()[0x0300] = 0x03;
        for (index, byte) in [0x12u8, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]
            .into_iter()
            .enumerate()
        {
            apu.ram_mut()[0x0301 + index] = byte;
        }
        {
            let dsp = apu.dsp_mut();
            dsp.write(0x0C, 0x7F);
            dsp.write(0x1C, 0x7F);
            dsp.write(0x00, 0x7F);
            dsp.write(0x01, 0x7F);
            dsp.write(0x02, 0x00);
            dsp.write(0x03, 0x10);
            dsp.write(0x04, 0x00);
            dsp.write(0x07, 0x7F);
            dsp.write(0x5D, 0x02);
            dsp.write(0x6C, 0x00);
            dsp.write(0x4C, 0x01);
        }
        let mut steps = 0;
        while apu.sample_count() < 64 && steps < 100_000 {
            apu.step();
            steps += 1;
        }
        assert!(apu.sample_count() >= 64, "DSP did not produce samples");
        assert!(apu.drain_samples().any(|sample| sample != (0, 0)));
    }
}

#[cfg(test)]
mod wait_state_tests {
    use super::*;

    fn bus<'a>(apu: &'a mut Apu) -> ApuBus<'a> {
        ApuBus {
            ram: &mut apu.ram[..],
            ipl: &apu.ipl,
            ipl_loaded: apu.ipl_loaded,
            iplrom_enable: &mut apu.iplrom_enable,
            ram_writable: &mut apu.ram_writable,
            ram_disable: &mut apu.ram_disable,
            timers_enable: &mut apu.timers_enable,
            timers_disable: &mut apu.timers_disable,
            internal_wait: &mut apu.internal_wait,
            external_wait: &mut apu.external_wait,
            timers: &mut apu.timers,
            dsp_address: &mut apu.dsp_address,
            dsp: &mut apu.dsp,
            smp_clock: &mut apu.smp_clock,
            dsp_clock: &mut apu.dsp_clock,
            samples: &mut apu.samples,
            apu_ports: &mut apu.apu_ports,
            cpu_ports: &mut apu.cpu_ports,
            aux4: &mut apu.aux4,
            aux5: &mut apu.aux5,
            timer_targets: &mut apu.timer_targets,
            page_flag: false,
            spc_pc: 0,
            debug: None,
        }
    }

    #[test]
    fn control_register_scales_access_time() {
        let mut apu = Apu::new();
        apu.ram_mut()[0x0200] = 0xE8; // LDA #$42
        apu.ram_mut()[0x0201] = 0x42;
        apu.spc.pc = 0x0200;
        // Two RAM accesses at two clocks each.
        assert_eq!(apu.step(), 4);

        apu.spc.pc = 0x0200;
        {
            let mut bus = bus(&mut apu);
            // External wait state 1 (bits 4-5): four clocks per access.
            bus.write(0xF0, 0x10);
        }
        assert_eq!(apu.step(), 8);

        apu.spc.pc = 0x0200;
        {
            let mut bus = bus(&mut apu);
            // Internal wait state 2 (bits 6-7) applies to I/O and idle cycles;
            // RAM keeps using the external divider.
            bus.write(0xF0, 0x80);
            assert_eq!(bus.wait_cycles(Some(0x00F4)), 5);
            assert_eq!(bus.wait_cycles(None), 5);
            assert_eq!(bus.wait_cycles(Some(0x0200)), 1);
            // The timers are not affected by the 10/20-clock glitch: an
            // internal wait state of 2 costs 8 timer clocks, not 10.
            assert_eq!(bus.timer_cycles(Some(0x00F4)), 8);
            assert_eq!(bus.timer_cycles(None), 8);
        }
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn timer_reaches_output() {
        let mut apu = Apu::new();
        apu.timers[0].target = 1;
        apu.timers[0].enable = true;
        for _ in 0..2000 {
            apu.timers[0].step(2, 128, true, false);
        }
        assert!(apu.timers[0].stage3 > 0);
    }
}

#[cfg(test)]
mod clock_position_tests {
    use super::*;

    #[test]
    fn smp_clock_tracks_the_plan_position() {
        // record_instruction advances smp_clock to the instruction end; the
        // executed clock must instead follow the drained plan position.
        let mut apu = Apu::new();
        apu.plan = vec![
            PlanStep {
                clocks: 4,
                port_write: None,
                deferred: false,
                split: false,
            },
            PlanStep {
                clocks: 6,
                port_write: None,
                deferred: false,
                split: false,
            },
        ];
        apu.smp_clock = 10 * SMP_SCALAR;
        assert_eq!(apu.smp_clock_at_position() / SMP_SCALAR, 0);
        apu.plan_sub = 2;
        assert_eq!(apu.smp_clock_at_position() / SMP_SCALAR, 2);
        apu.plan_pos = 1;
        apu.plan_sub = 0;
        assert_eq!(apu.smp_clock_at_position() / SMP_SCALAR, 4);
        apu.plan_sub = 5;
        assert_eq!(apu.smp_clock_at_position() / SMP_SCALAR, 9);
    }
}

// ---------------------------------------------------------------------------
// Mutual S-CPU / S-SMP coroutine (ares `Thread` parity)
//
// The actor advances the S-SMP with the same clock credit as the scheduler
// ([`Apu::advance_master_clock`]): one S-SMP clock per unit of master-clock
// credit, stopping exactly at the S-CPU clock. A `$F4-$F7` read whose sample
// clock is ahead of the S-CPU parks the S-SMP and hands the `Apu` back
// ([`ApuOutput::Parked`]); the scheduler owns the `Apu` while parked, so S-CPU
// port accesses are ordinary bus accesses, and the next advance resolves the
// read. The `Apu` is owned by the scheduler between advances, so save states
// need no special case.
// ---------------------------------------------------------------------------

/// The S-SMP actor coroutine type.
pub type ApuCoroutine = corosensei::Coroutine<ApuInput, ApuOutput, ()>;

/// Input to the S-SMP actor coroutine.
#[derive(Debug)]
pub enum ApuInput {
    /// Hand the APU to the actor and advance the S-SMP by `clocks` master
    /// clocks, stopping exactly at the S-CPU clock.
    Advance {
        /// The APU, owned by the scheduler between advances.
        apu: Box<Apu>,
        /// Master clocks elapsed since the previous advance.
        clocks: u32,
        /// S-CPU master clock at the start of the access.
        clock: u64,
        /// Master clock rate paired with `clock`.
        rate: u64,
    },
}

/// Output from the S-SMP actor coroutine.
#[derive(Debug)]
pub enum ApuOutput {
    /// The S-SMP stopped at a clock boundary (or mid-instruction after a
    /// resolved `$F4-$F7` read); the actor hands the APU back.
    Stopped(Box<Apu>),
    /// The S-SMP is parked at a `$F4-$F7` read. Run the S-CPU until `target`,
    /// then advance again to resolve the read.
    Parked {
        /// The APU, owned by the scheduler while parked.
        apu: Box<Apu>,
        /// Master clock the S-CPU must reach before the read can sample.
        target: u64,
    },
}

/// Build the S-SMP actor coroutine.
pub fn new_apu_coroutine() -> ApuCoroutine {
    corosensei::Coroutine::new(|yielder, input| {
        let ApuInput::Advance {
            apu,
            clocks,
            clock,
            rate,
        } = input;
        let mut apu = apu;
        let (mut clocks, mut clock, mut rate) = (clocks, clock, rate);
        loop {
            let output = if apu.advance_master_clock(clocks, clock, rate) {
                let target = apu.smp_master_clock(rate);
                ApuOutput::Parked { apu, target }
            } else {
                ApuOutput::Stopped(apu)
            };
            let ApuInput::Advance {
                apu: next,
                clocks: c,
                clock: k,
                rate: r,
            } = yielder.suspend(output);
            apu = next;
            clocks = c;
            clock = k;
            rate = r;
        }
    })
}

#[cfg(test)]
mod actor_tests {
    use super::*;

    /// Seed an APU whose `$200` program reads `$F4` and stores it at `$00`.
    fn port_read_program() -> Apu {
        let mut apu = Apu::new();
        // E4 F4: mov a,$F4 ; C4 00: mov $00,a ; FF: stop
        apu.ram[0x200..0x205].copy_from_slice(&[0xE4, 0xF4, 0xC4, 0x00, 0xFF]);
        apu.spc.pc = 0x0200;
        apu.apu_ports[0] = 0x5A;
        apu
    }

    const RATE: u64 = 21_477_272;

    fn clocks() -> u32 {
        // Enough master clocks for about 40 S-SMP clocks (the test program is
        // shorter than that).
        u32::try_from(RATE * 40 / 2_050_560).unwrap()
    }

    /// Run one advance with the S-CPU held at clock 0, returning the `Apu` and
    /// the park target if parked. The credit is pre-loaded because a park only
    /// happens while the S-SMP is ahead of the S-CPU.
    fn park(coro: &mut ApuCoroutine, mut apu: Box<Apu>) -> (Box<Apu>, Option<u64>) {
        apu.add_credit(clocks());
        match coro.resume(ApuInput::Advance {
            apu,
            clocks: 0,
            clock: 0,
            rate: RATE,
        }) {
            corosensei::CoroutineResult::Yield(ApuOutput::Parked { apu, target }) => {
                (apu, Some(target))
            }
            corosensei::CoroutineResult::Yield(ApuOutput::Stopped(apu)) => (apu, None),
            corosensei::CoroutineResult::Return(()) => panic!("actor returned"),
        }
    }

    fn advance(coro: &mut ApuCoroutine, apu: Box<Apu>, clock: u64) -> Box<Apu> {
        match coro.resume(ApuInput::Advance {
            apu,
            clocks: clocks(),
            clock,
            rate: RATE,
        }) {
            corosensei::CoroutineResult::Yield(ApuOutput::Stopped(apu)) => apu,
            other => panic!("expected Stopped, got {other:?}"),
        }
    }

    #[test]
    fn actor_parks_at_the_port_sample_and_resumes() {
        let mut coro = new_apu_coroutine();
        let (apu, target) = park(&mut coro, Box::new(port_read_program()));
        let target = target.expect("the $F4 read must park the S-SMP");
        // The next advance with the S-CPU at the sample resolves the read and
        // runs the following instruction.
        let apu = advance(&mut coro, apu, target);
        assert_eq!(apu.ram[0], 0x5A);
    }

    #[test]
    fn port_writes_apply_while_the_read_is_parked() {
        let mut coro = new_apu_coroutine();
        let (mut apu, target) = park(&mut coro, Box::new(port_read_program()));
        let target = target.expect("the $F4 read must park the S-SMP");
        // The S-CPU writes the latch while the S-SMP is parked; the read
        // samples it when the next advance resolves.
        apu.cpu_write(0, 0x77);
        assert_eq!(apu.cpu_read(0), 0x00);
        let apu = advance(&mut coro, apu, target);
        assert_eq!(apu.ram[0], 0x77);
    }

    #[test]
    fn actor_resumes_a_serialized_mid_instruction_state() {
        // Park mid-instruction, serialize, and rebuild the actor from the
        // restored `Apu`: the S-SMP plan position must resume exactly.
        let mut coro = new_apu_coroutine();
        let (apu, target) = park(&mut coro, Box::new(port_read_program()));
        let target = target.expect("the $F4 read must park the S-SMP");
        let bytes = bincode::serialize(&*apu).expect("serialize");
        let restored: Apu = bincode::deserialize(&bytes).expect("deserialize");
        let mut coro = new_apu_coroutine();
        let apu = advance(&mut coro, Box::new(restored), target);
        assert_eq!(apu.ram[0], 0x5A);
    }
}
