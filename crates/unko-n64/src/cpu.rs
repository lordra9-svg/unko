//! VR4300 (MIPS III) integer interpreter used by the N64 system.
//!
//! This is the CPU foundation of the ares-derived N64 core: a big-endian,
//! 64-bit MIPS III interpreter with architectural delay slots. It implements
//! the integer instruction set needed by boot code (arithmetic, shifts,
//! logical ops, branches with delay slots, jumps, loads/stores, basic COP0
//! moves and TLB management), the TLB-mapped address translation path and a
//! single-precision COP1 FPU (arithmetic, conversions, compares and LWC1/SWC1).
//! Precise exceptions and the FPU pipeline are later milestones.

use std::collections::BTreeSet;
use std::fmt;

/// Byte/word access used by the CPU.
pub trait Bus {
    /// Reads one byte.
    fn read_u8(&mut self, address: u64) -> u8;
    /// Writes one byte.
    fn write_u8(&mut self, address: u64, value: u8);

    /// Reads a big-endian halfword.
    fn read_u16(&mut self, address: u64) -> u16 {
        u16::from_be_bytes([self.read_u8(address), self.read_u8(address + 1)])
    }
    /// Writes a big-endian halfword.
    fn write_u16(&mut self, address: u64, value: u16) {
        let bytes = value.to_be_bytes();
        self.write_u8(address, bytes[0]);
        self.write_u8(address + 1, bytes[1]);
    }
    /// Reads a big-endian word.
    fn read_u32(&mut self, address: u64) -> u32 {
        u32::from_be_bytes([
            self.read_u8(address),
            self.read_u8(address + 1),
            self.read_u8(address + 2),
            self.read_u8(address + 3),
        ])
    }
    /// Writes a big-endian word.
    fn write_u32(&mut self, address: u64, value: u32) {
        let bytes = value.to_be_bytes();
        for (offset, byte) in bytes.iter().enumerate() {
            self.write_u8(address + offset as u64, *byte);
        }
    }
    /// Reads a big-endian doubleword.
    fn read_u64(&mut self, address: u64) -> u64 {
        let high = self.read_u32(address);
        let low = self.read_u32(address + 4);
        (u64::from(high) << 32) | u64::from(low)
    }
    /// Writes a big-endian doubleword.
    fn write_u64(&mut self, address: u64, value: u64) {
        self.write_u32(address, (value >> 32) as u32);
        self.write_u32(address + 4, value as u32);
    }
}

/// One CPU fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuError {
    /// The opcode is not implemented yet.
    Unimplemented {
        /// Program counter of the instruction.
        pc: u64,
        /// Raw instruction word.
        instruction: u32,
    },
    /// A load/store was not naturally aligned.
    Unaligned {
        /// Virtual address.
        address: u64,
        /// Whether the faulting access was a store.
        store: bool,
    },
    /// A TLB miss or invalid mapping.
    Tlb {
        /// Virtual address that failed translation.
        address: u64,
        /// Whether the faulting access was a store.
        store: bool,
        /// Whether a matching entry existed but its page was invalid.
        invalid: bool,
    },
    /// A reserved instruction encoding was executed.
    ReservedInstruction {
        /// Program counter of the instruction.
        pc: u64,
        /// Raw instruction word.
        instruction: u32,
    },
    /// A coprocessor instruction was executed while that unit was disabled.
    CoprocessorUnusable {
        /// Coprocessor number (1 = FPU).
        coprocessor: u8,
    },
    /// A signed integer overflow in ADD/ADDI/SUB.
    Overflow {
        /// Program counter of the instruction.
        pc: u64,
    },
    /// A data/instruction cache parity error was observed.
    CacheError {
        /// Physical address of the faulty line.
        address: u64,
    },
    /// A floating-point exception with its enable bit set.
    Fpu {
        /// Program counter of the instruction.
        pc: u64,
    },
    /// `SYSCALL` or `BREAK` executed.
    Trap {
        /// Program counter of the instruction.
        pc: u64,
        /// Breakpoint code (`BREAK`) or zero (`SYSCALL`).
        code: u32,
        /// Whether this was `BREAK` rather than `SYSCALL`.
        breakpoint: bool,
    },
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unimplemented { pc, instruction } => {
                write!(f, "unimplemented opcode {instruction:08X} at {pc:016X}")
            }
            Self::Unaligned { address, .. } => write!(f, "unaligned access at {address:016X}"),
            Self::ReservedInstruction { pc, instruction } => {
                write!(f, "reserved instruction {instruction:08X} at {pc:016X}")
            }
            Self::CoprocessorUnusable { coprocessor } => {
                write!(f, "coprocessor {coprocessor} unusable")
            }
            Self::CacheError { address } => {
                write!(f, "cache parity error at {address:016X}")
            }
            Self::Overflow { pc } => write!(f, "integer overflow at {pc:016X}"),
            Self::Fpu { pc } => write!(f, "floating point exception at {pc:016X}"),
            Self::Tlb { address, .. } => write!(f, "TLB fault at {address:016X}"),
            Self::Trap {
                pc,
                code,
                breakpoint,
            } => {
                if *breakpoint {
                    write!(f, "break {code} at {pc:016X}")
                } else {
                    write!(f, "syscall at {pc:016X}")
                }
            }
        }
    }
}

impl std::error::Error for CpuError {}

/// One TLB entry (raw CP0 EntryHi/EntryLo0/EntryLo1/PageMask values).
#[derive(Debug, Clone, Copy, Default)]
pub struct TlbEntry {
    /// CP0 EntryHi: VPN2, ASID and global bit.
    pub entry_hi: u32,
    /// CP0 EntryLo0 (even page).
    pub lo0: u32,
    /// CP0 EntryLo1 (odd page).
    pub lo1: u32,
    /// CP0 PageMask.
    pub page_mask: u32,
}

impl TlbEntry {
    fn valid(&self) -> bool {
        self.lo0 & 2 != 0 || self.lo1 & 2 != 0
    }
}

/// Asynchronous error-level exception sources (ErrorEPC / Status.ERL path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSource {
    /// Power-on reset.
    Reset,
    /// Non-maskable interrupt.
    Nmi,
    /// Watchdog timeout.
    Watchdog,
    /// Cache error.
    CacheError,
}

const CACHE_SETS: usize = 512;

#[derive(Debug, Clone, Copy, Default)]
struct CacheLine {
    valid: bool,
    tag: u64,
    data: [u8; 16],
    ecc: [u8; 16],
}

#[derive(Debug, Clone, Copy, Default)]
struct CacheSet {
    ways: [CacheLine; 2],
    /// Way used most recently (the other way is replaced on a miss).
    lru: u8,
}

/// VR4300 architectural state.
#[derive(Debug, Clone)]
pub struct Cpu {
    /// General purpose registers; index 0 is hardwired to zero.
    pub gpr: [u64; 32],
    /// Multiply/divide high result.
    pub hi: u64,
    /// Multiply/divide low result.
    pub lo: u64,
    /// Next instruction to execute (branch delay slots included).
    pub pc: u64,
    /// Instruction after `pc`; updated by branches/jumps.
    pub next_pc: u64,
    /// Retired instructions.
    pub instructions: u64,
    /// Retired CPU clock cycles (per-instruction cost model).
    pub cycles: u64,
    /// COP0 register file (Status, Cause, EPC, Count, ...).
    pub cp0: [u64; 32],
    /// 32-entry TLB.
    pub tlb: [TlbEntry; 32],
    /// COP1 floating-point registers (32-bit floats in the low word).
    pub fpr: [u64; 32],
    /// COP1 control/status register (`fcr31`).
    pub fcr31: u32,
    /// Whether the instruction about to execute is a branch delay slot.
    pub delay_slot: bool,
    /// External NMI line pending.
    pending_nmi: bool,
    /// External cache-error line pending.
    pending_cache_error: bool,
    /// CP0 Count/Compare match already taken.
    compare_latched: bool,
    /// Two-way set-associative data/instruction cache (16-byte lines, ECC).
    cache: Box<[CacheSet]>,
    /// Physical cache lines that should be filled with a parity fault.
    injected_cache_errors: BTreeSet<u64>,
}

impl Cpu {
    /// Creates a CPU starting at `pc` (with an empty delay slot).
    #[must_use]
    pub fn new(pc: u64) -> Self {
        Self::with_cache(pc, true)
    }

    /// Cheap placeholder used while `Machine::step` temporarily moves the CPU
    /// out of the machine; it never executes.
    #[must_use]
    pub(crate) fn placeholder() -> Self {
        Self::with_cache(0, false)
    }

    fn with_cache(pc: u64, cache: bool) -> Self {
        Self {
            gpr: [0; 32],
            hi: 0,
            lo: 0,
            pc,
            next_pc: pc.wrapping_add(4),
            instructions: 0,
            cycles: 0,
            cp0: [0; 32],
            tlb: [TlbEntry::default(); 32],
            fpr: [0; 32],
            fcr31: 0,
            delay_slot: false,
            pending_nmi: false,
            pending_cache_error: false,
            compare_latched: false,
            cache: if cache {
                vec![CacheSet::default(); CACHE_SETS].into_boxed_slice()
            } else {
                Vec::new().into_boxed_slice()
            },
            injected_cache_errors: BTreeSet::new(),
        }
    }

    /// Executes one instruction.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<(), CpuError> {
        let pc = self.pc;
        if self.take_hardware_error() {
            return Ok(());
        }
        let delay_slot = self.delay_slot;
        if self.take_interrupt() {
            return Ok(());
        }
        self.pc = self.next_pc;
        self.next_pc = self.pc.wrapping_add(4);
        self.delay_slot = false;

