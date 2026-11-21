//! ST018 cartridge: Hitachi ST018 (ARMv3) coprocessor.
//!
//! Interface reference: the observable behaviour of the ST018 in `mesence`
//! (`St018.cpp`, GPLv3) - used only to determine the register handshake and the
//! ARM address space, not as a source. The CPU is written from the ARMv3
//! architecture (26-bit mode: the program counter shares bits 31..28 with the
//! N/Z/C/V flags).
//!
//! FIRMWARE (`st018.rom`, 0x28000 bytes): the first 0x20000 bytes are the
//! program ROM, the remaining 0x8000 bytes the data ROM.
//!
//! S-CPU registers at `$00-$3f:$3000-$3fff` / `$80-$bf:$3000-$3fff`
//! (`addr & 0xFF06`):
//!   `$3800` read  : data from the ARM; clears "has data for S-CPU"
//!   `$3802` read  : acknowledges the ARM-to-S-CPU transfer
//!   `$3802` write : data to the ARM
//!   `$3804` read  : status (bit 7 = data for S-CPU, bit 6 = data for ARM)
//!   `$3804` write : ARM reset (falling edge powers the ARM on)
//!
//! ARM address space: `$00` program ROM (128 KiB), `$0a` data ROM (32 KiB),
//! `$0e` work RAM (16 KiB), `$04` the S-CPU handshake registers
//! (`+$00` write data for SNES / `+$10` read data from SNES / `+$20` status).

/// Program ROM size (128 KiB).
pub const PRG_ROM_SIZE: usize = 0x2_0000;
/// Data ROM size (32 KiB).
pub const DATA_ROM_SIZE: usize = 0x8000;
/// Work RAM size (16 KiB).
pub const WORK_RAM_SIZE: usize = 0x4000;
/// Total firmware image size (`st018.rom`).
pub const FIRMWARE_SIZE: usize = PRG_ROM_SIZE + DATA_ROM_SIZE;

/// ARMv3 (26-bit) core.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ArmV3 {
    /// r0-r15; in 26-bit mode r15 holds the PC in bits 1..25 and the flags in
    /// bits 28..31.
    r: [u32; 16],
    /// Retired cycles, matched against the S-CPU master clock.
    cycles: u64,
}

impl ArmV3 {
    fn new() -> Self {
        Self {
            r: [0; 16],
            cycles: 0,
        }
    }

    fn flag_n(&self) -> bool {
        self.r[15] & 0x8000_0000 != 0
    }
    fn flag_z(&self) -> bool {
        self.r[15] & 0x4000_0000 != 0
    }
    fn flag_c(&self) -> bool {
        self.r[15] & 0x2000_0000 != 0
    }
    fn flag_v(&self) -> bool {
        self.r[15] & 0x1000_0000 != 0
    }
    fn set_flags(&mut self, n: bool, z: bool, c: bool, v: bool) {
        let mut pc = self.r[15] & 0x0FFF_FFFF;
        if n {
            pc |= 0x8000_0000;
        }
        if z {
            pc |= 0x4000_0000;
        }
        if c {
            pc |= 0x2000_0000;
        }
        if v {
            pc |= 0x1000_0000;
        }
        self.r[15] = pc;
    }
    fn pc(&self) -> u32 {
        self.r[15] & 0x03FF_FFFC
    }
    fn set_pc(&mut self, value: u32) {
        let flags = self.r[15] & 0xF000_0000;
        self.r[15] = (value & 0x03FF_FFFC) | flags;
    }
}

/// One ST018 cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct St018 {
    prg: Vec<u8>,
    data: Vec<u8>,
    ram: Vec<u8>,
    cpu: ArmV3,
    has_data_for_snes: bool,
    data_snes: u8,
    has_data_for_arm: bool,
    data_arm: u8,
    arm_reset: bool,
    ack: bool,
    /// Master clock of the last `run_to`, used as the cycle base on power-on.
    now: u64,
}

impl St018 {
    /// Create the coprocessor from a `st018.rom` image.
    pub fn new(firmware: &[u8]) -> Self {
        let mut prg = vec![0u8; PRG_ROM_SIZE];
        let mut data = vec![0u8; DATA_ROM_SIZE];
        if firmware.len() >= FIRMWARE_SIZE {
            prg.copy_from_slice(&firmware[..PRG_ROM_SIZE]);
            data.copy_from_slice(&firmware[PRG_ROM_SIZE..FIRMWARE_SIZE]);
        }
        Self {
            prg,
            data,
            ram: vec![0; WORK_RAM_SIZE],
            cpu: ArmV3::new(),
            has_data_for_snes: false,
            data_snes: 0,
            has_data_for_arm: false,
            data_arm: 0,
            arm_reset: false,
            ack: false,
            now: 0,
        }
    }

