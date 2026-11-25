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
    use crate::co_scheduler::ThreadClock;
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
    fn a_non_port_instruction_suspends_normally() {
        // MOV A,#$55 ; BRA -3
        let mut smp = smp_with(&[0xE8, 0x55, 0x2F, 0xFD], 0x0200);
        // Keep the S-CPU far ahead so no access parks.
        smp.set_cpu_clock_fixed(crate::co_scheduler::SECOND);
        assert_eq!(smp.step(), Need::None);
        assert!(smp.clock() > 0);
    }
}
