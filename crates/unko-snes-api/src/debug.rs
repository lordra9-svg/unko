//! Optional SNES debugger snapshots shared by frontends and the host adapter.
//!
//! Every accessor is optional: a core without debug hooks returns `None`, so
//! the frontend can hide the panels. The types deliberately mirror
//! `unko_nes_api::debug` where the concept is shared (register/PPU snapshots,
//! a borrowed disassembly row) and otherwise describe the SNES machine.

/// Address space used by SNES debugger reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebugSpace {
    /// Work RAM, 128 KiB.
    Wram,
    /// Video RAM, 16-bit words addressed as bytes (64 KiB).
    Vram,
    /// Object attribute memory (544 bytes).
    Oam,
    /// Colour RAM, 15-bit words addressed as bytes (512 bytes).
    Cgram,
    /// S-SMP APU RAM (64 KiB).
    ApuRam,
    /// S-DSP register file (`$00-$7F`).
    DspRegisters,
    /// Cartridge ROM image bytes (file offsets, up to 8 MiB).
    Rom,
}

impl DebugSpace {
    /// Human label for the space.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Wram => "WRAM",
            Self::Vram => "VRAM",
            Self::Oam => "OAM",
            Self::Cgram => "CGRAM",
            Self::ApuRam => "APU RAM",
            Self::DspRegisters => "DSP",
            Self::Rom => "ROM",
        }
    }

    /// Number of addressable bytes in the space.
    #[must_use]
    pub const fn size(self) -> u32 {
        match self {
            Self::Wram => 0x2_0000,
            Self::Vram => 0x1_0000,
            Self::Oam => 0x220,
            Self::Cgram => 0x200,
            Self::ApuRam => 0x1_0000,
            Self::DspRegisters => 0x80,
            Self::Rom => 0x80_0000,
        }
    }

    /// Clamps an address into the space.
    #[must_use]
    pub const fn clamp(self, address: u32) -> u32 {
        let mask = self.size() - 1;
        address & mask
    }
}

/// Snapshot of the 65C816 registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuState {
    /// Accumulator (16-bit).
    pub a: u16,
    /// X index (16-bit).
    pub x: u16,
    /// Y index (16-bit).
    pub y: u16,
    /// Stack pointer.
    pub s: u16,
    /// Direct page register.
    pub d: u16,
    /// Program bank.
    pub pbr: u8,
    /// Data bank.
    pub dbr: u8,
    /// Program counter within the program bank.
    pub pc: u16,
    /// Packed processor status.
    pub p: u8,
    /// Emulation mode.
    pub emulation: bool,
    /// Master clocks retired since reset.
    pub cycles: u64,
}

/// Snapshot of the S-PPU position and display state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuState {
    /// Completed frames.
    pub frame: u64,
    /// Current scanline.
    pub line: u16,
    /// Current dot within the scanline.
    pub dot: u16,
    /// Output width in pixels (256 or 512).
    pub width: u32,
    /// Output height in pixels (224 or 240).
    pub height: u32,
    /// Hi-res mode.
    pub hires: bool,
    /// Currently in vblank.
    pub vblank: bool,
    /// INIDISP brightness (`0-15`).
    pub brightness: u8,
    /// INIDISP forced blank.
    pub forced_blank: bool,
}

/// Snapshot of the S-DSP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DspState {
    /// Output sample rate.
    pub sample_rate: u32,
    /// Finished output samples since reset.
    pub samples: u64,
    /// Raw register file `$00-$7F`.
    pub registers: [u8; 128],
}

/// One decoded instruction for a disassembly listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
    /// Address of the opcode (24-bit).
    pub address: u32,
    /// Instruction bytes.
    pub bytes: Vec<u8>,
    /// Formatted instruction.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_clamp_and_size() {
        assert_eq!(DebugSpace::Wram.size(), 0x2_0000);
        assert_eq!(DebugSpace::Wram.clamp(0x3_FFFF), 0x1_FFFF);
        assert_eq!(DebugSpace::Oam.clamp(0x1_0000), 0);
        assert_eq!(DebugSpace::DspRegisters.size(), 0x80);
        assert_eq!(DebugSpace::Vram.label(), "VRAM");
        assert_eq!(DebugSpace::Rom.label(), "ROM");
        assert_eq!(DebugSpace::Rom.clamp(0x80_0001), 1);
    }
}

