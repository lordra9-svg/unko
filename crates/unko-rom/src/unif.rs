//! UNIF (Universal NES Image Format) reader.
//!
//! UNIF predates NES 2.0 and describes the board by name instead of a mapper
//! number. The loader converts the chunk stream into an iNES 1.0 image so the
//! accuracy core only ever sees the format it already validates. Common
//! Nintendo boards are mapped; unknown boards are rejected rather than being
//! silently assigned a wrong mapper.

use std::collections::BTreeMap;

use crate::Header;

const MAGIC: &[u8; 4] = b"UNIF";
const HEADER_LEN: usize = 32;
const MAX_VERSION: u32 = 7;

/// Board metadata read from a UNIF chunk stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// iNES mapper number.
    pub mapper: u16,
    /// NES 2.0 submapper, carried for documentation only.
    pub submapper: u8,
    /// PRG-ROM size in KiB.
    pub prg_kib: u32,
    /// CHR-ROM size in KiB (0 means CHR-RAM).
    pub chr_kib: u32,
    /// Battery-backed RAM declared by `BATR`.
    pub battery: bool,
    /// Four-screen mirroring declared by `MIRR`.
    pub four_screen: bool,
    /// Timing hint.
    pub region: &'static str,
}

struct Chunks {
    board: String,
    prg: BTreeMap<u8, Vec<u8>>,
    chr: BTreeMap<u8, Vec<u8>>,
    mirroring: Option<u8>,
    battery: bool,
    chr_ram: bool,
    region: Option<u8>,
}

/// Whether `bytes` begins with the UNIF magic.
#[must_use]
pub fn is_unif(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER_LEN && &bytes[..4] == MAGIC
}

/// Reads the board and size metadata, or `None` when the stream is malformed.
#[must_use]
pub fn header(bytes: &[u8]) -> Option<Header> {
    let chunks = parse_chunks(bytes).ok()?;
    let mapper = board_mapper(&chunks.board)?;
    let (prg_len, chr_len) = sizes(&chunks)?;
    let info = Info {
        mapper,
        submapper: board_submapper(&chunks.board).unwrap_or(0),
        prg_kib: (prg_len / 1024) as u32,
        chr_kib: (chr_len / 1024) as u32,
        battery: chunks.battery,
        four_screen: chunks.mirroring == Some(4),
        region: region(chunks.region),
    };
    Some(Header {
        mapper: info.mapper,
        submapper: info.submapper,
        prg_kib: info.prg_kib,
        chr_kib: info.chr_kib,
        battery: info.battery,
        four_screen: info.four_screen,
        nes2: false,
        region: info.region,
    })
}

/// Converts a UNIF image into an iNES 1.0 image.
///
/// # Errors
///
/// Returns a description when the chunk stream is truncated, the board is not
/// mapped, or a ROM size does not fit the iNES header.
pub fn to_ines(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if !is_unif(bytes) {
        return Err("not a UNIF image".into());
    }
    let version = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| "truncated UNIF version".to_string())?,
    );
    if version > MAX_VERSION {
        return Err(format!(
            "UNIF version {version} is newer than the supported version {MAX_VERSION}"
        ));
    }
    let chunks = parse_chunks(bytes)?;
    let mapper = board_mapper(&chunks.board)
        .ok_or_else(|| format!("UNIF board '{}' is not mapped", chunks.board))?;
    if mapper > 0xFF {
        return Err(format!("mapper {mapper} needs a NES 2.0 header"));
    }
    let (prg_len, chr_len) = sizes(&chunks).ok_or("PRG/CHR chunks are gapped or misaligned")?;
    let prg_banks = prg_len / 0x4000;
    let chr_banks = chr_len / 0x2000;
    if prg_banks == 0 || prg_banks > 0xFF || chr_banks > 0xFF {
        return Err("ROM size does not fit an iNES header".into());
    }

    let mut out = Vec::with_capacity(16 + prg_len + chr_len);
    out.extend_from_slice(b"NES\x1a");
    out.push(prg_banks as u8);
    out.push(chr_banks as u8);
    let mut flags6 = ((mapper & 0x0F) as u8) << 4;
    if chunks.battery {
        flags6 |= 0x02;
    }
    if chunks.mirroring == Some(4) {
        flags6 |= 0x08;
    } else if chunks.mirroring == Some(1) {
        flags6 |= 0x01;
    }
    out.push(flags6);
    out.push((mapper & 0xF0) as u8);
    out.extend_from_slice(&[0; 8]);
    // iNES 1.0 has no region byte, but the de-facto byte 9 bit 0 hint is what
    // the rest of this workspace reads.
    let pal = chunks.region == Some(1);
    if pal {
        out[9] = 1;
    }
    for data in chunks.prg.values() {
        out.extend_from_slice(data);
    }
    if !chunks.chr_ram {
        for data in chunks.chr.values() {
            out.extend_from_slice(data);
        }
    }
    Ok(out)
}

