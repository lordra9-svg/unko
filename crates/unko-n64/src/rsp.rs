//! RSP scaffold: DMEM/IMEM, the SP register block, DMA, a scalar interpreter
//! and the vector unit subset used by command streams.
//!
//! Vector support: 32 x 128-bit registers, the scalar element/element-decode
//! fields, `LQV/SQV`, `LHV/SHV`, `LBV/SBV`, `VADD/VSUB`, `VMULF/VMULU`,
//! `VMACF/VMACU` and the reciprocal/divide primitives `VRCP/VRCPL/VRCPH`.
//! Accumulate/compare flags and the remaining vector ops are later.

/// RSP data memory size.
pub const DMEM_SIZE: usize = 0x1000;
/// RSP instruction memory size.
pub const IMEM_SIZE: usize = 0x1000;

/// One RSP.
#[derive(Debug, Clone)]
pub struct Rsp {
    /// Scalar registers.
    pub gpr: [u32; 32],
    /// Vector registers (16 big-endian bytes each).
    pub v: [[u8; 16]; 32],
    /// Accumulator high/mid/low words per lane.
    pub acc: [[u16; 3]; 8],
    /// Carry flags from `VADD`/`VADDC`.
    pub vcol: u16,
    /// Carry flags from the high half.
    pub vcoh: u16,
    /// Divide input/output and divide-by-zero flag.
    pub divin: i16,
    /// Divide output.
    pub divout: i32,
    /// Divide by zero latency flag.
    pub divdp: bool,
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
    reciprocals: [u16; 512],
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
        let mut reciprocals = [0u16; 512];
        reciprocals[0] = u16::MAX;
        for (index, slot) in reciprocals.iter_mut().enumerate().skip(1) {
            let a = index as u64 + 512;
            let b = (1u64 << 34) / a;
            *slot = ((b + 1) >> 8) as u16;
        }
        Self {
            gpr: [0; 32],
            v: [[0; 16]; 32],
            acc: [[0; 3]; 8],
            vcol: 0,
            vcoh: 0,
            divin: 0,
            divout: 0,
            divdp: false,
            pc: 0,
            dmem: vec![0; DMEM_SIZE],
            imem: vec![0; IMEM_SIZE],
            status: 1,
            halted: true,
            broke: false,
            mem_addr: 0,
            dram_addr: 0,
            semaphore: 0,
            reciprocals,
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
            0x14 | 0x18 => 0,
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
                    (self.status & !0x3) | u32::from(self.halted) | (u32::from(self.broke) << 1);
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

    /// Reads a vector element.
    #[must_use]
    pub fn element(&self, register: usize, lane: usize) -> i16 {
        let bytes = self.v[register];
        i16::from_be_bytes([bytes[(lane & 7) * 2], bytes[(lane & 7) * 2 + 1]])
    }

    /// Writes a vector element.
    pub fn set_element(&mut self, register: usize, lane: usize, value: i16) {
        let bytes = value.to_be_bytes();
        self.v[register][(lane & 7) * 2] = bytes[0];
        self.v[register][(lane & 7) * 2 + 1] = bytes[1];
    }

    /// Executes up to `max` instructions.
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
                0x12 => self.cop2(instruction),
                0x23 => {
                    let address = value.wrapping_add(simm as u32) as usize & 0xFFF;
                    self.set(rt, self.read_dmem_word(address));
                }
                0x2B => {
                    let address = value.wrapping_add(simm as u32) as usize & 0xFFF;
                    let value = self.gpr[rt as usize];
                    self.write_dmem_word(address, value);
                }
                0x32 => self.load_vector(instruction),
                0x3A => self.store_vector(instruction),
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

    #[allow(clippy::needless_range_loop)]
    fn cop2(&mut self, instruction: u32) {
        let sub = (instruction >> 21) & 0x1F;
        if sub < 0x10 {
            let rt = ((instruction >> 16) & 31) as usize;
            let rd = ((instruction >> 11) & 31) as usize;
            match sub {
                0x00 => self.set(rt as u32, self.control_read(rd & 3)),
                0x04 => {
                    let value = self.gpr[rt];
                    self.control_write(rd & 3, value);
                }
                _ => self.halt(),
            }
            return;
        }
        let e = (sub & 0x0F) as usize;
        let de = ((instruction >> 11) & 7) as usize;
        let vs = ((instruction >> 11) & 31) as usize;
        let vt = ((instruction >> 16) & 31) as usize;
        let vd = ((instruction >> 6) & 31) as usize;
        let funct = instruction & 0x3F;
        let selected = self.select(vt, e);
        match funct {
            0x00 => self.vmulf(vd, vs, &selected, false),
            0x01 => self.vmulf(vd, vs, &selected, true),
            0x08 => self.vmacf(vd, vs, &selected, false),
            0x09 => self.vmacf(vd, vs, &selected, true),
            0x10 => {
                for n in 0..8 {
                    let result = i32::from(self.element(vs, n))
                        + i32::from(selected[n])
                        + i32::from((self.vcol >> n) & 1);
                    self.set_acc_low(n, result as u16);
                    self.set_element(vd, n, result.clamp(-32768, 32767) as i16);
                }
                self.vcol = 0;
                self.vcoh = 0;
            }
            0x11 => {
                for n in 0..8 {
                    let result = i32::from(self.element(vs, n))
                        - i32::from(selected[n])
                        - i32::from((self.vcol >> n) & 1);
                    self.set_acc_low(n, result as u16);
                    self.set_element(vd, n, result.clamp(-32768, 32767) as i16);
                }
                self.vcol = 0;
                self.vcoh = 0;
            }
            0x30..=0x32 => self.reciprocal(funct, vd, de, e, &selected),
            0x37 | 0x3F => {}
            _ => self.halt(),
        }
    }

    fn control_read(&self, control: usize) -> u32 {
        let (low, high) = match control {
            0 => (self.vcol, self.vcoh),
            _ => (0, 0),
        };
        let mut value = 0;
        for n in 0..8 {
            value |= u32::from((low >> n) & 1) << n;
            value |= u32::from((high >> n) & 1) << (8 + n);
        }
        value
    }

    fn control_write(&mut self, control: usize, value: u32) {
        if control == 0 {
            self.vcol = (value & 0xFF) as u16;
            self.vcoh = ((value >> 8) & 0xFF) as u16;
        }
    }

    fn select(&self, register: usize, e: usize) -> [i16; 8] {
        let source: [i16; 8] = std::array::from_fn(|n| self.element(register, n));
        match e {
            0 | 1 => source,
            2 => std::array::from_fn(|n| source[(n / 2) * 2]),
            3 => std::array::from_fn(|n| source[(n / 2) * 2 + 1]),
            4 => std::array::from_fn(|n| source[(n / 4) * 4]),
            5 => std::array::from_fn(|n| source[(n / 4) * 4 + 1]),
            6 => std::array::from_fn(|n| source[(n / 4) * 4 + 2]),
            7 => std::array::from_fn(|n| source[(n / 4) * 4 + 3]),
            _ => [source[e - 8]; 8],
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn vmulf(&mut self, vd: usize, vs: usize, selected: &[i16; 8], unsigned: bool) {
        for n in 0..8 {
            let product = i64::from(self.element(vs, n)) * i64::from(selected[n]);
            if unsigned {
                self.acc_set(n, (product * 2) as u64);
                let high = self.acc[n][0] as i16;
                let mid = self.acc[n][1] as i16;
                let value = if high < 0 {
                    0
                } else if (high ^ mid) < 0 {
                    0xFFFF
                } else {
                    self.acc[n][1]
                };
                self.set_element(vd, n, value as i16);
            } else {
                self.acc_set(n, (product * 2 + 0x8000) as u64);
                let value = self.acc_saturate(n, true, 0x8000, 0x7FFF);
                self.set_element(vd, n, value as i16);
            }
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn vmacf(&mut self, vd: usize, vs: usize, selected: &[i16; 8], unsigned: bool) {
        for n in 0..8 {
            let product = i64::from(self.element(vs, n)) * i64::from(selected[n]);
            self.acc_set(n, self.acc_get(n).wrapping_add((product * 2) as u64));
            if unsigned {
                let high = self.acc[n][0] as i16;
                let mid = self.acc[n][1] as i16;
                let value = if high < 0 {
                    0
                } else if (high ^ mid) < 0 {
                    0xFFFF
                } else {
                    self.acc[n][1]
                };
                self.set_element(vd, n, value as i16);
            } else {
                let value = self.acc_saturate(n, true, 0x8000, 0x7FFF);
                self.set_element(vd, n, value as i16);
            }
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn reciprocal(&mut self, funct: u32, vd: usize, de: usize, e: usize, selected: &[i16; 8]) {
        let lane = e & 7;
        match funct {
            0x30 => {
                let input = i32::from(selected[lane]);
                let result = self.divide(input);
                self.divdp = false;
                self.divout = result >> 16;
                for n in 0..8 {
                    self.set_acc_low(n, selected[n] as u16);
                }
                self.set_element(vd, de, result as i16);
            }
            0x31 => {
                let input = if self.divdp {
                    (i32::from(self.divin) << 16) | i32::from(selected[lane])
                } else {
                    i32::from(selected[lane])
                };
                let result = self.divide(input);
                self.divdp = false;
                self.divout = result >> 16;
                for n in 0..8 {
                    self.set_acc_low(n, selected[n] as u16);
                }
                self.set_element(vd, de, result as i16);
            }
            0x32 => {
                for n in 0..8 {
                    self.set_acc_low(n, selected[n] as u16);
                }
                self.divdp = true;
                self.divin = selected[lane];
                self.set_element(vd, de, self.divout as i16);
            }
            _ => unreachable!(),
        }
    }

    fn divide(&self, input: i32) -> i32 {
        if input == 0 {
            return 0x7FFF_FFFF;
        }
        let mask = input >> 31;
        let mut data = input ^ mask;
        if input > -32768 {
            data -= mask;
        }
        if input == -32768 {
            return 0xFFFF_0000u32 as i32;
        }
        let shift = (data as u32).leading_zeros();
        let index = (((data as u64) << shift) & 0x7FC0_0000) >> 22;
        let mut result = i32::from(self.reciprocals[index as usize]);
        result = (0x10000 | result) << 14;
        (result >> (31 - shift)) ^ mask
    }

    fn acc_get(&self, lane: usize) -> u64 {
        (u64::from(self.acc[lane][0]) << 32)
            | (u64::from(self.acc[lane][1]) << 16)
            | u64::from(self.acc[lane][2])
    }

    fn acc_set(&mut self, lane: usize, value: u64) {
        self.acc[lane][0] = (value >> 32) as u16;
        self.acc[lane][1] = (value >> 16) as u16;
        self.acc[lane][2] = value as u16;
    }

    fn set_acc_low(&mut self, lane: usize, value: u16) {
        self.acc[lane][2] = value;
    }

    fn acc_saturate(&self, lane: usize, slice: bool, negative: u16, positive: u16) -> u16 {
        let high = self.acc[lane][0] as i16;
        let mid = self.acc[lane][1] as i16;
        if high < 0 {
            if self.acc[lane][0] != 0xFFFF {
                return negative;
            }
            if mid >= 0 {
                return negative;
            }
        } else {
            if self.acc[lane][0] != 0 {
                return positive;
            }
            if mid < 0 {
                return positive;
            }
        }
        if slice {
            self.acc[lane][1]
        } else {
            self.acc[lane][2]
        }
    }

    fn load_vector(&mut self, instruction: u32) {
        let sub = (instruction >> 11) & 0x1F;
        let e = ((instruction >> 7) & 0x0F) as usize;
        let vt = ((instruction >> 16) & 31) as usize;
        let rs = ((instruction >> 21) & 31) as usize;
        let imm = (instruction & 0x7F) as i8 as i32;
        let scale = if sub == 0x00 { 1 } else { 16 };
        let base = self.gpr[rs].wrapping_add((imm * scale) as u32) as usize;
        match sub {
            0x04 => {
                let start = e;
                let end = (16 + e).saturating_sub(base & 15).min(16);
                for offset in start..end {
                    let byte = self.dmem[(base + (offset - start)) & 0xFFF];
                    self.v[vt][offset & 15] = byte;
                }
            }
            0x08 => {
                let index = (base & 7).wrapping_sub(e) & 15;
                let address = base & !7;
                for offset in 0..8 {
                    let byte = self.dmem[(address + ((index + offset * 2) & 15)) & 0xFFF];
                    let value = i16::from(byte) << 7;
                    self.set_element(vt, offset, value);
                }
            }
            0x00 => {
                let start = e;
                let end = (start + 1).min(16);
                for offset in start..end {
                    self.v[vt][offset & 15] = self.dmem[(base + offset - start) & 0xFFF];
                }
            }
            _ => self.halt(),
        }
    }

    fn store_vector(&mut self, instruction: u32) {
        let sub = (instruction >> 11) & 0x1F;
        let e = ((instruction >> 7) & 0x0F) as usize;
        let vt = ((instruction >> 16) & 31) as usize;
        let rs = ((instruction >> 21) & 31) as usize;
        let imm = (instruction & 0x7F) as i8 as i32;
        let scale = if sub == 0x00 { 1 } else { 16 };
        let base = self.gpr[rs].wrapping_add((imm * scale) as u32) as usize;
        match sub {
            0x04 => {
                let start = e;
                let end = start + (16 - (base & 15));
                for offset in start..end {
                    self.dmem[(base + offset - start) & 0xFFF] = self.v[vt][offset & 15];
                }
            }
            0x08 => {
                let index = base & 7;
                let address = base & !7;
                for offset in 0..8 {
                    let byte = e + offset * 2;
                    let value = (self.v[vt][byte & 15] << 1) | (self.v[vt][(byte + 1) & 15] >> 7);
                    self.dmem[(address + ((index + offset * 2) & 15)) & 0xFFF] = value;
                }
            }
            0x00 => {
                for offset in e..(e + 1).min(16) {
                    self.dmem[(base + offset - e) & 0xFFF] = self.v[vt][offset & 15];
                }
            }
            _ => self.halt(),
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

    #[test]
    fn vector_add_and_store() {
        // LQV v0,0x100(zero) ; LQV v1,0x110(zero) ; VADD v2,v0,v1 ; SQV v2,0x120(zero) ; break
        let mut rsp = program(&[
            0xC800_2010, // lqv v0, 0x100(zero)
            0xC801_2011, // lqv v1, 0x110(zero)
            0x4A01_0090, // vadd v2, v0, v1 (e=0)
            0xE802_2012, // sqv v2, 0x120(zero)
            0x0000_000D, // break
        ]);
        for lane in 0..8 {
            let a = (lane as i16 + 1).to_be_bytes();
            rsp.dmem[0x100 + lane * 2] = a[0];
            rsp.dmem[0x100 + lane * 2 + 1] = a[1];
            let b = (10 * (lane as i16 + 1)).to_be_bytes();
            rsp.dmem[0x110 + lane * 2] = b[0];
            rsp.dmem[0x110 + lane * 2 + 1] = b[1];
        }
        rsp.halted = false;
        rsp.run(32);
        assert!(rsp.broke);
        for lane in 0..8 {
            let value = (lane as i16 + 1) + 10 * (lane as i16 + 1);
            let bytes = value.to_be_bytes();
            assert_eq!(
                &rsp.dmem[0x120 + lane * 2..0x120 + lane * 2 + 2],
                &bytes,
                "lane {lane}"
            );
        }
    }

    #[test]
    fn vector_multiply_accumulate() {
        // v0 = [2,2,...], v1 = [3,3,...] ; VMULF v2,v0,v1 ; VMACF v2,v0,v1
        let mut rsp = program(&[
            0x4A01_0080, // vmulf v2, v0, v1
            0x4A01_0088, // vmacf v2, v0, v1
            0x0000_000D, // break
        ]);
        for lane in 0..8 {
            rsp.set_element(0, lane, 2);
            rsp.set_element(1, lane, 3);
        }
        rsp.halted = false;
        rsp.run(8);
        // 2*3*2 >> 16 rounded = 0, so both products round to 0 in 1.15 fixed.
        // Use 0x4000 (0.5) * 0x4000 to get 0.25 -> 0x2000.
        rsp.set_element(0, 0, 0x4000);
        rsp.set_element(1, 0, 0x4000);
        rsp.pc = 0;
        rsp.broke = false;
        rsp.halted = false;
        rsp.run(8);
        assert_eq!(rsp.element(2, 0), 0x4000);
    }

    #[test]
    fn reciprocal_of_four() {
        // VRCP v2[0], v1[e=0] with v1[0] = 4.0? In 1.15 input 0x0004.
        let mut rsp = program(&[0x4A01_00B0, 0x0000_000D]);
        rsp.set_element(1, 0, 4);
        rsp.halted = false;
        rsp.run(4);
        assert!(rsp.broke);
        assert_eq!(rsp.divout, 0x1FFF);
    }
}
