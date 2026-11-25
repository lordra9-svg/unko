//! S-SMP thread adapter for the cooperative scheduler (rewrite step 2).
//!
//! The S-SMP executes directly from the SPC700 access coroutine in [`SymEngine`]
//! (no plan/replay): each `step` runs to the next instruction/suspension and
//! reports the synchronization the scheduler must perform. When the S-SMP reads
//! or writes `$F4-$F7` ahead of the S-CPU it asks the scheduler to run the S-CPU
//! to its clock (ares `SMP::readIO -> synchronize(cpu)`).
//!
//! This is deliberately a thin adapter: the APU bus, timers, ports and DSP
//! registers remain the ones already exercised by `SymEngine`, which keeps the
//! diff reviewable while the scheduler is brought up incrementally.

use crate::co_scheduler::{Need, Thread, ThreadState};
use crate::sym_scheduler::{StepOutcome, SymEngine};

/// The S-SMP as a cooperative thread.
pub struct SmpThread {
    engine: SymEngine,
}

impl SmpThread {
    /// Build the S-SMP with an IPL ROM, power-on RAM/DSP registers and a core.
    pub fn new(
        ipl: [u8; 64],
        ram: &[u8],
        dsp_registers: &[u8],
        core: crate::spc700::Spc700,
    ) -> Self {
        let mut engine = SymEngine::new();
        engine.load_power_on_state(ram, dsp_registers);
        engine.load_ipl(ipl);
        engine.load_core(core);
        Self { engine }
    }

    /// The underlying engine.
    pub fn engine(&self) -> &SymEngine {
        &self.engine
    }

    /// The underlying engine, mutably.
    pub fn engine_mut(&mut self) -> &mut SymEngine {
        &mut self.engine
    }

    /// Publish the S-CPU clock (fixed-point `Second` units) before a step.
    pub fn set_cpu_clock_fixed(&mut self, clock: u128) {
        self.engine.set_cpu_clock_fixed(clock);
    }

    /// The shared APU RAM handle (also given to [`DspThread`]).
    pub fn ram_handle(&self) -> std::rc::Rc<std::cell::RefCell<Vec<u8>>> {
        self.engine.ram_handle()
    }

    /// The shared S-DSP handle (also given to [`DspThread`]).
    pub fn dsp_handle(&self) -> std::rc::Rc<std::cell::RefCell<crate::dsp::Dsp>> {
        self.engine.dsp_handle()
    }
}

impl ThreadState for SmpThread {
    fn clock(&self) -> u128 {
        self.engine.smp_fixed()
    }

