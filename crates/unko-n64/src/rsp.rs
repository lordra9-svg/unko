//! RSP scaffold: DMEM/IMEM, the SP register block, DMA and a small scalar
//! interpreter.
//!
//! Vector instructions and the full microcode surface are later milestones; the
//! scaffold executes scalar MIPS32 code so boot/command sequences can be
//! exercised and DMA can be validated.

/// RSP data memory size.
pub const DMEM_SIZE: usize = 0x1000;
/// RSP instruction memory size.
pub const IMEM_SIZE: usize = 0x1000;

/// One RSP.
#[derive(Debug, Clone)]
pub struct Rsp {
    /// Scalar registers.
    pub gpr: [u32; 32],
    /// Program counter (12-bit).
    pub pc: u32,
    /// Data memory.
    pub dmem: Vec<u8>,
    /// Instruction memory.
    pub imem: Vec<u8>,
    /// SP_STATUS value.
    pub status: u32,
    /// Halted.
    pub halted: bool,
    /// Hit a BREAK.
    pub broke: bool,
    /// SP_MEM_ADDR.
    pub mem_addr: u32,
    /// SP_DRAM_ADDR.
    pub dram_addr: u32,
    /// SP_SEMAPHORE.
    pub semaphore: u32,
}

impl Default for Rsp {
    fn default() -> Self {
        Self::new()
    }
}

impl Rsp {
    /// Creates an idle RSP.
    #[must_use]
    pub fn new() -> Self {
        Self {
            gpr: [0; 32],
            pc: 0,
            dmem: vec![0; DMEM_SIZE],
            imem: vec![0; IMEM_SIZE],
            status: 1, // halt
            halted: true,
            broke: false,
            mem_addr: 0,
            dram_addr: 0,
            semaphore: 0,
        }
    }

    /// Reads an RSP register (`0x0404_0000 + offset`).
    #[must_use]
    pub fn read_register(&self, offset: u32) -> u32 {
        match offset & 0x3F {
            0x00 => self.mem_addr,
            0x04 => self.dram_addr,
            0x08 | 0x0C => 0,
            0x10 => self.status,
            0x14 | 0x18 => 0, // DMA idle
            0x1C => self.semaphore,
            0x20 => self.pc,
            _ => 0,
        }
    }

    /// Writes an RSP register, triggering DMA on the length registers.
    pub fn write_register(&mut self, offset: u32, value: u32, rdram: &mut [u8]) {
        match offset & 0x3F {
            0x00 => self.mem_addr = value,
            0x04 => self.dram_addr = value,
            0x08 => self.dma(rdram, value, false),
            0x0C => self.dma(rdram, value, true),
            0x10 => {
                self.halted = value & 1 != 0;
                if value & 2 != 0 {
                    self.broke = false;
                }
                self.status =
                    (self.status & !0x3) | (u32::from(self.halted)) | (u32::from(self.broke) << 1);
            }
            0x1C => self.semaphore = value,
            0x20 => self.pc = value & 0xFFC,
            _ => {}
        }
    }

    fn dma(&mut self, rdram: &mut [u8], value: u32, write: bool) {
        let length = ((value & 0x00FF_FFFF) | 7) as usize + 1;
        let mem = (self.mem_addr & 0x1FFF) as usize;
        let dram = self.dram_addr as usize;
        for offset in 0..length {
            let memory_address = mem + offset;
            let dram_address = dram + offset;
            if write {
                let byte = if memory_address < DMEM_SIZE {
                    self.dmem[memory_address]
                } else if memory_address < DMEM_SIZE + IMEM_SIZE {
                    self.imem[memory_address - DMEM_SIZE]
                } else {
                    0
                };
                if let Some(slot) = rdram.get_mut(dram_address) {
                    *slot = byte;
                }
            } else {
                let byte = rdram.get(dram_address).copied().unwrap_or(0);
                if memory_address < DMEM_SIZE {
                    self.dmem[memory_address] = byte;
                } else if memory_address < DMEM_SIZE + IMEM_SIZE {
                    self.imem[memory_address - DMEM_SIZE] = byte;
                }
            }
        }
    }

    /// Halts the RSP with a break.
    pub fn halt(&mut self) {
        self.halted = true;
        self.broke = true;
        self.status |= 0x3;
    }

