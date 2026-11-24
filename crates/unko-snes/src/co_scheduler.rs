//! Cooperative three-thread scheduler (ares `Scheduler`/`Thread` model).
//!
//! This is the foundation of the APU rewrite: the S-CPU, S-SMP and S-DSP are
//! explicit resumable states with clocks in ares' fixed-point `Second` base, and
//! every thread can synchronize another thread to its own clock. Unlike a
//! stackful-coroutine design, the state is a plain value, so it serializes into
//! save states and the switch order is testable.
//!
//! # Model (ares)
//!
//! * `Thread::step(clocks)` does `_clock += _scalar * clocks`.
//! * `Thread::synchronize(other)` runs `other` while `other.clock < self.clock`.
//! * `SMP::step` ends with `synchronize(cpu)` and `synchronize(dsp)`.
//! * `DSP::tick` does `Thread::step(3 * 8)` then `Thread::synchronize(smp)`.
//!
//! A state reports [`Need::Synchronize`] from [`ThreadState::step`] instead of
//! calling back re-entrantly; the scheduler performs the run, which avoids
//! borrowing the scheduler from inside a thread and keeps every suspension point
//! explicit.

/// ares `Thread::Second`: the fixed-point base of all thread clocks.
pub const SECOND: u128 = (1 << 63) - 1;

/// One hardware clock rate expressed in the scheduler's fixed-point base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadClock {
    scalar: u128,
    clock: u128,
}

impl ThreadClock {
    /// ares `Thread::create(frequency)`.
    pub fn new(frequency: u64) -> Self {
        Self {
            scalar: SECOND / u128::from(frequency.max(1)),
            clock: 0,
        }
    }

    /// Create at an explicit absolute clock (ares `Scheduler::power` staggering).
    pub fn with_clock(frequency: u64, clock: u128) -> Self {
        let mut thread = Self::new(frequency);
        thread.clock = clock;
        thread
    }

    /// Fixed-point units per hardware clock.
    pub const fn scalar(&self) -> u128 {
        self.scalar
    }

    /// Absolute fixed-point clock.
    pub const fn clock(&self) -> u128 {
        self.clock
    }

    /// Set the absolute fixed-point clock.
    pub fn set_clock(&mut self, clock: u128) {
        self.clock = clock;
    }

    /// Hardware cycles elapsed (`clock / scalar`).
    pub const fn cycles(&self) -> u64 {
        (self.clock / self.scalar) as u64
    }

    /// ares `Thread::step(clocks)`.
    pub fn step(&mut self, clocks: u32) {
        self.clock += self.scalar * u128::from(clocks);
    }
}

/// One of the three cooperative threads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Thread {
    /// The S-CPU.
    Cpu = 0,
    /// The S-SMP.
    Smp = 1,
    /// The S-DSP.
    Dsp = 2,
}

impl Thread {
    /// All threads in ares append order.
    pub const ALL: [Thread; 3] = [Thread::Cpu, Thread::Smp, Thread::Dsp];

    /// Array index for this thread.
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// What a thread needs after one suspension interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// The thread suspended normally; the scheduler may pick any thread next.
    None,
    /// Run `other` until its clock reaches this thread's clock, then resume this
    /// thread (ares `Thread::synchronize(other)`).
    Synchronize(Thread),
    /// The thread cannot advance (halted).
    Halt,
}

/// A resumable thread state.
pub trait ThreadState {
    /// Absolute clock in the fixed-point base.
    fn clock(&self) -> u128;

    /// Execute one suspension interval (to the next access/micro-step) and report
    /// what the scheduler must do next.
    fn step(&mut self) -> Need;
}

/// A cooperative thread set.
pub struct Scheduler {
    clocks: [ThreadClock; 3],
    states: [Option<Box<dyn ThreadState>>; 3],
    switch_log: Vec<(Thread, u128, Thread, u128)>,
    log_enabled: bool,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    /// Create an empty scheduler (all threads must be installed).
    pub fn new() -> Self {
        Self {
            clocks: [
                ThreadClock::new(21_477_273),
                ThreadClock::new(2_050_560),
                ThreadClock::new(24_606_720),
            ],
            states: [None, None, None],
            switch_log: Vec::new(),
            log_enabled: false,
        }
    }

