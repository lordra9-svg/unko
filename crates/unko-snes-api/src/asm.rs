//! Minimal 65816 assembler: one instruction at a time, numeric operands only.
//!
//! The encoder is the inverse of [`crate::debug::OPCODES`]: a mnemonic is
//! matched against every opcode with that name and the first whose addressing
//! mode fits the operand text wins. Branch operands are absolute targets and
//! are converted to a relative displacement from the instruction address.
//!
//! Labels, expressions and multi-line blocks are deliberately out of scope;
//! this exists for the debugger's "assemble and patch" field.

use std::fmt;

use crate::debug::{Mode, OPCODES};

/// Why an instruction could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsmError {
    /// The text had no mnemonic.
    Empty,
    /// The mnemonic is not part of the 65816 set.
    UnknownMnemonic(String),
    /// The operand shape or range matches no opcode for the mnemonic.
    InvalidOperand {
        /// Mnemonic that was being encoded.
        mnemonic: String,
        /// Operand text as understood by the encoder.
        operand: String,
    },
    /// A branch target is out of range for the mnemonic's displacement.
    BranchOutOfRange {
        /// Mnemonic.
        mnemonic: String,
        /// Requested absolute target.
        target: u32,
    },
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty instruction"),
            Self::UnknownMnemonic(mnemonic) => write!(f, "unknown mnemonic '{mnemonic}'"),
            Self::InvalidOperand { mnemonic, operand } => {
                write!(f, "operand '{operand}' does not fit {mnemonic}")
            }
            Self::BranchOutOfRange { mnemonic, target } => {
                write!(
                    f,
                    "branch target {target:06X} is out of range for {mnemonic}"
                )
            }
        }
    }
}

impl std::error::Error for AsmError {}

/// Assembles one instruction that would live at `address` (24-bit).
///
/// Operands use `$` or `0x` hex (decimal is accepted too). Branch operands are
/// absolute targets within the instruction's bank.
pub fn assemble(text: &str, address: u32) -> Result<Vec<u8>, AsmError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AsmError::Empty);
    }
    let (mnemonic, operand) = match text.split_once(char::is_whitespace) {
        Some((mnemonic, operand)) => (mnemonic.to_ascii_uppercase(), operand.trim()),
        None => (text.to_ascii_uppercase(), ""),
    };
    let compact: String = operand
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();

    let mut known = false;
    for (opcode, entry) in OPCODES.iter().enumerate() {
        if entry.mnemonic != mnemonic {
            continue;
        }
        known = true;
        if let Ok(encoded) = encode(entry.mode, &mnemonic, &compact, address) {
            let mut bytes = Vec::with_capacity(1 + encoded.len());
            bytes.push(opcode as u8);
            bytes.extend_from_slice(&encoded);
            return Ok(bytes);
        }
    }
    if !known {
        return Err(AsmError::UnknownMnemonic(mnemonic));
    }
    // Branch mnemonics only have a relative mode; report range separately.
    if let Some(entry) = OPCODES.iter().find(|entry| entry.mnemonic == mnemonic) {
        if matches!(entry.mode, Mode::Rel8 | Mode::Rel16) {
            if let Some(target) = value(&compact) {
                return Err(AsmError::BranchOutOfRange { mnemonic, target });
            }
        }
    }
    Err(AsmError::InvalidOperand {
        mnemonic,
        operand: compact,
    })
}

fn value(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('$') {
        return u32::from_str_radix(hex, 16).ok();
    }
    if let Some(hex) = text.strip_prefix("0X") {
        return u32::from_str_radix(hex, 16).ok();
    }
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        return text.parse().ok();
    }
    None
}

fn byte(value: u32) -> Option<Vec<u8>> {
    u8::try_from(value).ok().map(|value| vec![value])
}

fn word(value: u32) -> Option<Vec<u8>> {
    u16::try_from(value)
        .ok()
        .map(|value| value.to_le_bytes().to_vec())
}