        let instruction = match self.translate(pc, false) {
            Ok(address) => match self.cache_read_u32(address, bus) {
                Ok(value) => value,
                Err(error) => {
                    self.deliver_error(&error, pc, delay_slot);
                    self.advance_cycles(1);
                    return Ok(());
                }
            },
            Err(error) => {
                self.deliver_error(&error, pc, delay_slot);
                self.advance_cycles(1);
                return Ok(());
            }
        };
        self.instructions += 1;
        let cycles = instruction_cycles(instruction);
        match self.execute(instruction, pc, bus) {
            Err(
                error @ (CpuError::Tlb { .. }
                | CpuError::Unaligned { .. }
                | CpuError::Trap { .. }
                | CpuError::ReservedInstruction { .. }
                | CpuError::CoprocessorUnusable { .. }
                | CpuError::Fpu { .. }
                | CpuError::CacheError { .. }
                | CpuError::Overflow { .. }),
            ) => {
                self.deliver_error(&error, pc, delay_slot);
                self.advance_cycles(1);
                Ok(())
            }
            Ok(()) => {
                self.advance_cycles(cycles);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Takes a pending maskable interrupt when `Status.IE` is set, `EXL`/`ERL`
    /// are clear and a `Cause.IP` bit is enabled in `Status.IM`.
    fn take_interrupt(&mut self) -> bool {
        let status = self.cp0[12];
        if status & 1 == 0 || status & (2 | 4) != 0 {
            return false;
        }
        if self.cp0[13] & status & 0xFF00 == 0 {
            return false;
        }
        let pc = self.pc;
        self.deliver_exception(0, None, false, pc, false);
        true
    }

    /// Enters an error-level exception (NMI, cache error, soft reset): saves
    /// the return address in ErrorEPC, sets Status.ERL and vectors. ERET then
    /// returns through ErrorEPC and clears ERL.
    pub fn raise_error_exception(&mut self, vector: u64) {
        self.cp0[30] = self.pc & 0xFFFF_FFFF;
        self.cp0[12] |= 4;
        self.pc = vector;
        self.next_pc = vector.wrapping_add(4);
    }

    fn deliver_error(&mut self, error: &CpuError, pc: u64, delay_slot: bool) {
        match *error {
            CpuError::Tlb {
                address,
                store,
                invalid,
            } => {
                let code = if store { 3 } else { 2 };
                self.deliver_exception(code, Some(address), delay_slot, pc, !invalid);
            }
            CpuError::Unaligned { address, store } => {
                let code = if store { 5 } else { 4 };
                self.deliver_exception(code, Some(address), delay_slot, pc, false);
            }
            CpuError::Trap {
                code, breakpoint, ..
            } => {
                let exception = if breakpoint { 9 } else { 8 };
                let _ = code;
                self.deliver_exception(exception, None, delay_slot, pc, false);
            }
            CpuError::ReservedInstruction { .. } => {
                self.deliver_exception(10, None, delay_slot, pc, false);
            }
            CpuError::CoprocessorUnusable { coprocessor } => {
                self.deliver_exception(11, None, delay_slot, pc, false);
                self.cp0[13] |= u64::from(coprocessor) << 28;
            }
            CpuError::Fpu { .. } => {
                self.deliver_exception(15, None, delay_slot, pc, false);
            }
            CpuError::CacheError { .. } => {
                self.take_error(ErrorSource::CacheError);
            }
            CpuError::Overflow { .. } => {
                self.deliver_exception(12, None, delay_slot, pc, false);
            }
            _ => {}
        }
    }

    /// Delivers a precise exception through CP0.
    #[allow(clippy::too_many_arguments)]
    fn deliver_exception(
        &mut self,
        code: u64,
        bad_address: Option<u64>,
        delay_slot: bool,
        pc: u64,
        refill: bool,
    ) {
        let status = self.cp0[12];
        let already_exl = status & 2 != 0;
        let error_level = status & 4 != 0;
        if !already_exl && !error_level {
            let epc = if delay_slot { pc.wrapping_sub(4) } else { pc };
            self.cp0[14] = epc & 0xFFFF_FFFF;
        }
        let bd = if delay_slot { 1u64 << 31 } else { 0 };
        self.cp0[13] = (self.cp0[13] & !(0x1F << 2) & !(1 << 31) & !(3 << 28)) | (code << 2) | bd;
        if let Some(address) = bad_address {
            self.cp0[8] = address & 0xFFFF_FFFF;
        }
        self.cp0[12] = status | 2;
        let bev = status & (1 << 22) != 0;
        let refill = refill && !already_exl && !error_level;
        let vector = match (bev, refill) {
            (false, true) => 0x8000_0000,
            (false, false) => 0x8000_0180,
            (true, true) => 0xBFC0_0200,
            (true, false) => 0xBFC0_0380,
        };
        self.pc = vector;
        self.next_pc = vector.wrapping_add(4);
    }

    fn require_coprocessor(&self, coprocessor: u8) -> Result<(), CpuError> {
        // Status.CU1 enables COP1; the VR4300 has no other coprocessors.
        if coprocessor == 1 && self.cp0[12] & (1 << 29) == 0 {
            return Err(CpuError::CoprocessorUnusable { coprocessor });
        }
        Ok(())
    }

    /// Asserts or clears a maskable hardware interrupt line (0 = Int0/IP2 ...
    /// 5 = Int5/IP7) in `Cause.IP`.
    pub fn set_interrupt_line(&mut self, line: u32, asserted: bool) {
        if line < 6 {
            let bit = 10 + line;
            if asserted {
                self.cp0[13] |= 1 << bit;
            } else {
                self.cp0[13] &= !(1 << bit);
            }
        }
    }

    /// Raises the external non-maskable-interrupt line.
    pub fn signal_nmi(&mut self) {
        self.pending_nmi = true;
    }

    /// Raises the external cache-error line.
    pub fn signal_cache_error(&mut self) {
        self.pending_cache_error = true;
    }

    fn parity_of(byte: u8) -> u8 {
        (byte.count_ones() & 1) as u8
    }

    fn cache_fill<B: Bus>(&mut self, physical: u64, bus: &mut B) {
        let line_number = physical / 16;
        let set = (line_number as usize) % CACHE_SETS;
        let tag = line_number / CACHE_SETS as u64;
        let way = {
            let ways = &self.cache[set].ways;
            if !ways[0].valid {
                0
            } else if !ways[1].valid {
                1
            } else {
                1usize - usize::from(self.cache[set].lru & 1)
            }
        };
        let base = line_number * 16;
        let mut line = CacheLine {
            valid: true,
            tag,
            ..CacheLine::default()
        };
        for (index, slot) in line.data.iter_mut().enumerate() {
            let byte = bus.read_u8(base + index as u64);
            *slot = byte;
            line.ecc[index] = Self::parity_of(byte);
        }
        if self.injected_cache_errors.remove(&line_number) {
            line.ecc[0] ^= 1;
        }
        self.cache[set].ways[way] = line;
        self.cache[set].lru = way as u8;
    }

    fn cache_line<B: Bus>(
        &mut self,
        address: u64,
        bus: &mut B,
    ) -> Result<(usize, usize, usize), CpuError> {
        let line_number = address / 16;
        let set = (line_number as usize) % CACHE_SETS;
        let tag = line_number / CACHE_SETS as u64;
        let offset = (address % 16) as usize;
        if let Some(way) = self.cache[set]
            .ways
            .iter()
            .position(|line| line.valid && line.tag == tag)
        {
            self.cache[set].lru = way as u8;
            return Ok((set, way, offset));
        }
        self.cache_fill(address, bus);
        self.cache[set]
            .ways
            .iter()
            .position(|line| line.valid && line.tag == tag)
            .map(|way| (set, way, offset))
            .ok_or(CpuError::CacheError { address })
    }

    fn uncached(address: u64) -> bool {
        address >= 0x0400_0000
    }

    fn cache_read_u8<B: Bus>(&mut self, address: u64, bus: &mut B) -> Result<u8, CpuError> {
        if Self::uncached(address) {
            return Ok(bus.read_u8(address));
        }
        let (set, way, offset) = self.cache_line(address, bus)?;
        let line = &self.cache[set].ways[way];
        let byte = line.data[offset];
        if Self::parity_of(byte) != line.ecc[offset] {
            return Err(CpuError::CacheError { address });
        }
        Ok(byte)
    }

    fn cache_write_u8<B: Bus>(
        &mut self,
        address: u64,
        value: u8,
        bus: &mut B,
    ) -> Result<(), CpuError> {
        if Self::uncached(address) {
            bus.write_u8(address, value);
            return Ok(());
        }
        let (set, way, offset) = self.cache_line(address, bus)?;
        let line = &mut self.cache[set].ways[way];
        line.data[offset] = value;
        line.ecc[offset] = Self::parity_of(value);
        self.cache[set].lru = way as u8;
        // Write-through keeps RDRAM coherent with the CPU cache.
        bus.write_u8(address, value);
        Ok(())
    }

    fn cache_read_u16<B: Bus>(&mut self, address: u64, bus: &mut B) -> Result<u16, CpuError> {
        if Self::uncached(address) {
            return Ok(bus.read_u16(address));
        }
        let mut bytes = [0u8; 2];
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = self.cache_read_u8(address + index as u64, bus)?;
        }
        Ok(u16::from_be_bytes(bytes))
    }

    fn cache_read_u32<B: Bus>(&mut self, address: u64, bus: &mut B) -> Result<u32, CpuError> {
        if Self::uncached(address) {
            return Ok(bus.read_u32(address));
        }
        let (set, way, offset) = self.cache_line(address, bus)?;
        let line = &self.cache[set].ways[way];
        if offset <= 12 {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&line.data[offset..offset + 4]);
            for (index, byte) in bytes.iter().enumerate() {
                if Self::parity_of(*byte) != line.ecc[offset + index] {
                    return Err(CpuError::CacheError {
                        address: address + index as u64,
                    });
                }
            }
            return Ok(u32::from_be_bytes(bytes));
        }
        let mut bytes = [0u8; 4];
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = self.cache_read_u8(address + index as u64, bus)?;
        }
        Ok(u32::from_be_bytes(bytes))
    }

    fn cache_read_u64<B: Bus>(&mut self, address: u64, bus: &mut B) -> Result<u64, CpuError> {
        if Self::uncached(address) {
            return Ok(bus.read_u64(address));
        }
        let mut bytes = [0u8; 8];
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = self.cache_read_u8(address + index as u64, bus)?;
        }
        Ok(u64::from_be_bytes(bytes))
    }

    fn cache_write_u16<B: Bus>(
        &mut self,
        address: u64,
        value: u16,
        bus: &mut B,
    ) -> Result<(), CpuError> {
        if Self::uncached(address) {
            bus.write_u16(address, value);
            return Ok(());
        }
        for (index, byte) in value.to_be_bytes().iter().enumerate() {
            self.cache_write_u8(address + index as u64, *byte, bus)?;
        }
        Ok(())
    }

    fn cache_write_u32<B: Bus>(
        &mut self,
        address: u64,
        value: u32,
        bus: &mut B,
    ) -> Result<(), CpuError> {
        if Self::uncached(address) {
            bus.write_u32(address, value);
            return Ok(());
        }
        let (set, way, offset) = self.cache_line(address, bus)?;
        let bytes = value.to_be_bytes();
        if offset <= 12 {
            let line = &mut self.cache[set].ways[way];
            line.data[offset..offset + 4].copy_from_slice(&bytes);
            for (index, byte) in bytes.iter().enumerate() {
                line.ecc[offset + index] = Self::parity_of(*byte);
            }
            self.cache[set].lru = way as u8;
        } else {
            for (index, byte) in bytes.iter().enumerate() {
                self.cache_write_u8(address + index as u64, *byte, bus)?;
            }
        }
        bus.write_u32(address, value);
        Ok(())
    }

    fn cache_write_u64<B: Bus>(
        &mut self,
        address: u64,
        value: u64,
        bus: &mut B,
    ) -> Result<(), CpuError> {
        if Self::uncached(address) {
            bus.write_u64(address, value);
            return Ok(());
        }
        for (index, byte) in value.to_be_bytes().iter().enumerate() {
            self.cache_write_u8(address + index as u64, *byte, bus)?;
        }
        Ok(())
    }

    /// Marks the cache line containing `physical_address` as having a parity
    /// error. The next access to that line raises the cache-error exception.
    pub fn inject_cache_parity_error(&mut self, physical_address: u64) {
        let line_number = physical_address / 16;
        self.injected_cache_errors.insert(line_number);
        let set = (line_number as usize) % CACHE_SETS;
        let tag = line_number / CACHE_SETS as u64;
        for line in self.cache[set].ways.iter_mut() {
            if line.valid && line.tag == tag {
                line.valid = false;
            }
        }
    }

    /// Clears the cache contents and any injected parity faults.
    pub fn clear_cache(&mut self) {
        self.cache = vec![CacheSet::default(); CACHE_SETS].into_boxed_slice();
        self.injected_cache_errors.clear();
    }

    /// Services pending NMI/cache error and the CP0 Count/Compare watchdog.
    /// Returns true when an error-level exception was taken.
    fn take_hardware_error(&mut self) -> bool {
        if self.pending_nmi {
            self.pending_nmi = false;
            self.take_error(ErrorSource::Nmi);
            return true;
        }
        if self.pending_cache_error {
            self.pending_cache_error = false;
            self.take_error(ErrorSource::CacheError);
            return true;
        }
        false
    }

    /// Advances CP0 Count by the modelled cycle cost of one instruction and
    /// raises the watchdog error when the counter reaches Compare.
    fn advance_cycles(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.cycles = self.cycles.wrapping_add(1);
            self.cp0[9] = self.cp0[9].wrapping_add(1) & 0xFFFF_FFFF;
            let count = self.cp0[9];
            let compare = self.cp0[11];
            if compare != 0 && count == compare {
                if !self.compare_latched {
                    self.compare_latched = true;
                    self.take_error(ErrorSource::Watchdog);
                    return;
                }
            } else {
                self.compare_latched = false;
            }
        }
    }

    fn gpr(&self, index: u32) -> u64 {
        self.gpr[index as usize]
    }

    fn set_gpr(&mut self, index: u32, value: u64) {
        if index != 0 {
            self.gpr[index as usize] = value;
        }
    }

    /// Takes an asynchronous error-level exception. `NMI`, watchdog and cache
    /// errors save `ErrorEPC`, set `Status.ERL` and vector to the boot/cache
    /// error vectors; reset vectors without saving a return address.
    pub fn take_error(&mut self, source: ErrorSource) {
        self.delay_slot = false;
        match source {
            ErrorSource::Reset => {
                self.cp0[12] |= 4;
                self.pc = 0xBFC0_0000;
                self.next_pc = self.pc.wrapping_add(4);
            }
            ErrorSource::Nmi | ErrorSource::Watchdog => {
                self.raise_error_exception(0xBFC0_0000);
            }
            ErrorSource::CacheError => {
                self.raise_error_exception(0xA000_0000);
            }
        }
    }

    fn execute<B: Bus>(&mut self, instruction: u32, pc: u64, bus: &mut B) -> Result<(), CpuError> {
        let op = instruction >> 26;
        let rs = (instruction >> 21) & 0x1F;
        let rt = (instruction >> 16) & 0x1F;
        let rd = (instruction >> 11) & 0x1F;
        let sa = (instruction >> 6) & 0x1F;
        let funct = instruction & 0x3F;
        let imm = instruction as u16;
        let simm = i64::from(imm as i16);
        let uimm = u64::from(imm);

        match op {
            0 => self.special(instruction, pc, rs, rt, rd, sa, funct),
            1 => {
                self.delay_slot = true;
                let target = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                let value = self.gpr(rs);
                let take = match rt {
                    0x00 | 0x10 => (value as i64) < 0,  // BLTZ / BLTZAL
                    0x01 | 0x11 => (value as i64) >= 0, // BGEZ / BGEZAL
                    _ => false,
                };
                if matches!(rt, 0x10 | 0x11) {
                    self.set_gpr(31, pc.wrapping_add(8));
                }
                if take {
                    self.next_pc = target;
                }
                Ok(())
            }
            2 | 3 => {
                self.delay_slot = true;
                let target = (pc.wrapping_add(4) & !0x0FFF_FFFF)
                    | (u64::from(instruction & 0x03FF_FFFF) << 2);
                if op == 3 {
                    self.set_gpr(31, pc.wrapping_add(8));
                }
                self.next_pc = target;
                Ok(())
            }
            4 => {
                self.delay_slot = true;
                if self.gpr(rs) == self.gpr(rt) {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            5 => {
                self.delay_slot = true;
                if self.gpr(rs) != self.gpr(rt) {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            6 => {
                self.delay_slot = true;
                if (self.gpr(rs) as i64) <= 0 {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            7 => {
                self.delay_slot = true;
                if (self.gpr(rs) as i64) > 0 {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            8 => {
                let Some(value) = (self.gpr(rs) as i32).checked_add(i32::from(imm as i16)) else {
                    return Err(CpuError::Overflow { pc });
                };
                self.set_gpr(rt, value as i64 as u64);
                Ok(())
            }
            9 => {
                let value = self.gpr(rs).wrapping_add(simm as u64);
                self.set_gpr(rt, sign_extend32(value));
                Ok(())
            }
            10 => {
                let value = (self.gpr(rs) as i64) < simm;
                self.set_gpr(rt, u64::from(value));
                Ok(())
            }
            11 => {
                let value = self.gpr(rs) < (simm as u64);
                self.set_gpr(rt, u64::from(value));
                Ok(())
            }
            12 => {
                self.set_gpr(rt, self.gpr(rs) & uimm);
                Ok(())
            }
            13 => {
                self.set_gpr(rt, self.gpr(rs) | uimm);
                Ok(())
            }
            14 => {
                self.set_gpr(rt, self.gpr(rs) ^ uimm);
                Ok(())
            }
            15 => {
                self.set_gpr(rt, uimm << 16);
                Ok(())
            }
            24 | 25 => {
                self.set_gpr(rt, self.gpr(rs).wrapping_add(simm as u64));
                Ok(())
            }
            16 => self.cop0(instruction, pc, rs, rt, rd, funct),
            17 => {
                self.require_coprocessor(1)?;
                self.cop1(pc, rs, rt, rd, sa, funct)
            }
            49 => {
                self.require_coprocessor(1)?;
                self.load_f32(bus, rs, rt, simm)
            }
            53 => {
                self.require_coprocessor(1)?;
                self.load_f64(bus, rs, rt, simm)
            }
            57 => {
                self.require_coprocessor(1)?;
                self.store_f32(bus, rs, rt, simm)
            }
            61 => {
                self.require_coprocessor(1)?;
                self.store_f64(bus, rs, rt, simm)
            }
            32 => self.load8(bus, rs, rt, simm, true),
            33 => self.load16(bus, rs, rt, simm, true),
            35 => self.load32(bus, rs, rt, simm, true),
            36 => self.load8(bus, rs, rt, simm, false),
            37 => self.load16(bus, rs, rt, simm, false),
            39 => self.load32(bus, rs, rt, simm, false),
            40 => self.store8(bus, rs, rt, simm),
            41 => self.store16(bus, rs, rt, simm),
            43 => self.store32(bus, rs, rt, simm),
            18 | 19 => Err(CpuError::ReservedInstruction { pc, instruction }),
            47 => Ok(()), // CACHE
            55 => self.load64(bus, rs, rt, simm),
            63 => self.store64(bus, rs, rt, simm),
            _ => Err(CpuError::Unimplemented { pc, instruction }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn special(
        &mut self,
        instruction: u32,
        pc: u64,
        rs: u32,
        rt: u32,
        rd: u32,
        sa: u32,
        funct: u32,
    ) -> Result<(), CpuError> {
        let a = self.gpr(rs);
        let b = self.gpr(rt);
        match funct {
            0x00 => {
                self.set_gpr(rd, sign_extend32(b << sa));
                Ok(())
            }
            0x02 => {
                self.set_gpr(rd, sign_extend32((b as u32) >> sa));
                Ok(())
            }
            0x03 => {
                self.set_gpr(rd, sign_extend32(((b as u32) as i32 >> sa) as u32));
                Ok(())
            }
            0x04 => {
                self.set_gpr(rd, sign_extend32(b << (a & 31)));
                Ok(())
            }
            0x06 => {
                self.set_gpr(rd, sign_extend32((b as u32) >> (a & 31)));
                Ok(())
            }
            0x07 => {
                self.set_gpr(rd, sign_extend32(((b as u32) as i32 >> (a & 31)) as u32));
                Ok(())
            }
            0x08 => {
                self.delay_slot = true;
                self.next_pc = a;
                Ok(())
            }
            0x09 => {
                self.delay_slot = true;
                let target = a;
                self.set_gpr(rd, pc.wrapping_add(8));
                self.next_pc = target;
                Ok(())
            }
            0x0A => {
                if b == 0 {
                    self.set_gpr(rd, a);
                }
                Ok(())
            }
            0x0B => {
                if b != 0 {
                    self.set_gpr(rd, a);
                }
                Ok(())
            }
            0x0C => Err(CpuError::Trap {
                pc,
                code: 0,
                breakpoint: false,
            }),
            0x0D => Err(CpuError::Trap {
                pc,
                code: instruction >> 6 & 0xFFFFF,
                breakpoint: true,
            }),
            0x0F => Ok(()), // SYNC
            0x10 => {
                self.set_gpr(rd, self.hi);
                Ok(())
            }
            0x11 => {
                self.hi = a;
                Ok(())
            }
            0x12 => {
                self.set_gpr(rd, self.lo);
                Ok(())
            }
            0x13 => {
                self.lo = a;
                Ok(())
            }
            0x14 => {
                self.set_gpr(rd, sign_extend32(b << (a & 31)));
                Ok(())
            }
            0x16 => {
                self.set_gpr(rd, sign_extend32((b as u32) >> (a & 31)));
                Ok(())
            }
            0x17 => {
                self.set_gpr(rd, sign_extend32(((b as u32) as i32 >> (a & 31)) as u32));
                Ok(())
            }
            0x18 => {
                let product = i64::from(a as i32) * i64::from(b as i32);
                self.lo = product as u64;
                self.hi = (product >> 32) as u64;
                Ok(())
            }
            0x19 => {
                let product = u64::from(a as u32) * u64::from(b as u32);
                self.lo = product;
                self.hi = product >> 32;
                Ok(())
            }
            0x1A => {
                let divisor = b as i32;
                let dividend = a as i32;
                match (dividend.checked_div(divisor), dividend.checked_rem(divisor)) {
                    (Some(quotient), Some(remainder)) => {
                        self.lo = quotient as u32 as u64;
                        self.hi = remainder as u32 as u64;
                    }
                    _ => {
                        self.lo = if dividend >= 0 { u64::MAX } else { 1 };
                        self.hi = dividend as u32 as u64;
                    }
                }
                Ok(())
            }
            0x1B => {
                let divisor = b as u32;
                let dividend = a as u32;
                match (dividend.checked_div(divisor), dividend.checked_rem(divisor)) {
                    (Some(quotient), Some(remainder)) => {
                        self.lo = u64::from(quotient);
                        self.hi = u64::from(remainder);
                    }
                    _ => {
                        self.lo = u64::MAX;
                        self.hi = u64::from(dividend);
                    }
                }
                Ok(())
            }
            0x20 => {
                let Some(value) = (a as i32).checked_add(b as i32) else {
                    return Err(CpuError::Overflow { pc });
                };
                self.set_gpr(rd, value as i64 as u64);
                Ok(())
            }
            0x21 => {
                self.set_gpr(rd, sign_extend32(a.wrapping_add(b)));
                Ok(())
            }
            0x22 => {
                let Some(value) = (a as i32).checked_sub(b as i32) else {
                    return Err(CpuError::Overflow { pc });
                };
                self.set_gpr(rd, value as i64 as u64);
                Ok(())
            }
            0x23 => {
                self.set_gpr(rd, sign_extend32(a.wrapping_sub(b)));
                Ok(())
            }
            0x24 => {
                self.set_gpr(rd, a & b);
                Ok(())
            }
            0x25 => {
                self.set_gpr(rd, a | b);
                Ok(())
            }
            0x26 => {
                self.set_gpr(rd, a ^ b);
                Ok(())
            }
            0x27 => {
                self.set_gpr(rd, !(a | b));
                Ok(())
            }
            0x2A => {
                self.set_gpr(rd, u64::from((a as i64) < (b as i64)));
                Ok(())
            }
            0x2B => {
                self.set_gpr(rd, u64::from(a < b));
                Ok(())
            }
            0x2C => {
                self.set_gpr(rd, a.wrapping_add(b));
                Ok(())
            }
            0x2D => {
                self.set_gpr(rd, a.wrapping_add(b));
                Ok(())
            }
            0x2E => {
                self.set_gpr(rd, a.wrapping_sub(b));
                Ok(())
            }
            0x2F => {
                self.set_gpr(rd, a.wrapping_sub(b));
                Ok(())
            }
            0x38 => {
                self.set_gpr(rd, b << sa);
                Ok(())
            }
            0x3A => {
                self.set_gpr(rd, b >> sa);
                Ok(())
            }
            0x3B => {
                self.set_gpr(rd, ((b as i64) >> sa) as u64);
                Ok(())
            }
            0x3C => {
                self.set_gpr(rd, b << (sa + 32));
                Ok(())
            }
            0x3E => {
                self.set_gpr(rd, b >> (sa + 32));
                Ok(())
            }
            0x3F => {
                self.set_gpr(rd, ((b as i64) >> (sa + 32)) as u64);
                Ok(())
            }
            _ => Err(CpuError::Unimplemented { pc, instruction }),
        }
    }

    fn cop0(
        &mut self,
        instruction: u32,
        pc: u64,
        rs: u32,
        rt: u32,
        rd: u32,
        funct: u32,
    ) -> Result<(), CpuError> {
        match rs {
            0x00 => {
                self.set_gpr(rt, self.cp0[rd as usize]);
                Ok(())
            }
            0x04 => {
                // CP0 registers are 32-bit on the VR4300.
                self.cp0[rd as usize] = self.gpr(rt) & 0xFFFF_FFFF;
                Ok(())
            }
            0x10 => match funct {
                0x18 => {
                    // ERET: return to ErrorEPC when ERL is set, otherwise EPC,
                    // clearing the matching status bit.
                    let status = self.cp0[12];
                    let pc = if status & 4 != 0 {
                        self.cp0[12] &= !4;
                        self.cp0[30]
                    } else {
                        self.cp0[12] &= !2;
                        self.cp0[14]
                    };
                    self.pc = pc;
                    self.next_pc = pc.wrapping_add(4);
                    Ok(())
                }
                0x01 => {
                    self.tlb_read();
                    Ok(())
                }
                0x02 => {
                    self.tlb_write_indexed();
                    Ok(())
                }
                0x06 => {
                    self.tlb_write_random();
                    Ok(())
                }
                0x08 => {
                    self.tlb_probe();
                    Ok(())
                }
                _ => Err(CpuError::Unimplemented { pc, instruction }),
            },
            _ => Err(CpuError::Unimplemented { pc, instruction }),
        }
    }

    fn tlb_read(&mut self) {
        let index = (self.cp0[0] & 0x1F) as usize;
        let entry = self.tlb[index];
        self.cp0[5] = u64::from(entry.page_mask);
        self.cp0[10] = u64::from(entry.entry_hi);
        self.cp0[2] = u64::from(entry.lo0);
        self.cp0[3] = u64::from(entry.lo1);
    }

    fn tlb_write_indexed(&mut self) {
        let index = (self.cp0[0] & 0x1F) as usize;
        self.tlb[index] = self.tlb_entry_from_cp0();
    }

    fn tlb_write_random(&mut self) {
        let wired = (self.cp0[6] & 0x1F) as u8;
        let mut random = (self.cp0[1] & 0x1F) as u8;
        if random < wired {
            random = 31;
        }
        self.tlb[usize::from(random)] = self.tlb_entry_from_cp0();
        self.cp0[1] = if random <= wired {
            31
        } else {
            u64::from(random - 1)
        };
    }

    fn tlb_entry_from_cp0(&self) -> TlbEntry {
        TlbEntry {
            entry_hi: self.cp0[10] as u32,
            lo0: self.cp0[2] as u32,
            lo1: self.cp0[3] as u32,
            page_mask: self.cp0[5] as u32,
        }
    }

    fn tlb_probe(&mut self) {
        let vpn2 = (self.cp0[10] >> 13) & 0x7FF_FFFF;
        let asid = ((self.cp0[10] >> 8) & 0xFF) as u32;
        let mut found = None;
        for (index, entry) in self.tlb.iter().enumerate() {
            if !entry.valid() {
                continue;
            }
            let mask = u64::from((entry.page_mask >> 13) & 0x0FFF);
            let entry_vpn2 = (u64::from(entry.entry_hi) >> 13) & 0x7FF_FFFF;
            if vpn2 & !mask != entry_vpn2 & !mask {
                continue;
            }
            let global = entry.entry_hi & 1 != 0 || entry.lo0 & 1 != 0 || entry.lo1 & 1 != 0;
            if global || (entry.entry_hi >> 8) & 0xFF == asid {
                found = Some(index as u64);
                break;
            }
        }
        self.cp0[0] = found.unwrap_or(0x8000_0000);
    }

    /// Translates a virtual address using the fixed segments and the TLB.
    ///
    /// Before any TLB entry is valid the interpreter falls back to identity
    /// mapping for KUSEG so early boot code (and synthetic programs) still run.
    fn translate(&self, address: u64, store: bool) -> Result<u64, CpuError> {
        if (0x8000_0000..0xC000_0000).contains(&address) {
            return Ok(address & 0x1FFF_FFFF);
        }
        if !self.tlb.iter().any(TlbEntry::valid) {
            return Ok(address);
        }
        let vpn2 = (address >> 13) & 0x7FF_FFFF;
        // The current ASID lives in CP0 EntryHi, not in the virtual address.
        let asid = ((self.cp0[10] >> 8) & 0xFF) as u32;
        for entry in &self.tlb {
            if !entry.valid() {
                continue;
            }
            let mask = u64::from((entry.page_mask >> 13) & 0x0FFF);
            let entry_vpn2 = (u64::from(entry.entry_hi) >> 13) & 0x7FF_FFFF;
            if vpn2 & !mask != entry_vpn2 & !mask {
                continue;
            }
            let global = entry.entry_hi & 1 != 0 || entry.lo0 & 1 != 0 || entry.lo1 & 1 != 0;
            if !global && (entry.entry_hi >> 8) & 0xFF != asid {
                continue;
            }
            let lo = if (address >> 12) & 1 != 0 {
                entry.lo1
            } else {
                entry.lo0
            };
            if lo & 2 == 0 {
                return Err(CpuError::Tlb {
                    address,
                    store,
                    invalid: true,
                });
            }
            let pfn = u64::from((lo >> 6) & 0x0F_FFFF);
            return Ok((pfn << 12) | (address & 0x0FFF));
        }
        Err(CpuError::Tlb {
            address,
            store,
            invalid: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn cop1(
        &mut self,
        pc: u64,
        format: u32,
        ft: u32,
        fs: u32,
        fd: u32,
        funct: u32,
    ) -> Result<(), CpuError> {
        match format {
            0x00 => {
                self.set_gpr(ft, sign_extend32(self.fpr[fs as usize] as u32));
                Ok(())
            }
            0x01 => {
                // DMFC1
                self.set_gpr(ft, self.fpr[fs as usize]);
                Ok(())
            }
            0x02 => {
                // CFC1
                self.set_gpr(ft, sign_extend32(self.cp1_control(fs)));
                Ok(())
            }
            0x04 => {
                // MTC1
                self.fpr[fs as usize] = u64::from(self.gpr(ft) as u32);
                Ok(())
            }
            0x05 => {
                // DMTC1
                self.fpr[fs as usize] = self.gpr(ft);
                Ok(())
            }
            0x06 => {
                // CTC1
                self.set_cp1_control(fs, self.gpr(ft) as u32);
                Ok(())
            }
            0x08 => {
                self.delay_slot = true;
                let condition = self.fcr31 & (1 << 23) != 0;
                let expected = ft & 1 != 0;
                if condition == expected {
                    self.next_pc = pc.wrapping_add(4).wrapping_add(4);
                }
                Ok(())
            }
            0x10 => {
                if funct == 0x21 {
                    // CVT.D.S
                    let value = f32::from_bits(self.fpr[fs as usize] as u32);
                    self.fpr[fd as usize] = (f64::from(value)).to_bits();
                    Ok(())
                } else {
                    self.fpu_single(fs, ft, fd, funct, pc)
                }
            }
            0x11 => self.fpu_double(fs, ft, fd, funct, pc),
            0x14 => {
                if funct == 0x21 {
                    // CVT.D.W
                    let value = self.fpr[fs as usize] as u32 as i32;
                    self.fpr[fd as usize] = (f64::from(value)).to_bits();
                    Ok(())
                } else if funct == 0x20 {
                    let value = self.fpr[fs as usize] as u32 as i32;
                    self.fpr[fd as usize] = u64::from((value as f32).to_bits());
                    Ok(())
                } else {
                    self.fpu_unimplemented(pc)
                }
            }
            0x15 => {
                if funct == 0x21 {
                    // CVT.D.L
                    let value = self.fpr[fs as usize] as i64;
                    self.fpr[fd as usize] = (value as f64).to_bits();
                    Ok(())
                } else if funct == 0x20 {
                    let value = self.fpr[fs as usize] as i64;
                    self.fpr[fd as usize] = u64::from((value as f32).to_bits());
                    Ok(())
                } else {
                    self.fpu_unimplemented(pc)
                }
            }
            _ => self.fpu_unimplemented(pc),
        }
    }

    fn cp1_control(&self, register: u32) -> u32 {
        if register == 31 {
            self.fcr31
        } else {
            0
        }
    }

    fn set_cp1_control(&mut self, register: u32, value: u32) {
        if register == 31 {
            self.fcr31 = value & 0x0183_FFFF;
        }
    }

    fn fpu_single(
        &mut self,
        fs: u32,
        ft: u32,
        fd: u32,
        funct: u32,
        pc: u64,
    ) -> Result<(), CpuError> {
        let a = f32::from_bits(self.fpr[fs as usize] as u32);
        let b = f32::from_bits(self.fpr[ft as usize] as u32);
        let mut result = match funct {
            0x00 => a + b,
            0x01 => a - b,
            0x02 => a * b,
            0x03 => a / b,
            0x04 => a.sqrt(),
            0x05 => a.abs(),
            0x06 => a,
            0x07 => -a,
            0x24 => {
                if a.is_nan() {
                    self.fcr31 |= (1 << 2) | (1 << 6) | (1 << 12) | (1 << 16);
                    self.check_fpu_exception(pc)?;
                }
                let value = round_to_i32(a, self.fcr31 & 3);
                self.fpr[fd as usize] = (value as i64) as u64;
                return Ok(());
            }
            0x25 => {
                if a.is_nan() {
                    self.fcr31 |= (1 << 2) | (1 << 6) | (1 << 12) | (1 << 16);
                    self.check_fpu_exception(pc)?;
                }
                let value = round_to_i64(a, self.fcr31 & 3);
                self.fpr[fd as usize] = value as u64;
                return Ok(());
            }
            0x30..=0x3F => {
                let unordered = a.is_nan() || b.is_nan();
                let condition = match funct {
                    0x30 | 0x38 => false,
                    0x31 => unordered,
                    0x32 | 0x3A => a == b,
                    0x33 => unordered || a == b,
                    0x34 | 0x3C => a < b,
                    0x35 => unordered || a < b,
                    0x36 | 0x3E => a <= b,
                    0x37 => unordered || a <= b,
                    0x39 | 0x3B | 0x3F => unordered || a <= b,
                    0x3D => !unordered && a < b,
                    _ => false,
                };
                if condition {
                    self.fcr31 |= 1 << 23;
                } else {
                    self.fcr31 &= !(1 << 23);
                }
                return Ok(());
            }
            _ => return self.fpu_unimplemented(pc),
        };
        if funct <= 0x04 {
            let wide = match funct {
                0x00 => f64::from(a) + f64::from(b),
                0x01 => f64::from(a) - f64::from(b),
                0x02 => f64::from(a) * f64::from(b),
                0x03 => f64::from(a) / f64::from(b),
                _ => f64::from(a).sqrt(),
            };
            let mut bits = 0u32;
            if result.is_nan() && !a.is_nan() && !b.is_nan() {
                bits |= 1 << 4; // invalid
            }
            if funct == 0x03 && b == 0.0 && a != 0.0 && !a.is_nan() {
                bits |= 1 << 3; // division by zero
            }
            if result.is_infinite() && a.is_finite() && b.is_finite() && b != 0.0 {
                bits |= 1 << 2; // overflow
            }
            let flush = self.fcr31 & (1 << 24) != 0
                && result.is_finite()
                && result != 0.0
                && result.abs() < f32::MIN_POSITIVE;
            if flush {
                bits |= (1 << 1) | (1 << 0);
            } else if result.is_finite() && f64::from(result) != wide {
                bits |= 1 << 0; // inexact
            }
            if result.is_finite() && result != 0.0 && result.abs() < f32::MIN_POSITIVE {
                bits |= 1 << 1; // underflow
            }
            self.set_fpu_flags(bits);
            self.check_fpu_exception(pc)?;
            if flush {
                result = 0.0f32.copysign(result);
            }
        }
        self.fpr[fd as usize] = u64::from(result.to_bits());
        Ok(())
    }

    fn set_fpu_flags(&mut self, bits: u32) {
        self.fcr31 |= (bits << 2) | (bits << 12);
    }

    fn fpu_unimplemented(&mut self, pc: u64) -> Result<(), CpuError> {
        // fcr31 flag bit 6 and cause bit 17; unimplemented operation raises a
        // precise FPU exception regardless of the enable bits.
        self.fcr31 |= (1 << 6) | (1 << 17);
        Err(CpuError::Fpu { pc })
    }

    fn check_fpu_exception(&mut self, pc: u64) -> Result<(), CpuError> {
        let cause = (self.fcr31 >> 12) & 0x1F;
        let enable = (self.fcr31 >> 7) & 0x1F;
        if cause & enable != 0 {
            return Err(CpuError::Fpu { pc });
        }
        Ok(())
    }

    fn fpu_double(
        &mut self,
        fs: u32,
        ft: u32,
        fd: u32,
        funct: u32,
        pc: u64,
    ) -> Result<(), CpuError> {
        let a = f64::from_bits(self.fpr[fs as usize]);
        let b = f64::from_bits(self.fpr[ft as usize]);
        let result = match funct {
            0x00 => a + b,
            0x01 => a - b,
            0x02 => a * b,
            0x03 => a / b,
            0x04 => a.sqrt(),
            0x05 => a.abs(),
            0x06 => a,
            0x07 => -a,
            0x20 => {
                // CVT.S.D
                let value = a as f32;
                self.fpr[fd as usize] = u64::from(value.to_bits());
                return Ok(());
            }
            0x24 => {
                let value = round_f64_to_i32(a, self.fcr31 & 3);
                if a.is_nan() {
                    self.set_fpu_flags(1 << 4);
                    self.check_fpu_exception(pc)?;
                }
                self.fpr[fd as usize] = (value as i64) as u64;
                return Ok(());
            }
            0x25 => {
                let value = round_f64_to_i64(a, self.fcr31 & 3);
                if a.is_nan() {
                    self.set_fpu_flags(1 << 4);
                    self.check_fpu_exception(pc)?;
                }
                self.fpr[fd as usize] = value as u64;
                return Ok(());
            }
            0x30..=0x3F => {
                let unordered = a.is_nan() || b.is_nan();
                let condition = match funct {
                    0x30 | 0x38 => false,
                    0x31 => unordered,
                    0x32 | 0x3A => a == b,
                    0x33 => unordered || a == b,
                    0x34 | 0x3C => a < b,
                    0x35 => unordered || a < b,
                    0x36 | 0x3E => a <= b,
                    0x37 => unordered || a <= b,
                    0x39 | 0x3B | 0x3F => unordered || a <= b,
                    0x3D => !unordered && a < b,
                    _ => false,
                };
                if condition {
                    self.fcr31 |= 1 << 23;
                } else {
                    self.fcr31 &= !(1 << 23);
                }
                return Ok(());
            }
            _ => return self.fpu_unimplemented(pc),
        };
        if funct <= 0x04 {
            let mut bits = 0u32;
            if result.is_nan() && !a.is_nan() && !b.is_nan() {
                bits |= 1 << 4;
            }
            if funct == 0x03 && b == 0.0 && a != 0.0 && !a.is_nan() {
                bits |= 1 << 3;
            }
            if result.is_infinite() && a.is_finite() && b.is_finite() && b != 0.0 {
                bits |= 1 << 2;
            }
            let exact = match funct {
                0x00 => two_sum_error(a, b, result) == 0.0,
                0x01 => two_sum_error(a, -b, result) == 0.0,
                0x02 => a.mul_add(b, -result) == 0.0,
                0x03 => a == 0.0 || result * b == a,
                _ => result * result == a,
            };
            if result.is_finite() && !exact {
                bits |= 1 << 0;
            }
            if result.is_finite() && result != 0.0 && result.abs() < f64::MIN_POSITIVE {
                bits |= 1 << 1;
            }
            self.set_fpu_flags(bits);
            self.check_fpu_exception(pc)?;
        }
        self.fpr[fd as usize] = result.to_bits();
        Ok(())
    }

    fn address(&self, rs: u32, simm: i64) -> u64 {
        self.gpr(rs).wrapping_add(simm as u64)
    }

    fn load8<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
        signed: bool,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        let value = {
            let physical = self.translate(address, false)?;
            self.cache_read_u8(physical, bus)?
        };
        self.set_gpr(
            rt,
            if signed {
                value as i8 as i64 as u64
            } else {
                u64::from(value)
            },
        );
        Ok(())
    }

    fn load16<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
        signed: bool,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 1 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: false,
            });
        }
        let value = {
            let physical = self.translate(address, false)?;
            self.cache_read_u16(physical, bus)?
        };
        self.set_gpr(
            rt,
            if signed {
                value as i16 as i64 as u64
            } else {
                u64::from(value)
            },
        );
        Ok(())
    }

    fn load32<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
        signed: bool,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 3 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: false,
            });
        }
        let value = {
            let physical = self.translate(address, false)?;
            self.cache_read_u32(physical, bus)?
        };
        self.set_gpr(
            rt,
            if signed {
                value as i32 as i64 as u64
            } else {
                u64::from(value)
            },
        );
        Ok(())
    }

    fn load64<B: Bus>(&mut self, bus: &mut B, rs: u32, rt: u32, simm: i64) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 7 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: false,
            });
        }
        let value = {
            let physical = self.translate(address, false)?;
            self.cache_read_u64(physical, bus)?
        };
        self.set_gpr(rt, value);
        Ok(())
    }

    fn store8<B: Bus>(&mut self, bus: &mut B, rs: u32, rt: u32, simm: i64) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        {
            let physical = self.translate(address, true)?;
            self.cache_write_u8(physical, self.gpr(rt) as u8, bus)?;
        }
        Ok(())
    }

    fn store16<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 1 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: true,
            });
        }
        {
            let physical = self.translate(address, true)?;
            self.cache_write_u16(physical, self.gpr(rt) as u16, bus)?;
        }
        Ok(())
    }

    fn store32<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 3 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: true,
            });
        }
        {
            let physical = self.translate(address, true)?;
            self.cache_write_u32(physical, self.gpr(rt) as u32, bus)?;
        }
        Ok(())
    }

    fn store64<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 7 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: true,
            });
        }
        {
            let physical = self.translate(address, true)?;
            self.cache_write_u64(physical, self.gpr(rt), bus)?;
        }
        Ok(())
    }

    fn load_f32<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 3 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: false,
            });
        }
        let value = {
            let physical = self.translate(address, false)?;
            self.cache_read_u32(physical, bus)?
        };
        self.fpr[rt as usize] = u64::from(value);
        Ok(())
    }

    fn store_f32<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 3 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: true,
            });
        }
        {
            let physical = self.translate(address, true)?;
            self.cache_write_u32(physical, self.fpr[rt as usize] as u32, bus)?;
        }
        Ok(())
    }

    fn load_f64<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 7 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: false,
            });
        }
        let value = {
            let physical = self.translate(address, false)?;
            self.cache_read_u64(physical, bus)?
        };
        self.fpr[rt as usize] = value;
        Ok(())
    }

    fn store_f64<B: Bus>(
        &mut self,
        bus: &mut B,
        rs: u32,
        rt: u32,
        simm: i64,
    ) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        if address & 7 != 0 {
            return Err(CpuError::Unaligned {
                address,
                store: true,
            });
        }
        {
            let physical = self.translate(address, true)?;
            self.cache_write_u64(physical, self.fpr[rt as usize], bus)?;
        }
        Ok(())
    }
}

