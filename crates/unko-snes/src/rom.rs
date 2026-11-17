//! SNES ROM container parsing, header decoding and cartridge bus mapping.
//!
//! This module deliberately has no dependency on the rest of the emulator. It
//! turns a raw `.sfc`/`.smc` image into a validated [`Cartridge`] with a
//! decoded header, SRAM and the address mapping needed by the CPU bus.

use std::fmt;

/// Size of the optional copier header prepended by old `.smc` dumps.
pub const COPIER_HEADER_SIZE: usize = 512;

/// Size of the SNES internal header, from the title to the vector table.
pub const HEADER_SIZE: usize = 0x40;

const LOROM_HEADER: usize = 0x0000_7FC0;
const HIROM_HEADER: usize = 0x0000_FFC0;
const EXLOROM_HEADER: usize = 0x0040_7FC0;
const EXHIROM_HEADER: usize = 0x0040_FFC0;

/// Largest SRAM image accepted from an untrusted header (1 MiB).
const MAX_SRAM_SIZE: usize = 0x10_0000;

/// Lowest header score accepted during layout detection.
const MIN_HEADER_SCORE: u8 = 5;

/// The four standard SNES ROM address layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mapping {
    /// 32 KiB banks mapped at `$8000-$FFFF` (`$20`/`$30` map mode).
    LoRom,
    /// Linear 64 KiB banks (`$21`/`$31` map mode).
    HiRom,
    /// LoROM extended past 4 MiB (`$22`/`$32` map mode).
    ExLoRom,
    /// HiROM extended past 4 MiB (`$25`/`$35` map mode).
    ExHiRom,
}

impl Mapping {
    /// Every layout, in detection tie-break order.
    pub const ALL: [Self; 4] = [Self::LoRom, Self::HiRom, Self::ExLoRom, Self::ExHiRom];

    /// File offset of the internal header for this layout.
    pub const fn header_offset(self) -> usize {
        match self {
            Self::LoRom => LOROM_HEADER,
            Self::HiRom => HIROM_HEADER,
            Self::ExLoRom => EXLOROM_HEADER,
            Self::ExHiRom => EXHIROM_HEADER,
        }
    }

    /// Low nibble of the header map-mode byte for this layout.
    const fn map_mode_low(self) -> u8 {
        match self {
            Self::LoRom => 0x00,
            Self::HiRom => 0x01,
            Self::ExLoRom => 0x02,
            Self::ExHiRom => 0x05,
        }
    }

    /// Whether this layout places ROM above the first 4 MiB.
    pub const fn is_extended(self) -> bool {
        matches!(self, Self::ExLoRom | Self::ExHiRom)
    }

    /// Human-readable layout name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::LoRom => "LoROM",
            Self::HiRom => "HiROM",
            Self::ExLoRom => "ExLoROM",
            Self::ExHiRom => "ExHiROM",
        }
    }
}

impl fmt::Display for Mapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A decoded SNES cartridge header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomHeader {
    /// Detected address layout.
    pub mapping: Mapping,
    /// File offset of the header inside the headerless ROM image.
    pub header_offset: usize,
    /// 21-byte internal title, trimmed at the first NUL.
    pub title: String,
    /// Raw map-mode byte at `$FFD5`.
    pub map_mode: u8,
    /// Raw cartridge-type byte at `$FFD6`.
    pub cartridge_type: u8,
    /// Raw ROM-size code at `$FFD7`.
    pub rom_size_code: u8,
    /// Raw SRAM-size code at `$FFD8`.
    pub sram_size_code: u8,
    /// Raw country code at `$FFD9`.
    pub country_code: u8,
    /// Raw licensee code at `$FFDA`.
    pub license_code: u8,
    /// Raw version byte at `$FFDB`.
    pub version: u8,
    /// Stored checksum at `$FFDE-$FFDF`.
    pub checksum: u16,
    /// Stored checksum complement at `$FFDC-$FFDD`.
    pub checksum_complement: u16,
    /// Emulation-mode reset vector at `$FFFC-$FFFD`.
    pub reset_vector: u16,
    /// ROM size declared by the header, when the code is understood.
    pub declared_rom_size: Option<usize>,
    /// SRAM size declared by the header, when the code is understood.
    pub declared_sram_size: Option<usize>,
    /// Heuristic confidence used to select this layout.
    pub header_score: u8,
}

impl RomHeader {
    /// Whether the cartridge has battery-backed save RAM.
    pub fn has_battery(&self) -> bool {
        self.cartridge_type & 0x02 != 0
    }