    /// Install a thread state and its clock.
    pub fn install(&mut self, thread: Thread, clock: ThreadClock, state: Box<dyn ThreadState>) {
        self.clocks[thread.index()] = clock;
        self.states[thread.index()] = Some(state);
    }

    /// Publish a thread clock directly.
    pub fn set_clock(&mut self, thread: Thread, clock: ThreadClock) {
        self.clocks[thread.index()] = clock;
    }

    /// A thread's clock.
    pub fn clock(&self, thread: Thread) -> u128 {
        self.states[thread.index()]
            .as_ref()
            .map_or(self.clocks[thread.index()].clock(), |state| state.clock())
    }

    /// Enable the switch log.
    pub fn enable_switch_log(&mut self) {
        self.log_enabled = true;
    }

    /// Recorded `(main, main_clock, other, other_clock)` co-switches.
    pub fn switch_log(&self) -> &[(Thread, u128, Thread, u128)] {
        &self.switch_log
    }

    /// ares `Thread::synchronize(other)`: run `other` until its clock reaches
    /// `main`'s clock.
    pub fn synchronize(&mut self, main: Thread, other: Thread) {
        let target = self.clock(main);
        let mut guard = 0u32;
        while self.clock(other) < target {
            if self.log_enabled {
                self.switch_log
                    .push((main, target, other, self.clock(other)));
            }
            let need = self.states[other.index()]
                .as_mut()
                .expect("thread installed")
                .step();
            match need {
                Need::Halt => return,
                Need::None => {}
                Need::Synchronize(pull) => {
                    if pull != other {
                        self.synchronize(other, pull);
                    }
                }
            }
            guard += 1;
            if guard > 10_000_000 {
                return;
            }
        }
    }

    /// Run one suspension interval of `thread`, applying its synchronizations.
    ///
    /// ares' `SMP::step` synchronizes the S-CPU *and then* the S-DSP before the
    /// S-SMP continues, so a thread is resumed after each requested pull until it
    /// suspends normally or halts.
    pub fn step(&mut self, thread: Thread) -> Need {
        let mut guard = 0u32;
        loop {
            let need = self.states[thread.index()]
                .as_mut()
                .expect("thread installed")
                .step();
            match need {
                Need::Synchronize(pull) if pull != thread => {
                    self.synchronize(thread, pull);
                }
                other => return other,
            }
            guard += 1;
            if guard > 10_000_000 {
                return Need::Halt;
            }
        }
    }

