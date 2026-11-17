//! NEC uPD7725 (DSP-1) coprocessor.
//!
//! A port of the ares `uPD96050` component restricted to the `uPD7725` revision
//! (the DSP-1/1A/1B, DSP-2/3/4). The chip is a 24-bit-word RISC core with a
//! 2K-word program ROM, a 1K-word data ROM and 256 words of data RAM. The SNES
//! only sees two registers, the data register (DR) and the status register
//! (SR), mapped into a board-dependent window.
//!
//! The DSP free-runs on its own 7.6 MHz oscillator. We tick it from the S-CPU
//! master clock with a rational credit, like the APU/PPU; host reads and writes
//! exchange the current DR/SR and the firmware handshakes through RQM/DRS.

use crate::rom::Mapping;

/// Status-register bits (the 16-bit SR image).
const SR_P0: u16 = 1 << 0;
const SR_P1: u16 = 1 << 1;
const SR_EI: u16 = 1 << 7;
const SR_SIC: u16 = 1 << 8;
const SR_SOC: u16 = 1 << 9;
const SR_DRC: u16 = 1 << 10;
const SR_DMA: u16 = 1 << 11;
const SR_DRS: u16 = 1 << 12;
const SR_USF0: u16 = 1 << 13;
const SR_USF1: u16 = 1 << 14;
const SR_RQM: u16 = 1 << 15;

/// The S-CPU window a DSP-1 board maps the DR/SR ports into.
///
/// Selected with the de-facto map-mode/ROM-size heuristic used by bsnes/snes9x
/// when no cartridge database is present (RustySNES documents the same table);
/// it coincides with every ares DSP-1 board definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DspWindow {
    /// HiROM: `$00-$1F,$80-$9F : $6000-$7FFF`; SR when address bit 12 is set.
    HiRom,
    /// LoROM with SRAM, ROM <= 1 MiB: `$20-$3F,$A0-$BF : $8000-$FFFF`; SR when
    /// bit 14 is set (ares `NEC-LOROM-RAM#A`).
    LoRomWide,
    /// LoROM without SRAM: `$30-$3F,$B0-$BF : $8000-$FFFF`; SR when bit 14 is
    /// set (ares `NEC-LOROM`).
    LoRomSmall,
    /// LoROM, ROM > 1 MiB: `$60-$6F,$E0-$EF : $0000-$7FFF`; SR when bit 14 is set.
    LoRomLarge,
}

impl DspWindow {
    /// Pick the window for a cartridge's mapping, ROM size and SRAM size.
    ///
    /// This follows ares/mia's fallback board choice: a LoROM DSP cart with
    /// battery RAM of at most 1 MiB uses the wide `$20-$3F` window, a plain
    /// LoROM cart uses `$30-$3F`, and a >1 MiB LoROM cart uses `$60-$6F`.
    pub const fn select(mapping: Mapping, rom_len: usize, sram_len: usize) -> Self {
        match mapping {
            Mapping::HiRom | Mapping::ExHiRom => Self::HiRom,
            Mapping::LoRom | Mapping::ExLoRom => {
                if rom_len > 0x10_0000 {
                    Self::LoRomLarge
                } else if sram_len != 0 {
                    Self::LoRomWide
                } else {
                    Self::LoRomSmall
                }
            }
        }
    }

    /// Classify a 24-bit S-CPU address.
    ///
    /// Returns `Some(true)` for the SR port, `Some(false)` for the DR port and
    /// `None` when the address is outside the DSP window. The lower page of the
    /// window is DR and the upper page SR because ares maps the window with a
    /// mask covering the page-select bits (`reduce(addr, mask)` leaves the page
    /// number, and `NECDSP::read` selects SR on its low bit).
    pub fn classify(self, address: u32) -> Option<bool> {
        let bank = (address >> 16) & 0x7F;
        let addr = address & 0xFFFF;
        match self {
            Self::HiRom => {
                if (0x00..=0x1F).contains(&bank) && (0x6000..=0x7FFF).contains(&addr) {
                    Some(addr & 0x1000 != 0)
                } else {
                    None
                }
            }
            Self::LoRomWide => {
                if (0x20..=0x3F).contains(&bank) && (0x8000..=0xFFFF).contains(&addr) {
                    Some(addr & 0x4000 != 0)
                } else {
                    None
                }
            }
            Self::LoRomSmall => {
                if (0x30..=0x3F).contains(&bank) && (0x8000..=0xFFFF).contains(&addr) {
                    Some(addr & 0x4000 != 0)
                } else {
                    None
                }
            }
            Self::LoRomLarge => {
                if (0x60..=0x6F).contains(&bank) && (0x0000..=0x7FFF).contains(&addr) {
                    Some(addr & 0x4000 != 0)
                } else {
                    None
                }
            }
        }
    }
}