/// Translates a virtual address through the fixed KSEG0/KSEG1 windows.
///
/// KUSEG is treated as identity (no TLB yet).
#[must_use]
pub const fn physical(address: u64) -> u64 {
    if address & 0x8000_0000 != 0 {
        address & 0x1FFF_FFFF
    } else {
        address
    }
}

fn two_sum_error(a: f64, b: f64, sum: f64) -> f64 {
    let bv = sum - a;
    let av = sum - bv;
    (a - av) + (b - bv)
}

fn round_f64_to_i32(value: f64, mode: u32) -> i32 {
    let rounded = match mode {
        0 => value.round_ties_even(),
        1 => value.trunc(),
        2 => value.ceil(),
        _ => value.floor(),
    };
    if rounded.is_nan() {
        0
    } else {
        rounded.clamp(i32::MIN as f64, i32::MAX as f64) as i32
    }
}

fn round_f64_to_i64(value: f64, mode: u32) -> i64 {
    let rounded = match mode {
        0 => value.round_ties_even(),
        1 => value.trunc(),
        2 => value.ceil(),
        _ => value.floor(),
    };
    if rounded.is_nan() {
        0
    } else {
        rounded.clamp(i64::MIN as f64, i64::MAX as f64) as i64
    }
}

fn round_to_i32(value: f32, mode: u32) -> i32 {
    let rounded = match mode {
        0 => value.round_ties_even(),
        1 => value.trunc(),
        2 => value.ceil(),
        _ => value.floor(),
    };
    if rounded.is_nan() {
        0
    } else {
        rounded.clamp(i32::MIN as f32, i32::MAX as f32) as i32
    }
}

