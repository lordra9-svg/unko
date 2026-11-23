//! Mutually-resumable S-CPU / S-SMP / S-DSP scheduler (experimental scaffolding).
//!
//! This module is a dedicated, default-off branch for the driver rewrite
//! described in `docs/snes-accuracy-divergences.md`. It does **not** run
//! emulation and is not wired into [`crate::system::Snes`]; the verified
//! record/plan driver remains the only active one.
//!
//! # Target model (ares)
//!
//! ares runs the S-CPU and the S-SMP as cooperative threads and the S-DSP as a
//! third thread. Every thread entry calls `scheduler.synchronize()`, which runs
//! the other threads until their clocks reach the current thread's clock, in
//! whole instructions (`Thread::Enter`). Inside `DSP::tick`, the DSP advances 24
//! clocks and then calls `synchronize(smp)`, so the S-SMP is pulled forward
//! from *inside* the DSP micro-step. `SMP::step` in turn calls
//! `synchronize(cpu)` and `synchronize(dsp)`. The result is a mutually
//! recursive schedule that can only be expressed when all three threads can
//! suspend and resume each other.
//!
//! # Why the current driver cannot express it
//!
//! The plan/replay driver executes the S-SMP synchronously (recording an
//! instruction into a micro-op timeline) and advances the S-DSP from the same
//! call stack. Neither an S-SMP instruction nor the S-DSP can suspend the S-CPU,
//! so the direction "DSP pulls the S-SMP pulls the S-CPU" is not expressible.
//!
//! # Scaffolding
//!
//! [`RoundRobin`] models the thread-selection bookkeeping the rewritten driver
//! needs: each thread publishes its clock, the scheduler picks the thread that
//! is furthest behind, and DSP micro-step resumption points are recorded so the
//! driver can re-enter the S-SMP at the same access. It is pure state, with
//! unit tests, so the scheduler semantics can be validated before any
//! emulation is moved onto it.

/// One of the three cooperative threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thread {
    /// The S-CPU.
    Cpu,
    /// The S-SMP.
    Smp,
    /// The S-DSP.
    Dsp,
}

/// The point at which a DSP micro-step needs to resume the S-SMP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicroStep {
    /// DSP clock (24 DSP clocks per micro-step) at the resume point.
    pub dsp_clock: u128,
    /// S-SMP clock the pulled thread must reach before the DSP continues.
    pub resume_smp_at: u128,
}

/// Round-robin bookkeeping for the mutually-resumable scheduler.
///
/// Clocks are in the emulator's fixed-point time base, so they are directly
/// comparable between the three threads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoundRobin {
    cpu_clock: u128,
    smp_clock: u128,
    dsp_clock: u128,
    pending: Vec<MicroStep>,
}

impl RoundRobin {
    /// Create a scheduler at the given initial clocks.
    pub const fn new(cpu_clock: u128, smp_clock: u128, dsp_clock: u128) -> Self {
        Self {
            cpu_clock,
            smp_clock,
            dsp_clock,
            pending: Vec::new(),
        }
    }

    /// Publish the S-CPU clock.
    pub fn set_cpu_clock(&mut self, clock: u128) {
        self.cpu_clock = clock;
    }

    /// Publish the S-SMP clock.
    pub fn set_smp_clock(&mut self, clock: u128) {
        self.smp_clock = clock;
    }

    /// Publish the S-DSP clock.
    pub fn set_dsp_clock(&mut self, clock: u128) {
        self.dsp_clock = clock;
    }

    /// The thread with the smallest clock, i.e. the one that should run next.
    pub fn next(&self) -> Thread {
        let mut thread = Thread::Cpu;
        let mut clock = self.cpu_clock;
        if self.smp_clock < clock {
            thread = Thread::Smp;
            clock = self.smp_clock;
        }
        if self.dsp_clock < clock {
            thread = Thread::Dsp;
        }
        thread
    }

    /// Record that a DSP micro-step at `dsp_clock` must resume the S-SMP at
    /// `resume_smp_at` before the DSP can continue.
    pub fn push_resume(&mut self, dsp_clock: u128, resume_smp_at: u128) {
        self.pending.push(MicroStep {
            dsp_clock,
            resume_smp_at,
        });
    }

    /// The next pending DSP micro-step resumption point whose S-SMP target has
    /// been reached.
    pub fn take_ready(&mut self) -> Option<MicroStep> {
        let ready = self
            .pending
            .iter()
            .position(|step| self.smp_clock >= step.resume_smp_at)?;
        Some(self.pending.remove(ready))
    }

    /// Number of micro-step resumptions still waiting on the S-SMP.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_thread_furthest_behind() {
        let mut rr = RoundRobin::new(100, 50, 75);
        assert_eq!(rr.next(), Thread::Smp);
        rr.set_smp_clock(200);
        assert_eq!(rr.next(), Thread::Dsp);
        rr.set_dsp_clock(300);
        assert_eq!(rr.next(), Thread::Cpu);
    }

    #[test]
    fn resume_points_wait_for_the_smp_clock() {
        let mut rr = RoundRobin::new(0, 10, 0);
        rr.push_resume(48, 12);
        rr.push_resume(72, 20);
        assert!(rr.take_ready().is_none());
        rr.set_smp_clock(12);
        assert_eq!(rr.take_ready().unwrap().resume_smp_at, 12);
        assert_eq!(rr.pending_len(), 1);
        rr.set_smp_clock(20);
        assert_eq!(rr.take_ready().unwrap().dsp_clock, 72);
        assert_eq!(rr.pending_len(), 0);
    }
}