    /// Country/region name for the header code.
    pub fn country_name(&self) -> &'static str {
        country_name(self.country_code)
    }

    /// Video region declared by the header country code.
    pub const fn region(&self) -> Region {
        region_for_country(self.country_code)
    }

    /// Whether the cartridge declares an SPC7110 coprocessor.
    ///
    /// mia selects an `SPC7110-` board from cartridge type `$F5` (Epson RTC
    /// variants use `$F9`) or the `SPC7110-` HiROM map modes `$2A`/`$3A`.
    pub const fn has_spc7110(&self) -> bool {
        matches!(self.cartridge_type, 0xf5 | 0xf9) || matches!(self.map_mode, 0x2a | 0x3a)
    }

    /// Whether the cartridge declares an SA-1 (Super Accelerator) coprocessor.
    ///
    /// ares/mia select the `SA1-` board from map modes `$23`/`$33`.
    pub const fn has_sa1(&self) -> bool {
        matches!(self.map_mode, 0x23 | 0x33)
    }

    /// Whether the cartridge declares an S-DD1 decompression coprocessor.
    ///
    /// ares/mia select the `SDD1-` board from map modes `$22`/`$32`, the
    /// extended-LoROM layouts used by the three S-DD1 titles.
    pub const fn has_sdd1(&self) -> bool {
        matches!(self.map_mode, 0x22 | 0x32)
    }

    /// Whether the cartridge type declares a CX4 (Hitachi HG51B) coprocessor.
    ///
    /// CX4 carts use LoROM map modes with cartridge type `$F3`.
    pub const fn has_cx4(&self) -> bool {
        matches!(self.cartridge_type, 0xF3)
            && matches!(self.mapping, Mapping::LoRom | Mapping::ExLoRom)
    }

    /// Whether the cartridge type declares a SuperFX (GSU) coprocessor.
    ///
    /// `$13` is SuperFX, `$14` is SuperFX + RAM + battery and `$15` is
    /// SuperFX + RAM.
    pub const fn has_superfx(&self) -> bool {
        matches!(self.cartridge_type, 0x13..=0x15)
    }

    /// Human-readable cartridge type.
    pub fn cartridge_type_name(&self) -> &'static str {
        cartridge_type_name(self.cartridge_type)
    }

    /// Compute the cartridge checksum.
    ///
    /// The SNES checksum is the byte sum over the ROM, including the stored
    /// checksum and complement bytes. Non-power-of-two images are expanded by
    /// repeatedly doubling the final `2^n` bytes, where `2^n` is the lowest set
    /// bit of the current size. That matches the address-line mirroring used by
    /// real boards and the header checksums in commercial ROMs.
    pub fn computed_checksum(&self, rom: &[u8]) -> u16 {
        if rom.is_empty() {
            return 0;
        }
        let mut image = rom.to_vec();
        while !image.len().is_power_of_two() {
            let lowest = image.len() & image.len().wrapping_neg();
            let start = image.len() - lowest;
            let tail = image[start..].to_vec();
            image.extend_from_slice(&tail);
        }
        (image.iter().map(|&byte| u32::from(byte)).sum::<u32>() & 0xFFFF) as u16
    }

    /// Whether the stored checksum matches a fresh checksum calculation.
    pub fn checksum_matches(&self, rom: &[u8]) -> bool {
        self.computed_checksum(rom) == self.checksum
    }
}

/// Where a 24-bit CPU address points inside the cartridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusTarget {
    /// Offset into the headerless ROM image.
    Rom(usize),
    /// Offset into the cartridge SRAM image.
    Sram(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawTarget {
    Rom(usize),
    Sram(usize),
}

/// A parsed SNES cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cartridge {
    rom: Vec<u8>,
    header: RomHeader,
    sram: Vec<u8>,
    has_copier_header: bool,
}

impl Cartridge {
    /// Parse a raw ROM image, stripping a 512-byte copier header if present.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RomError> {
        let (rom, has_copier_header) = strip_copier_header(bytes);
        if rom.len() < 0x8000 {
            return Err(RomError::TooSmall { size: rom.len() });
        }