fn parse_chunks(bytes: &[u8]) -> Result<Chunks, String> {
    if !is_unif(bytes) {
        return Err("not a UNIF image".into());
    }
    let version = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| "truncated UNIF version".to_string())?,
    );
    if version > MAX_VERSION {
        return Err(format!(
            "UNIF version {version} is newer than the supported version {MAX_VERSION}"
        ));
    }
    let mut chunks = Chunks {
        board: String::new(),
        prg: BTreeMap::new(),
        chr: BTreeMap::new(),
        mirroring: None,
        battery: false,
        chr_ram: false,
        region: None,
    };
    let mut position = HEADER_LEN;
    while position + 8 <= bytes.len() {
        let ty = &bytes[position..position + 4];
        let length = u32::from_le_bytes(
            bytes[position + 4..position + 8]
                .try_into()
                .map_err(|_| "truncated chunk length".to_string())?,
        ) as usize;
        let start = position + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "truncated UNIF chunk".to_string())?;
        let data = &bytes[start..end];
        match ty {
            b"MAPR" => chunks.board = cstr(data),
            b"PRG0" | b"PRG1" | b"PRG2" | b"PRG3" | b"PRG4" | b"PRG5" | b"PRG6" | b"PRG7"
            | b"PRG8" | b"PRG9" | b"PRGA" | b"PRGB" | b"PRGC" | b"PRGD" | b"PRGE" | b"PRGF" => {
                insert_chunk(&mut chunks.prg, ty[3], data)?;
            }
            b"CHR0" | b"CHR1" | b"CHR2" | b"CHR3" | b"CHR4" | b"CHR5" | b"CHR6" | b"CHR7"
            | b"CHR8" | b"CHR9" | b"CHRA" | b"CHRB" | b"CHRC" | b"CHRD" | b"CHRE" | b"CHRF" => {
                insert_chunk(&mut chunks.chr, ty[3], data)?;
            }
            b"MIRR" => chunks.mirroring = data.first().copied(),
            b"BATR" => chunks.battery = data.first().is_some_and(|byte| *byte != 0),
            b"VROR" => chunks.chr_ram = true,
            b"TVCI" => chunks.region = data.first().copied(),
            _ => {}
        }
        position = end;
    }
    if chunks.board.is_empty() {
        return Err("UNIF image has no MAPR board".into());
    }
    if chunks.prg.is_empty() {
        return Err("UNIF image has no PRG ROM".into());
    }
    Ok(chunks)
}

fn insert_chunk(map: &mut BTreeMap<u8, Vec<u8>>, digit: u8, data: &[u8]) -> Result<(), String> {
    let index = hex_digit(digit).ok_or_else(|| "bad UNIF chunk index".to_string())?;
    if map.insert(index, data.to_vec()).is_some() {
        return Err(format!("duplicate UNIF chunk {index:X}"));
    }
    Ok(())
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn cstr(data: &[u8]) -> String {
    let end = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    String::from_utf8_lossy(&data[..end]).trim().to_string()
}

fn sizes(chunks: &Chunks) -> Option<(usize, usize)> {
    // Chunk indices must start at zero and be contiguous; a gap would move
    // every later bank and silently corrupt the image.
    for (expected, index) in chunks.prg.keys().enumerate() {
        if usize::from(*index) != expected {
            return None;
        }
    }
    for (expected, index) in chunks.chr.keys().enumerate() {
        if usize::from(*index) != expected {
            return None;
        }
    }
    let prg_len: usize = chunks.prg.values().map(Vec::len).sum();
    let chr_len: usize = if chunks.chr_ram {
        0
    } else {
        chunks.chr.values().map(Vec::len).sum()
    };
    if prg_len == 0 || !prg_len.is_multiple_of(0x4000) || !chr_len.is_multiple_of(0x2000) {
        return None;
    }
    Some((prg_len, chr_len))
}

fn region(value: Option<u8>) -> &'static str {
    match value {
        Some(1) => "PAL",
        Some(2) => "Dual",
        _ => "NTSC",
    }
}

fn board_submapper(board: &str) -> Option<u8> {
    let board = board.to_ascii_uppercase();
    if board.contains("BNROM") {
        Some(2)
    } else if board.contains("ANROM") || board.contains("AN1ROM") {
        Some(1)
    } else if !board.contains("SUNSOFT")
        && (board.contains("AMROM") || board.contains("UNROM") || board.contains("UOROM"))
    {
        Some(2)
    } else {
        None
    }
}