/// ares `Thread::Second`: the fixed-point base of the scheduler clocks.
pub const SECOND: u128 = (1u128 << 63) - 1;

/// One ares `Thread` clock: an absolute fixed-point value plus the scalar that
/// converts hardware clocks to it.
///
/// ares `Thread::setFrequency(f)` stores `_frequency = f + 0.5` (as a `u64`,
/// so integer frequencies are unchanged) and `_scalar = Second / _frequency`.
/// `Thread::step(clocks)` then does `_clock += _scalar * clocks`. Reproducing
/// this arithmetic exactly is the prerequisite for the cycle-accurate rewrite;
/// the emulator's active driver still uses its own carried-credit model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadClock {
    scalar: u128,
    clock: u128,
}

impl ThreadClock {
    /// ares `Thread::create(frequency)`: zero clock, `scalar = Second / f`.
    pub fn new(frequency: u64) -> Self {
        Self {
            scalar: SECOND / u128::from(frequency.max(1)),
            clock: 0,
        }
    }

    /// Create at an explicit initial clock (ares `Scheduler::power`).
    pub fn with_clock(frequency: u64, clock: u128) -> Self {
        let mut thread = Self::new(frequency);
        thread.clock = clock;
        thread
    }

    /// Fixed-point units per hardware clock.
    pub const fn scalar(&self) -> u128 {
        self.scalar
    }

    /// The absolute fixed-point clock.
    pub const fn clock(&self) -> u128 {
        self.clock
    }

    /// Set the absolute clock (ares `Thread::setClock`).
    pub fn set_clock(&mut self, clock: u128) {
        self.clock = clock;
    }

    /// ares `Thread::step(clocks)`.
    pub fn step(&mut self, clocks: u32) {
        self.clock += self.scalar * u128::from(clocks);
    }

    /// The clock in whole hardware cycles: `clock() / scalar()`.
    pub fn cycles(&self) -> u64 {
        u64::try_from(self.clock / self.scalar).unwrap_or(u64::MAX)
    }

    /// Whether this thread is behind `target` and must be run.
    pub const fn is_behind(&self, target: u128) -> bool {
        self.clock < target
    }
}

/// ares `Thread::synchronize(thread)`: run `main` until `thread` reaches
/// `target`, i.e. `while(thread.clock() < target) co_switch(thread)`.
///
/// `main` is responsible for advancing `thread`'s clock (ares' thread entry
/// does this inside `Thread::step`). Returning `false` stops the catch-up (used
/// when a thread parks on an S-CPU pull).
pub fn synchronize(
    thread: &mut ThreadClock,
    target: u128,
    mut main: impl FnMut(&mut ThreadClock) -> bool,
) {
    while thread.is_behind(target) {
        if !main(thread) {
            break;
        }
    }
}

/// Input to the S-DSP coroutine.
#[derive(Debug)]
pub enum DspInput {
    /// Advance one micro-step.
    Tick,
}

/// What the S-DSP coroutine yields back to the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DspEvent {
    /// A micro-step finished at this DSP clock; the scheduler must resume the
    /// S-SMP to this clock before ticking the DSP again (ares `DSP::tick` ->
    /// `Thread::synchronize(smp)`).
    MicroStep(u128),
    /// The DSP was handed back for shutdown.
    Done,
}

/// Resumable S-DSP micro-step coroutine.
///
/// Each `Tick` advances the DSP clock by `step_clocks` (24 DSP clocks) and
/// yields [`DspEvent::MicroStep`] so the scheduler can pull the S-SMP before
/// the next tick. The real `Dsp` will replace the clock counter with
/// `Dsp::tick`; the request/yield shape is what the driver depends on.
pub struct DspCoroutine(corosensei::Coroutine<DspInput, DspEvent, ()>);

impl DspCoroutine {
    /// Create a DSP coroutine starting at `clock`, advancing `step_clocks` per
    /// micro-step.
    pub fn new(clock: u128, step_clocks: u128) -> Self {
        Self(corosensei::Coroutine::new(move |yielder, mut input| {
            let mut clock = clock;
            loop {
                input = match input {
                    DspInput::Tick => {
                        clock += step_clocks;
                        yielder.suspend(DspEvent::MicroStep(clock))
                    }
                };
            }
        }))
    }

    /// Advance one micro-step, returning its clock.
    pub fn tick(&mut self) -> u128 {
        match self.0.resume(DspInput::Tick) {
            corosensei::CoroutineResult::Yield(DspEvent::MicroStep(clock)) => clock,
            corosensei::CoroutineResult::Yield(DspEvent::Done) => 0,
            corosensei::CoroutineResult::Return(()) => 0,
        }
    }
}

/// Drive `cpu_step` (one S-CPU instruction) until the S-CPU clock reaches
/// `target`, then return that clock.
///
/// This is the S-CPU side of the mutual pull: ares' `SMP::step` calls
/// `Thread::synchronize(cpu)`, so an S-SMP access can require the S-CPU to run
/// forward to the S-SMP clock. `cpu_step` must return the new S-CPU clock; a
/// non-advancing step (halted CPU) stops the pull instead of spinning.
pub fn pull_cpu(cpu_clock: u128, target: u128, mut cpu_step: impl FnMut() -> u128) -> u128 {
    let mut clock = cpu_clock;
    while clock < target {
        let next = cpu_step();
        if next <= clock {
            break;
        }
        clock = next;
    }
    clock
}

