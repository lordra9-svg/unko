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
    fn pull_stops_when_the_smp_cannot_advance() {
        // A stopped core (0-clock step) must not spin forever.
        let clock = pull_smp(10, 100, || 10);
        assert_eq!(clock, 10);
    }
}
