//! Library facade over `unko-rom`.
//!
//! Scanning and archive handling live in the shared crate so the CLI and GUI
//! see exactly the same ROMs, container rules and save ids. This facade adds a
//! single-file fallback for SNES images, which the NES-oriented `unko-rom`
//! probe does not recognise.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

pub use unko_rom::{probe_path, scan, RomEntry as GameEntry};
use unko_rom::{Container, Source};

/// Resolves a path to playable entries, recognising SNES cartridges as well as
/// the NES/FDS/UNIF containers `unko-rom` understands.
///
/// A directory still uses the NES scan; the SNES fallback is single-file only.
pub fn probe_path_any(path: &Path) -> Result<Vec<GameEntry>, String> {
    match probe_path(path) {
        Ok(entries) if !entries.is_empty() => return Ok(entries),
        Ok(_) => {}
        Err(error) => {
            return match snes_entry(path).or_else(|| n64_entry(path)) {
                Some(entry) => Ok(vec![entry]),
                None => Err(error.to_string()),
            };
        }
    }
    match snes_entry(path).or_else(|| n64_entry(path)) {
        Some(entry) => Ok(vec![entry]),
        None => Ok(Vec::new()),
    }
}

/// Recursively indexes NES containers and SNES images under `root`.
///
/// The NES scan is delegated to `unko-rom`; SNES files are recognised by their
/// header (extension is only a cheap pre-filter).
pub fn scan_any(root: &Path) -> Vec<GameEntry> {
    let mut entries = scan(root);
    collect_media(root, 0, &mut entries);
    entries.sort_by_key(|entry| entry.title.to_lowercase());
    entries.dedup_by(|left, right| left.source == right.source);
    entries
}

fn collect_media(dir: &Path, depth: usize, out: &mut Vec<GameEntry>) {
    if depth > 16 {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_media(&path, depth + 1, out);
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        let entry = match extension.as_deref() {
            Some("sfc" | "smc" | "fig" | "swc") => snes_entry(&path),
            Some("z64" | "n64" | "v64") => n64_entry(&path),
            _ => None,
        };
        if let Some(entry) = entry {
            out.push(entry);
        }
    }
}

/// Whether `bytes` start with an N64 image header (`.z64`, `.v64` or `.n64`).
#[must_use]
pub fn is_n64_bytes(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0x80, 0x37, 0x12, 0x40])
            | Some([0x37, 0x80, 0x40, 0x12])
            | Some([0x40, 0x12, 0x37, 0x80])
    )
}

/// Builds a library entry for an N64 image, tagged `"n64"`.
///
/// No N64 core is linked into this build, so launching one reports a clear
/// "no N64 core" status rather than treating the file as another system.
fn n64_entry(path: &Path) -> Option<GameEntry> {
    if !path.is_file() {
        return None;
    }
    let mut header = [0u8; 4];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    if !is_n64_bytes(&header) {
        return None;
    }
    let metadata = fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    let size = metadata.map(|meta| meta.len()).unwrap_or(0);
    let title = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "N64".to_owned());
    let mut hash = 0x811c_9dc5u32;
    for byte in header
        .iter()
        .chain(title.as_bytes())
        .chain(size.to_le_bytes().iter())
    {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    Some(GameEntry {
        source: Source::File(path.to_path_buf()),
        container: Container::File,
        system: "n64",
        title,
        file_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size,
        modified,
        header: None,
        id: hash,
    })
}

/// Builds a library entry for a single SNES image, if the bytes parse.
fn snes_entry(path: &Path) -> Option<GameEntry> {
    if !path.is_file() {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if !unko_snes_host::is_snes_rom(&bytes) {
        return None;
    }
    let metadata = fs::metadata(path).ok();
    let modified = metadata
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    Some(GameEntry {
        source: Source::File(path.to_path_buf()),
        container: Container::File,
        system: "snes",
        title: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "SNES".to_owned()),
        file_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        modified,
        header: None,
        id: snes_id(&bytes),
    })
}