/// Errors from installing DSP-1 firmware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DspError {
    /// The firmware image is not the expected 8 KiB.
    BadFirmwareSize {
        /// Expected size in bytes.
        expected: usize,
        /// Supplied size in bytes.
        found: usize,
    },
}

impl std::fmt::Display for DspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadFirmwareSize { expected, found } => {
                write!(f, "DSP-1 firmware is {found} bytes, expected {expected}")
            }
        }
    }
}

impl std::error::Error for DspError {}

/// One of the two ALU flag sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DspFlags {
    /// Overflow 0.
    pub ov0: bool,
    /// Overflow 1.
    pub ov1: bool,
    /// Zero.
    pub z: bool,
    /// Carry.
    pub c: bool,
    /// Sign 0.
    pub s0: bool,
    /// Sign 1.
    pub s1: bool,
}

/// The uPD7725 register file.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DspRegisters {
    /// 16-entry LIFO call stack.
    pub stack: [u16; 16],
    /// Program counter (11 bits).
    pub pc: u16,
    /// Data-ROM pointer (10 bits).
    pub rp: u16,
    /// Data-RAM pointer (8 bits).
    pub dp: u16,
    /// Stack pointer (4 bits).
    pub sp: u8,
    /// Serial input.
    pub si: u16,
    /// Serial output.
    pub so: u16,
    /// ALU operand K.
    pub k: i16,
    /// ALU operand L.
    pub l: i16,
    /// Multiply result, high.
    pub m: i16,
    /// Multiply result, low.
    pub n: i16,
    /// Accumulator A.
    pub a: i16,
    /// Accumulator B.
    pub b: i16,
    /// Temporary register.
    pub tr: u16,
    /// Temporary register B.
    pub trb: u16,
    /// Data register.
    pub dr: u16,
    /// Raw status-register image.
    pub sr: u16,
    /// Serial input acknowledge (not driven).
    pub siack: bool,
    /// Serial output acknowledge (not driven).
    pub soack: bool,
}

/// The DSP-1 (uPD7725) core and its host ports.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NecDsp {
    /// 2K x 24-bit program ROM.
    pub program: Vec<u32>,
    /// 1K x 16-bit data ROM.
    pub data_rom: Vec<u16>,
    /// 256 x 16-bit data RAM.
    pub data_ram: Vec<u16>,
    /// Registers.
    pub regs: DspRegisters,
    /// A-side flags.
    pub flags_a: DspFlags,
    /// B-side flags.
    pub flags_b: DspFlags,
    /// Bus window selected for this cartridge.
    pub window: DspWindow,
    /// Number of program words executed (debug).
    pub ticks: u64,
}

impl NecDsp {
    /// Parse an 8 KiB `dsp1.rom` image (2K x 3-byte program then 1K x 2-byte
    /// data, both little-endian) and attach it to the supplied window.
    pub fn from_firmware(firmware: &[u8], window: DspWindow) -> Result<Self, DspError> {
        const PROGRAM_WORDS: usize = 2048;
        const DATA_WORDS: usize = 1024;
        const EXPECTED: usize = PROGRAM_WORDS * 3 + DATA_WORDS * 2;
        if firmware.len() != EXPECTED {
            return Err(DspError::BadFirmwareSize {
                expected: EXPECTED,
                found: firmware.len(),
            });
        }

        let mut program = Vec::with_capacity(PROGRAM_WORDS);
        for chunk in firmware[..PROGRAM_WORDS * 3].as_chunks::<3>().0 {
            program.push(
                u32::from(chunk[0]) | (u32::from(chunk[1]) << 8) | (u32::from(chunk[2]) << 16),
            );
        }
        let mut data_rom = Vec::with_capacity(DATA_WORDS);
        for chunk in firmware[PROGRAM_WORDS * 3..].as_chunks::<2>().0 {
            data_rom.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }

        let mut dsp = Self {
            program,
            data_rom,
            data_ram: vec![0; 256],
            regs: DspRegisters::default(),
            flags_a: DspFlags::default(),
            flags_b: DspFlags::default(),
            window,
            ticks: 0,
        };
        dsp.power();
        Ok(dsp)
    }