fn round_to_i64(value: f32, mode: u32) -> i64 {
    let rounded = match mode {
        0 => f64::from(value).round_ties_even(),
        1 => f64::from(value).trunc(),
        2 => f64::from(value).ceil(),
        _ => f64::from(value).floor(),
    };
    if rounded.is_nan() {
        0
    } else {
        rounded.clamp(i64::MIN as f64, i64::MAX as f64) as i64
    }
}

/// Modelled retire cost used by CP0 Count: integer ALU 1, loads/stores and
/// branches 2, COP0 2, COP1 4, multiplies 5 and divides 37 cycles.
fn instruction_cycles(instruction: u32) -> u32 {
    let op = instruction >> 26;
    match op {
        0 => match instruction & 0x3F {
            0x18 | 0x19 => 5,
            0x1A | 0x1B => 37,
            0x08 | 0x09 | 0x0C | 0x0D => 2,
            _ => 1,
        },
        1..=7 => 2,
        16 => 2,
        17 => 4,
        32..=63 => 2,
        _ => 1,
    }
}

fn sign_extend32(value: impl Into<u64>) -> u64 {
    value.into() as u32 as i32 as i64 as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Memory(Vec<u8>);

    impl Memory {
        fn new(size: usize) -> Self {
            Self(vec![0; size])
        }
    }

    impl Bus for Memory {
        fn read_u8(&mut self, address: u64) -> u8 {
            self.0.get(address as usize).copied().unwrap_or(0)
        }
        fn write_u8(&mut self, address: u64, value: u8) {
            if let Some(byte) = self.0.get_mut(address as usize) {
                *byte = value;
            }
        }
    }

    fn program(words: &[u32]) -> Memory {
        let mut memory = Memory::new(0x1000);
        for (index, word) in words.iter().enumerate() {
            memory.write_u32(index as u64 * 4, *word);
        }
        memory
    }

    fn run(cpu: &mut Cpu, memory: &mut Memory, steps: usize) {
        for _ in 0..steps {
            cpu.step(memory).expect("step");
        }
    }

    /// Installs the general-vector handler at physical 0x180: EPC += 4,
    /// optionally clear Cause, then ERET.
    fn install_handler(memory: &mut Memory, clear_cause: bool) {
        let mut words = vec![0x401A_7000u32, 0x275A_0004, 0x409A_7000];
        if clear_cause {
            words.push(0x4080_6800); // mtc0 zero,$13
        }
        words.push(0x4200_0018); // eret
        for (index, word) in words.iter().enumerate() {
            memory.write_u32(0x180 + index as u64 * 4, *word);
        }
    }

    #[test]
    fn arithmetic_and_logical() {
        // LUI t0,0x1234 ; ORI t0,t0,0x5678 ; ADDIU t1,t0,1 ; SLL t2,t0,4
        let mut memory = program(&[
            0x3C08_1234, // lui t0,0x1234
            0x3508_5678, // ori t0,t0,0x5678
            0x2509_0001, // addiu t1,t0,1
            0x0008_5100, // sll t2,t0,4
            0x0000_0000,
        ]);
        let mut cpu = Cpu::new(0);
        run(&mut cpu, &mut memory, 4);
        assert_eq!(cpu.gpr[8], 0x1234_5678);
        assert_eq!(cpu.gpr[9], 0x1234_5679);
        assert_eq!(cpu.gpr[10], 0x2345_6780);
    }

    #[test]
    fn multiply_and_divide() {
        // addiu t0,zero,7 ; addiu t1,zero,3 ; mult t0,t1 ; mflo t2 ; mfhi t3
        let mut memory = program(&[
            0x2408_0007, // addiu t0,zero,7
            0x2409_0003, // addiu t1,zero,3
            0x0109_0018, // mult t0,t1
            0x0000_5012, // mflo t2
            0x0000_5810, // mfhi t3
            0x0000_0000,
        ]);
        let mut cpu = Cpu::new(0);
        run(&mut cpu, &mut memory, 5);
        assert_eq!(cpu.gpr[10], 21);
        assert_eq!(cpu.gpr[11], 0);
    }

    #[test]
    fn loads_and_stores() {
        // addiu t0,zero,0x100 ; addiu t1,zero,-1 ; sw t1,0(t0) ; lw t2,0(t0) ; lbu t3,0(t0)
        let mut memory = program(&[
            0x2408_0100, // addiu t0,zero,0x100
            0x2409_FFFF, // addiu t1,zero,-1
            0xAD09_0000, // sw t1,0(t0)
            0x8D0A_0000, // lw t2,0(t0)
            0x910B_0000, // lbu t3,0(t0)
            0x0000_0000,
        ]);
        let mut cpu = Cpu::new(0);
        run(&mut cpu, &mut memory, 5);
        assert_eq!(cpu.gpr[9], 0xFFFF_FFFF_FFFF_FFFF);
        assert_eq!(cpu.gpr[10], 0xFFFF_FFFF_FFFF_FFFF);
        assert_eq!(cpu.gpr[11], 0xFF);
        assert_eq!(memory.read_u32(0x100), 0xFFFF_FFFF);
    }

    #[test]
    fn branch_delay_slot_and_loop() {
        // addiu t0,zero,3 ; loop: addiu t0,t0,-1 ; bne t0,zero,loop ; addiu t1,zero,9
        let mut memory = program(&[
            0x2408_0003, // addiu t0,zero,3
            0x2508_FFFF, // addiu t0,t0,-1
            0x1500_FFFE, // bne t0,zero,-2
            0x2409_0009, // addiu t1,zero,9 (delay slot)
            0x0000_0000,
        ]);
        let mut cpu = Cpu::new(0);
        run(&mut cpu, &mut memory, 3 + 6);
        assert_eq!(cpu.gpr[8], 0);
        assert_eq!(cpu.gpr[9], 9);
    }

    #[test]
    fn jal_and_jr() {
        // jal 0x20 ; addiu t0,zero,1 (delay) ; ... ; jr ra ; nop
        let mut memory = program(&[
            0x0C00_0008, // jal 0x20
            0x2408_0001, // addiu t0,zero,1 (delay slot)
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
            0x03E0_0008, // jr ra
            0x2409_0002, // addiu t1,zero,2 (delay slot)
        ]);
        let mut cpu = Cpu::new(0);
        cpu.step(&mut memory).expect("jal");
        cpu.step(&mut memory).expect("delay");
        assert_eq!(cpu.pc, 0x20);
        assert_eq!(cpu.gpr[8], 1);
        assert_eq!(cpu.gpr[31], 8);
        run(&mut cpu, &mut memory, 2);
        assert_eq!(cpu.pc, 8);
        assert_eq!(cpu.gpr[9], 2);
    }

    #[test]
    fn sixty_four_bit_ops() {
        // lui t0,0x1234 ; daddiu t1,t0,1 ; dsll32 t2,t1,32 ;
        // dsrl32 t3,t2,32 ; dadd t4,t2,t2
        let mut memory = program(&[
            0x3C08_1234, // lui t0,0x1234
            0x6509_0001, // daddiu t1,t0,1
            0x0009_503C, // dsll32 t2,t1,32
            0x000A_583E, // dsrl32 t3,t2,32
            0x014A_602C, // dadd t4,t2,t2
            0x0000_0000,
        ]);
        let mut cpu = Cpu::new(0);
        run(&mut cpu, &mut memory, 5);
        assert_eq!(cpu.gpr[9], 0x1234_0001);
        assert_eq!(cpu.gpr[10], 0x1234_0001_0000_0000);
        assert_eq!(cpu.gpr[11], 0x1234_0001);
        assert_eq!(cpu.gpr[12], 0x2468_0002_0000_0000);
    }

    #[test]
    fn unaligned_access_is_reported() {
        let mut memory = program(&[0x8D0A_0001]); // lw t2,1(t0)
        let mut cpu = Cpu::new(0);
        cpu.step(&mut memory).expect("exception delivered");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 4, "AdEL");
        assert_eq!(cpu.cp0[8], 1);
        assert_eq!(cpu.cp0[14], 0);
    }
    #[test]
    fn tlb_maps_virtual_pages_to_physical_memory() {
        let mut cpu = Cpu::new(0);
        // VA page 0x0000-0x1FFF -> PA 0x2000 (even) / 0x3000 (odd).
        cpu.tlb[0] = TlbEntry {
            entry_hi: 0,
            lo0: 2,            // even page: PFN 0 (code)
            lo1: (2 << 6) | 2, // odd page: PFN 2 (data)
            page_mask: 0,
        };
        let mut memory = Memory::new(0x4000);
        memory.write_u32(0x2000, 0xCAFE_BABE);
        // LW t0, 0x1000(zero) ; SW t0, 0x1000(zero) ; LW t1, 0x5000(zero)
        let image = program(&[0x8C08_1000, 0xAC08_1000]);
        memory.0[..image.0.len()].copy_from_slice(&image.0);
        run(&mut cpu, &mut memory, 2);
        assert_eq!(cpu.gpr[8], 0xFFFF_FFFF_CAFE_BABE);
        assert_eq!(memory.read_u32(0x2000), 0xCAFE_BABE);
        // A page outside the mapped entry now faults.
        let mut cpu2 = Cpu::new(0);
        cpu2.tlb[0] = cpu.tlb[0];
        let mut memory2 = program(&[0x8C09_5000]);
        cpu2.step(&mut memory2).expect("fault delivered");
        assert_eq!(cpu2.cp0[8], 0x5000);
        assert_eq!((cpu2.cp0[13] >> 2) & 0x1F, 2);
        assert_eq!(cpu2.pc, 0x8000_0000);
    }

    #[test]
    fn tlb_instructions_update_cp0_registers() {
        let mut cpu = Cpu::new(0);
        let mut memory = Memory::new(0x100);
        cpu.cp0[0] = 0; // Index
        cpu.cp0[10] = 0x2000; // EntryHi (VPN2 bit 13 set)
        cpu.cp0[2] = (5 << 6) | 2; // EntryLo0: PFN 5, valid, global
        cpu.cp0[3] = (6 << 6) | 2;
        cpu.cp0[5] = 0; // PageMask
                        // TLBWI
        cpu.execute(0x4200_0002, 0, &mut memory).expect("tlbwi");
        assert_eq!(cpu.tlb[0].entry_hi, 0x2000);
        assert_eq!(cpu.tlb[0].lo0, (5 << 6) | 2);
        // TLBP finds the entry (Index bit 31 clear).
        cpu.cp0[10] = 0x2000;
        cpu.execute(0x4200_0008, 0, &mut memory).expect("tlbp");
        assert_eq!(cpu.cp0[0], 0);
        // TLBR reloads the registers from the entry.
        cpu.cp0[10] = 0;
        cpu.cp0[2] = 0;
        cpu.execute(0x4200_0001, 0, &mut memory).expect("tlbr");
        assert_eq!(cpu.cp0[10], 0x2000);
        assert_eq!(cpu.cp0[2], (5 << 6) | 2);
        // A failed probe sets Index bit 31.
        cpu.cp0[10] = 0x8000;
        cpu.execute(0x4200_0008, 0, &mut memory).expect("tlbp miss");
        assert_ne!(cpu.cp0[0] & 0x8000_0000, 0);
    }

    #[test]
    fn fpu_arithmetic_and_conversions() {
        let mut memory = Memory::new(0x1000);
        let image = program(&[
            0xC400_0100, // lwc1 f0, 0x100(zero)
            0xC401_0104, // lwc1 f1, 0x104(zero)
            0x4601_0080, // add.s f2, f0, f1
            0x4408_1000, // mfc1 t0, f2
            0x4600_10E4, // cvt.w.s f3, f2
            0x4409_1800, // mfc1 t1, f3
            0x4600_1104, // sqrt.s f4, f2
            0x440A_2000, // mfc1 t2, f4
        ]);
        memory.0[..image.0.len()].copy_from_slice(&image.0);
        memory.write_u32(0x100, 1.5f32.to_bits());
        memory.write_u32(0x104, 2.25f32.to_bits());
        let mut cpu = Cpu::new(0);
        cpu.cp0[12] = 1 << 29; // Status.CU1 enables the FPU
        run(&mut cpu, &mut memory, 8);
        assert_eq!(cpu.gpr[8], u64::from(3.75f32.to_bits()));
        // Default round-to-nearest-even converts 3.75 to 4.
        assert_eq!(cpu.gpr[9], 4);
        assert_eq!(cpu.gpr[10], u64::from(3.75f32.sqrt().to_bits()));

        // cvt.s.w (int -> float) and cvt.s from 64-bit integer.
        cpu.fpr[6] = 7;
        cpu.cop1(0, 0x14, 0, 6, 7, 0x20).expect("cvt.s.w");
        assert_eq!(cpu.fpr[7] as u32, 7.0f32.to_bits());
        cpu.fpr[6] = (-9i64) as u64;
        cpu.cop1(0, 0x15, 0, 6, 7, 0x20).expect("cvt.s.l");
        assert_eq!(cpu.fpr[7] as u32, (-9.0f32).to_bits());
    }
    #[test]
    fn tlb_miss_refills_and_eret_returns() {
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.tlb[0] = TlbEntry {
            entry_hi: 0,
            lo0: 2,
            lo1: (1 << 6) | 2,
            page_mask: 0,
        };
        let mut memory = Memory::new(0x3000);
        // Refill vector 0x80000000 -> physical 0: EPC += 4 ; eret.
        for (index, word) in [0x401A_7000u32, 0x275A_0004, 0x409A_7000, 0x4200_0018]
            .iter()
            .enumerate()
        {
            memory.write_u32(index as u64 * 4, *word);
        }
        // Program at 0x80001000: faulting load then ADDIU t1,zero,7.
        memory.write_u32(0x1000, 0x8C08_5000); // lw t0,0x5000(zero)
        memory.write_u32(0x1004, 0x2409_0007); // addiu t1,zero,7

        cpu.step(&mut memory).expect("refill delivered");
        assert_eq!(cpu.pc, 0x8000_0000);
        assert_eq!(cpu.cp0[14], 0x8000_1000);
        assert_eq!(cpu.cp0[8], 0x5000);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 2, "TLBL");
        assert_ne!(cpu.cp0[12] & 2, 0, "EXL set");

        run(&mut cpu, &mut memory, 4);
        assert_eq!(cpu.pc, 0x8000_1004);
        assert_eq!(cpu.cp0[12] & 2, 0, "ERET clears EXL");
        cpu.step(&mut memory).expect("resume");
        assert_eq!(cpu.gpr[9], 7);

        // A store miss reports TLBS (code 3) and takes the refill vector.
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.tlb[0] = TlbEntry {
            entry_hi: 0,
            lo0: 2,
            lo1: (1 << 6) | 2,
            page_mask: 0,
        };
        memory.write_u32(0x1000, 0xAC00_5000); // sw zero,0x5000(zero)
        cpu.step(&mut memory).expect("store refill delivered");
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 3, "TLBS");
        assert_eq!(cpu.pc, 0x8000_0000);
    }

    #[test]
    fn tlb_invalid_page_takes_the_general_vector() {
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.tlb[0] = TlbEntry {
            entry_hi: 0,
            lo0: 2, // even page valid, odd page invalid
            lo1: 0,
            page_mask: 0,
        };
        let mut memory = Memory::new(0x3000);
        memory.write_u32(0x1000, 0x8C08_1000); // lw t0,0x1000(zero)
        cpu.step(&mut memory).expect("invalid TLB exception");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 2);
        assert_eq!(cpu.cp0[8], 0x1000);
        // With BEV set the boot vectors are used instead.
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[12] = 1 << 22;
        cpu.tlb[0] = TlbEntry {
            entry_hi: 0,
            lo0: 2,
            lo1: 0,
            page_mask: 0,
        };
        cpu.step(&mut memory).expect("BEV invalid TLB exception");
        assert_eq!(cpu.pc, 0xBFC0_0380);
    }
    #[test]
    fn non_tlb_exceptions_deliver_and_eret_resumes() {
        // Unaligned load -> AdEL.
        let mut cpu = Cpu::new(0x8000_1000);
        let mut memory = Memory::new(0x3000);
        install_handler(&mut memory, false);
        memory.write_u32(0x1000, 0x8C08_0001); // lw t0,1(zero)
        memory.write_u32(0x1004, 0x2409_0007); // addiu t1,zero,7
        cpu.step(&mut memory).expect("AdEL");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 4);
        assert_eq!(cpu.cp0[8], 1);
        assert_eq!(cpu.cp0[14], 0x8000_1000);
        run(&mut cpu, &mut memory, 4);
        assert_eq!(cpu.pc, 0x8000_1004);
        cpu.step(&mut memory).expect("resume");
        assert_eq!(cpu.gpr[9], 7);

        // Unaligned store -> AdES.
        let mut cpu = Cpu::new(0x8000_1000);
        memory.write_u32(0x1000, 0xAC00_0002); // sw zero,2(zero)
        cpu.step(&mut memory).expect("AdES");
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 5);
        assert_eq!(cpu.cp0[8], 2);

        // SYSCALL -> exception 8, BREAK -> 9.
        let mut cpu = Cpu::new(0x8000_1000);
        memory.write_u32(0x1000, 0x0000_000C);
        memory.write_u32(0x1004, 0x2409_0007);
        cpu.step(&mut memory).expect("syscall");
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 8);
        run(&mut cpu, &mut memory, 4);
        cpu.step(&mut memory).expect("resume after syscall");
        assert_eq!(cpu.gpr[9], 7);

        let mut cpu = Cpu::new(0x8000_1000);
        memory.write_u32(0x1000, 0x0000_48CD); // break 0x123
        cpu.step(&mut memory).expect("break");
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 9);
    }

    #[test]
    fn delay_slot_exception_sets_bd_and_branch_epc() {
        let mut cpu = Cpu::new(0x8000_1000);
        let mut memory = Memory::new(0x3000);
        memory.write_u32(0x1000, 0x1000_0002); // beq zero,zero,+2
        memory.write_u32(0x1004, 0x0000_000C); // syscall (delay slot)
        cpu.step(&mut memory).expect("branch");
        cpu.step(&mut memory).expect("delay slot exception");
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 8);
        assert_ne!(cpu.cp0[13] & (1 << 31), 0, "BD set");
        assert_eq!(cpu.cp0[14], 0x8000_1000, "EPC points at the branch");
        assert_eq!(cpu.pc, 0x8000_0180);
    }

    #[test]
    fn interrupt_delivery_and_eret() {
        let mut cpu = Cpu::new(0x8000_1000);
        let mut memory = Memory::new(0x3000);
        install_handler(&mut memory, true);
        memory.write_u32(0x1000, 0x0000_0000); // nop
        memory.write_u32(0x1004, 0x2409_0007); // addiu t1,zero,7
                                               // IE + IM2 enabled, IP2 pending.
        cpu.cp0[12] = 1 | (1 << 10);
        cpu.cp0[13] = 1 << 10;
        cpu.step(&mut memory).expect("interrupt");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!(cpu.cp0[14], 0x8000_1000);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 0, "interrupt code");
        assert_ne!(cpu.cp0[12] & 2, 0, "EXL set");
        // Handler: EPC += 4, clear Cause, ERET.
        run(&mut cpu, &mut memory, 5);
        assert_eq!(cpu.pc, 0x8000_1004);
        assert_eq!(cpu.cp0[12] & 2, 0);
        cpu.step(&mut memory).expect("resume after interrupt");
        assert_eq!(cpu.gpr[9], 7);
    }
    #[test]
    fn reserved_and_coprocessor_unusable_exceptions_resume() {
        // COP2 is reserved on the VR4300.
        let mut cpu = Cpu::new(0x8000_1000);
        let mut memory = Memory::new(0x3000);
        install_handler(&mut memory, false);
        memory.write_u32(0x1000, 0x4800_0000); // COP2 instruction
        memory.write_u32(0x1004, 0x2409_0007); // addiu t1,zero,7
        cpu.step(&mut memory).expect("reserved");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 10);
        assert_eq!(cpu.cp0[14], 0x8000_1000);
        run(&mut cpu, &mut memory, 4);
        cpu.step(&mut memory).expect("resume");
        assert_eq!(cpu.gpr[9], 7);

        // COP1 with Status.CU1 clear is unusable; Cause.CE identifies unit 1.
        let mut cpu = Cpu::new(0x8000_1000);
        memory.write_u32(0x1000, 0x4601_0080); // add.s f2,f0,f1
        memory.write_u32(0x1004, 0x2409_0007);
        cpu.step(&mut memory).expect("coprocessor unusable");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 11);
        assert_ne!(cpu.cp0[13] & (1 << 28), 0, "CE = 1");
        run(&mut cpu, &mut memory, 4);
        cpu.step(&mut memory).expect("resume");
        assert_eq!(cpu.gpr[9], 7);
    }

    #[test]
    fn fpu_flags_and_enabled_exception() {
        let mut memory = Memory::new(0x3000);
        install_handler(&mut memory, false);
        // 1.0 / 3.0 sets inexact (flag bit 2, cause bit 12).
        memory.write_u32(0x1000, 0x4601_0083); // div.s f2,f0,f1
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[12] = 1 << 29;
        cpu.fpr[0] = u64::from(1.0f32.to_bits());
        cpu.fpr[1] = u64::from(3.0f32.to_bits());
        cpu.step(&mut memory).expect("div");
        assert_eq!(cpu.fpr[2] as u32, (1.0f32 / 3.0).to_bits());
        assert_ne!(cpu.fcr31 & (1 << 2), 0, "inexact flag");
        assert_ne!(cpu.fcr31 & (1 << 12), 0, "inexact cause");

        // Division by zero sets Z; enabling it raises a precise FPU exception.
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[12] = 1 << 29;
        cpu.fpr[0] = u64::from(1.0f32.to_bits());
        cpu.fpr[1] = 0;
        cpu.step(&mut memory).expect("div zero");
        assert_ne!(cpu.fcr31 & (1 << 5), 0, "divide-by-zero flag");
        assert_ne!(cpu.fcr31 & (1 << 15), 0, "divide-by-zero cause");
        assert!(f32::from_bits(cpu.fpr[2] as u32).is_infinite());

        cpu.pc = 0x8000_1000;
        cpu.next_pc = 0x8000_1004;
        cpu.fcr31 = 1 << 10; // enable divide-by-zero
        cpu.step(&mut memory).expect("FPU exception delivered");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 15);
        assert_eq!(cpu.cp0[14], 0x8000_1000);

        // FS flushes a subnormal result to zero and reports underflow+inexact.
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[12] = 1 << 29;
        cpu.fcr31 = 1 << 24; // FS
        memory.write_u32(0x1000, 0x4601_0082); // mul.s f2,f0,f1
        cpu.fpr[0] = u64::from(f32::MIN_POSITIVE.to_bits());
        cpu.fpr[1] = u64::from(0.5f32.to_bits());
        cpu.step(&mut memory).expect("mul");
        assert_eq!(cpu.fpr[2] as u32, 0, "subnormal flushed to zero");
        assert_ne!(cpu.fcr31 & (1 << 3), 0, "underflow flag");
        assert_ne!(cpu.fcr31 & (1 << 2), 0, "inexact flag");
    }
    #[test]
    fn double_precision_arithmetic_and_conversions() {
        let mut memory = Memory::new(0x3000);
        let image = program(&[
            0xD400_0100, // ldc1 f0, 0x100(zero)
            0xD401_0108, // ldc1 f1, 0x108(zero)
            0x4621_0080, // add.d f2, f0, f1
            0x4620_10E0, // cvt.s.d f3, f2
            0x4408_1800, // mfc1 t0, f3
            0x4600_1921, // cvt.d.s f4, f3
            0x4429_2000, // dmfc1 t1, f4
            0x4621_0083, // div.d f2, f0, f1
            0x442A_1000, // dmfc1 t2, f2
            0x4620_1144, // sqrt.d f5, f2
            0x442B_2800, // dmfc1 t3, f5
            0x4620_11A4, // cvt.w.d f6, f2
            0x440C_3000, // mfc1 t4, f6
            0x4621_003C, // c.lt.d f0, f1
        ]);
        memory.0[..image.0.len()].copy_from_slice(&image.0);
        memory.write_u64(0x100, 1.5f64.to_bits());
        memory.write_u64(0x108, 2.25f64.to_bits());
        let mut cpu = Cpu::new(0);
        cpu.cp0[12] = 1 << 29;
        run(&mut cpu, &mut memory, 14);
        assert_eq!(cpu.gpr[8], u64::from(3.75f32.to_bits()));
        assert_eq!(cpu.gpr[9], 3.75f64.to_bits());
        let quotient = 1.5f64 / 2.25;
        assert_eq!(cpu.gpr[10], quotient.to_bits());
        assert_eq!(cpu.gpr[11], quotient.sqrt().to_bits());
        assert_eq!(cpu.gpr[12], 1);
        assert_ne!(cpu.fcr31 & (1 << 23), 0, "c.lt.d condition");

        // CVT.D.W / CVT.D.L and enabled D-format exceptions through ERET.
        cpu.fpr[7] = 7;
        cpu.cop1(0, 0x14, 0, 7, 8, 0x21).expect("cvt.d.w");
        assert_eq!(cpu.fpr[8], 7.0f64.to_bits());
        cpu.fpr[7] = (-9i64) as u64;
        cpu.cop1(0, 0x15, 0, 7, 8, 0x21).expect("cvt.d.l");
        assert_eq!(cpu.fpr[8], (-9.0f64).to_bits());

        let mut cpu = Cpu::new(0x8000_1000);
        let mut memory = Memory::new(0x3000);
        install_handler(&mut memory, false);
        memory.write_u32(0x1000, 0x4621_0083); // div.d f2, f0, f1
        memory.write_u32(0x1004, 0x2409_0007); // addiu t1,zero,7
        cpu.cp0[12] = 1 << 29;
        cpu.fcr31 = 1 << 10; // enable divide-by-zero
        cpu.fpr[0] = 1.0f64.to_bits();
        cpu.fpr[1] = 0.0f64.to_bits();
        cpu.step(&mut memory).expect("FPU D exception delivered");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 15);
        run(&mut cpu, &mut memory, 4);
        cpu.step(&mut memory).expect("resume after FPU exception");
        assert_eq!(cpu.gpr[9], 7);
    }
    #[test]
    fn integer_overflow_delivers_and_resumes() {
        let mut cpu = Cpu::new(0x8000_1000);
        let mut memory = Memory::new(0x3000);
        install_handler(&mut memory, false);
        memory.write_u32(0x1000, 0x3C08_7FFF); // lui t0,0x7FFF
        memory.write_u32(0x1004, 0x3508_FFFF); // ori t0,t0,0xFFFF
        memory.write_u32(0x1008, 0x2409_0001); // addiu t1,zero,1
        memory.write_u32(0x100C, 0x0109_5020); // add t2,t0,t1
        memory.write_u32(0x1010, 0x240B_0007); // addiu t3,zero,7
        run(&mut cpu, &mut memory, 3);
        assert_eq!(cpu.gpr[8], 0x7FFF_FFFF);
        assert_eq!(cpu.gpr[9], 1);
        cpu.step(&mut memory).expect("ADD overflow delivered");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 12, "Ov");
        assert_eq!(cpu.cp0[14], 0x8000_100C);
        assert_eq!(cpu.gpr[10], 0, "destination unchanged on overflow");
        run(&mut cpu, &mut memory, 4);
        assert_eq!(cpu.pc, 0x8000_1010);
        cpu.step(&mut memory).expect("resume");
        assert_eq!(cpu.gpr[11], 7);

        // ADDI and SUB also trap.
        let mut cpu = Cpu::new(0x8000_1000);
        memory.write_u32(0x1000, 0x2109_0001); // addi t1,t0,1 with t0=0x7FFFFFFF
        cpu.gpr[8] = 0x7FFF_FFFF;
        cpu.step(&mut memory).expect("ADDI overflow");
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 12);
        assert_eq!(cpu.cp0[14], 0x8000_1000);

        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[12] = 1; // IE does not matter here, but keep a clean status
        cpu.cp0[12] = 0;
        cpu.gpr[8] = 0x8000_0000;
        cpu.gpr[9] = 1;
        memory.write_u32(0x1000, 0x0109_5022); // sub t2,t0,t1
        cpu.step(&mut memory).expect("SUB overflow");
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 12);
        assert_eq!(cpu.cp0[14], 0x8000_1000);
    }

    #[test]
    fn erl_error_exception_entry_and_eret() {
        let mut cpu = Cpu::new(0x8000_1000);
        let mut memory = Memory::new(0x3000);
        memory.write_u32(0x180, 0x4200_0018); // eret
                                              // NMI/cache-error style entry: ErrorEPC saved, ERL set.
        cpu.raise_error_exception(0x8000_0180);
        assert_eq!(cpu.cp0[30], 0x8000_1000);
        assert_ne!(cpu.cp0[12] & 4, 0, "ERL set");
        assert_eq!(cpu.pc, 0x8000_0180);
        cpu.step(&mut memory).expect("eret");
        assert_eq!(cpu.pc, 0x8000_1000);
        assert_eq!(cpu.cp0[12] & 4, 0, "ERET clears ERL");

        // With ERL set a regular exception uses the general vector and leaves
        // EPC untouched (return goes through ErrorEPC).
        cpu.cp0[12] = 4;
        cpu.cp0[14] = 0x1234;
        memory.write_u32(0x1000, 0x8C08_0001); // lw t0,1(zero) -> AdEL
        cpu.step(&mut memory).expect("AdEL with ERL");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 4);
        assert_eq!(cpu.cp0[14], 0x1234, "EPC not overwritten at error level");
    }
    #[test]
    fn fpu_unimplemented_and_precise_inexact() {
        // An unimplemented COP1 operation sets the unimplemented-operation
        // flag/cause and raises a precise FPU exception that ERET resumes.
        let mut memory = Memory::new(0x3000);
        install_handler(&mut memory, false);
        memory.write_u32(0x1000, 0x4600_0011); // movf.s f0,f0 (unimplemented here)
        memory.write_u32(0x1004, 0x2409_0007); // addiu t1,zero,7
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[12] = 1 << 29;
        cpu.step(&mut memory).expect("unimplemented FP exception");
        assert_eq!(cpu.pc, 0x8000_0180);
        assert_eq!((cpu.cp0[13] >> 2) & 0x1F, 15);
        assert_eq!(cpu.cp0[14], 0x8000_1000);
        assert_ne!(cpu.fcr31 & (1 << 6), 0, "unimplemented flag");
        assert_ne!(cpu.fcr31 & (1 << 17), 0, "unimplemented cause");
        run(&mut cpu, &mut memory, 4);
        cpu.step(&mut memory).expect("resume");
        assert_eq!(cpu.gpr[9], 7);

        // D-format inexact detection: 0.1 + 0.2 is inexact, 1.5 + 2.25 is exact.
        memory.write_u32(0x1000, 0x4621_0080); // add.d f2,f0,f1
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[12] = 1 << 29;
        cpu.fpr[0] = 0.1f64.to_bits();
        cpu.fpr[1] = 0.2f64.to_bits();
        cpu.fcr31 = 0;
        cpu.step(&mut memory).expect("add.d");
        assert_ne!(cpu.fcr31 & (1 << 2), 0, "inexact flag");
        assert_ne!(cpu.fcr31 & (1 << 12), 0, "inexact cause");
        assert_eq!(cpu.fpr[2], (0.1f64 + 0.2f64).to_bits());

        cpu.pc = 0x8000_1000;
        cpu.next_pc = 0x8000_1004;
        cpu.fcr31 = 0;
        cpu.fpr[0] = 1.5f64.to_bits();
        cpu.fpr[1] = 2.25f64.to_bits();
        cpu.step(&mut memory).expect("exact add.d");
        assert_eq!(cpu.fcr31 & (1 << 2), 0, "exact sum has no inexact flag");
        assert_eq!(cpu.fcr31 & (1 << 12), 0);
    }
    #[test]
    fn error_level_sources_and_eret() {
        let mut memory = Memory::new(0x3000);
        memory.write_u32(0x180, 0x4200_0018); // eret

        // Watchdog: ErrorEPC saved, ERL set, boot vector; ERET returns.
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.take_error(ErrorSource::Watchdog);
        assert_eq!(cpu.pc, 0xBFC0_0000);
        assert_eq!(cpu.cp0[30], 0x8000_1000);
        assert_ne!(cpu.cp0[12] & 4, 0, "ERL set");
        cpu.pc = 0x8000_0180;
        cpu.next_pc = 0x8000_0184;
        cpu.step(&mut memory).expect("eret");
        assert_eq!(cpu.pc, 0x8000_1000);
        assert_eq!(cpu.cp0[12] & 4, 0, "ERET clears ERL");

        // Cache error uses the 0xA0000000 vector and saves ErrorEPC.
        let mut cpu = Cpu::new(0x8000_2000);
        cpu.take_error(ErrorSource::CacheError);
        assert_eq!(cpu.pc, 0xA000_0000);
        assert_eq!(cpu.cp0[30], 0x8000_2000);
        assert_ne!(cpu.cp0[12] & 4, 0);

        // NMI behaves like a watchdog error.
        let mut cpu = Cpu::new(0x8000_3000);
        cpu.take_error(ErrorSource::Nmi);
        assert_eq!(cpu.pc, 0xBFC0_0000);
        assert_eq!(cpu.cp0[30], 0x8000_3000);

        // Reset vectors without overwriting ErrorEPC.
        let mut cpu = Cpu::new(0x8000_4000);
        cpu.cp0[30] = 0x1234;
        cpu.take_error(ErrorSource::Reset);
        assert_eq!(cpu.pc, 0xBFC0_0000);
        assert_eq!(cpu.cp0[30], 0x1234);
        assert_ne!(cpu.cp0[12] & 4, 0);
    }
    #[test]
    fn watchdog_count_compare_enters_and_eret_returns() {
        let mut memory = Memory::new(0x3000);
        memory.write_u32(0x180, 0x4200_0018); // eret
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[9] = 0xFFFF_FFFE; // Count
        cpu.cp0[11] = 0xFFFF_FFFF; // Compare
        cpu.step(&mut memory).expect("watchdog");
        assert_eq!(cpu.cp0[9], 0xFFFF_FFFF);
        assert_eq!(cpu.pc, 0xBFC0_0000);
        assert_eq!(cpu.cp0[30], 0x8000_1004);
        assert_ne!(cpu.cp0[12] & 4, 0);

        cpu.pc = 0x8000_0180;
        cpu.next_pc = 0x8000_0184;
        cpu.step(&mut memory).expect("eret");
        assert_eq!(cpu.pc, 0x8000_1004);
        assert_eq!(cpu.cp0[12] & 4, 0);
        // Count wrapped past Compare, so the match does not re-fire.
        cpu.step(&mut memory).expect("resume");
        assert_eq!(cpu.cp0[9], 2);

        // Compare reached mid-run triggers exactly once.
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.cp0[9] = 0;
        cpu.cp0[11] = 3;
        run(&mut cpu, &mut memory, 2);
        assert_eq!(cpu.cp0[9], 2);
        assert_eq!(cpu.cp0[12] & 4, 0, "no error before the match");
        cpu.step(&mut memory).expect("watchdog at 3");
        assert_eq!(cpu.pc, 0xBFC0_0000);
        assert_eq!(cpu.cp0[30], 0x8000_100C);
    }

    #[test]
    fn hardware_nmi_and_cache_error_lines() {
        let mut memory = Memory::new(0x3000);
        memory.write_u32(0x180, 0x4200_0018);
        let mut cpu = Cpu::new(0x8000_2000);
        cpu.signal_nmi();
        cpu.step(&mut memory).expect("NMI");
        assert_eq!(cpu.pc, 0xBFC0_0000);
        assert_eq!(cpu.cp0[30], 0x8000_2000);
        assert_ne!(cpu.cp0[12] & 4, 0);
        cpu.pc = 0x8000_0180;
        cpu.next_pc = 0x8000_0184;
        cpu.step(&mut memory).expect("eret");
        assert_eq!(cpu.pc, 0x8000_2000);
        assert_eq!(cpu.cp0[12] & 4, 0);

        cpu.signal_cache_error();
        cpu.step(&mut memory).expect("cache error");
        assert_eq!(cpu.pc, 0xA000_0000);
        assert_eq!(cpu.cp0[30], 0x8000_2000);
        assert_ne!(cpu.cp0[12] & 4, 0);
    }
    #[test]
    fn count_advances_by_instruction_cycle_cost() {
        let mut memory = program(&[
            0x2408_0001, // addiu t0,zero,1 (1 cycle)
            0x8C09_0000, // lw t1,0(zero)   (2 cycles)
            0x0000_001A, // div zero,zero   (37 cycles)
        ]);
        let mut cpu = Cpu::new(0);
        cpu.step(&mut memory).expect("addiu");
        assert_eq!(cpu.cp0[9], 1);
        cpu.step(&mut memory).expect("lw");
        assert_eq!(cpu.cp0[9], 3);
        cpu.step(&mut memory).expect("div");
        assert_eq!(cpu.cp0[9], 40);

        // Compare in the middle of a multi-cycle instruction fires during the
        // cycle advance.
        let mut cpu = Cpu::new(0);
        cpu.pc = 8;
        cpu.next_pc = 12;
        cpu.cp0[9] = 0;
        cpu.cp0[11] = 5;
        cpu.step(&mut memory).expect("div -> watchdog");
        assert_eq!(cpu.cp0[9], 5);
        assert_eq!(cpu.pc, 0xBFC0_0000);
        assert_eq!(cpu.cp0[30], 0x000C);
    }
    #[test]
    fn cache_parity_error_triggers_entry_and_eret() {
        let mut memory = Memory::new(0x3000);
        memory.write_u32(0x180, 0x4200_0018); // eret

        // Instruction-fetch parity error on the code line.
        let mut cpu = Cpu::new(0x8000_1000);
        cpu.inject_cache_parity_error(0x1000);
        cpu.step(&mut memory).expect("cache error");
        assert_eq!(cpu.pc, 0xA000_0000);
        assert_eq!(cpu.cp0[30], 0x8000_1004);
        assert_ne!(cpu.cp0[12] & 4, 0, "ERL set");
        cpu.pc = 0x8000_0180;
        cpu.next_pc = 0x8000_0184;
        cpu.step(&mut memory).expect("eret");
        assert_eq!(cpu.pc, 0x8000_1004);
        assert_eq!(cpu.cp0[12] & 4, 0, "ERET clears ERL");

        // Data-load parity error: the fetch succeeds, the load raises it.
        let mut cpu = Cpu::new(0);
        let mut memory = program(&[0x8C08_0100]); // lw t0,0x100(zero)
        cpu.inject_cache_parity_error(0x100);
        cpu.step(&mut memory).expect("data cache error");
        assert_eq!(cpu.pc, 0xA000_0000);
        assert_eq!(cpu.cp0[30], 4);
        assert_ne!(cpu.cp0[12] & 4, 0);

        // A clean access refills the line and clears the parity state.
        let mut cpu = Cpu::new(0);
        cpu.inject_cache_parity_error(0x100);
        cpu.clear_cache();
        cpu.step(&mut memory).expect("clean load");
        assert_eq!(cpu.cp0[30], 0);
        assert_eq!(cpu.cp0[12] & 4, 0);
    }
    #[test]
    fn cache_stores_data_with_lru_replacement_and_ecc() {
        let mut cpu = Cpu::new(0);
        let mut memory = Memory::new(0x10_000);
        memory.write_u8(0x100, 0xAA);
        assert_eq!(cpu.cache_read_u8(0x100, &mut memory), Ok(0xAA));

        // Write-through updates both the cache line and the bus.
        cpu.cache_write_u8(0x100, 0x55, &mut memory).unwrap();
        assert_eq!(memory.read_u8(0x100), 0x55);
        assert_eq!(cpu.cache_read_u8(0x100, &mut memory), Ok(0x55));

        // A hit serves the cached line even if memory changes behind it.
        memory.write_u8(0x101, 0x77);
        assert_eq!(cpu.cache_read_u8(0x101, &mut memory), Ok(0x00));

        // Fill the other way of set 16 (line 528), touch line 16 again, then
        // fill a third tag: the least-recently-used way is replaced.
        assert_eq!(cpu.cache_read_u8(0x2100, &mut memory), Ok(0x00));
        assert_eq!(cpu.cache_read_u8(0x100, &mut memory), Ok(0x55));
        assert_eq!(cpu.cache_read_u8(0x4100, &mut memory), Ok(0x00));
        // Line 16 survived the replacement.
        memory.write_u8(0x103, 0x44);
        assert_eq!(cpu.cache_read_u8(0x103, &mut memory), Ok(0x00));
        // Line 528 was evicted, so the next access refills from memory.
        memory.write_u8(0x2100, 0xCC);
        assert_eq!(cpu.cache_read_u8(0x2100, &mut memory), Ok(0xCC));

        // Injected ECC/parity faults reject hits on that line.
        cpu.clear_cache();
        memory.write_u8(0x200, 0xBB);
        cpu.inject_cache_parity_error(0x200);
        assert!(matches!(
            cpu.cache_read_u8(0x200, &mut memory),
            Err(CpuError::CacheError { .. })
        ));
    }
}