/// Drive `smp_step` until its clock reaches `target`, then return that clock.
///
/// `smp_step` executes one S-SMP unit/instruction and returns the new S-SMP
/// clock; it is the hook the real `SpcCoroutine` will be connected to.
pub fn pull_smp(smp_clock: u128, target: u128, mut smp_step: impl FnMut() -> u128) -> u128 {
    let mut clock = smp_clock;
    while clock < target {
        let next = smp_step();
        if next <= clock {
            break;
        }
        clock = next;
    }
    clock
}

#[cfg(test)]
mod mutual_tests {
    use super::*;

    #[test]
    fn dsp_micro_step_pulls_the_smp_to_its_clock() {
        let mut dsp = DspCoroutine::new(0, 24);
        let mut smp_clock = 0u128;
        let mut step_clock = 0u128;
        let mut order = Vec::new();
        for _ in 0..3 {
            let dsp_clock = dsp.tick();
            order.push(("dsp", dsp_clock));
            smp_clock = pull_smp(smp_clock, dsp_clock, || {
                order.push(("smp", step_clock));
                step_clock += 4;
                step_clock
            });
            assert!(smp_clock >= dsp_clock, "SMP must reach the DSP clock");
        }
        assert_eq!(smp_clock, 72);
        assert_eq!(order[0], ("dsp", 24));
        assert_eq!(order[1], ("smp", 0));
        assert_eq!(order[2], ("smp", 4));
    }

    #[test]
    fn pull_cpu_runs_until_the_smp_clock() {
        let mut cpu_clock = 0u128;
        let mut step_clock = 0u128;
        let mut order = Vec::new();
        cpu_clock = pull_cpu(cpu_clock, 20, || {
            order.push(step_clock);
            step_clock += 6;
            step_clock
        });
        assert_eq!(cpu_clock, 24);
        assert_eq!(order, vec![0, 6, 12, 18]);
    }

    #[test]
    fn pull_cpu_stops_on_a_halted_cpu() {
        let clock = pull_cpu(10, 100, || 10);
        assert_eq!(clock, 10);
    }

    #[test]
    fn pull_stops_when_the_smp_cannot_advance() {
        // A stopped core (0-clock step) must not spin forever.
        let clock = pull_smp(10, 100, || 10);
        assert_eq!(clock, 10);
    }
}

/// One S-SMP timer, mirroring ares `SMP::Timer<Frequency>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SymTimer {
    stage0: u16,
    stage1: bool,
    stage2: u8,
    stage3: u8,
    line: bool,
    enable: bool,
    target: u8,
}

impl SymTimer {
    /// ares `Timer::step`: advance stage 0, then count stage-1 falling edges
    /// into stage 2 and wrap into the 4-bit stage 3.
    pub fn step(&mut self, clocks: u32, frequency: u16, timers_enable: bool, timers_disable: bool) {
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

    /// Read and clear the 4-bit output stage (ares `$FD-$FF` read).
    pub fn read(&mut self) -> u8 {
        let value = self.stage3;
        self.stage3 = 0;
        value
    }

    /// Set the stage-2 target (`$FA-$FC`).
    pub fn set_target(&mut self, target: u8) {
        self.target = target;
    }

    /// Enable the timer.
    pub fn set_enable(&mut self, enable: bool) {
        self.enable = enable;
    }

    /// Current output stage (tests).
    pub const fn stage3(&self) -> u8 {
        self.stage3
    }
}

/// Outcome of one parked-aware S-SMP step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// One S-SMP instruction retired.
    Ran,
    /// The S-SMP parked on an ahead `$F4-$F7` read; the S-CPU must run to this
    /// fixed-point clock before resuming (ares `SMP::readIO -> synchronize(cpu)`).
    Parked(u128),
    /// No core is loaded or the coroutine finished.
    Halted,
}

/// Experimental engine wiring the **real** S-SMP and S-DSP into the mutual
/// scheduler.
///
/// It owns the SPC700, the per-access [`crate::spc_coroutine::SpcCoroutine`],
/// the real S-DSP, APU RAM and the sample queue. The S-SMP runs one instruction
/// at a time; each [`crate::spc_coroutine::SpcEvent`] is answered against the
/// RAM/DSP. A DSP micro-step advances 2 S-SMP clocks, after which the engine
/// pulls the S-SMP to the DSP clock ([`pull_smp`]) before the next micro-step.
///
/// Not wired into [`crate::system::Snes`]; the default driver is untouched.
pub struct SymEngine {
    /// Thread clocks and DSP resume points.
    pub rr: RoundRobin,
    core: Option<crate::spc700::Spc700>,
    spc: crate::spc_coroutine::SpcCoroutine,
    dsp: crate::dsp::Dsp,
    ram: Box<[u8]>,
    samples: std::collections::VecDeque<crate::dsp::Sample>,
    smp_clock: u128,
    dsp_units: u128,
    instructions: u64,
    cpu_clock: u128,
    dsp_address: u8,
    /// S-CPU -> S-SMP `$2140-$2143` latches (read by the S-SMP as `$F4-$F7`).
    apu_ports: [u8; 4],
    /// S-SMP -> S-CPU `$F4-$F7` latches (read by the S-CPU as `$2140-$2143`).
    cpu_ports: [u8; 4],
    ipl: [u8; 64],
    iplrom_enable: bool,
    timers: [SymTimer; 3],
    timers_enable: bool,
    timers_disable: bool,
    /// A parked `$F4-$F7` read: port index to sample when resumed.
    pending_port: Option<u8>,
}