fn board_mapper(board: &str) -> Option<u16> {
    let board = board.trim().to_ascii_uppercase();
    // Longest / most specific names first: NROM is a substring of UNROM,
    // CNROM, SNROM and TNROM, so a plain `contains("NROM")` must come last.
    if board.contains("SUNSOFT_UNROM") || board.contains("SUNSOFT-UNROM") {
        return Some(93);
    }
    if board.contains("UNROM-512") {
        return Some(30);
    }
    if board.contains("BNROM") {
        return Some(34);
    }
    if board.contains("CPROM") {
        return Some(13);
    }
    if ["AOROM", "ANROM", "AN1ROM", "AMROM"]
        .iter()
        .any(|name| board.contains(name))
    {
        return Some(7);
    }
    if [
        "TBROM",
        "TEROM",
        "TFROM",
        "TGROM",
        "TKROM",
        "TK1ROM",
        "TKEPROM",
        "TLROM",
        "TL1ROM",
        "TL2ROM",
        "TNROM",
        "TR1ROM",
        "TSROM",
        "TVROM",
        "TQROM",
        "TKSROM",
        "TLSROM",
        "B4",
        "HKROM",
        "ACCLAIM-MC-ACC",
    ]
    .iter()
    .any(|name| board.contains(name))
    {
        if board.contains("TKSROM") || board.contains("TLSROM") {
            return Some(118);
        }
        if board.contains("TQROM") {
            return Some(119);
        }
        return Some(4);
    }
    if [
        "SAROM", "SBROM", "SCROM", "SC1ROM", "SEROM", "SFROM", "SF1ROM", "SFEXPROM", "SGROM",
        "SHROM", "SH1ROM", "SIROM", "SJROM", "SKROM", "SLROM", "SL1ROM", "SL2ROM", "SL3ROM",
        "SLRROM", "SMROM", "SNROM", "SNWEPROM", "SOROM", "SUROM", "SXROM",
    ]
    .iter()
    .any(|name| board.contains(name))
    {
        return Some(1);
    }
    if board.contains("UNROM") || board.contains("UOROM") {
        return Some(2);
    }
    if board.contains("CNROM") {
        return Some(3);
    }
    if board.contains("NROM") || board.contains("FAMILYBASIC") {
        return Some(0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + data.len());
        out.extend_from_slice(ty);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    fn unif(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = vec![0u8; HEADER_LEN];
        out[..4].copy_from_slice(b"UNIF");
        out[4..8].copy_from_slice(&7u32.to_le_bytes());
        for (ty, data) in chunks {
            out.extend_from_slice(&chunk(ty, data));
        }
        out
    }

    #[test]
    fn converts_an_nrom_image() {
        let prg = vec![0xAA; 0x4000];
        let chr = vec![0x55; 0x2000];
        let bytes = unif(&[
            (b"MAPR", b"NES-NROM-256\0"),
            (b"PRG0", &prg),
            (b"CHR0", &chr),
            (b"MIRR", &[1]),
            (b"TVCI", &[0]),
        ]);
        assert!(is_unif(&bytes));
        let info = header(&bytes).expect("header");
        assert_eq!(info.mapper, 0);
        assert_eq!(info.prg_kib, 16);
        assert_eq!(info.chr_kib, 8);
        let ines = to_ines(&bytes).expect("convert");
        assert_eq!(&ines[..4], b"NES\x1a");
        assert_eq!(ines[4], 1);
        assert_eq!(ines[5], 1);
        assert_eq!(ines[6] & 0x01, 1, "vertical mirroring");
        assert_eq!(ines[16], 0xAA);
        assert_eq!(ines[16 + 0x4000], 0x55);
    }

    #[test]
    fn converts_an_mmc1_board_with_chr_ram_and_battery() {
        let prg = vec![0x11; 0x8000];
        let bytes = unif(&[
            (b"MAPR", b"NES-SNROM\0"),
            (b"PRG0", &prg),
            (b"BATR", &[1]),
            (b"MIRR", &[0]),
        ]);
        let info = header(&bytes).expect("header");
        assert_eq!(info.mapper, 1);
        assert_eq!(info.chr_kib, 0);
        assert!(info.battery);
        let ines = to_ines(&bytes).expect("convert");
        assert_eq!(ines[4], 2);
        assert_eq!(ines[5], 0);
        assert_eq!(ines[6] & 0x02, 0x02, "battery flag");
    }

    #[test]
    fn rejects_unknown_boards_and_truncated_chunks() {
        let bytes = unif(&[(b"MAPR", b"MADE-UP-BOARD\0"), (b"PRG0", &[0; 0x4000])]);
        assert!(to_ines(&bytes).is_err());
        assert!(header(&bytes).is_none());

        let mut truncated = unif(&[(b"MAPR", b"NES-NROM-256\0"), (b"PRG0", &[0; 0x4000])]);
        truncated.truncate(truncated.len() - 1);
        assert!(to_ines(&truncated).is_err());
    }

    #[test]
    fn rejects_gapped_prg_chunks() {
        let bytes = unif(&[(b"MAPR", b"NES-NROM-256\0"), (b"PRG1", &[0; 0x4000])]);
        assert!(to_ines(&bytes).is_err());
    }
}