    /// Program ROM.
    pub fn program_rom(&self) -> &[u8] {
        &self.prg
    }

    /// Work RAM.
    pub fn work_ram(&self) -> &[u8] {
        &self.ram
    }

    /// ARM program counter (debug/tests).
    pub fn arm_pc(&self) -> u32 {
        self.cpu.pc()
    }

    /// Status byte as the S-CPU reads it.
    pub fn status(&self) -> u8 {
        (u8::from(self.has_data_for_snes) << 7) | (u8::from(self.has_data_for_arm) << 6)
    }

    /// Run the ARM until it reaches `target` master clocks.
    pub fn run_to(&mut self, target: u64) {
        self.now = target;
        if self.arm_reset {
            self.cpu.cycles = target;
            return;
        }
        while self.cpu.cycles < target {
            self.step();
        }
    }

    /// S-CPU read from the coprocessor window.
    pub fn read(&mut self, address: u32, mdr: u8) -> u8 {
        match address & 0xFF06 {
            0x3800 => {
                self.has_data_for_snes = false;
                self.data_snes
            }
            0x3802 => {
                self.ack = false;
                0
            }
            0x3804 => self.status(),
            _ => mdr,
        }
    }

    /// S-CPU write to the coprocessor window.
    pub fn write(&mut self, address: u32, value: u8) {
        match address & 0xFF06 {
            0x3802 => {
                self.data_arm = value;
                self.has_data_for_arm = true;
            }
            0x3804 => {
                let previous = self.arm_reset;
                self.arm_reset = value != 0;
                if previous && !self.arm_reset {
                    self.power_on();
                }
            }
            _ => {}
        }
    }

    fn power_on(&mut self) {
        self.cpu = ArmV3::new();
        self.cpu.r[15] = 0x0000_0000;
        self.cpu.cycles = self.now;
    }

    /// Register read with ARM pipeline semantics: reading `r15` yields
    /// `PC + 8` (`pc()` is already the following instruction).
    fn reg(&self, index: usize) -> u32 {
        if index == 15 {
            self.cpu.pc().wrapping_add(4)
        } else {
            self.cpu.r[index]
        }
    }

    // ---- ARM bus -------------------------------------------------------

    fn arm_read8(&self, address: u32) -> u8 {
        // ARMv3 26-bit address space decoded by the top byte: `$00` program
        // ROM, `$04` S-CPU registers, `$0a` data ROM, `$0e` work RAM.
        match (address >> 24) & 0x0F {
            0x0 => self.prg[(address as usize) & (PRG_ROM_SIZE - 1)],
            0x4 => match address & 0x3F {
                0x10 => {
                    // Reading the transfer register consumes the S-CPU byte.
                    self.data_arm
                }
                0x20 => self.status(),
                _ => 0,
            },
            0xA => self.data[(address as usize) & (DATA_ROM_SIZE - 1)],
            0xE => self.ram[(address as usize) & (WORK_RAM_SIZE - 1)],
            _ => 0,
        }
    }

    fn arm_read32(&mut self, address: u32) -> u32 {
        let address = address & !3;
        if (address >> 24) & 0x0F == 0x4 && address & 0x3F == 0x10 {
            self.has_data_for_arm = false;
        }
        self.cpu.cycles += 1;
        let mut value = 0u32;
        for index in 0..4 {
            value |= u32::from(self.arm_read8(address.wrapping_add(index))) << (index * 8);
        }
        value
    }

    fn arm_write8(&mut self, address: u32, value: u8) {
        match (address >> 24) & 0x0F {
            0x0 => {} // program ROM is read-only
            0x4 => match address & 0x3F {
                0x00 => {
                    self.data_snes = value;
                    self.has_data_for_snes = true;
                }
                0x10 => self.ack = true,
                _ => {}
            },
            0xA => {} // data ROM is read-only
            0xE => {
                let index = (address as usize) & (WORK_RAM_SIZE - 1);
                self.ram[index] = value;
            }
            _ => {}
        }
    }

    fn arm_write32(&mut self, address: u32, value: u32) {
        let address = address & !3;
        self.cpu.cycles += 1;
        for index in 0..4 {
            self.arm_write8(address.wrapping_add(index), (value >> (index * 8)) as u8);
        }
    }

