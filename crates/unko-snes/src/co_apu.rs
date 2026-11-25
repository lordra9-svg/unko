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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::co_scheduler::{Need, Scheduler, Thread, ThreadClock, ThreadState};
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
        // The cooperative scheduler owns the S-DSP as a separate thread; the
        // engine must not tick it from `SMP::step`.
        engine.use_external_dsp();
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

    /// Start recording `(S-SMP clock, PC)` per instruction (debug).
    pub fn enable_trace(&mut self) {
        self.engine.enable_trace();
    }

    /// The recorded instruction trace.
    pub fn trace(&self) -> &[(u128, u16)] {
        self.engine.trace()
    }
}

impl ThreadState for SmpThread {
    fn clock(&self) -> u128 {
        self.engine.smp_fixed()
    }

    fn step(&mut self) -> Need {
        match self.engine.smp_step_parking() {
            StepOutcome::Ran => Need::None,
            // ares `SMP::step -> synchronize(cpu, dsp)`: the driver ticks the
            // S-DSP to the S-SMP clock before the instruction continues.
            StepOutcome::Cycle => Need::Synchronize(Thread::Dsp),
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
        let mut need = smp.step();
        // Cycle steps (`SMP::step -> synchronize(dsp)`) come first.
        while need == Need::Synchronize(Thread::Dsp) {
            need = smp.step();
        }
        assert_eq!(need, Need::Synchronize(Thread::Cpu));
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
        let mut need = smp.step();
        let mut guard = 0;
        while need == Need::Synchronize(Thread::Dsp) && guard < 32 {
            need = smp.step();
            guard += 1;
        }
        assert_eq!(need, Need::None);
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
        self.clock.step(24);
        if let Some(sample) = self.dsp.borrow_mut().tick(&mut self.ram.borrow_mut()[..]) {
            self.samples.push_back(sample);
        }
        // ares `DSP::tick`: `Thread::step(3 * 8)` then `synchronize(smp)`.
        // Every call is one whole micro-step; the driver runs the S-SMP to the
        // DSP clock before calling again.
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
    fn a_micro_step_pulls_the_smp() {
        let mut dsp = thread();
        assert_eq!(dsp.step(), Need::Synchronize(Thread::Smp));
        let after_tick = dsp.clock();
        assert!(after_tick > 0);
        // One micro-step is 24 DSP clocks in the fixed-point base.
        let one_step = ThreadClock::new(24_606_720).scalar() * 24;
        assert_eq!(after_tick, one_step);
        assert_eq!(dsp.step(), Need::Synchronize(Thread::Smp));
        assert_eq!(dsp.clock(), one_step * 2);
    }

    #[test]
    fn a_sample_is_emitted_every_32_micro_steps() {
        let mut dsp = thread();
        for _ in 0..32 {
            assert_eq!(dsp.step(), Need::Synchronize(Thread::Smp));
        }
        assert_eq!(dsp.sample_count(), 1);
    }
}

/// [`SmpThread`] as a scheduler-owned state, shared with the driver.
///
/// The driver owns the S-CPU (it needs the whole [`crate::system::Snes`] bus),
/// so the S-SMP is installed behind an `Rc<RefCell<..>>`: the scheduler steps
/// it, the driver inspects its clock and its `$2140-$2143` latches.
pub struct SmpState(pub Rc<RefCell<SmpThread>>);

impl ThreadState for SmpState {
    fn clock(&self) -> u128 {
        self.0.borrow().clock()
    }

    fn step(&mut self) -> Need {
        self.0.borrow_mut().step()
    }
}

/// [`DspThread`] as a scheduler-owned state, shared with the driver.
pub struct DspState(pub Rc<RefCell<DspThread>>);

impl ThreadState for DspState {
    fn clock(&self) -> u128 {
        self.0.borrow().clock()
    }

    fn step(&mut self) -> Need {
        self.0.borrow_mut().step()
    }
}

/// The S-CPU's scheduler clock, published by the driver (which steps the CPU
/// coroutine itself). It never advances the CPU; it only lets `Scheduler`
/// answer clock queries and keeps the three clocks in one place.
pub struct CpuState(pub Rc<Cell<u128>>);

impl ThreadState for CpuState {
    fn clock(&self) -> u128 {
        self.0.get()
    }

    fn step(&mut self) -> Need {
        // The driver, not the scheduler, runs the S-CPU.
        Need::None
    }
}

/// The cooperative APU: the S-SMP and S-DSP states plus their shared RAM/DSP.
pub struct CoSched {
    /// The S-SMP thread state (also installed in `scheduler`).
    pub smp: Rc<RefCell<SmpThread>>,
    /// The S-DSP thread state (also installed in `scheduler`).
    pub dsp: Rc<RefCell<DspThread>>,
    /// The three-thread scheduler; the driver uses `step_report`.
    pub scheduler: Scheduler,
    /// The published S-CPU clock (fixed-point `Second` units).
    pub cpu_clock: Rc<Cell<u128>>,
}

impl CoSched {
    /// Build from an IPL ROM, power-on RAM/DSP registers and an SPC700 core.
    pub fn new(
        ipl: [u8; 64],
        ram: &[u8],
        dsp_registers: &[u8],
        core: crate::spc700::Spc700,
    ) -> Self {
        let smp = Rc::new(RefCell::new(SmpThread::new(ipl, ram, dsp_registers, core)));
        let dsp = {
            let smp = smp.borrow();
            Rc::new(RefCell::new(DspThread::new(
                smp.ram_handle(),
                smp.dsp_handle(),
            )))
        };
        let cpu_clock = Rc::new(Cell::new(0));
        let mut scheduler = Scheduler::new();
        scheduler.install(
            Thread::Cpu,
            ThreadClock::new(21_477_273),
            Box::new(CpuState(cpu_clock.clone())),
        );
        scheduler.install(
            Thread::Smp,
            ThreadClock::new(2_050_560),
            Box::new(SmpState(smp.clone())),
        );
        scheduler.install(
            Thread::Dsp,
            ThreadClock::new(24_606_720),
            Box::new(DspState(dsp.clone())),
        );
        Self {
            smp,
            dsp,
            scheduler,
            cpu_clock,
        }
    }
}

impl Default for CoSched {
    fn default() -> Self {
        let mut core = crate::spc700::Spc700::new();
        core.pc = 0xFFC0;
        Self::new([0u8; 64], &[0u8; 0x1_0000], &[0u8; 128], core)
    }
}

/// Derivable-transient slot for the cooperative APU inside `Snes`.
#[derive(Default)]
pub struct CoSchedSlot(pub Option<Box<CoSched>>);

impl Clone for CoSchedSlot {
    fn clone(&self) -> Self {
        Self(None)
    }
}
impl PartialEq for CoSchedSlot {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for CoSchedSlot {}
impl std::fmt::Debug for CoSchedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CoSched")
    }
}
