//! RGB PPU palettes (2C03/2C04/2C05) and 2C05 identifiers.
//!
//! Data transcribed independently from the public NESdev wiki
//! `PPU_palettes` and `PPU_registers` pages. Each channel is a 3-bit DAC
//! value; the wiki's conversion is `C = 255 * DAC / 7` (integer division,
//! matching the captured 2C03 values).

use crate::ppu::PpuModel;

const fn dac(value: u8) -> u8 {
    (value as u16 * 255 / 7) as u8
}

const fn rgb(digits: [u8; 3]) -> [u8; 3] {
    [dac(digits[0]), dac(digits[1]), dac(digits[2])]
}

/// 2C03 / 2C05 palette (identical to the 2C03 table in the wiki).
const PALETTE_2C03: [[u8; 3]; 64] = [
    [3, 3, 3],
    [0, 1, 4],
    [0, 0, 6],
    [3, 2, 6],
    [4, 0, 3],
    [5, 0, 3],
    [5, 1, 0],
    [4, 2, 0],
    [3, 2, 0],
    [1, 2, 0],
    [0, 3, 1],
    [0, 4, 0],
    [0, 2, 2],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [5, 5, 5],
    [0, 3, 6],
    [0, 2, 7],
    [4, 0, 7],
    [5, 0, 7],
    [7, 0, 4],
    [7, 0, 0],
    [6, 3, 0],
    [4, 3, 0],
    [1, 4, 0],
    [0, 4, 0],
    [0, 5, 3],
    [0, 4, 4],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [7, 7, 7],
    [3, 5, 7],
    [4, 4, 7],
    [6, 3, 7],
    [7, 0, 7],
    [7, 3, 7],
    [7, 4, 0],
    [7, 5, 0],
    [6, 6, 0],
    [3, 6, 0],
    [0, 7, 0],
    [2, 7, 6],
    [0, 7, 7],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [7, 7, 7],
    [5, 6, 7],
    [6, 5, 7],
    [7, 5, 7],
    [7, 4, 7],
    [7, 5, 5],
    [7, 6, 4],
    [7, 7, 2],
    [7, 7, 3],
    [5, 7, 2],
    [4, 7, 3],
    [2, 7, 6],
    [4, 6, 7],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
];

/// Unscrambled 2C04 master palette (the wiki's "ordered" table).
const PALETTE_2C04: [[u8; 3]; 64] = [
    [3, 3, 3],
    [0, 1, 4],
    [0, 0, 6],
    [3, 2, 6],
    [4, 0, 3],
    [5, 0, 3],
    [5, 1, 0],
    [4, 2, 0],
    [3, 2, 0],
    [1, 2, 0],
    [0, 3, 1],
    [0, 4, 0],
    [0, 2, 2],
    [1, 1, 1],
    [0, 0, 3],
    [0, 2, 0],
    [5, 5, 5],
    [0, 3, 6],
    [0, 2, 7],
    [4, 0, 7],
    [5, 0, 7],
    [7, 0, 4],
    [7, 0, 0],
    [6, 3, 0],
    [4, 3, 0],
    [1, 4, 0],
    [0, 4, 0],
    [0, 5, 3],
    [0, 4, 4],
    [2, 2, 2],
    [2, 0, 0],
    [3, 1, 0],
    [7, 7, 7],
    [3, 5, 7],
    [4, 4, 7],
    [6, 3, 7],
    [7, 0, 7],
    [7, 3, 7],
    [7, 4, 0],
    [7, 5, 0],
    [6, 6, 0],
    [3, 6, 0],
    [0, 7, 0],
    [2, 7, 6],
    [0, 7, 7],
    [4, 4, 4],
    [0, 0, 0],
    [0, 0, 0],
    [7, 7, 7],
    [5, 6, 7],
    [6, 5, 7],
    [7, 5, 7],
    [7, 4, 7],
    [7, 5, 5],
    [7, 6, 4],
    [7, 7, 0],
    [7, 7, 3],
    [5, 7, 2],
    [4, 7, 3],
    [2, 7, 6],
    [4, 6, 7],
    [6, 6, 6],
    [6, 5, 3],
    [7, 6, 0],
];

const LUT_2C04_0001: [u8; 64] = [
    0x35, 0x23, 0x16, 0x22, 0x1C, 0x09, 0x1D, 0x15, 0x20, 0x00, 0x27, 0x05, 0x04, 0x28, 0x08, 0x20,
    0x21, 0x3E, 0x1F, 0x29, 0x3C, 0x32, 0x36, 0x12, 0x3F, 0x2B, 0x2E, 0x1E, 0x3D, 0x2D, 0x24, 0x01,
    0x0E, 0x31, 0x33, 0x2A, 0x2C, 0x0C, 0x1B, 0x14, 0x2E, 0x07, 0x34, 0x06, 0x13, 0x02, 0x26, 0x2E,
    0x2E, 0x19, 0x10, 0x0A, 0x39, 0x03, 0x37, 0x17, 0x0F, 0x11, 0x0B, 0x0D, 0x38, 0x25, 0x18, 0x3A,
];

const LUT_2C04_0002: [u8; 64] = [
    0x2E, 0x27, 0x18, 0x39, 0x3A, 0x25, 0x1C, 0x31, 0x16, 0x13, 0x38, 0x34, 0x20, 0x23, 0x3C, 0x0B,
    0x0F, 0x21, 0x06, 0x3D, 0x1B, 0x29, 0x1E, 0x22, 0x1D, 0x24, 0x0E, 0x2B, 0x32, 0x08, 0x2E, 0x03,
    0x04, 0x36, 0x26, 0x33, 0x11, 0x1F, 0x10, 0x02, 0x14, 0x3F, 0x00, 0x09, 0x12, 0x2E, 0x28, 0x20,
    0x3E, 0x0D, 0x2A, 0x17, 0x0C, 0x01, 0x15, 0x19, 0x2E, 0x2C, 0x07, 0x37, 0x35, 0x05, 0x0A, 0x2D,
];