/// 65816 addressing mode, used for operand length and formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Implied / no operand.
    Imp,
    /// Accumulator (`ASL A`).
    Acc,
    /// 8-bit immediate or signature byte.
    Imm,
    /// 16-bit immediate.
    Imm16,
    /// Direct page.
    Dp,
    /// Direct page indexed by X.
    DpX,
    /// Direct page indexed by Y.
    DpY,
    /// Absolute.
    Abs,
    /// Absolute indexed by X.
    AbsX,
    /// Absolute indexed by Y.
    AbsY,
    /// Long (24-bit).
    Long,
    /// Long indexed by X.
    LongX,
    /// `(dp,X)`.
    IndDpX,
    /// `(dp),Y`.
    IndDpY,
    /// `(dp)`.
    IndDp,
    /// `sr,S`.
    StackS,
    /// `(sr,S),Y`.
    IndStackSY,
    /// `[dp]`.
    IndLong,
    /// `[dp],Y`.
    IndLongY,
    /// 8-bit relative branch.
    Rel8,
    /// 16-bit relative branch.
    Rel16,
    /// `(abs)`.
    AbsInd,
    /// `(abs,X)`.
    AbsIndX,
    /// `[abs]`.
    AbsIndLong,
    /// Block move (`MVP/MVN`).
    BlockMove,
}

impl Mode {
    /// Number of bytes the instruction occupies.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::Imp | Self::Acc => 1,
            Self::Imm
            | Self::Dp
            | Self::DpX
            | Self::DpY
            | Self::IndDpX
            | Self::IndDpY
            | Self::IndDp
            | Self::StackS
            | Self::IndStackSY
            | Self::IndLong
            | Self::IndLongY
            | Self::Rel8 => 2,
            Self::Imm16
            | Self::Abs
            | Self::AbsX
            | Self::AbsY
            | Self::Rel16
            | Self::AbsInd
            | Self::AbsIndX
            | Self::AbsIndLong
            | Self::BlockMove => 3,
            Self::Long | Self::LongX => 4,
        }
    }
}

/// One entry of the 65816 opcode matrix.
#[derive(Debug, Clone, Copy)]
pub struct Opcode {
    /// Mnemonic.
    pub mnemonic: &'static str,
    /// Addressing mode.
    pub mode: Mode,
}

const fn op(mnemonic: &'static str, mode: Mode) -> Opcode {
    Opcode { mnemonic, mode }
}