    /// Run the furthest-behind thread until every thread reaches `target`.
    pub fn run_to(&mut self, target: u128) {
        let mut guard = 0u64;
        loop {
            if Thread::ALL.iter().all(|&t| self.clock(t) >= target) {
                return;
            }
            let next = *Thread::ALL
                .iter()
                .min_by_key(|&&t| self.clock(t))
                .expect("three threads");
            if self.step(next) == Need::Halt && self.clock(next) < target {
                // A halted thread cannot reach the target; stop rather than spin.
                return;
            }
            guard += 1;
            if guard > 50_000_000 {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mock {
        clock: ThreadClock,
        step_clocks: u32,
        need: Need,
        steps: u32,
    }

    impl Mock {
        fn new(frequency: u64, step_clocks: u32, need: Need) -> Self {
            Self {
                clock: ThreadClock::new(frequency),
                step_clocks,
                need,
                steps: 0,
            }
        }
    }

    impl ThreadState for Mock {
        fn clock(&self) -> u128 {
            self.clock.clock()
        }

        fn step(&mut self) -> Need {
            self.clock.step(self.step_clocks);
            self.steps += 1;
            self.need
        }
    }

    #[test]
    fn synchronize_runs_the_target_to_the_caller_clock() {
        let mut scheduler = Scheduler::new();
        let mut smp = Mock::new(2_050_560, 2, Need::None);
        smp.clock
            .set_clock(ThreadClock::new(2_050_560).scalar() * 50);
        scheduler.install(Thread::Smp, ThreadClock::new(2_050_560), Box::new(smp));
        scheduler.install(
            Thread::Dsp,
            ThreadClock::new(24_606_720),
            Box::new(Mock::new(24_606_720, 24, Need::None)),
        );
        scheduler.install(
            Thread::Cpu,
            ThreadClock::new(21_477_273),
            Box::new(Mock::new(21_477_273, 52, Need::None)),
        );

        scheduler.synchronize(Thread::Smp, Thread::Dsp);
        let target = scheduler.clock(Thread::Smp);
        assert!(scheduler.clock(Thread::Dsp) >= target);
        // One 24-clock DSP step is a whole micro-step; it must not overshoot by
        // more than that step.
        let step = ThreadClock::new(24_606_720).scalar() * 24;
        assert!(scheduler.clock(Thread::Dsp) - target < step);
    }

    #[test]
    fn synchronize_does_not_run_an_ahead_thread() {
        let mut scheduler = Scheduler::new();
        let mut dsp = Mock::new(24_606_720, 24, Need::None);
        dsp.clock = ThreadClock::with_clock(24_606_720, 1_000_000);
        scheduler.install(
            Thread::Smp,
            ThreadClock::new(2_050_560),
            Box::new(Mock::new(2_050_560, 8, Need::None)),
        );
        scheduler.install(Thread::Dsp, ThreadClock::new(24_606_720), Box::new(dsp));
        scheduler.install(
            Thread::Cpu,
            ThreadClock::new(21_477_273),
            Box::new(Mock::new(21_477_273, 52, Need::None)),
        );

        let before = scheduler.clock(Thread::Dsp);
        scheduler.synchronize(Thread::Smp, Thread::Dsp);
        assert_eq!(scheduler.clock(Thread::Dsp), before);
    }

    #[test]
    fn step_resumes_after_each_synchronize() {
        // `SMP::step` asks for the CPU, then the DSP, before continuing.
        struct TwoPulls {
            clock: ThreadClock,
            stage: u8,
        }
        impl ThreadState for TwoPulls {
            fn clock(&self) -> u128 {
                self.clock.clock()
            }
            fn step(&mut self) -> Need {
                self.clock.step(8);
                match self.stage {
                    0 => {
                        self.stage = 1;
                        Need::Synchronize(Thread::Cpu)
                    }
                    1 => {
                        self.stage = 2;
                        Need::Synchronize(Thread::Dsp)
                    }
                    _ => Need::None,
                }
            }
        }
        struct Pulled {
            clock: ThreadClock,
            step_clocks: u32,
        }
        impl ThreadState for Pulled {
            fn clock(&self) -> u128 {
                self.clock.clock()
            }
            fn step(&mut self) -> Need {
                self.clock.step(self.step_clocks);
                Need::None
            }
        }

        let mut scheduler = Scheduler::new();
        scheduler.install(
            Thread::Smp,
            ThreadClock::new(2_050_560),
            Box::new(TwoPulls {
                clock: ThreadClock::new(2_050_560),
                stage: 0,
            }),
        );
        scheduler.install(
            Thread::Cpu,
            ThreadClock::new(21_477_273),
            Box::new(Pulled {
                clock: ThreadClock::new(21_477_273),
                step_clocks: 52,
            }),
        );
        scheduler.install(
            Thread::Dsp,
            ThreadClock::new(24_606_720),
            Box::new(Pulled {
                clock: ThreadClock::new(24_606_720),
                step_clocks: 24,
            }),
        );

        assert_eq!(scheduler.step(Thread::Smp), Need::None);
        // Both requested pulls ran (each is pulled to the clock at that moment).
        assert!(scheduler.clock(Thread::Cpu) > 0);
        assert!(scheduler.clock(Thread::Dsp) > 0);
    }

    #[test]
    fn three_thread_cadence_is_dsp_driven() {
        // ares order: each DSP micro-step (24 DSP clocks = 2 S-SMP clocks) pulls
        // the S-SMP; each S-SMP access (8 clocks) pulls the S-CPU.
        struct Dsp {
            clock: ThreadClock,
            pending: bool,
            steps: u32,
        }
        impl ThreadState for Dsp {
            fn clock(&self) -> u128 {
                self.clock.clock()
            }
            fn step(&mut self) -> Need {
                if self.pending {
                    self.pending = false;
                    return Need::None;
                }
                self.clock.step(24);
                self.steps += 1;
                self.pending = true;
                Need::Synchronize(Thread::Smp)
            }
        }
        struct Smp {
            clock: ThreadClock,
            stage: u8,
            steps: u32,
        }
        impl ThreadState for Smp {
            fn clock(&self) -> u128 {
                self.clock.clock()
            }
            fn step(&mut self) -> Need {
                match self.stage {
                    0 => {
                        self.clock.step(8);
                        self.steps += 1;
                        self.stage = 1;
                        Need::Synchronize(Thread::Cpu)
                    }
                    1 => {
                        self.stage = 2;
                        Need::Synchronize(Thread::Dsp)
                    }
                    _ => {
                        self.stage = 0;
                        Need::None
                    }
                }
            }
        }
        struct Cpu {
            clock: ThreadClock,
            step_clocks: u32,
        }
        impl ThreadState for Cpu {
            fn clock(&self) -> u128 {
                self.clock.clock()
            }
            fn step(&mut self) -> Need {
                self.clock.step(self.step_clocks);
                Need::None
            }
        }

        let mut scheduler = Scheduler::new();
        scheduler.install(
            Thread::Cpu,
            ThreadClock::new(21_477_273),
            Box::new(Cpu {
                clock: ThreadClock::new(21_477_273),
                step_clocks: 52,
            }),
        );
        scheduler.install(
            Thread::Smp,
            ThreadClock::new(2_050_560),
            Box::new(Smp {
                clock: ThreadClock::new(2_050_560),
                stage: 0,
                steps: 0,
            }),
        );
        scheduler.install(
            Thread::Dsp,
            ThreadClock::new(24_606_720),
            Box::new(Dsp {
                clock: ThreadClock::new(24_606_720),
                pending: false,
                steps: 0,
            }),
        );

        // Half a second of master time.
        scheduler.run_to(SECOND / 2);
        let smp_clock = scheduler.clock(Thread::Smp);
        let dsp_clock = scheduler.clock(Thread::Dsp);
        let cpu_clock = scheduler.clock(Thread::Cpu);
        assert!(smp_clock > 0 && dsp_clock > 0 && cpu_clock > 0);
        // The threads must stay within one access/micro-step of each other.
        let smp_step = ThreadClock::new(2_050_560).scalar() * 8;
        assert!(smp_clock.abs_diff(dsp_clock) <= smp_step * 2);
        assert!(cpu_clock > 0);
    }

    #[test]
    fn switch_log_records_a_synchronize() {
        struct Ahead {
            clock: ThreadClock,
        }
        impl ThreadState for Ahead {
            fn clock(&self) -> u128 {
                self.clock.clock()
            }
            fn step(&mut self) -> Need {
                Need::None
            }
        }
        struct Behind {
            clock: ThreadClock,
            step_clocks: u32,
        }
        impl ThreadState for Behind {
            fn clock(&self) -> u128 {
                self.clock.clock()
            }
            fn step(&mut self) -> Need {
                self.clock.step(self.step_clocks);
                Need::None
            }
        }
        let mut scheduler = Scheduler::new();
        let mut smp = Ahead {
            clock: ThreadClock::with_clock(2_050_560, 1_000_000_000),
        };
        smp.clock.set_clock(1_000_000_000);
        scheduler.install(Thread::Smp, ThreadClock::new(2_050_560), Box::new(smp));
        scheduler.install(
            Thread::Dsp,
            ThreadClock::new(24_606_720),
            Box::new(Behind {
                clock: ThreadClock::new(24_606_720),
                step_clocks: 24,
            }),
        );
        scheduler.install(
            Thread::Cpu,
            ThreadClock::new(21_477_273),
            Box::new(Behind {
                clock: ThreadClock::new(21_477_273),
                step_clocks: 52,
            }),
        );
        scheduler.enable_switch_log();
        scheduler.synchronize(Thread::Smp, Thread::Dsp);
        let log = scheduler.switch_log();
        assert!(!log.is_empty());
        assert!(log
            .iter()
            .any(|(m, _, o, _)| *m == Thread::Smp && *o == Thread::Dsp));
    }
}