/// S-SMP clocks per DSP micro-step (24 DSP clocks = 2 S-SMP clocks).
const SMP_PER_DSP_STEP: u128 = 2;

impl SymEngine {
    /// Create the engine with a fresh S-SMP/DSP and 64 KiB of APU RAM.
    pub fn new() -> Self {
        Self {
            rr: RoundRobin::new(0, 0, 0),
            core: None,
            spc: crate::spc_coroutine::new_spc_coroutine_inline(),
            dsp: crate::dsp::Dsp::new(),
            ram: vec![0; 0x1_0000].into_boxed_slice(),
            samples: std::collections::VecDeque::new(),
            smp_clock: 0,
            dsp_units: 0,
            instructions: 0,
            cpu_clock: 0,
            dsp_address: 0,
            apu_ports: [0; 4],
            cpu_ports: [0; 4],
            ipl: [0; 64],
            iplrom_enable: false,
            timers: [SymTimer::default(); 3],
            timers_enable: false,
            timers_disable: false,
            pending_port: None,
        }
    }

    /// Load an SPC700 core (e.g. one positioned at a test program).
    pub fn load_core(&mut self, core: crate::spc700::Spc700) {
        self.core = Some(core);
    }

    /// Current S-SMP clock.
    pub const fn smp_clock(&self) -> u128 {
        self.smp_clock
    }

    /// DSP progress in S-SMP-clock units (2 per micro-step).
    pub const fn dsp_units(&self) -> u128 {
        self.dsp_units
    }

    /// Number of retired S-SMP instructions.
    pub const fn instructions(&self) -> u64 {
        self.instructions
    }

    /// Finished samples waiting to be consumed.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Run one S-DSP micro-step and record it in the round-robin state.
    fn dsp_micro_step(&mut self) {
        if let Some(sample) = self.dsp.tick(&mut self.ram) {
            self.samples.push_back(sample);
            if self.samples.len() > 16_384 {
                self.samples.pop_front();
            }
        }
        self.dsp_units += SMP_PER_DSP_STEP;
        self.rr.set_dsp_clock(self.dsp_units);
        self.rr.push_resume(self.dsp_units, self.dsp_units);
    }

    /// Advance the DSP to the current S-SMP clock.
    fn catch_dsp_to_smp(&mut self) {
        while self.dsp_units + SMP_PER_DSP_STEP <= self.smp_clock {
            self.dsp_micro_step();
        }
    }

    /// Answer one bus event from the S-SMP coroutine.
    fn handle(
        &mut self,
        event: crate::spc_coroutine::SpcEvent,
        pull: &mut dyn FnMut(u128) -> u128,
    ) -> crate::spc_coroutine::SpcResume {
        use crate::spc_coroutine::{SpcEvent, SpcResume};
        match event {
            SpcEvent::Wait(Some(address)) if (0x00F0..=0x00FF).contains(&address) => {
                SpcResume::Clocks(1)
            }
            SpcEvent::Wait(_) => SpcResume::Clocks(2),
            SpcEvent::Timer(_) => SpcResume::Clocks(2),
            SpcEvent::Read(address @ 0x00FD..=0x00FF) => {
                SpcResume::Byte(self.timers[usize::from(address - 0x00FD)].read())
            }
            SpcEvent::Read(0x00F2) => SpcResume::Byte(self.dsp_address),
            SpcEvent::Read(0x00F3) => SpcResume::Byte(self.dsp.read(self.dsp_address)),
            SpcEvent::Read(address @ 0x00F4..=0x00F7) => {
                // ares `SMP::readIO`: synchronize(cpu) before sampling the port.
                if self.smp_clock > self.cpu_clock {
                    let next = pull(self.smp_clock);
                    if next > self.cpu_clock {
                        self.cpu_clock = next;
                        self.rr.set_cpu_clock(next);
                    }
                }
                SpcResume::Byte(self.apu_ports[usize::from(address - 0x00F4)])
            }
            SpcEvent::Read(address) if self.iplrom_enable && address >= 0xFFC0 => {
                SpcResume::Byte(self.ipl[usize::from(address & 0x3F)])
            }
            SpcEvent::Read(address) => SpcResume::Byte(self.ram[usize::from(address)]),
            SpcEvent::Write(0x00F1, value) => {
                self.timers_enable = value & 0x07 != 0;
                self.timers_disable = value & 0x80 != 0;
                for (index, timer) in self.timers.iter_mut().enumerate() {
                    timer.set_enable(value & (1 << index) != 0);
                }
                SpcResume::Unit
            }
            SpcEvent::Write(address @ 0x00FA..=0x00FC, value) => {
                self.timers[usize::from(address - 0x00FA)].set_target(value & 0x0F);
                SpcResume::Unit
            }
            SpcEvent::Write(0x00F2, value) => {
                self.dsp_address = value;
                SpcResume::Unit
            }
            SpcEvent::Write(0x00F3, value) => {
                self.dsp.write(self.dsp_address, value);
                SpcResume::Unit
            }
            SpcEvent::Write(address @ 0x00F4..=0x00F7, value) => {
                self.cpu_ports[usize::from(address - 0x00F4)] = value;
                SpcResume::Unit
            }
            SpcEvent::Write(address, value) => {
                self.ram[usize::from(address)] = value;
                SpcResume::Unit
            }
            SpcEvent::StepClock(clocks) => {
                self.smp_clock += u128::from(clocks);
                self.rr.set_smp_clock(self.smp_clock);
                self.catch_dsp_to_smp();
                SpcResume::Unit
            }
            SpcEvent::Advance(clocks) => {
                for (index, timer) in self.timers.iter_mut().enumerate() {
                    let frequency: u16 = if index == 2 { 16 } else { 128 };
                    timer.step(clocks, frequency, self.timers_enable, self.timers_disable);
                }
                self.smp_clock += u128::from(clocks);
                self.rr.set_smp_clock(self.smp_clock);
                SpcResume::Unit
            }
            SpcEvent::Idle => SpcResume::Unit,
            SpcEvent::Done(_) => SpcResume::Unit,
        }
    }

