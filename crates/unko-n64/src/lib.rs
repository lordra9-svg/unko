//! Nintendo 64 cartridge parsing and system scaffold.
//!
//! The image layer (byte order, header, region) is complete, and the machine
//! scaffold boots a cartridge's IPL3 on the [`cpu::Cpu`] (VR4300 interpreter
//! with a 32-entry TLB and single-precision COP1 FPU) with RDRAM/RSP memory
//! and PI/VI register stubs, runs RSP
//! scalar/vector code and DMA ([`rsp::Rsp`]), applies RDP solid-fill commands
//! and rasterises unshaded/shaded/textured triangles ([`rdp::Rdp`]) with the
//! RDP edgewalker, two-cycle combiner and the SET_OTHER_MODES blender
//! (cycle/copy, fog, coverage, dither), presents the VI
//! framebuffer from
//! RDRAM and runs double-buffered AI DMA with interrupts and DAC-to-host
//! resampling ([`ai::Ai`]). CI/TLUT palettes and
//! perspective-correct W interpolation and the compressed Z-buffer depth
//! pipeline are implemented, and precise exceptions (TLB, unaligned access,
//! SYSCALL/BREAK, reserved instructions, coprocessor-unusable, delay-slot BD,
//! maskable interrupts and enabled FPU exceptions) are delivered through CP0
//! EPC/Cause/BadVAddr/CE, Status.EXL and the refill/general vectors, with ERET
//! returning to the faulting context; the FPU tracks `fcr31` flags/causes and
//! flush-to-zero. The FPU also implements double-precision arithmetic,
//! comparisons and conversions with paired 64-bit FPR storage. Exact
//! depth-blend shifters and broader GPU coverage are the next milestones.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod ai;
pub mod cpu;
pub mod machine;
pub mod rdp;
pub mod rsp;

use std::fmt;

/// N64 image byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// Big-endian (`.z64`).
    Big,
    /// Byte-swapped (`.v64`).
    ByteSwapped,
    /// Little-endian words (`.n64`).
    Little,
}

impl ByteOrder {
    /// Stable identifier.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Big => "z64",
            Self::ByteSwapped => "v64",
            Self::Little => "n64",
        }
    }
}

/// Timing region of the cartridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// 60 Hz NTSC.
    Ntsc,
    /// 50 Hz PAL.
    Pal,
}

/// Why an image was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum N64Error {
    /// The file is shorter than the 64-byte header.
    TooSmall {
        /// Actual length.
        found: usize,
    },
    /// The first four bytes are not an N64 magic.
    BadMagic([u8; 4]),
}

impl fmt::Display for N64Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall { found } => write!(f, "image is {found} bytes; need at least 64"),
            Self::BadMagic(magic) => write!(f, "not an N64 image (magic {magic:02X?})"),
        }
    }
}

impl std::error::Error for N64Error {}

/// Parsed N64 cartridge metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Image name from `0x20-0x33`.
    pub title: String,
    /// Cartridge id characters (`0x3C-0x3D`).
    pub cart_id: [u8; 2],
    /// Country code (`0x3E`).
    pub country: u8,
    /// Version byte (`0x3F`).
    pub version: u8,
    /// Clock rate (`0x04`).
    pub clock_rate: u32,
    /// Entry point (`0x08`).
    pub entry_point: u32,
    /// CRC1 (`0x10`).
    pub crc1: u32,
    /// CRC2 (`0x14`).
    pub crc2: u32,
}

impl Header {
    /// Timing region implied by the country code.
    #[must_use]
    pub const fn region(&self) -> Region {
        match self.country {
            b'P' | b'D' | b'F' | b'H' | b'I' | b'S' | b'X' | b'Y' => Region::Pal,
            _ => Region::Ntsc,
        }
    }
}

/// A normalised (big-endian) N64 cartridge image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cartridge {
    rom: Vec<u8>,
    order: ByteOrder,
    header: Header,
}