    // ---- ARM core ------------------------------------------------------

    fn condition_passed(&self, condition: u32) -> bool {
        let (n, z, c, v) = (
            self.cpu.flag_n(),
            self.cpu.flag_z(),
            self.cpu.flag_c(),
            self.cpu.flag_v(),
        );
        match condition {
            0x0 => z,
            0x1 => !z,
            0x2 => c,
            0x3 => !c,
            0x4 => n,
            0x5 => !n,
            0x6 => v,
            0x7 => !v,
            0x8 => c && !z,
            0x9 => !c || z,
            0xA => n == v,
            0xB => n != v,
            0xC => !z && n == v,
            0xD => z || n != v,
            0xE => true,
            _ => false,
        }
    }

    fn shift(&mut self, value: u32, shift_type: u32, amount: u32) -> u32 {
        match (shift_type, amount) {
            (0, 0) => value,
            (0, n) => {
                if n < 32 {
                    let carry = (value >> (32 - n)) & 1 != 0;
                    self.cpu.set_flags(
                        self.cpu.flag_n(),
                        self.cpu.flag_z(),
                        carry,
                        self.cpu.flag_v(),
                    );
                    value << n
                } else {
                    self.cpu.set_flags(
                        self.cpu.flag_n(),
                        self.cpu.flag_z(),
                        false,
                        self.cpu.flag_v(),
                    );
                    0
                }
            }
            (1, 0) => {
                let carry = value & 1 != 0;
                self.cpu.set_flags(
                    self.cpu.flag_n(),
                    self.cpu.flag_z(),
                    carry,
                    self.cpu.flag_v(),
                );
                0
            }
            (1, n) => {
                let carry = (value >> (n - 1)) & 1 != 0;
                self.cpu.set_flags(
                    self.cpu.flag_n(),
                    self.cpu.flag_z(),
                    carry,
                    self.cpu.flag_v(),
                );
                value >> n
            }
            (2, 0) => {
                let carry = self.cpu.flag_c();
                let result = (value >> 1) | (u32::from(carry) << 31);
                self.cpu.set_flags(
                    self.cpu.flag_n(),
                    self.cpu.flag_z(),
                    value & 1 != 0,
                    self.cpu.flag_v(),
                );
                result
            }
            (2, n) => {
                let carry = (value >> (n - 1)) & 1 != 0;
                self.cpu.set_flags(
                    self.cpu.flag_n(),
                    self.cpu.flag_z(),
                    carry,
                    self.cpu.flag_v(),
                );
                value >> n
            }
            (3, 0) => {
                let carry = self.cpu.flag_c();
                let result = (value << 1) | u32::from(carry);
                self.cpu.set_flags(
                    self.cpu.flag_n(),
                    self.cpu.flag_z(),
                    value & 0x8000_0000 != 0,
                    self.cpu.flag_v(),
                );
                result
            }
            (3, n) => {
                let carry = (value >> (32 - n)) & 1 != 0;
                self.cpu.set_flags(
                    self.cpu.flag_n(),
                    self.cpu.flag_z(),
                    carry,
                    self.cpu.flag_v(),
                );
                value << n
            }
            _ => value,
        }
    }

    /// Execute one ARM instruction.
    fn step(&mut self) {
        self.cpu.cycles += 1;
        let pc = self.cpu.pc();
        let opcode = self.arm_read32(pc);
        if std::env::var_os("ST018_PC_LOG").is_some() {
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 160 {
                eprintln!("ARM {pc:08X} {opcode:08X}");
            }
        }
        self.cpu.set_pc(pc.wrapping_add(4));
        if !self.condition_passed(opcode >> 28) {
            return;
        }
        let bits = |from: u32, to: u32| (opcode >> from) & ((1 << (to - from + 1)) - 1);
        match bits(26, 27) {
            0b00 => {
                if bits(22, 27) == 0b000000 {
                    self.data_processing(opcode);
                } else if bits(22, 27) == 0b000001 {
                    self.misc(opcode);
                } else {
                    // Multiplies and halfword transfers: multiply subset only.
                    self.multiply(opcode);
                }
            }
            0b01 => self.single_transfer(opcode),
            0b10 => {
                // Bits 27-26 are `10` for both block transfers and branches;
                // bit 25 selects between them.
                if bits(25, 25) != 0 {
                    self.branch(opcode);
                } else {
                    self.block_transfer(opcode);
                }
            }
            0b11 => {
                // Coprocessor instructions are not present on the ST018 board;
                // only SWI is modelled.
                if bits(25, 27) == 0b111 {
                    self.swi();
                }
            }
            _ => {}
        }
    }