    /// Executes up to `max` scalar instructions.
    pub fn run(&mut self, max: u32) {
        let mut next_pc = self.pc.wrapping_add(4);
        for _ in 0..max {
            if self.halted || self.broke {
                break;
            }
            let index = (self.pc as usize) & 0xFFC;
            let instruction = u32::from_be_bytes([
                self.imem[index],
                self.imem[index + 1],
                self.imem[index + 2],
                self.imem[index + 3],
            ]);
            self.pc = next_pc;
            next_pc = self.pc.wrapping_add(4);
            let op = instruction >> 26;
            let rs = (instruction >> 21) & 31;
            let rt = (instruction >> 16) & 31;
            let rd = (instruction >> 11) & 31;
            let sa = (instruction >> 6) & 31;
            let funct = instruction & 63;
            let simm = instruction as u16 as i16 as i32;
            let value = self.gpr[rs as usize];
            let other = self.gpr[rt as usize];
            match op {
                0 => match funct {
                    0x00 => self.set(rd, other << sa),
                    0x02 => self.set(rd, other >> sa),
                    0x03 => self.set(rd, ((other as i32) >> sa) as u32),
                    0x08 => next_pc = value,
                    0x0D => {
                        self.halt();
                        return;
                    }
                    0x20 | 0x21 => self.set(rd, value.wrapping_add(other)),
                    0x22 | 0x23 => self.set(rd, value.wrapping_sub(other)),
                    0x24 => self.set(rd, value & other),
                    0x25 => self.set(rd, value | other),
                    0x26 => self.set(rd, value ^ other),
                    0x2A => self.set(rd, u32::from((value as i32) < (other as i32))),
                    0x2B => self.set(rd, u32::from(value < other)),
                    _ => {
                        self.halt();
                        return;
                    }
                },
                2 => next_pc = (next_pc & 0xF000_0000) | ((instruction & 0x03FF_FFFF) << 2),
                4 => {
                    if value == other {
                        next_pc = next_pc.wrapping_add((simm as u32) << 2);
                    }
                }
                5 => {
                    if value != other {
                        next_pc = next_pc.wrapping_add((simm as u32) << 2);
                    }
                }
                9 => self.set(rt, value.wrapping_add(simm as u32)),
                0x0C => self.set(rt, value & (instruction & 0xFFFF)),
                0x0D => self.set(rt, value | (instruction & 0xFFFF)),
                0x0E => self.set(rt, value ^ (instruction & 0xFFFF)),
                0x0F => self.set(rt, (instruction & 0xFFFF) << 16),
                0x23 => {
                    let address = value.wrapping_add(simm as u32) as usize & 0xFFF;
                    self.set(rt, self.read_dmem_word(address));
                }
                0x2B => {
                    let address = value.wrapping_add(simm as u32) as usize & 0xFFF;
                    let value = self.gpr[rt as usize];
                    self.write_dmem_word(address, value);
                }
                _ => {
                    self.halt();
                    return;
                }
            }
        }
        self.pc &= 0xFFC;
    }

    fn set(&mut self, index: u32, value: u32) {
        if index != 0 {
            self.gpr[index as usize] = value;
        }
    }

    fn read_dmem_word(&self, address: usize) -> u32 {
        u32::from_be_bytes([
            self.dmem[address],
            self.dmem[(address + 1) & 0xFFF],
            self.dmem[(address + 2) & 0xFFF],
            self.dmem[(address + 3) & 0xFFF],
        ])
    }

    fn write_dmem_word(&mut self, address: usize, value: u32) {
        let bytes = value.to_be_bytes();
        for (offset, byte) in bytes.iter().enumerate() {
            self.dmem[(address + offset) & 0xFFF] = *byte;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(words: &[u32]) -> Rsp {
        let mut rsp = Rsp::new();
        for (index, word) in words.iter().enumerate() {
            let offset = index * 4;
            rsp.imem[offset..offset + 4].copy_from_slice(&word.to_be_bytes());
        }
        rsp
    }

    #[test]
    fn executes_scalar_and_breaks() {
        // addiu t0,zero,5 ; addiu t1,t0,3 ; sw t1,0x100(zero) ; break
        let mut rsp = program(&[
            0x2408_0005, // addiu t0,zero,5
            0x2509_0003, // addiu t1,t0,3
            0xAC09_0100, // sw t1,0x100(zero)
            0x0000_000D, // break
        ]);
        rsp.halted = false;
        rsp.run(16);
        assert_eq!(rsp.gpr[8], 5);
        assert_eq!(rsp.gpr[9], 8);
        assert!(rsp.broke);
        assert_eq!(rsp.read_dmem_word(0x100), 8);
    }

    #[test]
    fn dma_round_trip() {
        let mut rsp = Rsp::new();
        let mut rdram = vec![0u8; 0x1000];
        rdram[0x20..0x24].copy_from_slice(&[1, 2, 3, 4]);
        rsp.mem_addr = 0x40;
        rsp.dram_addr = 0x20;
        rsp.write_register(0x08, 7, &mut rdram);
        assert_eq!(&rsp.dmem[0x40..0x44], &[1, 2, 3, 4]);

        rsp.dmem[0x40] = 9;
        rsp.write_register(0x0C, 0, &mut rdram);
        assert_eq!(rdram[0x20], 9);
    }
}