/// Complete 65816 opcode matrix.
#[rustfmt::skip]
pub const OPCODES: [Opcode; 256] = [
    op("BRK", Mode::Imm), op("ORA", Mode::IndDpX), op("COP", Mode::Imm), op("ORA", Mode::StackS),
    op("TSB", Mode::Dp), op("ORA", Mode::Dp), op("ASL", Mode::Dp), op("ORA", Mode::IndLong),
    op("PHP", Mode::Imp), op("ORA", Mode::Imm), op("ASL", Mode::Acc), op("PHD", Mode::Imp),
    op("TSB", Mode::Abs), op("ORA", Mode::Abs), op("ASL", Mode::Abs), op("ORA", Mode::Long),
    op("BPL", Mode::Rel8), op("ORA", Mode::IndDpY), op("ORA", Mode::IndDp), op("ORA", Mode::IndStackSY),
    op("TRB", Mode::Dp), op("ORA", Mode::DpX), op("ASL", Mode::DpX), op("ORA", Mode::IndLongY),
    op("CLC", Mode::Imp), op("ORA", Mode::AbsY), op("INC", Mode::Acc), op("TCS", Mode::Imp),
    op("TRB", Mode::Abs), op("ORA", Mode::AbsX), op("ASL", Mode::AbsX), op("ORA", Mode::LongX),
    op("JSR", Mode::Abs), op("AND", Mode::IndDpX), op("JSL", Mode::Long), op("AND", Mode::StackS),
    op("BIT", Mode::Dp), op("AND", Mode::Dp), op("ROL", Mode::Dp), op("AND", Mode::IndLong),
    op("PLP", Mode::Imp), op("AND", Mode::Imm), op("ROL", Mode::Acc), op("PLD", Mode::Imp),
    op("BIT", Mode::Abs), op("AND", Mode::Abs), op("ROL", Mode::Abs), op("AND", Mode::Long),
    op("BMI", Mode::Rel8), op("AND", Mode::IndDpY), op("AND", Mode::IndDp), op("AND", Mode::IndStackSY),
    op("BIT", Mode::DpX), op("AND", Mode::DpX), op("ROL", Mode::DpX), op("AND", Mode::IndLongY),
    op("SEC", Mode::Imp), op("AND", Mode::AbsY), op("DEC", Mode::Acc), op("TSC", Mode::Imp),
    op("BIT", Mode::AbsX), op("AND", Mode::AbsX), op("ROL", Mode::AbsX), op("AND", Mode::LongX),
    op("RTI", Mode::Imp), op("EOR", Mode::IndDpX), op("WDM", Mode::Imm), op("EOR", Mode::StackS),
    op("MVP", Mode::BlockMove), op("EOR", Mode::Dp), op("LSR", Mode::Dp), op("EOR", Mode::IndLong),
    op("PHA", Mode::Imp), op("EOR", Mode::Imm), op("LSR", Mode::Acc), op("PHK", Mode::Imp),
    op("JMP", Mode::Abs), op("EOR", Mode::Abs), op("LSR", Mode::Abs), op("EOR", Mode::Long),
    op("BVC", Mode::Rel8), op("EOR", Mode::IndDpY), op("EOR", Mode::IndDp), op("EOR", Mode::IndStackSY),
    op("MVN", Mode::BlockMove), op("EOR", Mode::DpX), op("LSR", Mode::DpX), op("EOR", Mode::IndLongY),
    op("CLI", Mode::Imp), op("EOR", Mode::AbsY), op("PHY", Mode::Imp), op("TCD", Mode::Imp),
    op("JMP", Mode::Long), op("EOR", Mode::AbsX), op("LSR", Mode::AbsX), op("EOR", Mode::LongX),
    op("RTS", Mode::Imp), op("ADC", Mode::IndDpX), op("PER", Mode::Rel16), op("ADC", Mode::StackS),
    op("STZ", Mode::Dp), op("ADC", Mode::Dp), op("ROR", Mode::Dp), op("ADC", Mode::IndLong),
    op("PLA", Mode::Imp), op("ADC", Mode::Imm), op("ROR", Mode::Acc), op("RTL", Mode::Imp),
    op("JMP", Mode::AbsInd), op("ADC", Mode::Abs), op("ROR", Mode::Abs), op("ADC", Mode::Long),
    op("BVS", Mode::Rel8), op("ADC", Mode::IndDpY), op("ADC", Mode::IndDp), op("ADC", Mode::IndStackSY),
    op("STZ", Mode::DpX), op("ADC", Mode::DpX), op("ROR", Mode::DpX), op("ADC", Mode::IndLongY),
    op("SEI", Mode::Imp), op("ADC", Mode::AbsY), op("PLY", Mode::Imp), op("TDC", Mode::Imp),
    op("JMP", Mode::AbsIndX), op("ADC", Mode::AbsX), op("ROR", Mode::AbsX), op("ADC", Mode::LongX),
    op("BRA", Mode::Rel8), op("STA", Mode::IndDpX), op("BRL", Mode::Rel16), op("STA", Mode::StackS),
    op("STY", Mode::Dp), op("STA", Mode::Dp), op("STX", Mode::Dp), op("STA", Mode::IndLong),
    op("DEY", Mode::Imp), op("BIT", Mode::Imm), op("TXA", Mode::Imp), op("PHB", Mode::Imp),
    op("STY", Mode::Abs), op("STA", Mode::Abs), op("STX", Mode::Abs), op("STA", Mode::Long),
    op("BCC", Mode::Rel8), op("STA", Mode::IndDpY), op("STA", Mode::IndDp), op("STA", Mode::IndStackSY),
    op("STY", Mode::DpX), op("STA", Mode::DpX), op("STX", Mode::DpY), op("STA", Mode::IndLongY),
    op("TYA", Mode::Imp), op("STA", Mode::AbsY), op("TXS", Mode::Imp), op("TXY", Mode::Imp),
    op("STZ", Mode::Abs), op("STA", Mode::AbsX), op("STZ", Mode::AbsX), op("STA", Mode::LongX),
    op("LDY", Mode::Imm), op("LDA", Mode::IndDpX), op("LDX", Mode::Imm), op("LDA", Mode::StackS),
    op("LDY", Mode::Dp), op("LDA", Mode::Dp), op("LDX", Mode::Dp), op("LDA", Mode::IndLong),
    op("TAY", Mode::Imp), op("LDA", Mode::Imm), op("TAX", Mode::Imp), op("PLB", Mode::Imp),
    op("LDY", Mode::Abs), op("LDA", Mode::Abs), op("LDX", Mode::Abs), op("LDA", Mode::Long),
    op("BCS", Mode::Rel8), op("LDA", Mode::IndDpY), op("LDA", Mode::IndDp), op("LDA", Mode::IndStackSY),
    op("LDY", Mode::DpX), op("LDA", Mode::DpX), op("LDX", Mode::DpY), op("LDA", Mode::IndLongY),
    op("CLV", Mode::Imp), op("LDA", Mode::AbsY), op("TSX", Mode::Imp), op("TYX", Mode::Imp),
    op("LDY", Mode::AbsX), op("LDA", Mode::AbsX), op("LDX", Mode::AbsY), op("LDA", Mode::LongX),
    op("CPY", Mode::Imm), op("CMP", Mode::IndDpX), op("REP", Mode::Imm), op("CMP", Mode::StackS),
    op("CPY", Mode::Dp), op("CMP", Mode::Dp), op("DEC", Mode::Dp), op("CMP", Mode::IndLong),
    op("INY", Mode::Imp), op("CMP", Mode::Imm), op("DEX", Mode::Imp), op("WAI", Mode::Imp),
    op("CPY", Mode::Abs), op("CMP", Mode::Abs), op("DEC", Mode::Abs), op("CMP", Mode::Long),
    op("BNE", Mode::Rel8), op("CMP", Mode::IndDpY), op("CMP", Mode::IndDp), op("CMP", Mode::IndStackSY),
    op("PEI", Mode::IndDp), op("CMP", Mode::DpX), op("DEC", Mode::DpX), op("CMP", Mode::IndLongY),
    op("CLD", Mode::Imp), op("CMP", Mode::AbsY), op("PHX", Mode::Imp), op("STP", Mode::Imp),
    op("JMP", Mode::AbsIndLong), op("CMP", Mode::AbsX), op("DEC", Mode::AbsX), op("CMP", Mode::LongX),
    op("CPX", Mode::Imm), op("SBC", Mode::IndDpX), op("SEP", Mode::Imm), op("SBC", Mode::StackS),
    op("CPX", Mode::Dp), op("SBC", Mode::Dp), op("INC", Mode::Dp), op("SBC", Mode::IndLong),
    op("INX", Mode::Imp), op("SBC", Mode::Imm), op("NOP", Mode::Imp), op("XBA", Mode::Imp),
    op("CPX", Mode::Abs), op("SBC", Mode::Abs), op("INC", Mode::Abs), op("SBC", Mode::Long),
    op("BEQ", Mode::Rel8), op("SBC", Mode::IndDpY), op("SBC", Mode::IndDp), op("SBC", Mode::IndStackSY),
    op("PEA", Mode::Imm16), op("SBC", Mode::DpX), op("INC", Mode::DpX), op("SBC", Mode::IndLongY),
    op("SED", Mode::Imp), op("SBC", Mode::AbsY), op("PLX", Mode::Imp), op("XCE", Mode::Imp),
    op("JSR", Mode::AbsIndX), op("SBC", Mode::AbsX), op("INC", Mode::AbsX), op("SBC", Mode::LongX),
];