        let mut best: Option<(u8, Mapping)> = None;
        for mapping in Mapping::ALL {
            let offset = mapping.header_offset();
            let Some(header) = rom.get(offset..offset + HEADER_SIZE) else {
                continue;
            };
            let score = score_header(header, mapping);
            if score < MIN_HEADER_SCORE {
                continue;
            }
            let checksum = u16::from_le_bytes([header[0x1E], header[0x1F]]);
            let complement = u16::from_le_bytes([header[0x1C], header[0x1D]]);
            let reset_vector = u16::from_le_bytes([header[0x3C], header[0x3D]]);
            if checksum ^ complement != 0xFFFF && reset_vector < 0x8000 {
                continue;
            }
            if best.is_none_or(|(best_score, _)| score > best_score) {
                best = Some((score, mapping));
            }
        }

        let (header_score, mapping) = best.ok_or(RomError::NoValidHeader)?;
        let header = parse_header(rom, mapping, header_score);
        let sram_size = header.declared_sram_size.unwrap_or(0).min(MAX_SRAM_SIZE);

        Ok(Self {
            rom: rom.to_vec(),
            header,
            sram: vec![0; sram_size],
            has_copier_header,
        })
    }

    /// Parse a ROM image, falling back to a synthesized LoROM/HiROM header when
    /// no header passes validation.
    ///
    /// Homebrew images and tech demos often ship with an empty (`$FF`) header
    /// table. The fallback keeps the reset vector and image size, assumes
    /// LoROM when the `$7FFC` vector is plausible and HiROM when only the
    /// `$FFFC` vector is, and reports no SRAM. Region defaults to NTSC/Japan
    /// and the cartridge type to plain ROM, so a caller that knows the board
    /// (e.g. a DSP-1 demo) must still attach the coprocessor explicitly.
    pub fn from_bytes_lenient(bytes: &[u8]) -> Result<Self, RomError> {
        match Self::from_bytes(bytes) {
            Ok(cartridge) => return Ok(cartridge),
            Err(RomError::NoValidHeader) => {}
            Err(error) => return Err(error),
        }

        let (rom, has_copier_header) = strip_copier_header(bytes);
        if rom.len() < 0x8000 {
            return Err(RomError::TooSmall { size: rom.len() });
        }

        let lorom_reset = u16::from_le_bytes([rom[0x7FFC], rom[0x7FFD]]);
        let hirom_reset = if rom.len() >= 0x1_0000 {
            u16::from_le_bytes([rom[0xFFFC], rom[0xFFFD]])
        } else {
            0
        };

        let (mapping, header_offset, reset_vector) = if lorom_reset >= 0x8000 {
            (Mapping::LoRom, LOROM_HEADER, lorom_reset)
        } else if hirom_reset >= 0x8000 {
            (Mapping::HiRom, HIROM_HEADER, hirom_reset)
        } else {
            return Err(RomError::NoValidHeader);
        };

        let map_mode = match mapping {
            Mapping::LoRom => 0x20,
            Mapping::HiRom => 0x21,
            Mapping::ExLoRom => 0x22,
            Mapping::ExHiRom => 0x25,
        };
        let header = RomHeader {
            mapping,
            header_offset,
            title: String::new(),
            map_mode,
            cartridge_type: 0x00,
            rom_size_code: 0,
            sram_size_code: 0,
            country_code: 0,
            license_code: 0,
            version: 0,
            checksum: 0,
            checksum_complement: 0,
            reset_vector,
            declared_rom_size: Some(rom.len()),
            declared_sram_size: Some(0),
            header_score: 0,
        };

        Ok(Self {
            rom: rom.to_vec(),
            header,
            sram: Vec::new(),
            has_copier_header,
        })
    }

    /// Headerless ROM image.
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Decoded cartridge header.
    pub fn header(&self) -> &RomHeader {
        &self.header
    }

    /// Battery-backed save RAM.
    pub fn sram(&self) -> &[u8] {
        &self.sram
    }

    /// ROM and mutable save RAM at once.
    ///
    /// Coprocessor buses (the SA-1) read ROM and write save RAM within a single
    /// operation, so they need both halves of the cartridge without tripping
    /// the borrow checker.
    pub fn rom_and_sram_mut(&mut self) -> (&[u8], &mut [u8]) {
        (&self.rom, &mut self.sram)
    }

    /// Mutable save RAM, for persistence and host save files.
    pub fn sram_mut(&mut self) -> &mut [u8] {
        &mut self.sram
    }

    /// Whether a 512-byte copier header was removed during parsing.
    pub const fn has_copier_header(&self) -> bool {
        self.has_copier_header
    }

    /// Copy a save file into SRAM, returning the number of bytes copied.
    pub fn load_sram(&mut self, data: &[u8]) -> usize {
        let count = data.len().min(self.sram.len());
        self.sram[..count].copy_from_slice(&data[..count]);
        count
    }

    /// Map a 24-bit CPU address to a ROM or SRAM offset.
    ///
    /// Offsets are folded into the actual image size, matching the mirroring
    /// used by the hardware for smaller-than-bank ROMs and SRAM.
    pub fn map(&self, address: u32) -> Option<BusTarget> {
        match raw_map(self.header.mapping, address)? {
            RawTarget::Rom(offset) if !self.rom.is_empty() => {
                Some(BusTarget::Rom(offset % self.rom.len()))
            }
            RawTarget::Sram(offset) if !self.sram.is_empty() => {
                Some(BusTarget::Sram(offset % self.sram.len()))
            }
            _ => None,
        }
    }

    /// Read a mapped cartridge byte.
    pub fn read(&self, address: u32) -> Option<u8> {
        match self.map(address)? {
            BusTarget::Rom(offset) => self.rom.get(offset).copied(),
            BusTarget::Sram(offset) => self.sram.get(offset).copied(),
        }
    }

    /// Write a mapped cartridge byte. ROM writes are ignored and return
    /// `false`; SRAM writes return `true`.
    pub fn write(&mut self, address: u32, value: u8) -> bool {
        match self.map(address) {
            Some(BusTarget::Sram(offset)) => {
                if let Some(byte) = self.sram.get_mut(offset) {
                    *byte = value;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

/// Errors produced while parsing a ROM image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomError {
    /// The image is smaller than a single LoROM bank.
    TooSmall {
        /// Headerless image size in bytes.
        size: usize,
    },
    /// No candidate layout produced a plausible SNES header.
    NoValidHeader,
}

impl fmt::Display for RomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall { size } => {
                write!(formatter, "ROM image is too small ({size} bytes)")
            }
            Self::NoValidHeader => formatter.write_str("no valid SNES header found"),
        }
    }
}

