//! VR4300 (MIPS III) integer interpreter used by the N64 system.
//!
//! This is the CPU foundation of the ares-derived N64 core: a big-endian,
//! 64-bit MIPS III interpreter with architectural delay slots. It implements
//! the integer instruction set needed by boot code (arithmetic, shifts,
//! logical ops, branches with delay slots, jumps, loads/stores, basic COP0
//! moves and TLB management), the TLB-mapped address translation path and a
//! single-precision COP1 FPU (arithmetic, conversions, compares and LWC1/SWC1).
//! Precise exceptions and the FPU pipeline are later milestones.

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
    /// A TLB miss or invalid mapping.
    Tlb {
        /// Virtual address that failed translation.
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
            Self::Tlb { address } => write!(f, "TLB fault at {address:016X}"),
            Self::Trap { pc, code } => write!(f, "trap {code} at {pc:016X}"),
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
    /// 32-entry TLB.
    pub tlb: [TlbEntry; 32],
    /// COP1 floating-point registers (32-bit floats in the low word).
    pub fpr: [u64; 32],
    /// COP1 control/status register (`fcr31`).
    pub fcr31: u32,
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
            tlb: [TlbEntry::default(); 32],
            fpr: [0; 32],
            fcr31: 0,
        }
    }

    /// Executes one instruction.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> Result<(), CpuError> {
        let pc = self.pc;
        self.pc = self.next_pc;
        self.next_pc = self.pc.wrapping_add(4);

        let instruction = bus.read_u32(self.translate(pc)?);
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
            17 => self.cop1(pc, rs, rt, rd, sa, funct),
            49 => self.load_f32(bus, rs, rt, simm),
            53 => self.load_f64(bus, rs, rt, simm),
            57 => self.store_f32(bus, rs, rt, simm),
            61 => self.store_f64(bus, rs, rt, simm),
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
            0x10 => match funct {
                0x18 => {
                    // ERET: return to EPC.
                    self.pc = self.cp0[14];
                    self.next_pc = self.pc.wrapping_add(4);
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
    fn translate(&self, address: u64) -> Result<u64, CpuError> {
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
                return Err(CpuError::Tlb { address });
            }
            let pfn = u64::from((lo >> 6) & 0x0F_FFFF);
            return Ok((pfn << 12) | (address & 0x0FFF));
        }
        Err(CpuError::Tlb { address })
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
                let condition = self.fcr31 & (1 << 23) != 0;
                let expected = ft & 1 != 0;
                if condition == expected {
                    self.next_pc = pc.wrapping_add(4).wrapping_add(4);
                }
                Ok(())
            }
            0x10 => self.fpu_single(fs, ft, fd, funct, pc),
            0x14 => {
                if funct == 0x20 {
                    let value = self.fpr[fs as usize] as u32 as i32;
                    self.fpr[fd as usize] = u64::from((value as f32).to_bits());
                    Ok(())
                } else {
                    Err(CpuError::Unimplemented { pc, instruction: 0 })
                }
            }
            0x15 => {
                if funct == 0x20 {
                    let value = self.fpr[fs as usize] as i64;
                    self.fpr[fd as usize] = u64::from((value as f32).to_bits());
                    Ok(())
                } else {
                    Err(CpuError::Unimplemented { pc, instruction: 0 })
                }
            }
            _ => Err(CpuError::Unimplemented { pc, instruction: 0 }),
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
        let result = match funct {
            0x00 => a + b,
            0x01 => a - b,
            0x02 => a * b,
            0x03 => a / b,
            0x04 => a.sqrt(),
            0x05 => a.abs(),
            0x06 => a,
            0x07 => -a,
            0x24 => {
                let value = round_to_i32(a, self.fcr31 & 3);
                self.fpr[fd as usize] = (value as i64) as u64;
                return Ok(());
            }
            0x25 => {
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
                    0x39 | 0x3F => unordered || a <= b,
                    0x3B => unordered || a <= b,
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
            _ => {
                return Err(CpuError::Unimplemented {
                    pc,
                    instruction: funct,
                })
            }
        };
        self.fpr[fd as usize] = u64::from(result.to_bits());
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
        let value = bus.read_u8(self.translate(address)?);
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
        let value = bus.read_u16(self.translate(address)?);
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
        let value = bus.read_u32(self.translate(address)?);
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
        let value = bus.read_u64(self.translate(address)?);
        self.set_gpr(rt, value);
        Ok(())
    }

    fn store8<B: Bus>(&mut self, bus: &mut B, rs: u32, rt: u32, simm: i64) -> Result<(), CpuError> {
        let address = self.address(rs, simm);
        bus.write_u8(self.translate(address)?, self.gpr(rt) as u8);
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
        bus.write_u16(self.translate(address)?, self.gpr(rt) as u16);
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
        bus.write_u32(self.translate(address)?, self.gpr(rt) as u32);
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
        bus.write_u64(self.translate(address)?, self.gpr(rt));
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
            return Err(CpuError::Unaligned { address });
        }
        let value = bus.read_u32(self.translate(address)?);
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
            return Err(CpuError::Unaligned { address });
        }
        bus.write_u32(self.translate(address)?, self.fpr[rt as usize] as u32);
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
            return Err(CpuError::Unaligned { address });
        }
        let value = bus.read_u64(self.translate(address)?);
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
            return Err(CpuError::Unaligned { address });
        }
        bus.write_u64(self.translate(address)?, self.fpr[rt as usize]);
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
        assert!(matches!(
            cpu2.step(&mut memory2),
            Err(CpuError::Tlb { address: 0x5000 })
        ));
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
        run(&mut cpu, &mut memory, 8);
        eprintln!(
            "fpr={:08X?} gpr8={:X}",
            cpu.fpr.map(|v| v as u32),
            cpu.gpr[8]
        );
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
}
