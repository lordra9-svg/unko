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

/// ARM address regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmRegion {
    Program,
    Data,
    Work,
    Io,
    Open,
}

/// Decode an ARM address.
///
/// The ST018 firmware uses `$00000000` program ROM, `$0a000000` data ROM,
/// `$e0000000` work RAM and `$04000000` for the S-CPU handshake registers.
/// (`$e0` in the top byte is region `$e`, not region `$0`.) Older documents
/// place the registers at `$04000000`; both are accepted.
fn arm_region(address: u32) -> ArmRegion {
    if (address >> 24) == 0x04 || (address >> 28) == 0x4 {
        return ArmRegion::Io;
    }
    match address >> 28 {
        0x0 => ArmRegion::Program,
        0xA => ArmRegion::Data,
        0xE => ArmRegion::Work,
        _ => ArmRegion::Open,
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
    /// Instructions logged since the last power-on (debug).
    traced: u64,
    /// Recent ARM PCs (debug ring buffer).
    recent: Vec<u32>,
}

impl St018 {
    /// Create the coprocessor from a `st018.rom` image.
    pub fn new(firmware: &[u8]) -> Self {
        let mut prg = vec![0u8; PRG_ROM_SIZE];
        let mut data = vec![0u8; DATA_ROM_SIZE];
        if firmware.len() >= FIRMWARE_SIZE {
            prg.copy_from_slice(&firmware[..PRG_ROM_SIZE]);
            data.copy_from_slice(&firmware[PRG_ROM_SIZE..FIRMWARE_SIZE]);
        } else if firmware.len() >= PRG_ROM_SIZE {
            // The firmware is also distributed split as `st018.program.rom`
            // (128 KiB); the data ROM is then supplied separately.
            prg.copy_from_slice(&firmware[..PRG_ROM_SIZE]);
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
            traced: 0,
            recent: Vec::new(),
        }
    }

    /// Install the 32 KiB data ROM (split-firmware form).
    pub fn load_data_rom(&mut self, image: &[u8]) {
        let count = image.len().min(DATA_ROM_SIZE);
        self.data[..count].copy_from_slice(&image[..count]);
    }

    /// Whether the program ROM contains firmware (first word not blank).
    pub fn has_firmware(&self) -> bool {
        self.prg.iter().any(|&byte| byte != 0)
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

    /// ARM general register (debug/tests).
    pub fn arm_register(&self, index: usize) -> u32 {
        self.cpu.r[index & 0xF]
    }

    /// Status byte as the S-CPU reads it (reference `GetStatus`):
    /// bit 0 = data for the S-CPU, bit 2 = ack, bit 3 = data for the ARM,
    /// bit 7 = the ARM is running (reset released).
    pub fn status(&self) -> u8 {
        u8::from(self.has_data_for_snes)
            | (u8::from(self.ack) << 2)
            | (u8::from(self.has_data_for_arm) << 3)
            | (u8::from(!self.arm_reset) << 7)
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
        self.traced = 0;
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
        if std::env::var_os("ST018_MEM_LOG").is_some() && arm_region(address) != ArmRegion::Program
        {
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 40 {
                eprintln!("ARMRD {address:08X} region={:?}", arm_region(address));
            }
        }
        match arm_region(address) {
            ArmRegion::Program => self.prg[(address as usize) & (PRG_ROM_SIZE - 1)],
            ArmRegion::Io => match address & 0x3F {
                0x10 => {
                    if std::env::var_os("ST018_CALL_LOG").is_some() {
                        eprintln!(
                            "IO10 read pc={:08X} -> {:02X} (pending={})",
                            self.cpu.pc(),
                            self.data_arm,
                            self.has_data_for_arm
                        );
                    }
                    // Reading the transfer register consumes the S-CPU byte.
                    self.data_arm
                }
                0x20 => self.status(),
                _ => 0,
            },
            ArmRegion::Data => self.data[(address as usize) & (DATA_ROM_SIZE - 1)],
            ArmRegion::Work => self.ram[(address as usize) & (WORK_RAM_SIZE - 1)],
            ArmRegion::Open => 0,
        }
    }

    fn arm_read32(&mut self, address: u32) -> u32 {
        let address = address & !3;
        if arm_region(address) == ArmRegion::Io && address & 0x3F == 0x10 {
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
        if std::env::var_os("ST018_MEM_LOG").is_some() && arm_region(address) != ArmRegion::Program
        {
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 40 {
                eprintln!(
                    "ARMWR {address:08X} = {value:02X} region={:?}",
                    arm_region(address)
                );
            }
        }
        match arm_region(address) {
            ArmRegion::Program => {} // program ROM is read-only
            ArmRegion::Io => match address & 0x3F {
                0x00 => {
                    if std::env::var_os("ST018_IO_LOG").is_some() {
                        eprintln!(
                            "ARMIO wr {:02X} pc={:08X} r0={:08X} r1={:08X} r2={:08X}",
                            value,
                            self.cpu.pc(),
                            self.cpu.r[0],
                            self.cpu.r[1],
                            self.cpu.r[2]
                        );
                    }
                    self.data_snes = value;
                    self.has_data_for_snes = true;
                }
                0x10 => self.ack = true,
                _ => {}
            },
            ArmRegion::Data => {} // data ROM is read-only
            ArmRegion::Work => {
                let index = (address as usize) & (WORK_RAM_SIZE - 1);
                self.ram[index] = value;
            }
            ArmRegion::Open => {}
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

    /// Barrel shifter (ARMv3): returns the shifted value and the carry-out.
    /// `immediate` selects the encoding's zero-amount meaning (LSR/ASR/ROR by
    /// 32 or RRX for immediate shifts; no shift for register shifts).
    fn shift_value(
        &self,
        value: u32,
        shift_type: u32,
        amount: u32,
        immediate: bool,
    ) -> (u32, bool) {
        let c = self.cpu.flag_c();
        match (shift_type, amount) {
            (0, 0) => (value, c), // LSL #0 keeps C
            (0, n) if n < 32 => (value << n, (value >> (32 - n)) & 1 != 0),
            (0, n) => {
                if n == 32 {
                    (0, value & 1 != 0)
                } else {
                    (0, false)
                }
            }
            (1, 0) => {
                if immediate {
                    (0, value & 0x8000_0000 != 0) // LSR #32
                } else {
                    (value, c)
                }
            }
            (1, n) => (value >> n, (value >> (n - 1)) & 1 != 0),
            (2, 0) => {
                if immediate {
                    // ROR #0 is RRX.
                    ((u32::from(c) << 31) | (value >> 1), value & 1 != 0)
                } else {
                    (value, c)
                }
            }
            (2, n) => (value.rotate_right(n), (value >> (n - 1)) & 1 != 0),
            (3, _) => {
                let n = if amount == 0 { 32 } else { amount };
                let result = if n >= 32 {
                    if value & 0x8000_0000 != 0 {
                        0xFFFF_FFFF
                    } else {
                        0
                    }
                } else {
                    ((value as i32) >> n) as u32
                };
                (result, value & 0x8000_0000 != 0)
            }
            _ => (value, c),
        }
    }

    fn step(&mut self) {
        self.cpu.cycles += 1;
        let pc = self.cpu.pc();
        let opcode = self.arm_read32(pc);
        self.cpu.set_pc(pc.wrapping_add(4));
        if std::env::var_os("ST018_CALL_LOG").is_some() {
            if pc == 0x2B4 && self.cpu.r[1] == 0 && self.recent.len() == 8 {
                eprintln!(
                    "CALL 0x2B4 lr={:08X} prev={:08X?}",
                    self.cpu.r[14], self.recent
                );
                self.recent.clear();
            }
            self.recent.push(pc);
            if self.recent.len() > 8 {
                self.recent.remove(0);
            }
        }
        if std::env::var_os("ST018_CALL_LOG").is_some() && matches!(pc, 0x1C8 | 0x1CC) {
            eprintln!(
                "PIC pc={pc:08X} op={opcode:08X} r1={:08X} pc8={:08X}",
                self.cpu.r[1],
                self.reg(15)
            );
        }
        if std::env::var_os("ST018_CALL_LOG").is_some()
            && matches!(pc, 0x1D0 | 0x1D4 | 0x1E8 | 0x204)
        {
            eprintln!(
                "TBL pc={pc:08X} r0={:08X} r1={:08X} r2={:08X} z={}",
                self.cpu.r[0],
                self.cpu.r[1],
                self.cpu.r[2],
                self.cpu.flag_z()
            );
        }
        if std::env::var_os("ST018_CALL_LOG").is_some() && pc == 0x300 {
            eprintln!(
                "CALL300 lr={:08X} r0={:08X} r1={:08X} r2={:08X}",
                self.cpu.r[14], self.cpu.r[0], self.cpu.r[1], self.cpu.r[2]
            );
        }
        if !self.condition_passed(opcode >> 28) {
            return;
        }
        let bits = |from: u32, to: u32| (opcode >> from) & ((1 << (to - from + 1)) - 1);
        match bits(26, 27) {
            0b00 => {
                // Bits 27-26 are `00` for every opcode in this class; the
                // multiply/misc forms are identified by bits 7-4, **not** by
                // bits 27-22 (which include the immediate bit and the opcode,
                // so testing them routed every MOV/ADD/CMP to the multiply
                // path and silently dropped it).
                // Multiply is the only form with bits 7-4 = 1001 and
                // bits 27-22 = 0. MRS/MSR are identified by their canonical
                // bit masks; everything else is data processing.
                let misc = (opcode & 0x0FBF_0FFF) == 0x010F_0000
                    || (opcode & 0x0FB0_FFF0) == 0x0120_F000
                    || (opcode & 0x0FB0_F000) == 0x0320_F000;
                if bits(4, 7) == 0b1001 && bits(22, 27) == 0 {
                    self.multiply(opcode);
                } else if misc {
                    self.misc(opcode);
                } else {
                    self.data_processing(opcode);
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
        let (operand, shift_carry) = if immediate {
            let rotate = bits(8, 11) * 2;
            let raw = bits(0, 7);
            let value = raw.rotate_right(rotate);
            let carry = if rotate == 0 {
                self.cpu.flag_c()
            } else {
                value & 0x8000_0000 != 0
            };
            (value, carry)
        } else {
            let rm = (opcode & 0xF) as usize;
            let value = self.reg(rm);
            if bits(4, 4) != 0 {
                let rs = bits(8, 11) as usize;
                let amount = self.reg(rs) & 0xFF;
                self.cpu.cycles += 1;
                self.shift_value(value, bits(5, 6), amount, false)
            } else {
                self.shift_value(value, bits(5, 6), bits(7, 11), true)
            }
        };
        let a = self.reg(rn);
        if std::env::var_os("ST018_CALL_LOG").is_some() && opcode == 0xE08F_1001 {
            eprintln!(
                "ADDDBG a={a:08X} operand={operand:08X} rn={rn} rd={rd} op={op:08X} imm={immediate}"
            );
        }
        let (result, carry) = match op {
            0x0 => (a & operand, shift_carry),              // AND
            0x1 => (a ^ operand, shift_carry),              // EOR
            0x2 => self.sub(a, operand, true),              // SUB
            0x3 => self.sub(operand, a, true),              // RSB
            0x4 => self.add(a, operand, false),             // ADD
            0x5 => self.add(a, operand, self.cpu.flag_c()), // ADC
            0x6 => self.sub(a, operand, self.cpu.flag_c()), // SBC
            0x7 => self.sub(operand, a, self.cpu.flag_c()), // RSC
            0x8 => (a & operand, shift_carry),              // TST
            0x9 => (a ^ operand, shift_carry),              // TEQ
            0xA => self.sub(a, operand, true),              // CMP
            0xB => self.add(a, operand, false),             // CMN
            0xC => (a | operand, shift_carry),              // ORR
            0xD => (operand, shift_carry),                  // MOV
            0xE => (a & !operand, shift_carry),             // BIC
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
                // `MUL Rd, Rm, Rs` / `MLA Rd, Rm, Rs, Rn`: the product is
                // Rm * Rs and Rn is the accumulate operand (reference
                // ArmMultiply: mul(R(rm), R(rs)), mla(R(rm), R(rs), R(rn))).
                let rd = bits(16, 19) as usize;
                let rn = bits(12, 15) as usize;
                let rs = bits(8, 11) as usize;
                let rm = (opcode & 0xF) as usize;
                let mut result = self.reg(rm).wrapping_mul(self.reg(rs));
                if accumulate {
                    result = result.wrapping_add(self.reg(rn));
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
                    // Reference `ArmMsr`: the operand is immediate when bit 25
                    // is set (rotated by bits 8-11), and only the fields named
                    // by the mask (bits 16-19) are written. Mask bit 3 selects
                    // the condition flags.
                    let immediate = bits(25, 25) != 0;
                    let mask = bits(16, 19);
                    let raw = if immediate {
                        let rotate = bits(8, 11) * 2;
                        bits(0, 7).rotate_right(rotate)
                    } else {
                        self.cpu.r[opcode as usize & 0xF]
                    };
                    if mask & 0b1000 != 0 {
                        self.cpu.set_flags(
                            raw & 0x8000_0000 != 0,
                            raw & 0x4000_0000 != 0,
                            raw & 0x2000_0000 != 0,
                            raw & 0x1000_0000 != 0,
                        );
                    }
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
            self.shift_value(value, bits(5, 6), amount, true).0
        };
        let base = self.reg(rn);
        if std::env::var_os("ST018_CALL_LOG").is_some() && opcode == 0xE551_2000 {
            eprintln!(
                "LDRBDBG rn={rn} base={base:08X} offset={offset:08X} pre={pre} up={up} wb={writeback} r1={:08X}",
                self.cpu.r[1]
            );
        }
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
        // Write-back is applied when the first listed register is processed, so
        // a load into `rn` overwrites it (reference ArmBlockDataTransfer).
        let wb_value = if up {
            base.wrapping_add(count * 4)
        } else {
            base.wrapping_sub(count * 4)
        };
        let mut addr_first = true;
        let mut address = start;
        for index in 0..16 {
            if list & (1 << index) == 0 {
                continue;
            }
            if addr_first {
                if writeback {
                    self.cpu.r[rn] = wb_value;
                }
                addr_first = false;
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
        base = wb_value;
        let _ = base;
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
        assert_eq!(chip.status() & 0x08, 0x08);
        // The ARM consumes it (via the $04:0010 register) and returns one.
        let value = chip.arm_read32(0x04_000010);
        assert_eq!(value & 0xFF, 0x5A);
        assert_eq!(chip.status() & 0x08, 0x00);
        chip.arm_write8(0x0400_0000, 0xA5);
        assert_eq!(chip.status() & 0x01, 0x01);
        assert_eq!(chip.read(0x00_3800, 0), 0xA5);
        assert_eq!(chip.status() & 0x01, 0x00);
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
        chip.arm_write32(0xE000_0100, 0x1234_5678);
        assert_eq!(chip.arm_read32(0xE000_0100), 0x1234_5678);
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
    fn add_does_not_carry_in() {
        // ARM at 0: MOV r0,#0 ; ADD r0,r0,#1 ; B .
        // `ADD` must not add the carry flag (that is `ADC`); this caught a real
        // bug where ADD/CMN were implemented with carry-in set, producing
        // 0x1D4+0x68+1 instead of 0x23C in the firmware's table lookup.
        let mut image = vec![0u8; FIRMWARE_SIZE];
        let program: [u32; 3] = [0xE3A0_0000, 0xE280_0001, 0xEAFF_FFFE];
        for (index, word) in program.iter().enumerate() {
            image[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let mut chip = St018::new(&image);
        chip.write(0x00_3804, 0x80);
        chip.write(0x00_3804, 0x00); // falling edge: power on at PC 0
        chip.run_to(4);
        assert_eq!(chip.arm_register(0), 1);
    }

    fn run_program(words: &[u32], cycles: u64) -> St018 {
        let mut image = vec![0u8; FIRMWARE_SIZE];
        for (index, word) in words.iter().enumerate() {
            image[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let mut chip = St018::new(&image);
        chip.write(0x00_3804, 0x80);
        chip.write(0x00_3804, 0x00);
        chip.run_to(cycles);
        chip
    }

    #[test]
    fn shifter_matches_armv3_semantics() {
        // MOV r0,#1 ; MOV r0,r0,LSL #4 ; MOV r1,#0x81 ; MOV r1,r1,LSR #1
        // ; MOV r2,#0x80000001 ; MOV r3,r2,ROR #1 ; B .
        let chip = run_program(
            &[
                0xE3A0_0001,
                0xE1A0_0200, // MOV r0, r0, LSL #4
                0xE3A0_1081, // MOV r1, #0x81
                0xE1A0_10A1, // MOV r1, r1, LSR #1
                0xE3A0_2102, // MOV r2, #0x80000001
                0xE1A0_30E2, // MOV r3, r2, ROR #1
                0xEAFF_FFFE,
            ],
            16,
        );
        assert_eq!(chip.arm_register(0), 0x10, "LSL #4");
        assert_eq!(chip.arm_register(1), 0x40, "LSR #1");
        assert_eq!(chip.arm_register(3), 0xC000_0000, "ROR #1");
    }

    #[test]
    fn logical_op_carry_comes_from_the_shifter() {
        // MOV r0,#1 ; MOVS r1,r0,LSL #31  (carry out = 1) ; B .
        let chip = run_program(&[0xE3A0_0001, 0xE1B0_10C0, 0xEAFF_FFFE], 8);
        // C is bit 29 of the 26-bit r15 (0x2000_0000).
        assert_ne!(chip.arm_register(15) & 0x2000_0000, 0, "C from shifter");
        assert_ne!(chip.arm_register(15) & 0x8000_0000, 0, "N set (bit31)");
    }

    #[test]
    fn multiply_uses_rm_and_rs() {
        // MOV r2,#3 ; MOV r3,#5 ; MUL r0,r2,r3 ; B .
        let chip = run_program(&[0xE3A0_2003, 0xE3A0_3005, 0xE000_0392, 0xEAFF_FFFE], 10);
        assert_eq!(chip.arm_register(0), 15, "MUL r0 = r2 * r3");
    }

    #[test]
    fn multiply_accumulate_uses_rn() {
        // MOV r0,#100 ; MOV r2,#3 ; MOV r3,#5 ; MLA r0,r2,r3,r0 ; B .
        let chip = run_program(
            &[
                0xE3A0_0064,
                0xE3A0_2003,
                0xE3A0_3005,
                0xE020_0392,
                0xEAFF_FFFE,
            ],
            14,
        );
        assert_eq!(chip.arm_register(0), 115, "MLA r0 += r2 * r3");
    }

    #[test]
    fn msr_respects_the_field_mask() {
        // MOV r0,#0xF0000000 ; MSR CPSR_c, r0 (mask 0b1001) ; B .
        let chip = run_program(&[0xE3A0_04F0, 0xE129_F000, 0xEAFF_FFFE], 8);
        assert_ne!(chip.arm_register(15) & 0x8000_0000, 0, "N set by MSR");
        assert_ne!(chip.arm_register(15) & 0x2000_0000, 0, "C set by MSR");
    }

    #[test]
    fn ldm_writeback_loses_to_a_loaded_base_register() {
        let chip = run_program(
            &[
                0xE3A0_020E, // MOV r0, #0xE0000000
                0xE280_0C01, // ADD r0, r0, #0x100
                0xE3A0_2055, // MOV r2, #0x55
                0xE580_2000, // STR r2, [r0]
                0xE8B0_0003, // LDMIA r0!, {r0, r1}
                0xEAFF_FFFE,
            ],
            80,
        );
        assert_eq!(chip.arm_register(2), 0x55, "r2 = 0x55");
        assert_eq!(chip.work_ram()[0x100], 0x55, "STR wrote RAM");
        assert_eq!(
            chip.arm_register(0),
            0x55,
            "loaded value wins over write-back"
        );
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