fn long(value: u32) -> Option<Vec<u8>> {
    if value > 0x00FF_FFFF {
        return None;
    }
    Some(vec![value as u8, (value >> 8) as u8, (value >> 16) as u8])
}

fn encode(mode: Mode, mnemonic: &str, s: &str, address: u32) -> Result<Vec<u8>, ()> {
    let ok = |bytes: Option<Vec<u8>>| bytes.ok_or(());
    match mode {
        Mode::Imp => {
            if s.is_empty() {
                Ok(Vec::new())
            } else {
                Err(())
            }
        }
        Mode::Acc => {
            if s.is_empty() || s == "A" {
                Ok(Vec::new())
            } else {
                Err(())
            }
        }
        Mode::Imm => {
            if s.is_empty() && matches!(mnemonic, "BRK" | "COP" | "WDM") {
                return Ok(vec![0]);
            }
            let rest = s.strip_prefix('#').ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::Imm16 => {
            let rest = s.strip_prefix('#').unwrap_or(s);
            ok(word(value(rest).ok_or(())?))
        }
        Mode::Dp => ok(byte(value(s).ok_or(())?)),
        Mode::DpX => {
            let rest = s.strip_suffix(",X").ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::DpY => {
            let rest = s.strip_suffix(",Y").ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::Abs => ok(word(value(s).ok_or(())?)),
        Mode::AbsX => {
            let rest = s.strip_suffix(",X").ok_or(())?;
            ok(word(value(rest).ok_or(())?))
        }
        Mode::AbsY => {
            let rest = s.strip_suffix(",Y").ok_or(())?;
            ok(word(value(rest).ok_or(())?))
        }
        Mode::Long => ok(long(value(s).ok_or(())?)),
        Mode::LongX => {
            let rest = s.strip_suffix(",X").ok_or(())?;
            ok(long(value(rest).ok_or(())?))
        }
        Mode::IndDpX => {
            let rest = s
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(",X)"))
                .ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::IndDpY => {
            let rest = s
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix("),Y"))
                .ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::IndDp => {
            let rest = s
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::StackS => {
            let rest = s.strip_suffix(",S").ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::IndStackSY => {
            let rest = s
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(",S),Y"))
                .ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::IndLong => {
            let rest = s
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::IndLongY => {
            let rest = s
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix("],Y"))
                .ok_or(())?;
            ok(byte(value(rest).ok_or(())?))
        }
        Mode::AbsInd => {
            let rest = s
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .ok_or(())?;
            ok(word(value(rest).ok_or(())?))
        }
        Mode::AbsIndX => {
            let rest = s
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(",X)"))
                .ok_or(())?;
            ok(word(value(rest).ok_or(())?))
        }
        Mode::AbsIndLong => {
            let rest = s
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .ok_or(())?;
            ok(word(value(rest).ok_or(())?))
        }
        Mode::Rel8 => {
            let target = value(s).ok_or(())?;
            let displacement = i64::from(target) - (i64::from(address) + 2);
            let displacement = i8::try_from(displacement).map_err(|_| ())?;
            Ok(vec![displacement as u8])
        }
        Mode::Rel16 => {
            let target = value(s).ok_or(())?;
            let displacement = i64::from(target) - (i64::from(address) + 3);
            let displacement = i16::try_from(displacement).map_err(|_| ())?;
            Ok(displacement.to_le_bytes().to_vec())
        }
        Mode::BlockMove => {
            let (left, right) = s.split_once(',').ok_or(())?;
            let left = u8::try_from(value(left).ok_or(())?).map_err(|_| ())?;
            let right = u8::try_from(value(right).ok_or(())?).map_err(|_| ())?;
            Ok(vec![left, right])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asm(text: &str, address: u32) -> Vec<u8> {
        assemble(text, address).unwrap_or_else(|error| panic!("{text}: {error}"))
    }

    #[test]
    fn encodes_addressing_modes() {
        assert_eq!(asm("LDA #$10", 0x8000), [0xA9, 0x10]);
        assert_eq!(asm("LDA $10", 0x8000), [0xA5, 0x10]);
        assert_eq!(asm("LDA $1234", 0x8000), [0xAD, 0x34, 0x12]);
        assert_eq!(asm("LDA $123456", 0x8000), [0xAF, 0x56, 0x34, 0x12]);
        assert_eq!(asm("LDA $10,X", 0x8000), [0xB5, 0x10]);
        assert_eq!(asm("LDA $1234,X", 0x8000), [0xBD, 0x34, 0x12]);
        assert_eq!(asm("LDX $10,Y", 0x8000), [0xB6, 0x10]);
        assert_eq!(asm("LDA ($10,X)", 0x8000), [0xA1, 0x10]);
        assert_eq!(asm("LDA ($10),Y", 0x8000), [0xB1, 0x10]);
        assert_eq!(asm("LDA [$10]", 0x8000), [0xA7, 0x10]);
        assert_eq!(asm("LDA [$10],Y", 0x8000), [0xB7, 0x10]);
        assert_eq!(asm("LDA $10,S", 0x8000), [0xA3, 0x10]);
        assert_eq!(asm("LDA ($10,S),Y", 0x8000), [0xB3, 0x10]);
        assert_eq!(asm("LDA ($10)", 0x8000), [0xB2, 0x10]);
        assert_eq!(asm("JSR $8000", 0x8000), [0x20, 0x00, 0x80]);
        assert_eq!(asm("JSL $123456", 0x8000), [0x22, 0x56, 0x34, 0x12]);
        assert_eq!(asm("JMP ($1234)", 0x8000), [0x6C, 0x34, 0x12]);
        assert_eq!(asm("JMP ($1234,X)", 0x8000), [0x7C, 0x34, 0x12]);
        assert_eq!(asm("JMP [$1234]", 0x8000), [0xDC, 0x34, 0x12]);
        assert_eq!(asm("MVN $01,$02", 0x8000), [0x54, 0x01, 0x02]);
        assert_eq!(asm("PEA $1234", 0x8000), [0xF4, 0x34, 0x12]);
        assert_eq!(asm("RTL", 0x8000), [0x6B]);
        assert_eq!(asm("NOP", 0x8000), [0xEA]);
        assert_eq!(asm("ASL", 0x8000), [0x0A]);
        assert_eq!(asm("BRK", 0x8000), [0x00, 0x00]);
    }

    #[test]
    fn encodes_branches_from_the_instruction_address() {
        assert_eq!(asm("BNE $8000", 0x8000), [0xD0, 0xFE]);
        assert_eq!(asm("BNE $8003", 0x8000), [0xD0, 0x01]);
        assert_eq!(asm("BRL $8004", 0x8000), [0x82, 0x01, 0x00]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(assemble("", 0), Err(AsmError::Empty)));
        assert!(matches!(
            assemble("XYZ $10", 0),
            Err(AsmError::UnknownMnemonic(_))
        ));
        assert!(matches!(
            assemble("LDA #$1234", 0),
            Err(AsmError::InvalidOperand { .. })
        ));
        assert!(matches!(
            assemble("BNE $9000", 0x8000),
            Err(AsmError::BranchOutOfRange { .. })
        ));
    }

    #[test]
    fn disassembles_back_to_the_same_text() {
        for (text, address) in [
            ("LDA #$10", 0x8000),
            ("LDA $123456", 0x8000),
            ("JSR $8000", 0x8000),
            ("MVN $01,$02", 0x8000),
            ("RTL", 0x8000),
        ] {
            let bytes = asm(text, address);
            let row = crate::debug::disassemble(address, |offset| {
                bytes.get((offset - address) as usize).copied().unwrap_or(0)
            });
            assert_eq!(row.text, text, "round trip of {text}");
        }
    }
}