impl std::error::Error for RomError {}

fn strip_copier_header(bytes: &[u8]) -> (&[u8], bool) {
    if bytes.len() >= COPIER_HEADER_SIZE && bytes.len() % 1024 == COPIER_HEADER_SIZE {
        (&bytes[COPIER_HEADER_SIZE..], true)
    } else {
        (bytes, false)
    }
}

fn score_header(header: &[u8], mapping: Mapping) -> u8 {
    let checksum = u16::from_le_bytes([header[0x1E], header[0x1F]]);
    let complement = u16::from_le_bytes([header[0x1C], header[0x1D]]);
    let reset_vector = u16::from_le_bytes([header[0x3C], header[0x3D]]);
    let map_mode = header[0x15];

    let mut score = 0;
    if checksum ^ complement == 0xFFFF {
        score += 8;
    }
    if reset_vector >= 0x8000 {
        score += 2;
    }
    if map_mode & 0x0F == mapping.map_mode_low() {
        score += 4;
    }
    if map_mode & 0xE0 == 0x20 {
        score += 1;
    }
    if title_is_plausible(header) {
        score += 1;
    }
    if checksum != 0 && checksum != 0xFFFF {
        score += 1;
    }
    score
}

fn title_is_plausible(header: &[u8]) -> bool {
    let title = &header[..21];
    let end = title
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(title.len());
    end > 0
        && title[..end]
            .iter()
            .all(|&byte| byte >= 0x20 && byte != 0x7F)
}

fn parse_header(rom: &[u8], mapping: Mapping, header_score: u8) -> RomHeader {
    let offset = mapping.header_offset();
    let header = &rom[offset..offset + HEADER_SIZE];
    let rom_size_code = header[0x17];
    let sram_size_code = header[0x18];
    RomHeader {
        mapping,
        header_offset: offset,
        title: decode_title(&header[..21]),
        map_mode: header[0x15],
        cartridge_type: header[0x16],
        rom_size_code,
        sram_size_code,
        country_code: header[0x19],
        license_code: header[0x1A],
        version: header[0x1B],
        checksum: u16::from_le_bytes([header[0x1E], header[0x1F]]),
        checksum_complement: u16::from_le_bytes([header[0x1C], header[0x1D]]),
        reset_vector: u16::from_le_bytes([header[0x3C], header[0x3D]]),
        declared_rom_size: rom_size_bytes(rom_size_code),
        declared_sram_size: sram_size_bytes(sram_size_code),
        header_score,
    }
}