/// Disassembles one instruction at `address`, fetching operand bytes with
/// `fetch` (a side-effect-free peek of the 24-bit CPU address space).
#[must_use]
pub fn disassemble<F>(address: u32, mut fetch: F) -> Disassembly
where
    F: FnMut(u32) -> u8,
{
    let opcode = OPCODES[fetch(address) as usize];
    let mode = opcode.mode;
    let len = mode.bytes();
    let mut bytes = Vec::with_capacity(len);
    for offset in 0..len as u32 {
        bytes.push(fetch(address.wrapping_add(offset)));
    }
    let b1 = bytes.get(1).copied().unwrap_or(0);
    let b2 = bytes.get(2).copied().unwrap_or(0);
    let b3 = bytes.get(3).copied().unwrap_or(0);
    let abs = u16::from_le_bytes([b1, b2]);
    let long = u32::from(b1) | (u32::from(b2) << 8) | (u32::from(b3) << 16);
    let text = match mode {
        Mode::Imp => opcode.mnemonic.to_owned(),
        Mode::Acc => format!("{} A", opcode.mnemonic),
        Mode::Imm => format!("{} #${b1:02X}", opcode.mnemonic),
        Mode::Imm16 => format!("{} ${abs:04X}", opcode.mnemonic),
        Mode::Dp => format!("{} ${b1:02X}", opcode.mnemonic),
        Mode::DpX => format!("{} ${b1:02X},X", opcode.mnemonic),
        Mode::DpY => format!("{} ${b1:02X},Y", opcode.mnemonic),
        Mode::Abs => format!("{} ${abs:04X}", opcode.mnemonic),
        Mode::AbsX => format!("{} ${abs:04X},X", opcode.mnemonic),
        Mode::AbsY => format!("{} ${abs:04X},Y", opcode.mnemonic),
        Mode::Long => format!("{} ${long:06X}", opcode.mnemonic),
        Mode::LongX => format!("{} ${long:06X},X", opcode.mnemonic),
        Mode::IndDpX => format!("{} (${b1:02X},X)", opcode.mnemonic),
        Mode::IndDpY => format!("{} (${b1:02X}),Y", opcode.mnemonic),
        Mode::IndDp => format!("{} (${b1:02X})", opcode.mnemonic),
        Mode::StackS => format!("{} ${b1:02X},S", opcode.mnemonic),
        Mode::IndStackSY => format!("{} (${b1:02X},S),Y", opcode.mnemonic),
        Mode::IndLong => format!("{} [${b1:02X}]", opcode.mnemonic),
        Mode::IndLongY => format!("{} [${b1:02X}],Y", opcode.mnemonic),
        Mode::Rel8 => {
            let target = (address as i64 + 2 + i64::from(b1 as i8)) as u32;
            format!("{} ${:04X}", opcode.mnemonic, target & 0xFFFF)
        }
        Mode::Rel16 => {
            let disp = i16::from_le_bytes([b1, b2]);
            let target = (address as i64 + 3 + i64::from(disp)) as u32;
            format!("{} ${:04X}", opcode.mnemonic, target & 0xFFFF)
        }
        Mode::AbsInd => format!("{} (${abs:04X})", opcode.mnemonic),
        Mode::AbsIndX => format!("{} (${abs:04X},X)", opcode.mnemonic),
        Mode::AbsIndLong => format!("{} [${abs:04X}]", opcode.mnemonic),
        Mode::BlockMove => format!("{} ${b1:02X},${b2:02X}", opcode.mnemonic),
    };
    Disassembly {
        address,
        bytes,
        text,
    }
}