impl Cartridge {
    /// Parses an image, detecting and converting its byte order.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, N64Error> {
        let magic = bytes
            .get(..4)
            .ok_or(N64Error::TooSmall { found: bytes.len() })?;
        let order = match magic {
            [0x80, 0x37, 0x12, 0x40] => ByteOrder::Big,
            [0x37, 0x80, 0x40, 0x12] => ByteOrder::ByteSwapped,
            [0x40, 0x12, 0x37, 0x80] => ByteOrder::Little,
            _ => {
                return Err(N64Error::BadMagic([magic[0], magic[1], magic[2], magic[3]]));
            }
        };
        if bytes.len() < 0x40 {
            return Err(N64Error::TooSmall { found: bytes.len() });
        }
        let rom = match order {
            ByteOrder::Big => bytes.to_vec(),
            ByteOrder::ByteSwapped => {
                let mut rom = bytes.to_vec();
                for pair in rom.as_chunks_mut::<2>().0 {
                    pair.swap(0, 1);
                }
                rom
            }
            ByteOrder::Little => {
                let mut rom = bytes.to_vec();
                for word in rom.as_chunks_mut::<4>().0 {
                    word.reverse();
                }
                rom
            }
        };
        let header = parse_header(&rom);
        Ok(Self { rom, order, header })
    }

    /// Normalised big-endian image.
    #[must_use]
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Parsed header metadata.
    #[must_use]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Detected source byte order.
    #[must_use]
    pub const fn byte_order(&self) -> ByteOrder {
        self.order
    }

    /// Timing region.
    #[must_use]
    pub const fn region(&self) -> Region {
        self.header.region()
    }
}

fn parse_header(rom: &[u8]) -> Header {
    let word = |offset: usize| {
        u32::from_be_bytes([
            rom[offset],
            rom[offset + 1],
            rom[offset + 2],
            rom[offset + 3],
        ])
    };
    let title_bytes = &rom[0x20..0x34];
    let title = title_bytes
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                ' '
            }
        })
        .collect::<String>()
        .trim()
        .to_owned();
    Header {
        title,
        cart_id: [rom[0x3C], rom[0x3D]],
        country: rom[0x3E],
        version: rom[0x3F],
        clock_rate: word(0x04),
        entry_point: word(0x08),
        crc1: word(0x10),
        crc2: word(0x14),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(order: ByteOrder) -> Vec<u8> {
        let mut rom = vec![0u8; 0x100];
        rom[0x00..0x04].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        rom[0x04..0x08].copy_from_slice(&0x0000_0F00u32.to_be_bytes());
        rom[0x08..0x0C].copy_from_slice(&0x8000_0400u32.to_be_bytes());
        rom[0x10..0x14].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        rom[0x14..0x18].copy_from_slice(&0xCAFE_F00Du32.to_be_bytes());
        rom[0x20..0x20 + 12].copy_from_slice(b"TEST TITLE  ");
        rom[0x3C] = b'N';
        rom[0x3D] = b'D';
        rom[0x3E] = b'E';
        rom[0x3F] = 1;
        match order {
            ByteOrder::Big => rom,
            ByteOrder::ByteSwapped => {
                let mut swapped = rom.clone();
                for pair in swapped.as_chunks_mut::<2>().0 {
                    pair.swap(0, 1);
                }
                swapped
            }
            ByteOrder::Little => {
                let mut swapped = rom;
                for word in swapped.as_chunks_mut::<4>().0 {
                    word.reverse();
                }
                swapped
            }
        }
    }

    #[test]
    fn normalises_all_byte_orders() {
        for order in [ByteOrder::Big, ByteOrder::ByteSwapped, ByteOrder::Little] {
            let cart = Cartridge::from_bytes(&image(order)).expect("parse");
            assert_eq!(cart.byte_order(), order);
            assert_eq!(cart.rom()[0..4], [0x80, 0x37, 0x12, 0x40]);
            assert_eq!(cart.header().title, "TEST TITLE");
            assert_eq!(cart.header().cart_id, [b'N', b'D']);
            assert_eq!(cart.header().crc1, 0xDEAD_BEEF);
            assert_eq!(cart.header().crc2, 0xCAFE_F00D);
            assert_eq!(cart.header().version, 1);
            assert_eq!(cart.region(), Region::Ntsc);
        }
    }

    #[test]
    fn pal_country() {
        let mut raw = image(ByteOrder::Big);
        raw[0x3E] = b'P';
        let cart = Cartridge::from_bytes(&raw).expect("parse");
        assert_eq!(cart.region(), Region::Pal);
    }

    #[test]
    fn rejects_short_or_wrong_images() {
        assert!(matches!(
            Cartridge::from_bytes(&[0u8; 4]),
            Err(N64Error::BadMagic(_))
        ));
        let mut junk = vec![0u8; 0x100];
        junk[0..4].copy_from_slice(&[b'N', b'E', b'S', 0x1A]);
        assert!(matches!(
            Cartridge::from_bytes(&junk),
            Err(N64Error::BadMagic(_))
        ));
        assert!(matches!(
            Cartridge::from_bytes(&[0x80, 0x37, 0x12, 0x40]),
            Err(N64Error::TooSmall { .. })
        ));
    }
}