    /// Reset the core (ares `uPD96050::power` for the uPD7725 widths).
    pub fn power(&mut self) {
        self.regs = DspRegisters::default();
        self.flags_a = DspFlags::default();
        self.flags_b = DspFlags::default();
    }

    // --- Status register --------------------------------------------------

    /// Compose the SR image the S-CPU reads (ares `Status::operator u32`).
    ///
    /// ares hides the raw DRS latch when the data register is in 8-bit mode
    /// (`_drs = drs & !drc`); SIACK/SOACK are internal and are not part of the
    /// composed image.
    pub fn status(&self) -> u16 {
        let drs = self.regs.sr & SR_DRS != 0 && self.regs.sr & SR_DRC == 0;
        (self.regs.sr
            & (SR_P0
                | SR_P1
                | SR_EI
                | SR_SIC
                | SR_SOC
                | SR_DRC
                | SR_DMA
                | SR_USF0
                | SR_USF1
                | SR_RQM))
            | (u16::from(drs) << 12)
    }

    fn set_status(&mut self, value: u16) {
        const DEFINED: u16 = SR_P0
            | SR_P1
            | SR_EI
            | SR_SIC
            | SR_SOC
            | SR_DRC
            | SR_DMA
            | SR_DRS
            | SR_USF0
            | SR_USF1
            | SR_RQM;
        self.regs.sr = value & DEFINED;
    }

    fn sr_bit(&self, bit: u16) -> bool {
        self.regs.sr & bit != 0
    }

    fn set_sr_bit(&mut self, bit: u16, value: bool) {
        if value {
            self.regs.sr |= bit;
        } else {
            self.regs.sr &= !bit;
        }
    }

    // --- Host ports -------------------------------------------------------

    /// Read the S-CPU-visible port: `true` selects SR, `false` selects DR.
    pub fn read_port(&mut self, status: bool) -> u8 {
        if status {
            (self.status() >> 8) as u8
        } else {
            self.read_dr()
        }
    }

    /// Write the S-CPU-visible port: `true` selects SR, `false` selects DR.
    pub fn write_port(&mut self, status: bool, data: u8) {
        if status {
            // ares `writeSR` is a no-op: the DSP owns the status register.
        } else {
            self.write_dr(data);
        }
    }

    fn read_dr(&mut self) -> u8 {
        if !self.sr_bit(SR_DRC) {
            // 16-bit data register: two byte reads complete one word.
            if !self.sr_bit(SR_DRS) {
                self.set_sr_bit(SR_DRS, true);
                self.regs.dr as u8
            } else {
                self.set_sr_bit(SR_RQM, false);
                self.set_sr_bit(SR_DRS, false);
                (self.regs.dr >> 8) as u8
            }
        } else {
            self.set_sr_bit(SR_RQM, false);
            self.regs.dr as u8
        }
    }

    fn write_dr(&mut self, data: u8) {
        if !self.sr_bit(SR_DRC) {
            if !self.sr_bit(SR_DRS) {
                self.set_sr_bit(SR_DRS, true);
                self.regs.dr = (self.regs.dr & 0xFF00) | u16::from(data);
            } else {
                self.set_sr_bit(SR_RQM, false);
                self.set_sr_bit(SR_DRS, false);
                self.regs.dr = (u16::from(data) << 8) | (self.regs.dr & 0x00FF);
            }
        } else {
            self.set_sr_bit(SR_RQM, false);
            self.regs.dr = (self.regs.dr & 0xFF00) | u16::from(data);
        }
    }

    // --- Execution --------------------------------------------------------