#[cfg(test)]
mod disassembly_tests {
    use super::*;

    fn bytes_at(program: &[u8]) -> impl FnMut(u32) -> u8 + '_ {
        move |address| program.get(address as usize).copied().unwrap_or(0)
    }

    #[test]
    fn disassembles_common_instructions() {
        let program = [
            0xA9, 0x10, // LDA #$10
            0xAD, 0x34, 0x12, // LDA $1234
            0xAF, 0x56, 0x34, 0x12, // LDA $123456
            0x20, 0x00, 0x80, // JSR $8000
            0xD0, 0xFE, // BNE -2
            0xF4, 0x34, 0x12, // PEA $1234
            0x6B, // RTL
        ];
        let mut address = 0u32;
        let mut rows = Vec::new();
        while address < program.len() as u32 {
            let row = disassemble(address, bytes_at(&program));
            address += row.bytes.len() as u32;
            rows.push(row.text);
        }
        assert_eq!(
            rows,
            vec![
                "LDA #$10",
                "LDA $1234",
                "LDA $123456",
                "JSR $8000",
                "BNE $000C",
                "PEA $1234",
                "RTL",
            ]
        );
    }

    #[test]
    fn lengths_and_modes_are_consistent() {
        assert_eq!(OPCODES[0x00].mode, Mode::Imm); // BRK signature
        assert_eq!(Mode::Imp.bytes(), 1);
        assert_eq!(Mode::Long.bytes(), 4);
        assert_eq!(Mode::BlockMove.bytes(), 3);
        assert_eq!(OPCODES[0x4C].mnemonic, "JMP");
        assert_eq!(OPCODES[0x22].mode, Mode::Long);
        assert_eq!(OPCODES[0xDC].mode, Mode::AbsIndLong);
        assert_eq!(OPCODES[0xFC].mode, Mode::AbsIndX);
    }
}
