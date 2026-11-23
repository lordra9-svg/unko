//! Resumable S-CPU execution for the ares-style cooperative scheduler.
//!
//! ares runs the 65C816 as a cooperative thread: an S-SMP `$F4-$F7` port read
//! can switch to the S-CPU and resume it *mid-instruction*. `Cpu::step` here is
//! a recursive interpreter, so the same behavior needs a separate stack. This
//! module wraps one `Cpu::step` call in a stackful coroutine; every bus access
//! is yielded to the driver, which performs it against the real machine and
//! resumes the interpreter with the result. The interpreter's call stack is
//! preserved, so the scheduler can interleave S-CPU accesses at any point.

use corosensei::{Coroutine, Yielder};

use crate::cpu::{Cpu, CpuBus};

/// One bus operation requested by the suspended S-CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusRequest {
    /// Read one byte.
    Read {
        /// 24-bit address.
        address: u32,
        /// Open-bus value.
        mdr: u8,
    },
    /// Write one byte.
    Write {
        /// 24-bit address.
        address: u32,
        /// Data byte.
        value: u8,
    },
    /// Master clocks required by an access.
    AccessCycles {
        /// 24-bit address.
        address: u32,
    },
    /// Master clocks one idle cycle costs.
    IdleCycles,
    /// Advance one idle CPU cycle.
    Idle,
    /// ares `dmaEdge()` at the start of an internal cycle.
    IdleEdge,
    /// Advance the peripherals while the CPU is mid-instruction.
    Advance {
        /// Master clocks to advance.
        clocks: u32,
    },
}

/// The driver's answer to a [`BusRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusResponse {
    /// Byte returned by a read.
    Byte(u8),
    /// Clock count returned by an access/idle/advance.
    Clocks(u32),
    /// No value (writes and idle cycles).
    None,
}

/// Input passed into the S-CPU coroutine.
#[derive(Debug)]
pub enum CpuInput {
    /// Start one instruction with this CPU state.
    Start(Cpu),
    /// Result of the previous bus request.
    Response(BusResponse),
}

/// What the S-CPU coroutine yields.
#[derive(Debug)]
pub enum CpuOutput {
    /// The interpreter needs a bus operation.
    Request(BusRequest),
    /// The instruction completed; the CPU state and retired clocks are returned.
    Done(Cpu, u32),
}

/// The S-CPU coroutine type.
pub type CpuCoroutine = Coroutine<CpuInput, CpuOutput, ()>;

/// A `Snes` field wrapper. The coroutine is transient execution machinery, so
/// it is ignored by cloning/equality: a clone starts without one (it is
/// recreated lazily) and two systems compare equal regardless of it.
#[derive(Default)]
pub struct CpuCoroutineSlot(pub Option<CpuCoroutine>);

impl Clone for CpuCoroutineSlot {
    fn clone(&self) -> Self {
        Self(None)
    }
}

impl PartialEq for CpuCoroutineSlot {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for CpuCoroutineSlot {}

impl std::fmt::Debug for CpuCoroutineSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CpuCoroutine")
    }
}

/// Build the coroutine. It stays parked at `Done` between instructions and
/// owns no CPU state while suspended, so a save-state restore can replace the
/// CPU and continue.
pub fn new_coroutine() -> CpuCoroutine {
    Coroutine::new(|yielder, mut input| loop {
        let CpuInput::Start(mut cpu) = input else {
            unreachable!("S-CPU coroutine resumed without a CPU")
        };
        let cycles = cpu.step(&mut CoroBus { yielder });
        input = yielder.suspend(CpuOutput::Done(cpu, cycles));
    })
}

/// The interpreter-facing bus: every call suspends and returns the driver's
/// response as if it were a normal bus access.
struct CoroBus<'a> {
    yielder: &'a Yielder<CpuInput, CpuOutput>,
}

impl CoroBus<'_> {
    fn request(&mut self, request: BusRequest) -> BusResponse {
        match self.yielder.suspend(CpuOutput::Request(request)) {
            CpuInput::Response(response) => response,
            CpuInput::Start(_) => unreachable!("bus request resumed with a CPU"),
        }
    }
}

impl CpuBus for CoroBus<'_> {
    fn read(&mut self, address: u32, mdr: u8) -> u8 {
        match self.request(BusRequest::Read { address, mdr }) {
            BusResponse::Byte(value) => value,
            _ => unreachable!("read resumed without a byte"),
        }
    }

    fn write(&mut self, address: u32, value: u8) {
        self.request(BusRequest::Write { address, value });
    }

    fn access_cycles(&mut self, address: u32) -> u32 {
        match self.request(BusRequest::AccessCycles { address }) {
            BusResponse::Clocks(clocks) => clocks,
            _ => unreachable!("access_cycles resumed without clocks"),
        }
    }

    fn idle_cycles(&self) -> u32 {
        6
    }

    fn idle(&mut self) {
        self.request(BusRequest::Idle);
    }

    fn idle_edge(&mut self) {
        self.request(BusRequest::IdleEdge);
    }

    fn advance(&mut self, clocks: u32) -> u32 {
        match self.request(BusRequest::Advance { clocks }) {
            BusResponse::Clocks(extra) => extra,
            _ => unreachable!("advance resumed without clocks"),
        }
    }
}

/// Slot for the S-SMP actor coroutine (transient execution machinery).
#[derive(Default)]
pub struct ApuActorSlot(pub Option<crate::apu::ApuCoroutine>);
impl Clone for ApuActorSlot {
    fn clone(&self) -> Self {
        Self(None)
    }
}
impl PartialEq for ApuActorSlot {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for ApuActorSlot {}
impl std::fmt::Debug for ApuActorSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApuActor")
    }
}