    fn data_processing(&mut self, opcode: u32) {
        let bits = |from: u32, to: u32| (opcode >> from) & ((1 << (to - from + 1)) - 1);
        let immediate = bits(25, 25) != 0;
        let op = bits(21, 24);
        let set_flags = bits(20, 20) != 0;
        let rn = bits(16, 19) as usize;
        let rd = bits(12, 15) as usize;
        let operand = if immediate {
            let rotate = bits(8, 11) * 2;
            let value = bits(0, 7);
            value.rotate_right(rotate)
        } else {
            let rm = (opcode & 0xF) as usize;
            let mut value = self.reg(rm);
            if bits(25, 25) == 0 && bits(4, 4) != 0 {
                let rs = bits(8, 11) as usize;
                let amount = self.reg(rs) & 0xFF;
                value = self.shift(value, bits(5, 6), amount);
            } else if bits(4, 4) == 0 {
                let amount = bits(7, 11);
                value = self.shift(value, bits(5, 6), amount);
            }
            if bits(4, 4) != 0 {
                self.cpu.cycles += 1;
            }
            value
        };
        let a = self.reg(rn);
        let (result, carry) = match op {
            0x0 => (a & operand, self.cpu.flag_c()),        // AND
            0x1 => (a ^ operand, self.cpu.flag_c()),        // EOR
            0x2 => self.sub(a, operand, true),              // SUB
            0x3 => self.sub(operand, a, true),              // RSB
            0x4 => self.add(a, operand, true),              // ADD
            0x5 => self.add(a, operand, self.cpu.flag_c()), // ADC
            0x6 => self.sub(a, operand, self.cpu.flag_c()), // SBC
            0x7 => self.sub(operand, a, self.cpu.flag_c()), // RSC
            0x8 => (a & operand, self.cpu.flag_c()),        // TST
            0x9 => (a ^ operand, self.cpu.flag_c()),        // TEQ
            0xA => self.sub(a, operand, true),              // CMP
            0xB => self.add(a, operand, true),              // CMN
            0xC => (a | operand, self.cpu.flag_c()),        // ORR
            0xD => (operand, self.cpu.flag_c()),            // MOV
            0xE => (a & !operand, self.cpu.flag_c()),       // BIC
            _ => (!operand, self.cpu.flag_c()),             // MVN
        };
        let (n, z) = (result & 0x8000_0000 != 0, result == 0);
        if !(0x8..0xC).contains(&op) {
            if rd == 15 {
                self.cpu.set_pc(result);
            } else {
                self.cpu.r[rd] = result;
            }
        }
        if set_flags {
            let v = match op {
                0x2 | 0x3 | 0x6 | 0x7 | 0xA => self.cpu.flag_v(),
                _ => self.cpu.flag_v(),
            };
            self.cpu.set_flags(n, z, carry, v);
        }
    }

    fn add(&mut self, a: u32, b: u32, carry_in: bool) -> (u32, bool) {
        let c = u32::from(carry_in);
        let (sum1, c1) = a.overflowing_add(b);
        let (sum, c2) = sum1.overflowing_add(c);
        let carry = c1 || c2;
        let overflow = ((a ^ sum) & (b ^ sum) & 0x8000_0000) != 0;
        self.cpu.set_flags(
            self.cpu.flag_n(),
            self.cpu.flag_z(),
            self.cpu.flag_c(),
            overflow,
        );
        (sum, carry)
    }

    fn sub(&mut self, a: u32, b: u32, carry_in: bool) -> (u32, bool) {
        let borrow = u32::from(!carry_in);
        let (diff1, b1) = a.overflowing_sub(b);
        let (diff, b2) = diff1.overflowing_sub(borrow);
        let carry = !(b1 || b2);
        let overflow = ((a ^ b) & (a ^ diff) & 0x8000_0000) != 0;
        self.cpu.set_flags(
            self.cpu.flag_n(),
            self.cpu.flag_z(),
            self.cpu.flag_c(),
            overflow,
        );
        (diff, carry)
    }