const LUT_2C04_0003: [u8; 64] = [
    0x14, 0x25, 0x3A, 0x10, 0x0B, 0x20, 0x31, 0x09, 0x01, 0x2E, 0x36, 0x08, 0x15, 0x3D, 0x3E, 0x3C,
    0x22, 0x1C, 0x05, 0x12, 0x19, 0x18, 0x17, 0x1B, 0x00, 0x03, 0x2E, 0x02, 0x16, 0x06, 0x34, 0x35,
    0x23, 0x0F, 0x0E, 0x37, 0x0D, 0x27, 0x26, 0x20, 0x29, 0x04, 0x21, 0x24, 0x11, 0x2D, 0x2E, 0x1F,
    0x2C, 0x1E, 0x39, 0x33, 0x07, 0x2A, 0x28, 0x1D, 0x0A, 0x2E, 0x32, 0x38, 0x13, 0x2B, 0x3F, 0x0C,
];

const LUT_2C04_0004: [u8; 64] = [
    0x18, 0x03, 0x1C, 0x28, 0x2E, 0x35, 0x01, 0x17, 0x10, 0x1F, 0x2A, 0x0E, 0x36, 0x37, 0x0B, 0x39,
    0x25, 0x1E, 0x12, 0x34, 0x2E, 0x1D, 0x06, 0x26, 0x3E, 0x1B, 0x22, 0x19, 0x04, 0x2E, 0x3A, 0x21,
    0x05, 0x0A, 0x07, 0x02, 0x13, 0x14, 0x00, 0x15, 0x0C, 0x3D, 0x11, 0x0F, 0x0D, 0x38, 0x2D, 0x24,
    0x33, 0x20, 0x08, 0x16, 0x3F, 0x2B, 0x20, 0x3C, 0x2E, 0x27, 0x23, 0x31, 0x29, 0x32, 0x2C, 0x09,
];

/// RGB value for one palette index on an RGB PPU.
///
/// 2C02 returns `None`; the caller uses its composite palette.
pub(crate) fn colour(model: PpuModel, index: u8) -> Option<[u8; 3]> {
    let index = usize::from(index & 0x3F);
    Some(match model {
        PpuModel::Ppu2C03
        | PpuModel::Ppu2C05A
        | PpuModel::Ppu2C05B
        | PpuModel::Ppu2C05C
        | PpuModel::Ppu2C05D
        | PpuModel::Ppu2C05E => rgb(PALETTE_2C03[index]),
        PpuModel::Ppu2C04A => rgb(PALETTE_2C04[usize::from(LUT_2C04_0001[index])]),
        PpuModel::Ppu2C04B => rgb(PALETTE_2C04[usize::from(LUT_2C04_0002[index])]),
        PpuModel::Ppu2C04C => rgb(PALETTE_2C04[usize::from(LUT_2C04_0003[index])]),
        PpuModel::Ppu2C04D => rgb(PALETTE_2C04[usize::from(LUT_2C04_0004[index])]),
        PpuModel::Ppu2C02 => return None,
    })
}

impl PpuModel {
    /// Whether `$2000` and `$2001` are swapped (2C05A-D).
    #[must_use]
    pub(crate) const fn swaps_registers(self) -> bool {
        matches!(
            self,
            Self::Ppu2C05A | Self::Ppu2C05B | Self::Ppu2C05C | Self::Ppu2C05D
        )
    }

    /// Constant returned in bits 4-0 of `$2002` for 2C05A-D, and its mask.
    #[must_use]
    pub(crate) const fn status_identifier(self) -> Option<(u8, u8)> {
        match self {
            Self::Ppu2C05A | Self::Ppu2C05D => Some((0x1B, 0x1F)),
            Self::Ppu2C05B => Some((0x3D, 0x3F)),
            Self::Ppu2C05C => Some((0x1C, 0x1F)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dac_and_2c03_entries_match_the_published_values() {
        assert_eq!(dac(0), 0);
        assert_eq!(dac(7), 255);
        assert_eq!(dac(3), 109);
        assert_eq!(dac(4), 145);
        assert_eq!(colour(PpuModel::Ppu2C03, 0x00), Some([109, 109, 109]));
        assert_eq!(colour(PpuModel::Ppu2C03, 0x0F), Some([0, 0, 0]));
        assert_eq!(colour(PpuModel::Ppu2C03, 0x30), Some([255, 255, 255]));
    }

    #[test]
    fn palettes_2c04_variants_permute_the_master() {
        assert_eq!(colour(PpuModel::Ppu2C04A, 0x00), Some([255, 182, 182]));
        assert_eq!(colour(PpuModel::Ppu2C04B, 0x00), Some([0, 0, 0]));
        assert_eq!(colour(PpuModel::Ppu2C04C, 0x00), Some([182, 0, 255]));
        assert_eq!(colour(PpuModel::Ppu2C04D, 0x00), Some([145, 109, 0]));
    }

    #[test]
    fn identifiers_2c05_and_register_swaps() {
        assert_eq!(PpuModel::Ppu2C05B.status_identifier(), Some((0x3D, 0x3F)));
        assert_eq!(PpuModel::Ppu2C05C.status_identifier(), Some((0x1C, 0x1F)));
        assert_eq!(PpuModel::Ppu2C05E.status_identifier(), None);
        assert!(PpuModel::Ppu2C05A.swaps_registers());
        assert!(!PpuModel::Ppu2C05E.swaps_registers());
    }
}