    /// Execute one S-SMP instruction through the resumable coroutine.
    ///
    /// Returns `false` when there is no core to run.
    pub fn smp_step(&mut self) -> bool {
        let current = self.cpu_clock;
        self.smp_step_pulling(&mut |_| current)
    }

    /// Execute one S-SMP instruction, invoking `pull` (ares
    /// `SMP::readIO -> synchronize(cpu)`) when a `$F4-$F7` read is ahead of the
    /// S-CPU clock.
    pub fn smp_step_pulling(&mut self, pull: &mut dyn FnMut(u128) -> u128) -> bool {
        use crate::spc_coroutine::{SpcEvent, SpcInput};
        let Some(core) = self.core.take() else {
            return false;
        };
        let mut input = SpcInput::Start(core);
        loop {
            match self.spc.resume(input) {
                corosensei::CoroutineResult::Yield(SpcEvent::Done(core)) => {
                    self.core = Some(core);
                    self.instructions += 1;
                    return true;
                }
                corosensei::CoroutineResult::Yield(event) => {
                    let resume = self.handle(event, pull);
                    input = SpcInput::Resume(resume);
                }
                corosensei::CoroutineResult::Return(()) => return false,
            }
        }
    }

    /// Whether a `$F4-$F7` read is parked waiting for the S-CPU.
    pub const fn is_parked(&self) -> bool {
        self.pending_port.is_some()
    }

    /// Latch an S-CPU write to `$2140-$2143` without synchronizing (the parked
    /// protocol: the caller has already run the S-CPU to the target).
    pub fn write_apu_latch(&mut self, port: u8, value: u8) {
        self.apu_ports[usize::from(port & 3)] = value;
    }

    /// Execute one S-SMP instruction under the **park** protocol: an ahead
    /// `$F4-$F7` read returns [`StepOutcome::Parked`] with the S-SMP clock the
    /// S-CPU must reach; the next call resumes the read and samples the
    /// (possibly updated) latch.
    pub fn smp_step_parking(&mut self) -> StepOutcome {
        use crate::spc_coroutine::{SpcEvent, SpcInput, SpcResume};
        let mut input = if let Some(port) = self.pending_port.take() {
            SpcInput::Resume(SpcResume::Byte(self.apu_ports[usize::from(port)]))
        } else if let Some(core) = self.core.take() {
            SpcInput::Start(core)
        } else {
            return StepOutcome::Halted;
        };
        loop {
            match self.spc.resume(input) {
                corosensei::CoroutineResult::Yield(SpcEvent::Done(core)) => {
                    self.core = Some(core);
                    self.instructions += 1;
                    return StepOutcome::Ran;
                }
                corosensei::CoroutineResult::Yield(SpcEvent::Read(address))
                    if (0x00F4..=0x00F7).contains(&address) && self.smp_clock > self.cpu_clock =>
                {
                    self.pending_port = Some((address - 0x00F4) as u8);
                    return StepOutcome::Parked(self.smp_clock);
                }
                corosensei::CoroutineResult::Yield(event) => {
                    let cpu = self.cpu_clock;
                    let resume = self.handle(event, &mut |_| cpu);
                    input = SpcInput::Resume(resume);
                }
                corosensei::CoroutineResult::Return(()) => return StepOutcome::Halted,
            }
        }
    }

    /// ares scheduler step from the S-CPU side: run the S-SMP and S-DSP up to
    /// `cpu_clock`, invoking `cpu_step(target)` (which returns the new S-CPU
    /// clock) whenever an S-SMP `$F4-$F7` read is ahead (ares
    /// `SMP::readIO -> synchronize(cpu)`).
    ///
    /// This is the entry the `SNES_CA` adapter uses to drive the isolated
    /// engine from [`crate::system::Snes`].
    pub fn advance_to_cpu(&mut self, cpu_clock: u128, mut cpu_step: impl FnMut(u128) -> u128) {
        if cpu_clock > self.cpu_clock {
            self.cpu_clock = cpu_clock;
            self.rr.set_cpu_clock(cpu_clock);
        }
        let mut guard = 0u32;
        while self.smp_clock < self.cpu_clock && guard < 2_000_000 {
            if !self.smp_step_pulling(&mut cpu_step) {
                break;
            }
            guard += 1;
        }
        self.catch_dsp_to_smp();
    }