    fn misc(&mut self, opcode: u32) {
        let bits = |from: u32, to: u32| (opcode >> from) & ((1 << (to - from + 1)) - 1);
        match bits(4, 7) {
            0b1001 => {
                // MUL / MLA.
                let accumulate = bits(21, 21) != 0;
                let set = bits(20, 20) != 0;
                let rd = bits(16, 19) as usize;
                let rn = bits(12, 15) as usize;
                let rs = (opcode & 0xF) as usize;
                let mut result = self.reg(rs).wrapping_mul(self.reg(rn));
                if accumulate {
                    result = result.wrapping_add(self.reg(rd));
                }
                self.cpu.cycles += 3;
                self.cpu.r[rd] = result;
                if set {
                    self.cpu.set_flags(
                        result & 0x8000_0000 != 0,
                        result == 0,
                        self.cpu.flag_c(),
                        self.cpu.flag_v(),
                    );
                }
            }
            0b0000 => {
                // MRS / MSR (26-bit: the flags live in r15 bits 28..31).
                if bits(21, 22) == 0b00 {
                    let rd = bits(12, 15) as usize;
                    if rd != 15 {
                        self.cpu.r[rd] = self.cpu.r[15] & 0xF000_0000;
                    }
                } else {
                    let value = self.cpu.r[opcode as usize & 0xF] & 0xF000_0000;
                    self.cpu.set_flags(
                        value & 0x8000_0000 != 0,
                        value & 0x4000_0000 != 0,
                        value & 0x2000_0000 != 0,
                        value & 0x1000_0000 != 0,
                    );
                }
            }
            _ => {}
        }
    }

    fn multiply(&mut self, opcode: u32) {
        // Only the MUL/MLA encoding is decoded here; other encodings are
        // treated as no-ops rather than trapping.
        let bits = |from: u32, to: u32| (opcode >> from) & ((1 << (to - from + 1)) - 1);
        if bits(22, 27) == 0b000000 && bits(4, 7) == 0b1001 {
            self.misc(opcode);
        }
    }

    fn single_transfer(&mut self, opcode: u32) {
        let bits = |from: u32, to: u32| (opcode >> from) & ((1 << (to - from + 1)) - 1);
        // Single data transfer: bit 25 = 1 selects a *register* offset
        // (the opposite convention to the data-processing immediate bit).
        let immediate = bits(25, 25) == 0;
        let pre = bits(24, 24) != 0;
        let up = bits(23, 23) != 0;
        let byte = bits(22, 22) != 0;
        let writeback = bits(21, 21) != 0;
        let load = bits(20, 20) != 0;
        let rn = bits(16, 19) as usize;
        let rd = bits(12, 15) as usize;
        let offset = if immediate {
            bits(0, 11)
        } else {
            let rm = (opcode & 0xF) as usize;
            let value = self.reg(rm);
            let amount = bits(7, 11);
            self.shift(value, bits(5, 6), amount)
        };
        let base = self.reg(rn);
        let address = if pre {
            if up {
                base.wrapping_add(offset)
            } else {
                base.wrapping_sub(offset)
            }
        } else {
            base
        };
        if load {
            let value = if byte {
                u32::from(self.arm_read8(address))
            } else {
                self.arm_read32(address)
            };
            if std::env::var_os("ST018_PC_LOG").is_some() {
                eprintln!("LDR r{rd} <- {value:08X} @ {address:08X}");
            }
            if rd == 15 {
                self.cpu.set_pc(value);
            } else {
                self.cpu.r[rd] = value;
            }
            self.cpu.cycles += 1;
        } else {
            if byte {
                self.arm_write8(address, self.cpu.r[rd] as u8);
            } else {
                self.arm_write32(address, self.cpu.r[rd]);
            }
        }
        if !pre || writeback {
            let updated = if up {
                base.wrapping_add(offset)
            } else {
                base.wrapping_sub(offset)
            };
            self.cpu.r[rn] = updated;
        }
    }

    fn branch(&mut self, opcode: u32) {
        let link = opcode & (1 << 24) != 0;
        // 24-bit signed word offset, relative to PC + 8.
        let offset = (((opcode & 0x00FF_FFFF) << 8) as i32 >> 6) as u32;
        let next = self.cpu.pc();
        if link {
            self.cpu.r[14] = next;
        }
        // `pc()` is already the following instruction, so PC + 8 is `next + 4`.
        self.cpu.set_pc(next.wrapping_add(4).wrapping_add(offset));
        self.cpu.cycles += 1;
    }