fn decode_title(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[..end]);
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Decode the ROM-size header byte into bytes.
///
/// The code is `log2(size in KiB)`; the standard range ends at `$0D`
/// (8 MiB).
pub const fn rom_size_bytes(code: u8) -> Option<usize> {
    if code <= 0x0D {
        Some((1usize << code) * 1024)
    } else {
        None
    }
}

/// Decode the SRAM-size header byte into bytes.
///
/// `$00` means no SRAM; `$01-$0A` select 2 KiB through 1 MiB.
pub const fn sram_size_bytes(code: u8) -> Option<usize> {
    if code == 0 {
        Some(0)
    } else if code <= 0x0A {
        Some((1usize << code) * 1024)
    } else {
        None
    }
}

/// Console video region.
///
/// ares treats the region as a *system* setting, but every commercial image
/// declares one in its header; [`region_for_country`] maps that byte to the
/// timing the cart was built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Region {
    /// 60 Hz, 262-line frames, 21,477,272 Hz master clock.
    Ntsc,
    /// 50 Hz, 312-line frames, 21,281,370 Hz master clock.
    Pal,
}

impl Region {
    /// Whether this is the PAL 50 Hz region.
    pub const fn is_pal(self) -> bool {
        matches!(self, Self::Pal)
    }

    /// S-CPU master clock in Hz (ares `Constants::Colorburst::NTSC * 6` and
    /// `Constants::Colorburst::PAL * 4.8`).
    pub const fn master_clock(self) -> u64 {
        match self {
            Self::Ntsc => 21_477_272,
            Self::Pal => 21_281_370,
        }
    }

    /// Scanlines in a non-interlaced frame (ares `PPUcounter::vperiod`).
    pub const fn lines_per_frame(self) -> u16 {
        match self {
            Self::Ntsc => 262,
            Self::Pal => 312,
        }
    }

    /// Scanline at which vblank begins in non-overscan mode (ares
    /// `PPU::updateVideoMode`: `vdisp = 225`).
    pub const fn vdisp(self) -> u16 {
        225
    }
}

/// Video region implied by a header country code.
///
/// Europe and its national variants, China, Australia and Russia shipped
/// 50 Hz PAL hardware; Japan, the Americas and Korea are 60 Hz. Brazil's
/// PAL-M is a 60 Hz system, so it stays on NTSC timing.
pub const fn region_for_country(code: u8) -> Region {
    match code {
        2..=9 | 0x0B | 0x0C | 0x0E..=0x10 => Region::Pal,
        _ => Region::Ntsc,
    }
}

/// Country/region name for a header country code.
pub const fn country_name(code: u8) -> &'static str {
    match code {
        0 => "Japan",
        1 => "USA / Canada",
        2 => "Europe",
        3 => "Sweden",
        4 => "Finland",
        5 => "Denmark",
        6 => "France",
        7 => "Netherlands",
        8 => "Norway",
        9 => "Germany",
        10 => "Brazil",
        11 => "Italy",
        12 => "China",
        13 => "Korea",
        14 => "Australia",
        15 => "Spain",
        16 => "Russia",
        _ => "Unknown",
    }
}

/// Human-readable cartridge type for a header cartridge-type byte.
pub const fn cartridge_type_name(code: u8) -> &'static str {
    match code {
        0x00 => "ROM",
        0x01 => "ROM + RAM",
        0x02 => "ROM + RAM + battery",
        0x03 => "ROM + coprocessor",
        0x04 => "ROM + coprocessor + RAM",
        0x05 => "ROM + coprocessor + RAM + battery",
        0x13 => "ROM + coprocessor + RAM + battery",
        _ => "Other",
    }
}