    fn step(&mut self) -> Need {
        match self.engine.smp_step_parking() {
            StepOutcome::Ran => Need::None,
            // The S-SMP suspended on an ahead `$F4-$F7` access: the S-CPU must
            // reach the S-SMP clock before it can continue.
            StepOutcome::Parked(_) => Need::Synchronize(Thread::Cpu),
            StepOutcome::Halted => Need::Halt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spc700::Spc700;

    fn smp_with(program: &[u8], pc: u16) -> SmpThread {
        let mut ram = vec![0u8; 0x1_0000];
        let start = usize::from(pc);
        ram[start..start + program.len()].copy_from_slice(program);
        let mut core = Spc700::new();
        core.pc = pc;
        SmpThread::new([0u8; 64], &ram, &[0u8; 128], core)
    }

    #[test]
    fn a_port_read_pulls_the_cpu() {
        // MOV A,$F4 ; BRA -2
        let mut smp = smp_with(&[0xE4, 0xF4, 0x2F, 0xFC], 0x0200);
        smp.set_cpu_clock_fixed(0);
        assert_eq!(smp.step(), Need::Synchronize(Thread::Cpu));
    }

    #[test]
    fn smp_and_dsp_share_apu_ram() {
        let smp = smp_with(&[0xFF], 0x0200);
        let ram = smp.ram_handle();
        let dsp = smp.dsp_handle();
        let _dsp_thread = DspThread::new(ram.clone(), dsp.clone());
        ram.borrow_mut()[0x1234] = 0x5A;
        assert_eq!(smp.engine().ram_handle().borrow()[0x1234], 0x5A);
        // A register write through the shared DSP is visible to both threads.
        dsp.borrow_mut().write(0x0C, 0x7F);
        assert_eq!(smp.dsp_handle().borrow().read(0x0C), 0x7F);
    }

    #[test]
    fn a_non_port_instruction_suspends_normally() {
        // MOV A,#$55 ; BRA -3
        let mut smp = smp_with(&[0xE8, 0x55, 0x2F, 0xFD], 0x0200);
        // Keep the S-CPU far ahead so no access parks.
        smp.set_cpu_clock_fixed(crate::co_scheduler::SECOND);
        assert_eq!(smp.step(), Need::None);
        assert!(smp.clock() > 0);
    }
}

/// The S-DSP as a cooperative thread.
///
/// ares `DSP::main` runs a 32-micro-step sample; each `DSP::tick` does
/// `Thread::step(3 * 8)` (24 DSP clocks) and then `synchronize(smp)`. The DSP is
/// therefore the fastest-ticking thread and pulls the S-SMP every micro-step.
pub struct DspThread {
    clock: crate::co_scheduler::ThreadClock,
    dsp: std::rc::Rc<std::cell::RefCell<crate::dsp::Dsp>>,
    ram: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
    samples: std::collections::VecDeque<crate::dsp::Sample>,
    /// The thread is mid-micro-step and must suspend after the S-SMP has caught
    /// up (so a re-entry returns `None` rather than ticking again).
    pending: bool,
}

impl DspThread {
    /// Create the DSP thread sharing the APU RAM with the S-SMP.
    pub fn new(
        ram: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
        dsp: std::rc::Rc<std::cell::RefCell<crate::dsp::Dsp>>,
    ) -> Self {
        Self {
            clock: crate::co_scheduler::ThreadClock::new(24_606_720),
            dsp,
            ram,
            samples: std::collections::VecDeque::new(),
            pending: false,
        }
    }

    /// Number of finished samples waiting to be consumed.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Drain finished samples.
    pub fn drain_samples(&mut self) -> impl Iterator<Item = crate::dsp::Sample> + '_ {
        self.samples.drain(..)
    }
}

impl ThreadState for DspThread {
    fn clock(&self) -> u128 {
        self.clock.clock()
    }

    fn step(&mut self) -> Need {
        if self.pending {
            self.pending = false;
            return Need::None;
        }
        self.clock.step(24);
        if let Some(sample) = self.dsp.borrow_mut().tick(&mut self.ram.borrow_mut()[..]) {
            self.samples.push_back(sample);
        }
        self.pending = true;
        Need::Synchronize(Thread::Smp)
    }
}

#[cfg(test)]
mod dsp_tests {
    use super::*;
    use crate::co_scheduler::ThreadClock;

    fn thread() -> DspThread {
        DspThread::new(
            std::rc::Rc::new(std::cell::RefCell::new(vec![0u8; 0x1_0000])),
            std::rc::Rc::new(std::cell::RefCell::new(crate::dsp::Dsp::new())),
        )
    }

    #[test]
    fn a_micro_step_pulls_the_smp_then_suspends() {
        let mut dsp = thread();
        assert_eq!(dsp.step(), Need::Synchronize(Thread::Smp));
        let after_tick = dsp.clock();
        assert!(after_tick > 0);
        assert_eq!(dsp.step(), Need::None);
        assert_eq!(dsp.clock(), after_tick);
        // One micro-step is 24 DSP clocks in the fixed-point base.
        let one_step = ThreadClock::new(24_606_720).scalar() * 24;
        assert_eq!(after_tick, one_step);
    }

    #[test]
    fn a_sample_is_emitted_every_32_micro_steps() {
        let mut dsp = thread();
        for _ in 0..32 {
            assert_eq!(dsp.step(), Need::Synchronize(Thread::Smp));
            assert_eq!(dsp.step(), Need::None);
        }
        assert_eq!(dsp.sample_count(), 1);
    }
}
