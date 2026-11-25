//! VR4300 (MIPS III) integer interpreter used by the N64 system.
//!
//! This is the CPU foundation of the ares-derived N64 core: a big-endian,
//! 64-bit MIPS III interpreter with architectural delay slots. It implements
//! the integer instruction set needed by boot code (arithmetic, shifts,
//! logical ops, branches with delay slots, jumps, loads/stores and the basic
//! COP0 moves). Floating point, the TLB and precise exceptions are later
//! milestones.

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
    },
    /// `SYSCALL` or `BREAK` executed.
    Trap {
        /// Program counter of the instruction.
        pc: u64,
        /// Breakpoint code (`Break`) or zero (`Syscall`).
        code: u32,
    },
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unimplemented { pc, instruction } => {
                write!(f, "unimplemented opcode {instruction:08X} at {pc:016X}")
            }
            Self::Unaligned { address } => write!(f, "unaligned access at {address:016X}"),
            Self::Trap { pc, code } => write!(f, "trap {code} at {pc:016X}"),
        }
    }
}

impl std::error::Error for CpuError {}

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
    /// COP0 register file (Status, Cause, EPC, Count, ...).
    pub cp0: [u64; 32],
}

impl Cpu {
    /// Creates a CPU starting at `pc` (with an empty delay slot).
    #[must_use]
    pub fn new(pc: u64) -> Self {
        Self {
            gpr: [0; 32],
            hi: 0,
            lo: 0,
            pc,
            next_pc: pc.wrapping_add(4),
            instructions: 0,
            cp0: [0; 32],
        }
    }

    /// Executes one instruction.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<(), CpuError> {
        let pc = self.pc;
        self.pc = self.next_pc;
        self.next_pc = self.pc.wrapping_add(4);

        let instruction = bus.read_u32(physical(pc));
        self.instructions += 1;
        self.execute(instruction, pc, bus)
    }

    fn gpr(&self, index: u32) -> u64 {
        self.gpr[index as usize]
    }

    fn set_gpr(&mut self, index: u32, value: u64) {
        if index != 0 {
            self.gpr[index as usize] = value;
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
                let target = (pc.wrapping_add(4) & !0x0FFF_FFFF)
                    | (u64::from(instruction & 0x03FF_FFFF) << 2);
                if op == 3 {
                    self.set_gpr(31, pc.wrapping_add(8));
                }
                self.next_pc = target;
                Ok(())
            }
            4 => {
                if self.gpr(rs) == self.gpr(rt) {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            5 => {
                if self.gpr(rs) != self.gpr(rt) {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            6 => {
                if (self.gpr(rs) as i64) <= 0 {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            7 => {
                if (self.gpr(rs) as i64) > 0 {
                    self.next_pc = pc.wrapping_add(4).wrapping_add((simm as u64) << 2);
                }
                Ok(())
            }
            8 => {
                let value = self.gpr(rs).wrapping_add(simm as u64);
                self.set_gpr(rt, sign_extend32(value));
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
            32 => self.load8(bus, rs, rt, simm, true),
            33 => self.load16(bus, rs, rt, simm, true),
            35 => self.load32(bus, rs, rt, simm, true),
            36 => self.load8(bus, rs, rt, simm, false),
            37 => self.load16(bus, rs, rt, simm, false),
            39 => self.load32(bus, rs, rt, simm, false),
            40 => self.store8(bus, rs, rt, simm),
            41 => self.store16(bus, rs, rt, simm),
            43 => self.store32(bus, rs, rt, simm),
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
                self.next_pc = a;
                Ok(())
            }
            0x09 => {
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
            0x0C => Err(CpuError::Trap { pc, code: 0 }),
            0x0D => Err(CpuError::Trap {
                pc,
                code: instruction >> 6 & 0xFFFFF,
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
            0x20 | 0x21 => {
                self.set_gpr(rd, sign_extend32(a.wrapping_add(b)));
                Ok(())
            }
            0x22 | 0x23 => {
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
                self.cp0[rd as usize] = self.gpr(rt);
                Ok(())
            }
            0x10 if funct == 0x18 => {
                // ERET: return to EPC.
                self.pc = self.cp0[14];
                self.next_pc = self.pc.wrapping_add(4);
                Ok(())
            }
            _ => Err(CpuError::Unimplemented { pc, instruction }),
        }
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
        let value = bus.read_u8(physical(address));
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
            return Err(CpuError::Unaligned { address });
        }
        let value = bus.read_u16(physical(address));
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
            return Err(CpuError::Unaligned { address });
        }
        let value = bus.read_u32(physical(address));
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
            return Err(CpuError::Unaligned { address });
        }
        let value = bus.read_u64(physical(address));
        self.set_gpr(rt, value);
        Ok(())
    }

    fn store8<B: Bus>(&mut self, bus: &mut B, rs: u32, rt: u32, simm: i64) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        bus.write_u8(physical(address), self.gpr(rt) as u8);
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
            return Err(CpuError::Unaligned { address });
        }
        bus.write_u16(physical(address), self.gpr(rt) as u16);
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
            return Err(CpuError::Unaligned { address });
        }
        bus.write_u32(physical(address), self.gpr(rt) as u32);
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
            return Err(CpuError::Unaligned { address });
        }
        bus.write_u64(physical(address), self.gpr(rt));
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
        let error = cpu.step(&mut memory).expect_err("unaligned");
        assert!(matches!(error, CpuError::Unaligned { .. }));
    }
}