fn raw_map(mapping: Mapping, address: u32) -> Option<RawTarget> {
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as usize;

    match mapping {
        Mapping::LoRom => {
            if bank > 0x7D && bank < 0x80 {
                return None; // $7E-$7F is WRAM, not cartridge space.
            }
            let bank7 = usize::from(bank & 0x7F);
            if offset >= 0x8000 {
                Some(RawTarget::Rom((bank7 << 15) | (offset & 0x7FFF)))
            } else if (0x70..=0x7D).contains(&bank7) {
                Some(RawTarget::Sram(offset))
            } else if (0x40..=0x7D).contains(&bank7) {
                Some(RawTarget::Rom((bank7 << 15) | offset))
            } else {
                None
            }
        }
        Mapping::HiRom => {
            if bank <= 0x3F || (0x80..=0xBF).contains(&bank) {
                if offset >= 0x8000 {
                    Some(RawTarget::Rom((usize::from(bank & 0x3F) << 16) | offset))
                } else if (0x6000..=0x7FFF).contains(&offset) && bank & 0x20 != 0 {
                    Some(RawTarget::Sram(offset & 0x1FFF))
                } else {
                    None
                }
            } else if (0x40..=0x7D).contains(&bank) || bank >= 0xC0 {
                Some(RawTarget::Rom((usize::from(bank & 0x3F) << 16) | offset))
            } else {
                None
            }
        }
        Mapping::ExLoRom => {
            if bank > 0x7D && bank < 0x80 {
                return None;
            }
            let bank7 = usize::from(bank & 0x7F);
            let upper = if bank & 0x80 != 0 { 0x40_0000 } else { 0 };
            if offset >= 0x8000 {
                Some(RawTarget::Rom(upper | (bank7 << 15) | (offset & 0x7FFF)))
            } else if (0x70..=0x7D).contains(&bank7) {
                Some(RawTarget::Sram(offset))
            } else if (0x40..=0x7D).contains(&bank7) {
                Some(RawTarget::Rom(upper | (bank7 << 15) | offset))
            } else {
                None
            }
        }
        Mapping::ExHiRom => {
            if bank >= 0xC0 {
                // Banks $C0-$FF see the first 4 MiB linearly.
                Some(RawTarget::Rom((usize::from(bank - 0xC0) << 16) | offset))
            } else if (0x80..=0xBF).contains(&bank) {
                // Banks $80-$BF expose the upper half of the first 4 MiB.
                if offset >= 0x8000 {
                    Some(RawTarget::Rom((usize::from(bank & 0x3F) << 16) | offset))
                } else if (0x6000..=0x7FFF).contains(&offset) && bank & 0x20 != 0 {
                    Some(RawTarget::Sram(offset & 0x1FFF))
                } else {
                    None
                }
            } else if (0x40..=0x7D).contains(&bank) {
                // Banks $40-$7D see the additional 4 MiB.
                Some(RawTarget::Rom(
                    0x40_0000 | (usize::from(bank - 0x40) << 16) | offset,
                ))
            } else if bank <= 0x3F {
                if offset >= 0x8000 {
                    Some(RawTarget::Rom(
                        0x40_0000 | (usize::from(bank) << 16) | offset,
                    ))
                } else if (0x6000..=0x7FFF).contains(&offset) && bank & 0x20 != 0 {
                    Some(RawTarget::Sram(offset & 0x1FFF))
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_rom(
        mapping: Mapping,
        len: usize,
        title: &str,
        sram_code: u8,
        map_mode: u8,
    ) -> Vec<u8> {
        let mut rom: Vec<u8> = (0..len)
            .map(|index| (index.wrapping_mul(37) & 0xFF) as u8)
            .collect();
        let offset = mapping.header_offset();
        rom[offset..offset + 21].fill(0);
        let end = offset + title.len().min(21);
        rom[offset..end].copy_from_slice(&title.as_bytes()[..end - offset]);
        rom[offset + 0x15] = map_mode;
        rom[offset + 0x16] = 0x02;
        rom[offset + 0x17] = (len / 1024).trailing_zeros() as u8;
        rom[offset + 0x18] = sram_code;
        rom[offset + 0x19] = 1;
        rom[offset + 0x1A] = 0x01;
        rom[offset + 0x1B] = 0;
        rom[offset + 0x3C..offset + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());

        let mut checksum_source = rom.clone();
        checksum_source[offset + 0x1C..offset + 0x20].fill(0);
        let base = checksum_source
            .iter()
            .map(|&byte| u32::from(byte))
            .sum::<u32>();
        // The four checksum/complement bytes always sum to 0x01FE.
        let checksum = ((base + 0x01FE) & 0xFFFF) as u16;
        rom[offset + 0x1C..offset + 0x1E].copy_from_slice(&(!checksum).to_le_bytes());
        rom[offset + 0x1E..offset + 0x20].copy_from_slice(&checksum.to_le_bytes());
        rom
    }

    #[test]
    fn parses_lorom_header() {
        let rom = build_rom(Mapping::LoRom, 0x4_0000, "LOROM TEST", 3, 0x20);
        let cartridge = Cartridge::from_bytes(&rom).expect("valid LoROM");
        let header = cartridge.header();
        assert_eq!(header.mapping, Mapping::LoRom);
        assert_eq!(header.title, "LOROM TEST");
        assert_eq!(header.map_mode, 0x20);
        assert_eq!(header.country_name(), "USA / Canada");
        assert_eq!(header.declared_rom_size, Some(0x4_0000));
        assert_eq!(header.declared_sram_size, Some(0x2000));
        assert_eq!(header.reset_vector, 0x8000);
        assert!(header.checksum_matches(cartridge.rom()));
        assert_eq!(cartridge.sram().len(), 0x2000);
    }

    #[test]
    fn parses_hirom_header() {
        let rom = build_rom(Mapping::HiRom, 0x4_0000, "HIROM TEST", 5, 0x21);
        let cartridge = Cartridge::from_bytes(&rom).expect("valid HiROM");
        assert_eq!(cartridge.header().mapping, Mapping::HiRom);
        assert_eq!(cartridge.header().title, "HIROM TEST");
        assert_eq!(cartridge.header().declared_sram_size, Some(0x8000));
        assert!(cartridge.header().checksum_matches(cartridge.rom()));
    }

    #[test]
    fn parses_exlorom_header() {
        let rom = build_rom(Mapping::ExLoRom, 0x80_0000, "EXLOROM TEST", 3, 0x22);
        let cartridge = Cartridge::from_bytes(&rom).expect("valid ExLoROM");
        assert_eq!(cartridge.header().mapping, Mapping::ExLoRom);
        assert_eq!(cartridge.header().header_offset, EXLOROM_HEADER);
    }

    #[test]
    fn parses_exhirom_header() {
        let rom = build_rom(Mapping::ExHiRom, 0x80_0000, "EXHIROM TEST", 3, 0x25);
        let cartridge = Cartridge::from_bytes(&rom).expect("valid ExHiROM");
        assert_eq!(cartridge.header().mapping, Mapping::ExHiRom);
        assert_eq!(cartridge.header().header_offset, EXHIROM_HEADER);
    }

    #[test]
    fn strips_copier_header() {
        let rom = build_rom(Mapping::LoRom, 0x4_0000, "COPIER TEST", 0, 0x20);
        let mut smc = vec![0xAA; COPIER_HEADER_SIZE];
        smc.extend_from_slice(&rom);
        let cartridge = Cartridge::from_bytes(&smc).expect("valid SMC");
        assert!(cartridge.has_copier_header());
        assert_eq!(cartridge.rom(), rom.as_slice());
        assert_eq!(cartridge.header().title, "COPIER TEST");
    }

    #[test]
    fn rejects_blank_image() {
        let error = Cartridge::from_bytes(&vec![0u8; 0x8000]).expect_err("blank image");
        assert_eq!(error, RomError::NoValidHeader);
    }

    #[test]
    fn rejects_tiny_image() {
        let error = Cartridge::from_bytes(&[0u8; 16]).expect_err("tiny image");
        assert_eq!(error, RomError::TooSmall { size: 16 });
    }

    #[test]
    fn prefers_scoring_layout() {
        let mut rom = build_rom(Mapping::HiRom, 0x4_0000, "HIROM SCORE", 0, 0x21);
        // Put a deliberately weak LoROM-looking header in the first bank.
        rom[LOROM_HEADER + 0x15] = 0x00;
        rom[LOROM_HEADER + 0x1C..LOROM_HEADER + 0x20].fill(0);
        rom[LOROM_HEADER + 0x3C..LOROM_HEADER + 0x3E].fill(0);
        let cartridge = Cartridge::from_bytes(&rom).expect("valid HiROM");
        assert_eq!(cartridge.header().mapping, Mapping::HiRom);
    }

    #[test]
    fn maps_lorom_bus() {
        let rom = build_rom(Mapping::LoRom, 0x4_0000, "LOROM BUS", 3, 0x20);
        let mut cartridge = Cartridge::from_bytes(&rom).expect("valid LoROM");
        assert_eq!(cartridge.read(0x00_8000), Some(rom[0]));
        assert_eq!(cartridge.read(0x01_8000), Some(rom[0x8000]));
        assert_eq!(cartridge.read(0x80_8000), Some(rom[0]));
        assert_eq!(cartridge.read(0x7E_8000), None);
        assert_eq!(cartridge.read(0x40_0000), Some(rom[0]));

        assert!(cartridge.write(0x70_0000, 0x5A));
        assert_eq!(cartridge.read(0x70_0000), Some(0x5A));
        assert_eq!(cartridge.read(0x70_6000), Some(0x5A));
        assert_eq!(cartridge.read(0xF0_0000), Some(0x5A));
    }

    #[test]
    fn maps_hirom_bus() {
        let rom = build_rom(Mapping::HiRom, 0x4_0000, "HIROM BUS", 3, 0x21);
        let mut cartridge = Cartridge::from_bytes(&rom).expect("valid HiROM");
        assert_eq!(cartridge.read(0xC0_0000), Some(rom[0]));
        assert_eq!(cartridge.read(0x00_8000), Some(rom[0]));
        assert_eq!(cartridge.read(0xC0_FFFF), Some(rom[0xFFFF]));
        assert_eq!(cartridge.read(0x40_0000), Some(rom[0]));
        assert_eq!(cartridge.read(0x7E_8000), None);

        assert!(cartridge.write(0x20_6000, 0xA5));
        assert_eq!(cartridge.read(0x20_6000), Some(0xA5));
        assert_eq!(cartridge.read(0xA0_6000), Some(0xA5));
    }

    #[test]
    fn maps_exhirom_bus() {
        let rom = build_rom(Mapping::ExHiRom, 0x80_0000, "EXHIROM BUS", 0, 0x25);
        let cartridge = Cartridge::from_bytes(&rom).expect("valid ExHiROM");
        assert_eq!(cartridge.read(0xC0_0000), Some(rom[0]));
        assert_eq!(cartridge.read(0x40_0000), Some(rom[0x40_0000]));
        assert_eq!(cartridge.read(0x00_8000), Some(rom[0x40_8000]));
        assert_eq!(cartridge.read(0x00_FFC0), Some(rom[EXHIROM_HEADER]));
        assert_eq!(cartridge.read(0x80_8000), Some(rom[0x8000]));
        assert_eq!(cartridge.read(0x80_FFC0), Some(rom[0xFFC0]));
    }

    #[test]
    fn maps_exlorom_bus() {
        let rom = build_rom(Mapping::ExLoRom, 0x80_0000, "EXLOROM BUS", 0, 0x22);
        let cartridge = Cartridge::from_bytes(&rom).expect("valid ExLoROM");
        assert_eq!(cartridge.read(0x00_8000), Some(rom[0]));
        assert_eq!(cartridge.read(0x80_8000), Some(rom[0x40_0000]));
        assert_eq!(cartridge.read(0xC0_0000), Some(rom[0x60_0000]));
        assert_eq!(cartridge.read(0x80_FFC0), Some(rom[EXLOROM_HEADER]));
    }

    #[test]
    fn decodes_size_tables() {
        assert_eq!(rom_size_bytes(0x08), Some(0x4_0000));
        assert_eq!(rom_size_bytes(0x0D), Some(0x80_0000));
        assert_eq!(rom_size_bytes(0x0E), None);
        assert_eq!(sram_size_bytes(0x00), Some(0));
        assert_eq!(sram_size_bytes(0x01), Some(0x800));
        assert_eq!(sram_size_bytes(0x0A), Some(0x10_0000));
        assert_eq!(sram_size_bytes(0x0B), None);
    }

    #[test]
    fn lenient_parser_recovers_a_headerless_lorom() {
        // An empty ($ff) header table with a plausible $7ffc reset vector.
        let mut rom = vec![0xFFu8; 0x2_0000];
        rom[0x7FFC..0x7FFE].copy_from_slice(&0x8000u16.to_le_bytes());
        rom[0..4].copy_from_slice(&[0x78, 0x18, 0xFB, 0x4C]);
        let cartridge = Cartridge::from_bytes_lenient(&rom).expect("lenient LoROM");
        assert_eq!(cartridge.header().mapping, Mapping::LoRom);
        assert_eq!(cartridge.header().reset_vector, 0x8000);
        assert_eq!(cartridge.rom().len(), rom.len());
        assert!(Cartridge::from_bytes(&rom).is_err());
    }

    #[test]
    fn maps_country_codes_to_regions() {
        // 60 Hz.
        for code in [0x00u8, 0x01, 0x0A, 0x0D, 0xFF] {
            assert_eq!(region_for_country(code), Region::Ntsc, "code {code:02X}");
        }
        // 50 Hz: Europe and national variants, China, Australia, Russia.
        for code in [
            0x02u8, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0B, 0x0C, 0x0E, 0x0F, 0x10,
        ] {
            assert_eq!(region_for_country(code), Region::Pal, "code {code:02X}");
        }
        assert_eq!(Region::Ntsc.master_clock(), 21_477_272);
        assert_eq!(Region::Pal.master_clock(), 21_281_370);
        assert_eq!(Region::Ntsc.lines_per_frame(), 262);
        assert_eq!(Region::Pal.lines_per_frame(), 312);
    }
}