    fn block_transfer(&mut self, opcode: u32) {
        let bits = |from: u32, to: u32| (opcode >> from) & ((1 << (to - from + 1)) - 1);
        let pre = bits(24, 24) != 0;
        let up = bits(23, 23) != 0;
        let writeback = bits(21, 21) != 0;
        let load = bits(20, 20) != 0;
        let rn = bits(16, 19) as usize;
        let list = bits(0, 15);
        let count = list.count_ones();
        if count == 0 {
            return;
        }
        let mut base = self.reg(rn);
        let start = if up {
            base.wrapping_add(if pre { 4 } else { 0 })
        } else {
            base.wrapping_sub(count * 4)
                .wrapping_add(if pre { 0 } else { 4 })
        };
        let mut address = start;
        for index in 0..16 {
            if list & (1 << index) == 0 {
                continue;
            }
            if load {
                let value = self.arm_read32(address);
                if std::env::var_os("ST018_PC_LOG").is_some() && index == 15 {
                    eprintln!(
                        "LDMpc {address:08X} = {value:08X} sp={:08X}",
                        self.cpu.r[13]
                    );
                }
                if index == 15 {
                    self.cpu.set_pc(value);
                } else {
                    self.cpu.r[index] = value;
                }
            } else {
                let value = self.cpu.r[index];
                self.arm_write32(address, value);
            }
            address = address.wrapping_add(4);
            self.cpu.cycles += 1;
        }
        if writeback {
            base = if up {
                base.wrapping_add(count * 4)
            } else {
                base.wrapping_sub(count * 4)
            };
            self.cpu.r[rn] = base;
        }
    }

    fn swi(&mut self) {
        // The ST018 board has no SWI handler in the program ROM's vector table
        // that this core models; treat it as a return-to-supervisor no-op.
        self.cpu.cycles += 3;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firmware() -> Vec<u8> {
        let mut image = vec![0u8; FIRMWARE_SIZE];
        for (index, byte) in image.iter_mut().enumerate() {
            *byte = (index >> 1) as u8;
        }
        image
    }

    #[test]
    fn firmware_is_split_into_program_and_data() {
        let image = firmware();
        let chip = St018::new(&image);
        assert_eq!(chip.program_rom()[0], image[0]);
        assert_eq!(
            chip.program_rom()[PRG_ROM_SIZE - 1],
            image[PRG_ROM_SIZE - 1]
        );
        // Data ROM reads through the ARM address space at $0a000000.
        assert!(chip.work_ram().is_empty() || chip.work_ram()[0] == 0);
    }

    #[test]
    fn snes_handshake_round_trips() {
        let mut chip = St018::new(&firmware());
        // S-CPU writes a byte for the ARM, status reflects it.
        chip.write(0x00_3802, 0x5A);
        assert_eq!(chip.status() & 0x40, 0x40);
        // The ARM consumes it (via the $04:0010 register) and returns one.
        let value = chip.arm_read32(0x04_000010);
        assert_eq!(value & 0xFF, 0x5A);
        assert_eq!(chip.status() & 0x40, 0x00);
        chip.arm_write8(0x0400_0000, 0xA5);
        assert_eq!(chip.status() & 0x80, 0x80);
        assert_eq!(chip.read(0x00_3800, 0), 0xA5);
        assert_eq!(chip.status() & 0x80, 0x00);
    }

    #[test]
    fn arm_reset_edge_powers_the_core_on() {
        let mut chip = St018::new(&firmware());
        chip.write(0x00_3804, 0x80);
        assert_eq!(chip.status() & 0x80, 0);
        chip.write(0x00_3804, 0x00);
        assert_eq!(chip.arm_pc(), 0);
    }

    #[test]
    fn work_ram_round_trips_through_the_arm_bus() {
        let mut chip = St018::new(&firmware());
        chip.arm_write32(0x0E00_0100, 0x1234_5678);
        assert_eq!(chip.arm_read32(0x0E00_0100), 0x1234_5678);
        assert_eq!(&chip.work_ram()[0x100..0x104], &[0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn program_rom_is_read_only() {
        let image = firmware();
        let mut chip = St018::new(&image);
        chip.arm_write8(0x0000_0010, 0xFF);
        assert_eq!(chip.program_rom()[0x10], image[0x10]);
    }

    #[test]
    fn data_rom_maps_at_0x0a() {
        let image = firmware();
        let chip = St018::new(&image);
        assert_eq!(chip.arm_read8(0x0A00_0000), image[PRG_ROM_SIZE]);
        assert_eq!(
            chip.arm_read8(0x0A00_7FFF),
            image[PRG_ROM_SIZE + DATA_ROM_SIZE - 1]
        );
    }
}
