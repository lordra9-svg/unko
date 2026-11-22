//! Per-access resumable SPC700 recorder (corosensei).
//!
//! `Spc700::step` normally executes a whole instruction in one call. For the
//! mutual CPU/SMP/DSP scheduler the instruction must be able to stop at every
//! bus access so the DSP can be ticked between accesses. This module runs the
//! core inside a coroutine whose bus yields one [`SpcEvent`] per access; the
//! host (which owns the APU) performs the access and resumes with the result.
//!
//! It is deliberately independent of [`crate::apu`]: the bus only *describes*
//! what it needs, so no APU state is borrowed across a yield.

use crate::spc700::{Spc700, SpcBus};

/// Value handed back to the suspended core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpcResume {
    /// Result of a read.
    Byte(u8),
    /// Result of `wait_cycles`/`timer_cycles`.
    Clocks(u32),
    /// Acknowledgement for write/idle/advance/step-clock.
    Unit,
}

/// Input to the recorder coroutine.
#[derive(Debug)]
pub enum SpcInput {
    /// Hand the core to the recorder and begin.
    Start(Spc700),
    /// Response to the previously yielded event.
    Resume(SpcResume),
}

/// One bus access the core has reached and not yet performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpcEvent {
    /// Read one byte from `address`.
    Read(u16),
    /// Write one byte to `address`.
    Write(u16, u8),
    /// One idle cycle.
    Idle,
    /// Advance the S-SMP timers by `clocks`.
    Advance(u32),
    /// Advance the cooperative peripherals (the S-DSP) by `clocks`.
    StepClock(u32),
    /// Access cost in S-SMP time units for `address`.
    Wait(Option<u16>),
    /// Clocks the timers advance for `address`.
    Timer(Option<u16>),
    /// The current instruction retired; the core is at the next boundary.
    InstructionDone,
}

/// The resumable recorder.
pub type SpcCoroutine = corosensei::Coroutine<SpcInput, SpcEvent, ()>;

/// A bus that yields every access instead of performing it.
struct ResumeBus<F: FnMut(SpcEvent) -> SpcResume> {
    resume: F,
}

impl<F: FnMut(SpcEvent) -> SpcResume> SpcBus for ResumeBus<F> {
    fn read(&mut self, address: u16) -> u8 {
        match (self.resume)(SpcEvent::Read(address)) {
            SpcResume::Byte(value) => value,
            _ => 0,
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        let _ = (self.resume)(SpcEvent::Write(address, value));
    }

    fn idle(&mut self) {
        let _ = (self.resume)(SpcEvent::Idle);
    }

    fn advance(&mut self, clocks: u32) {
        let _ = (self.resume)(SpcEvent::Advance(clocks));
    }

    fn step_clock(&mut self, clocks: u32) {
        let _ = (self.resume)(SpcEvent::StepClock(clocks));
    }

    fn wait_cycles(&mut self, address: Option<u16>) -> u32 {
        match (self.resume)(SpcEvent::Wait(address)) {
            SpcResume::Clocks(clocks) => clocks,
            _ => 1,
        }
    }

    fn timer_cycles(&mut self, address: Option<u16>) -> u32 {
        match (self.resume)(SpcEvent::Timer(address)) {
            SpcResume::Clocks(clocks) => clocks,
            _ => 2,
        }
    }

    fn split_port_read(&mut self, address: u16) -> bool {
        address & 0xFFFC == 0x00F4
    }

    fn defer_io_reads(&self) -> bool {
        true
    }
}

/// Create a recorder coroutine.
pub fn new_spc_coroutine() -> SpcCoroutine {
    corosensei::Coroutine::new(|yielder, input| {
        let SpcInput::Start(mut spc) = input else {
            return;
        };
        loop {
            {
                let mut bus = ResumeBus {
                    resume: |event| match yielder.suspend(event) {
                        SpcInput::Resume(resume) => resume,
                        SpcInput::Start(_) => SpcResume::Unit,
                    },
                };
                spc.step(&mut bus);
            }
            let _ = yielder.suspend(SpcEvent::InstructionDone);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use corosensei::CoroutineResult;

    #[test]
    fn yields_each_access_and_retires_an_instruction() {
        let mut ram = vec![0u8; 0x1_0000];
        // MOV A,#$5A ; MOV $40,A
        ram[0] = 0xE8;
        ram[1] = 0x5A;
        ram[2] = 0xC4;
        ram[3] = 0x40;

        let mut spc = Spc700::new();
        spc.pc = 0;

        let mut coroutine = new_spc_coroutine();
        let mut input = SpcInput::Start(spc);
        let mut reads = 0;
        let mut writes = Vec::new();
        let mut instructions = 0;

        for _ in 0..256 {
            match coroutine.resume(input) {
                CoroutineResult::Yield(event) => {
                    input = SpcInput::Resume(match event {
                        SpcEvent::Read(address) => {
                            reads += 1;
                            SpcResume::Byte(ram[usize::from(address)])
                        }
                        SpcEvent::Write(address, value) => {
                            ram[usize::from(address)] = value;
                            writes.push((address, value));
                            SpcResume::Unit
                        }
                        SpcEvent::Wait(_) => SpcResume::Clocks(1),
                        SpcEvent::Timer(_) => SpcResume::Clocks(2),
                        SpcEvent::StepClock(clocks) | SpcEvent::Advance(clocks) => {
                            assert!(clocks > 0);
                            SpcResume::Unit
                        }
                        SpcEvent::Idle => SpcResume::Unit,
                        SpcEvent::InstructionDone => {
                            instructions += 1;
                            if instructions == 2 {
                                break;
                            }
                            SpcResume::Unit
                        }
                    });
                }
                CoroutineResult::Return(()) => break,
            }
        }

        assert_eq!(instructions, 2);
        assert!(
            reads >= 4,
            "expected at least the two opcode fetches: {reads}"
        );
        assert_eq!(writes, vec![(0x40, 0x5A)]);
    }
}