    /// Run `micro_steps` DSP micro-steps, pulling the S-SMP to each DSP clock
    /// before the next step (ares `DSP::tick -> synchronize(smp)`).
    pub fn run_mutual(&mut self, micro_steps: u32) {
        for _ in 0..micro_steps {
            self.dsp_micro_step();
            let target = self.dsp_units;
            let mut guard = 0u32;
            while self.smp_clock < target && guard < 100_000 {
                if !self.smp_step() {
                    break;
                }
                guard += 1;
            }
        }
    }
}

impl SymEngine {
    /// Publish the S-CPU clock.
    pub fn set_cpu_clock(&mut self, clock: u128) {
        self.cpu_clock = clock;
    }

    /// The S-CPU clock.
    pub const fn cpu_clock(&self) -> u128 {
        self.cpu_clock
    }

    /// A timer's current output stage (tests).
    pub const fn timer_stage3(&self, index: usize) -> u8 {
        self.timers[index].stage3()
    }

    /// Value the S-CPU currently sees on `$2140-$2143` (for tests).
    pub const fn cpu_port(&self, port: u8) -> u8 {
        self.cpu_ports[port as usize & 3]
    }

    /// Value the S-SMP currently sees on `$F4-$F7` (for tests).
    pub const fn apu_port(&self, port: u8) -> u8 {
        self.apu_ports[port as usize & 3]
    }

    /// Latch a value the S-CPU will see on `$2140-$2143` (for tests).
    pub fn set_cpu_port(&mut self, port: u8, value: u8) {
        self.cpu_ports[port as usize & 3] = value;
    }

    /// Run the S-SMP until its clock reaches `target` (ares
    /// `Thread::synchronize(smp)`).
    fn pull_smp_to(&mut self, target: u128) {
        let mut guard = 0u32;
        while self.smp_clock < target && guard < 1_000_000 {
            if !self.smp_step() {
                break;
            }
            guard += 1;
        }
    }

    /// Load the 64-byte IPL ROM and enable it at `$FFC0-$FFFF`.
    pub fn load_ipl(&mut self, ipl: [u8; 64]) {
        self.ipl = ipl;
        self.iplrom_enable = true;
    }

    /// ares `CPU::writeAPU`: synchronize the S-SMP to the S-CPU clock, then
    /// latch `$2140-$2143`.
    pub fn cpu_write_port(&mut self, port: u8, value: u8) {
        self.pull_smp_to(self.cpu_clock);
        self.apu_ports[usize::from(port & 3)] = value;
    }

    /// ares `CPU::readAPU`: synchronize the S-SMP to the S-CPU clock, then
    /// sample `$2140-$2143` (the S-SMP's `$F4-$F7` latches).
    pub fn cpu_read_port(&mut self, port: u8) -> u8 {
        self.pull_smp_to(self.cpu_clock);
        self.cpu_ports[usize::from(port & 3)]
    }
}

/// S-CPU-side peer for the cycle-accurate engine, used by `step_sym_sched`
/// in place of `ApuPeer` when `SNES_CA` is enabled.
///
/// It owns the engine and implements ares' outer-loop park pattern: the S-SMP
/// runs until it retires an instruction or parks on an ahead `$F4-$F7` read;
/// on a park, `run_cpu(target)` drives the S-CPU to that clock in the caller's
/// outer loop, then the S-SMP resumes and samples the updated latch.
pub struct CaPeer {
    engine: SymEngine,
}

impl CaPeer {
    /// Create a peer around an engine.
    pub fn new(engine: SymEngine) -> Self {
        Self { engine }
    }

    /// The owned engine.
    pub fn engine(&self) -> &SymEngine {
        &self.engine
    }

    /// The owned engine (mutable).
    pub fn engine_mut(&mut self) -> &mut SymEngine {
        &mut self.engine
    }

    /// Run the S-SMP toward `cpu_clock`, invoking `run_cpu(target)` whenever an
    /// ahead `$F4-$F7` read parks so the caller can drive the S-CPU.
    pub fn run_to(&mut self, cpu_clock: u128, mut run_cpu: impl FnMut(&mut SymEngine, u128)) {
        self.engine.set_cpu_clock(cpu_clock);
        let mut guard = 0u32;
        while (self.engine.smp_clock() < cpu_clock || self.engine.is_parked()) && guard < 2_000_000
        {
            match self.engine.smp_step_parking() {
                StepOutcome::Ran => {}
                StepOutcome::Parked(target) => {
                    run_cpu(&mut self.engine, target);
                    if target > self.engine.cpu_clock() {
                        self.engine.set_cpu_clock(target);
                    }
                }
                StepOutcome::Halted => break,
            }
            guard += 1;
        }
    }
}