/// Stable 32-bit id (FNV-1a), matching the size of the NES CRC id.
fn snes_id(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_snes_image() {
        let dir = std::env::temp_dir().join(format!("unko-snes-lib-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("Game.sfc");
        let mut rom = vec![0u8; 0x8000];
        rom[0x7FC0..0x7FC0 + 21].copy_from_slice(b"TEST ROM             ");
        rom[0x7FC0 + 0x15] = 0x20;
        rom[0x7FC0 + 0x16] = 0x02;
        rom[0x7FC0 + 0x17] = 5;
        rom[0x7FC0 + 0x18] = 0x03;
        rom[0x7FC0 + 0x1A] = 0x01;
        rom[0x7FC0 + 0x3C..0x7FC0 + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());
        let mut source = rom.clone();
        source[0x7FDC..0x7FE0].fill(0);
        let base: u32 = source.iter().map(|&b| u32::from(b)).sum();
        let checksum = ((base + 0x01FE) & 0xFFFF) as u16;
        rom[0x7FDC..0x7FDE].copy_from_slice(&(!checksum).to_le_bytes());
        rom[0x7FDE..0x7FE0].copy_from_slice(&checksum.to_le_bytes());
        fs::write(&path, &rom).expect("write");

        let entries = probe_path_any(&path).expect("probe");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].system, "snes");
        assert_eq!(entries[0].title, "Game");
        assert_eq!(entries[0].size, rom.len() as u64);
        let _ = fs::remove_dir_all(&dir);
    }

    fn write_snes(path: &std::path::Path) {
        let mut rom = vec![0u8; 0x8000];
        rom[0x7FC0..0x7FC0 + 21].copy_from_slice(b"TEST ROM             ");
        rom[0x7FC0 + 0x15] = 0x20;
        rom[0x7FC0 + 0x16] = 0x02;
        rom[0x7FC0 + 0x17] = 5;
        rom[0x7FC0 + 0x18] = 0x03;
        rom[0x7FC0 + 0x1A] = 0x01;
        rom[0x7FC0 + 0x3C..0x7FC0 + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());
        let mut source = rom.clone();
        source[0x7FDC..0x7FE0].fill(0);
        let base: u32 = source.iter().map(|&b| u32::from(b)).sum();
        let checksum = ((base + 0x01FE) & 0xFFFF) as u16;
        rom[0x7FDC..0x7FDE].copy_from_slice(&(!checksum).to_le_bytes());
        rom[0x7FDE..0x7FE0].copy_from_slice(&checksum.to_le_bytes());
        fs::write(path, &rom).expect("write");
    }

    #[test]
    fn scans_snes_images_recursively() {
        let dir = std::env::temp_dir().join(format!("unko-snes-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).expect("dirs");
        write_snes(&nested.join("Game.sfc"));
        let entries = scan_any(&dir);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].system, "snes");
        assert_eq!(entries[0].title, "Game");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recognises_and_scans_n64_images() {
        let dir = std::env::temp_dir().join(format!("unko-n64-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).expect("dirs");
        let z64 = nested.join("Game.z64");
        fs::write(&z64, [0x80, 0x37, 0x12, 0x40, 0, 0, 0, 0]).expect("write");
        let v64 = nested.join("Other.v64");
        fs::write(&v64, [0x37, 0x80, 0x40, 0x12, 0, 0, 0, 0]).expect("write");
        let junk = nested.join("junk.n64");
        fs::write(&junk, [0u8; 8]).expect("write");

        assert!(is_n64_bytes(&[0x80, 0x37, 0x12, 0x40]));
        assert!(is_n64_bytes(&[0x37, 0x80, 0x40, 0x12]));
        assert!(is_n64_bytes(&[0x40, 0x12, 0x37, 0x80]));
        assert!(!is_n64_bytes(&[0u8; 4]));

        let direct = probe_path_any(&z64).expect("probe n64");
        assert_eq!(direct[0].system, "n64");

        let entries = scan_any(&dir);
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert!(entries.iter().all(|entry| entry.system == "n64"));
        assert_eq!(entries[0].title, "Game", "{entries:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_cartridge_errors_are_preserved() {
        let dir = std::env::temp_dir().join(format!("unko-bad-lib-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("thing.bin");
        fs::write(&path, [0u8; 64]).expect("write");
        assert!(probe_path_any(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