    /// Execute one program word (ares `uPD96050::exec`).
    pub fn exec(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);
        let opcode = self.program[usize::from(self.regs.pc & 0x7FF)];
        self.regs.pc = (self.regs.pc + 1) & 0x7FF;
        match opcode >> 22 {
            0 => self.exec_op(opcode),
            1 => self.exec_rt(opcode),
            2 => self.exec_jp(opcode),
            _ => self.exec_ld(opcode),
        }
        let result = i32::from(self.regs.k) * i32::from(self.regs.l);
        self.regs.m = (result >> 15) as i16;
        self.regs.n = (result << 1) as i16;
    }

    fn exec_op(&mut self, opcode: u32) {
        let pselect = (opcode >> 20) & 0x3;
        let alu = (opcode >> 16) & 0xF;
        let asl = (opcode >> 15) & 1;
        let dpl = (opcode >> 13) & 0x3;
        let dphm = (opcode >> 9) & 0xF;
        let rpdcr = (opcode >> 8) & 1;
        let src = (opcode >> 4) & 0xF;
        let dst = opcode & 0xF;

        let idb = match src {
            0 => self.regs.trb,
            1 => self.regs.a as u16,
            2 => self.regs.b as u16,
            3 => self.regs.tr,
            4 => self.regs.dp,
            5 => self.regs.rp,
            6 => self.data_rom[usize::from(self.regs.rp & 0x3FF)],
            7 => 0x8000u16.wrapping_sub(u16::from(self.flags_a.s1)),
            8 => {
                let value = self.regs.dr;
                self.set_sr_bit(SR_RQM, true);
                value
            }
            9 => self.regs.dr,
            10 => self.status(),
            11 | 12 => self.regs.si,
            13 => self.regs.k as u16,
            14 => self.regs.l as u16,
            _ => self.data_ram[usize::from(self.regs.dp & 0xFF)],
        };

        if alu != 0 {
            let p = match pselect {
                0 => self.data_ram[usize::from(self.regs.dp & 0xFF)],
                1 => idb,
                2 => self.regs.m as u16,
                _ => self.regs.n as u16,
            };
            let (q, mut flag, c) = if asl == 0 {
                (self.regs.a as u16, self.flags_a, self.flags_b.c)
            } else {
                (self.regs.b as u16, self.flags_b, self.flags_a.c)
            };
            let mut p = p;

            let r = match alu {
                1 => q | p,
                2 => q & p,
                3 => q ^ p,
                4 => q.wrapping_sub(p),
                5 => q.wrapping_add(p),
                6 => q.wrapping_sub(p).wrapping_sub(u16::from(c)),
                7 => q.wrapping_add(p).wrapping_add(u16::from(c)),
                8 => {
                    p = 1;
                    q.wrapping_sub(1)
                }
                9 => {
                    p = 1;
                    q.wrapping_add(1)
                }
                10 => !q,
                11 => (q >> 1) | (q & 0x8000),
                12 => (q << 1) | u16::from(c),
                13 => (q << 2) | 3,
                14 => (q << 4) | 15,
                _ => q.rotate_right(8),
            };

            flag.z = r == 0;
            flag.s0 = r & 0x8000 != 0;
            if !flag.ov1 {
                flag.s1 = flag.s0;
            }

            match alu {
                1 | 2 | 3 | 10 | 13 | 14 | 15 => {
                    flag.ov0 = false;
                    flag.ov1 = false;
                    flag.c = false;
                }
                4..=9 => {
                    let carries = q ^ p ^ r;
                    let second = if alu & 1 != 0 { r } else { q };
                    let overflow = (q ^ r) & (p ^ second);
                    let ov0 = overflow & 0x8000 != 0;
                    flag.ov1 = if ov0 && flag.ov1 {
                        flag.s0 == flag.s1
                    } else {
                        ov0 || flag.ov1
                    };
                    flag.ov0 = ov0;
                    flag.c = (carries ^ overflow) & 0x8000 != 0;
                }
                11 => {
                    flag.ov0 = false;
                    flag.ov1 = false;
                    flag.c = q & 1 != 0;
                }
                12 => {
                    flag.ov0 = false;
                    flag.ov1 = false;
                    flag.c = q >> 15 != 0;
                }
                _ => {}
            }
            if asl == 0 {
                self.regs.a = r as i16;
                self.flags_a = flag;
            } else {
                self.regs.b = r as i16;
                self.flags_b = flag;
            }
        }

        self.exec_ld((u32::from(idb) << 6) | dst);

        if dst != 4 {
            self.regs.dp = match dpl {
                1 => (self.regs.dp & 0xF0) + ((self.regs.dp + 1) & 0x0F),
                2 => (self.regs.dp & 0xF0) + ((self.regs.dp.wrapping_sub(1)) & 0x0F),
                3 => self.regs.dp & 0xF0,
                _ => self.regs.dp,
            } & 0xFF;
            self.regs.dp ^= u16::from(dphm as u8) << 4;
            self.regs.dp &= 0xFF;
        }

        if dst != 5 && rpdcr != 0 {
            self.regs.rp = self.regs.rp.wrapping_sub(1) & 0x3FF;
        }
    }

    fn exec_rt(&mut self, opcode: u32) {
        self.exec_op(opcode);
        self.regs.sp = self.regs.sp.wrapping_sub(1) & 0x0F;
        self.regs.pc = self.regs.stack[usize::from(self.regs.sp)] & 0x7FF;
    }

    fn exec_jp(&mut self, opcode: u32) {
        let brch = (opcode >> 13) & 0x1FF;
        let na = (opcode >> 2) & 0x7FF;
        let bank = opcode & 0x3;
        let jp = u32::from(self.regs.pc & 0x2000) | (bank << 11) | na;

        let target = match brch {
            0x000 => u32::from(self.regs.so & 0x7FF), // JMPSO
            0x080 if !self.flags_a.c => jp,
            0x082 if self.flags_a.c => jp,
            0x084 if !self.flags_b.c => jp,
            0x086 if self.flags_b.c => jp,
            0x088 if !self.flags_a.z => jp,
            0x08A if self.flags_a.z => jp,
            0x08C if !self.flags_b.z => jp,
            0x08E if self.flags_b.z => jp,
            0x090 if !self.flags_a.ov0 => jp,
            0x092 if self.flags_a.ov0 => jp,
            0x094 if !self.flags_b.ov0 => jp,
            0x096 if self.flags_b.ov0 => jp,
            0x098 if !self.flags_a.ov1 => jp,
            0x09A if self.flags_a.ov1 => jp,
            0x09C if !self.flags_b.ov1 => jp,
            0x09E if self.flags_b.ov1 => jp,
            0x0A0 if !self.flags_a.s0 => jp,
            0x0A2 if self.flags_a.s0 => jp,
            0x0A4 if !self.flags_b.s0 => jp,
            0x0A6 if self.flags_b.s0 => jp,
            0x0A8 if !self.flags_a.s1 => jp,
            0x0AA if self.flags_a.s1 => jp,
            0x0AC if !self.flags_b.s1 => jp,
            0x0AE if self.flags_b.s1 => jp,
            0x0B0 if self.regs.dp & 0x0F == 0x00 => jp,
            0x0B1 if self.regs.dp & 0x0F != 0x00 => jp,
            0x0B2 if self.regs.dp & 0x0F == 0x0F => jp,
            0x0B3 if self.regs.dp & 0x0F != 0x0F => jp,
            0x0B4 if !self.regs.siack => jp,
            0x0B6 if self.regs.siack => jp,
            0x0B8 if !self.regs.soack => jp,
            0x0BA if self.regs.soack => jp,
            0x0BC if !self.sr_bit(SR_RQM) => jp,
            0x0BE if self.sr_bit(SR_RQM) => jp,
            0x100 => jp & !0x2000,
            0x101 => jp | 0x2000,
            0x140 => {
                self.regs.stack[usize::from(self.regs.sp)] = self.regs.pc;
                self.regs.sp = self.regs.sp.wrapping_add(1) & 0x0F;
                jp & !0x2000
            }
            0x141 => {
                self.regs.stack[usize::from(self.regs.sp)] = self.regs.pc;
                self.regs.sp = self.regs.sp.wrapping_add(1) & 0x0F;
                jp | 0x2000
            }
            _ => return,
        };
        self.regs.pc = (target & 0x7FF) as u16;
    }

    fn exec_ld(&mut self, opcode: u32) {
        let id = ((opcode >> 6) & 0xFFFF) as u16;
        let dst = opcode & 0xF;
        match dst {
            0 => {}
            1 => self.regs.a = id as i16,
            2 => self.regs.b = id as i16,
            3 => self.regs.tr = id,
            4 => self.regs.dp = id & 0xFF,
            5 => self.regs.rp = id & 0x3FF,
            6 => {
                self.regs.dr = id;
                self.set_sr_bit(SR_RQM, true);
            }
            7 => {
                // ares: only the composed DRS/RQM bits are preserved; the rest
                // comes from the immediate.
                let value = (self.status() & 0x907C) | (id & !0x907C);
                self.set_status(value);
            }
            8 | 9 => self.regs.so = id,
            10 => self.regs.k = id as i16,
            11 => {
                self.regs.k = id as i16;
                self.regs.l = self.data_rom[usize::from(self.regs.rp & 0x3FF)] as i16;
            }
            12 => {
                self.regs.l = id as i16;
                self.regs.k = self.data_ram[usize::from(self.regs.dp & 0xFF | 0x40)] as i16;
            }
            13 => self.regs.l = id as i16,
            14 => self.regs.trb = id,
            _ => self.data_ram[usize::from(self.regs.dp & 0xFF)] = id,
        }
    }

    /// Read the internal data RAM through the uPD96050-style DP port.
    pub fn read_dp(&self, address: u16) -> u8 {
        let hi = address & 1 != 0;
        let index = (address >> 1) as usize & 0xFF;
        let word = self.data_ram[index];
        if hi {
            (word >> 8) as u8
        } else {
            word as u8
        }
    }

    /// Write the internal data RAM through the uPD96050-style DP port.
    pub fn write_dp(&mut self, address: u16, data: u8) {
        let hi = address & 1 != 0;
        let index = (address >> 1) as usize & 0xFF;
        if hi {
            self.data_ram[index] = (self.data_ram[index] & 0x00FF) | (u16::from(data) << 8);
        } else {
            self.data_ram[index] = (self.data_ram[index] & 0xFF00) | u16::from(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an 8 KiB firmware image with the given program/data words.
    fn firmware(program: &[u32], data: &[u16]) -> Vec<u8> {
        let mut out = vec![0u8; 2048 * 3 + 1024 * 2];
        for (index, &word) in program.iter().enumerate() {
            out[index * 3] = word as u8;
            out[index * 3 + 1] = (word >> 8) as u8;
            out[index * 3 + 2] = (word >> 16) as u8;
        }
        for (index, &word) in data.iter().enumerate() {
            let base = 2048 * 3 + index * 2;
            out[base..base + 2].copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    fn ld_a(value: u16) -> u32 {
        (3 << 22) | (u32::from(value) << 6) | 1
    }

    #[test]
    fn firmware_parses_program_and_data_roms() {
        let dsp = NecDsp::from_firmware(&firmware(&[ld_a(0x1234)], &[0xABCD]), DspWindow::HiRom)
            .expect("firmware");
        assert_eq!(dsp.program.len(), 2048);
        assert_eq!(dsp.data_rom.len(), 1024);
        assert_eq!(dsp.data_ram.len(), 256);
        assert_eq!(dsp.program[0], ld_a(0x1234));
        assert_eq!(dsp.data_rom[0], 0xABCD);
    }

    #[test]
    fn firmware_rejects_the_wrong_size() {
        assert_eq!(
            NecDsp::from_firmware(&[0; 8191], DspWindow::HiRom).unwrap_err(),
            DspError::BadFirmwareSize {
                expected: 8192,
                found: 8191
            }
        );
    }

    #[test]
    fn loads_and_runs_a_program() {
        // LD A,#$1234; LD DR,#$5678; then two more loads to leave a wait loop.
        let dsp = NecDsp::from_firmware(
            &firmware(&[ld_a(0x1234), ld_a(0x4321)], &[]),
            DspWindow::LoRomSmall,
        )
        .expect("firmware");
        let mut dsp = dsp;
        dsp.exec();
        assert_eq!(dsp.regs.a as u16, 0x1234);
        dsp.exec();
        assert_eq!(dsp.regs.a as u16, 0x4321);
    }

    #[test]
    fn dr_port_is_a_two_byte_handshake() {
        let mut dsp =
            NecDsp::from_firmware(&firmware(&[], &[]), DspWindow::HiRom).expect("firmware");
        // The firmware drives RQM when it wants the host. Set it directly so the
        // byte-pair protocol can be exercised in isolation.
        dsp.set_sr_bit(SR_RQM, true);
        // First write: low byte, DRS latches.
        dsp.write_port(false, 0x34);
        assert_eq!(dsp.regs.dr, 0x0034);
        assert!(dsp.sr_bit(SR_DRS));
        assert!(dsp.sr_bit(SR_RQM));
        // Second write: high byte, RQM/DRS clear.
        dsp.write_port(false, 0x12);
        assert_eq!(dsp.regs.dr, 0x1234);
        assert!(!dsp.sr_bit(SR_DRS));
        assert!(!dsp.sr_bit(SR_RQM));

        dsp.set_sr_bit(SR_RQM, true);
        assert_eq!(dsp.read_port(false), 0x34);
        assert!(dsp.sr_bit(SR_DRS));
        assert_eq!(dsp.read_port(false), 0x12);
        assert!(!dsp.sr_bit(SR_RQM));
    }

    #[test]
    fn status_reports_rqm_and_the_8bit_drs_mask() {
        let mut dsp =
            NecDsp::from_firmware(&firmware(&[], &[]), DspWindow::HiRom).expect("firmware");
        dsp.set_sr_bit(SR_RQM, true);
        assert_eq!(dsp.read_port(true) & 0x80, 0x80);
        // In 16-bit mode the raw DRS bit is reported.
        dsp.set_sr_bit(SR_DRS, true);
        assert_eq!(dsp.read_port(true) & 0x10, 0x10);
        // In 8-bit mode DRS is hidden from the composed status.
        dsp.set_sr_bit(SR_DRC, true);
        assert_eq!(dsp.read_port(true) & 0x10, 0);
    }

    #[test]
    fn window_selection_and_classification() {
        assert_eq!(
            DspWindow::select(Mapping::HiRom, 0x80000, 0x800),
            DspWindow::HiRom
        );
        assert_eq!(
            DspWindow::select(Mapping::LoRom, 0x20000, 0),
            DspWindow::LoRomSmall
        );
        assert_eq!(
            DspWindow::select(Mapping::LoRom, 0x10_0000, 0x8000),
            DspWindow::LoRomWide
        );
        assert_eq!(
            DspWindow::select(Mapping::LoRom, 0x20_0000, 0x8000),
            DspWindow::LoRomLarge
        );

        // HiROM: $00-$1F,$80-$9F:$6000-$7FFF, SR in the upper page.
        assert_eq!(DspWindow::HiRom.classify(0x00_6000), Some(false));
        assert_eq!(DspWindow::HiRom.classify(0x00_7000), Some(true));
        assert_eq!(DspWindow::HiRom.classify(0x9F_7FFF), Some(true));
        assert_eq!(DspWindow::HiRom.classify(0x00_5000), None);
        // LoROM wide: $20-$3F,$A0-$BF:$8000-$FFFF.
        assert_eq!(DspWindow::LoRomWide.classify(0xA0_8000), Some(false));
        assert_eq!(DspWindow::LoRomWide.classify(0x3F_C000), Some(true));
        assert_eq!(DspWindow::LoRomWide.classify(0x1F_8000), None);
        // LoROM small: $30-$3F,$B0-$BF:$8000-$FFFF.
        assert_eq!(DspWindow::LoRomSmall.classify(0xBF_8000), Some(false));
        assert_eq!(DspWindow::LoRomSmall.classify(0xBF_C000), Some(true));
        assert_eq!(DspWindow::LoRomSmall.classify(0x30_FFFF), Some(true));
        assert_eq!(DspWindow::LoRomSmall.classify(0x20_8000), None);
    }
}