impl Default for SymEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    fn test_core() -> crate::spc700::Spc700 {
        let mut core = crate::spc700::Spc700::new();
        // `BRA -2` at $0200: a tight loop that retires one instruction.
        core.pc = 0x0200;
        core
    }

    #[test]
    fn real_dsp_produces_a_sample_every_32_micro_steps() {
        let mut engine = SymEngine::new();
        engine.run_mutual(31);
        assert_eq!(engine.sample_count(), 0);
        engine.run_mutual(1);
        assert_eq!(engine.sample_count(), 1);
        assert_eq!(engine.dsp_units(), 32 * SMP_PER_DSP_STEP);
    }

    #[test]
    fn smp_coroutine_retires_instructions_and_pulls_dsp() {
        let mut engine = SymEngine::new();
        engine.load_core(test_core());
        engine.ram[0x0200] = 0x2F; // BRA
        engine.ram[0x0201] = 0xFE; // -2
        engine.run_mutual(4);
        assert!(engine.instructions() >= 1, "S-SMP must run instructions");
        assert!(engine.dsp_units() >= 4 * SMP_PER_DSP_STEP);
        // The S-SMP's own StepClock events pull the DSP too, so the DSP may run
        // ahead of the four scheduler ticks; the invariant is the SMP clock
        // keeping up with the DSP.
        assert!(engine.smp_clock() >= engine.dsp_units());
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn scalars_match_ares_thread_create() {
        let smp = ThreadClock::new(2_050_560);
        let dsp = ThreadClock::new(24_606_720);
        assert_eq!(smp.scalar(), SECOND / 2_050_560);
        assert_eq!(dsp.scalar(), SECOND / 24_606_720);
        // DSP runs 12x the S-SMP, so its scalar is ~1/12.
        assert!(dsp.scalar() * 12 < smp.scalar() * 13);
    }

    #[test]
    fn step_advances_whole_cycles() {
        let mut t = ThreadClock::new(2_050_560);
        t.step(64);
        assert_eq!(t.cycles(), 64);
        t.step(0);
        assert_eq!(t.cycles(), 64);
    }

    #[test]
    fn synchronize_runs_until_caught_up() {
        let mut other = ThreadClock::new(1_000);
        let target = ThreadClock::new(1_000).scalar() * 10; // 10 cycles
        let mut runs = 0;
        synchronize(&mut other, target, |t| {
            runs += 1;
            t.step(3);
            true
        });
        assert!(other.clock() >= target);
        assert_eq!(runs, 4); // 3,6,9,12 cycles
    }

    #[test]
    fn synchronize_stops_when_main_parks() {
        let mut other = ThreadClock::new(1_000);
        let target = 10_000 * other.scalar();
        let mut calls = 0;
        synchronize(&mut other, target, |t| {
            calls += 1;
            t.step(1);
            calls < 2
        });
        assert_eq!(calls, 2);
        assert!(other.is_behind(target));
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;

    fn engine_with_core() -> SymEngine {
        let mut core = crate::spc700::Spc700::new();
        core.pc = 0x0200;
        let mut engine = SymEngine::new();
        engine.ram[0x0200] = 0x2F; // BRA -2
        engine.ram[0x0201] = 0xFE;
        engine.load_core(core);
        engine
    }

    #[test]
    fn cpu_write_synchronizes_smp_then_latches() {
        let mut engine = engine_with_core();
        engine.set_cpu_clock(100);
        engine.cpu_write_port(0, 0xAB);
        // ares `writeAPU`: synchronize(smp) to cpu.clock, then latch.
        assert!(engine.smp_clock() >= 100, "SMP must reach the CPU clock");
        assert_eq!(engine.apu_port(0), 0xAB);
    }

    #[test]
    fn cpu_read_returns_the_smp_latch() {
        let mut engine = engine_with_core();
        engine.set_cpu_clock(50);
        engine.set_cpu_port(1, 0xBB);
        assert_eq!(engine.cpu_read_port(1), 0xBB);
        assert!(engine.smp_clock() >= 50);
    }

    #[test]
    fn port_latches_are_independent_per_channel() {
        let mut engine = engine_with_core();
        engine.set_cpu_clock(10);
        engine.cpu_write_port(0, 1);
        engine.cpu_write_port(3, 4);
        assert_eq!((engine.apu_port(0), engine.apu_port(3)), (1, 4));
    }
}

#[cfg(test)]
mod pull_tests {
    use super::*;

    #[test]
    fn smp_port_read_pulls_the_cpu() {
        // `MOV A,$F4` then `BRA -2`.
        let mut core = crate::spc700::Spc700::new();
        core.pc = 0x0200;
        let mut engine = SymEngine::new();
        engine.ram[0x0200] = 0xE4;
        engine.ram[0x0201] = 0xF4;
        engine.ram[0x0202] = 0x2F;
        engine.ram[0x0203] = 0xFC;
        engine.load_core(core);
        engine.set_cpu_clock(0);
        let mut pulls = 0u32;
        let mut pull = |smp: u128| {
            // model the S-CPU running forward to the S-SMP clock.
            smp
        };
        // Run enough S-SMP instructions to execute the port read.
        for _ in 0..64 {
            engine.smp_step_pulling(&mut pull);
            if engine.cpu_clock() > 0 {
                pulls += 1;
            }
        }
        assert!(pulls > 0, "the $F4 read must pull the S-CPU");
        assert!(engine.cpu_clock() > 0);
    }
}

#[cfg(test)]
mod ipl_tests {
    use super::*;

    // Real ares `ipl.rom`.
    const IPL: [u8; 64] = [
        0xCD, 0xEF, 0xBD, 0xE8, 0x00, 0xC6, 0x1D, 0xD0, 0xFC, 0x8F, 0xAA, 0xF4, 0x8F, 0xBB, 0xF5,
        0x78, 0xCC, 0xF4, 0xD0, 0xFB, 0x2F, 0x19, 0xEB, 0xF4, 0xD0, 0xFC, 0x7E, 0xF4, 0xD0, 0x0B,
        0xE4, 0xF5, 0xCB, 0xF4, 0xD7, 0x00, 0xFC, 0xD0, 0xF3, 0xAB, 0x01, 0x10, 0xEF, 0x7E, 0xF4,
        0x10, 0xEB, 0xBA, 0xF6, 0xDA, 0x00, 0xBA, 0xF4, 0xC4, 0xF4, 0xDD, 0x5D, 0xD0, 0xDB, 0x1F,
        0x00, 0x00, 0xC0, 0xFF,
    ];

    #[test]
    fn engine_runs_the_ipl_boot_handshake() {
        let mut core = crate::spc700::Spc700::new();
        core.pc = 0xFFC0;
        core.sp = 0xEF;
        let mut engine = SymEngine::new();
        engine.load_ipl(IPL);
        engine.load_core(core);
        engine.set_cpu_clock(0);
        for _ in 0..5000 {
            if !engine.smp_step() {
                break;
            }
            if engine.cpu_port(0) == 0xAA && engine.cpu_port(1) == 0xBB {
                break;
            }
        }
        // ares IPL: after clearing RAM it writes $AA to $F4 and $BB to $F5.
        assert_eq!(engine.cpu_port(0), 0xAA);
        assert_eq!(engine.cpu_port(1), 0xBB);
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn timer_counts_at_its_frequency() {
        let mut timer = SymTimer::default();
        timer.set_enable(true);
        timer.set_target(1);
        // 128 S-SMP clocks per stage-1 flip; a falling edge needs one high and
        // one low half, so ~256 clocks per stage-3 increment.
        timer.step(512, 128, true, false);
        assert!(timer.stage3() >= 1, "stage3 = {}", timer.stage3());
    }

    #[test]
    fn timer_read_clears_the_output_stage() {
        let mut timer = SymTimer::default();
        timer.set_enable(true);
        timer.set_target(1);
        timer.step(512, 128, true, false);
        let value = timer.read();
        assert!(value >= 1);
        assert_eq!(timer.read(), 0);
    }
}

#[cfg(test)]
mod advance_tests {
    use super::*;

    fn loop_engine() -> SymEngine {
        let mut core = crate::spc700::Spc700::new();
        core.pc = 0x0200;
        let mut engine = SymEngine::new();
        engine.ram[0x0200] = 0x2F; // BRA -2
        engine.ram[0x0201] = 0xFE;
        engine.load_core(core);
        engine
    }

    #[test]
    fn advance_to_cpu_runs_the_smp_and_dsp() {
        let mut engine = loop_engine();
        let mut cpu_clock = 0u128;
        engine.advance_to_cpu(512, |target| {
            // model the S-CPU stepping in 6-clock units until it reaches target.
            while cpu_clock < target {
                cpu_clock += 6;
            }
            cpu_clock
        });
        assert!(engine.smp_clock() >= 512, "smp={}", engine.smp_clock());
        assert!(engine.dsp_units() > 0);
    }
}

#[cfg(test)]
mod parking_tests {
    use super::*;

    #[test]
    fn ahead_port_read_parks_until_the_cpu_catches_up() {
        let mut core = crate::spc700::Spc700::new();
        core.pc = 0x0200;
        let mut engine = SymEngine::new();
        // `MOV A,$F4` then `BRA -2`.
        engine.ram[0x0200] = 0xE4;
        engine.ram[0x0201] = 0xF4;
        engine.ram[0x0202] = 0x2F;
        engine.ram[0x0203] = 0xFC;
        engine.load_core(core);
        engine.set_cpu_clock(0);
        let target = match engine.smp_step_parking() {
            StepOutcome::Parked(t) => t,
            other => panic!("expected Parked, got {other:?}"),
        };
        assert!(target > 0, "the read must park at a positive SMP clock");
        assert!(engine.smp_clock() >= target);
        // The S-CPU reaches the target, then writes the latch.
        engine.set_cpu_clock(target);
        engine.write_apu_latch(0, 0x55);
        assert_eq!(engine.smp_step_parking(), StepOutcome::Ran);
        assert_eq!(engine.apu_port(0), 0x55);
    }
}

#[cfg(test)]
mod ca_peer_tests {
    use super::*;

    #[test]
    fn run_to_drives_the_cpu_on_a_port_park() {
        let mut core = crate::spc700::Spc700::new();
        core.pc = 0x0200;
        let mut engine = SymEngine::new();
        // `MOV A,$F4` then `BRA -2`.
        engine.ram[0x0200] = 0xE4;
        engine.ram[0x0201] = 0xF4;
        engine.ram[0x0202] = 0x2F;
        engine.ram[0x0203] = 0xFC;
        engine.load_core(core);
        engine.set_cpu_clock(0);
        let mut peer = CaPeer::new(engine);
        let mut pulled = Vec::new();
        peer.run_to(4, |engine, target| {
            pulled.push(target);
            // the S-CPU reaches the park target and writes $2140.
            engine.write_apu_latch(0, 0x5A);
        });
        assert!(!pulled.is_empty(), "the $F4 read must drive the CPU");
        assert!(peer.engine().smp_clock() >= 4);
        assert_eq!(peer.engine().apu_port(0), 0x5A);
    }
}
