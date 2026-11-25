//! Batch SNES ROM scanner used for library-wide boot/load smoke sweeps.
//!
//! Usage:
//! `xtask snes-scan --dir ROMS [--dir ROMS...] [--ipl IPL.ROM] [--frames N]
//!                  [--limit N] [--batch N] [--csv OUT.csv]`
//! `xtask snes-scan --list IMAGES [same options]` reads one image path per
//! line; a line may be `archive.zip!member.sfc` to select one archive member.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};
use unko_snes::{Cartridge, Snes};
use zip::ZipArchive;

#[derive(Debug)]
struct ScanRow {
    path: String,
    sha256: String,
    size: u64,
    status: String,
    strict: bool,
    mapping: String,
    region: String,
    country: String,
    title: String,
    cart_type: String,
    header_offset: usize,
    header_score: u8,
    rom_size: String,
    sram_size: String,
    checksum: String,
    reset_vector: String,
    frames: u64,
    video_fnv: u64,
    audio_fnv: u64,
    error: String,
}

pub fn run(args: &[String]) -> i32 {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut list: Option<PathBuf> = None;
    let mut ipl: Option<PathBuf> = std::env::var("UNKO_SNES_IPL").ok().map(PathBuf::from);
    let mut frames: u64 = 3;
    let mut limit: usize = 500;
    let mut batch: usize = 5;
    let mut csv: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].clone();
        index += 1;
        let value = |index: &mut usize| -> Option<String> {
            let value = args.get(*index).cloned();
            *index += 1;
            value
        };
        match arg.as_str() {
            "--dir" => dirs.push(PathBuf::from(value(&mut index).unwrap_or_default())),
            "--list" => list = value(&mut index).map(PathBuf::from),
            "--ipl" => ipl = value(&mut index).map(PathBuf::from),
            "--frames" => {
                frames = value(&mut index)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(frames)
            }
            "--limit" => {
                limit = value(&mut index)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(limit)
            }
            "--batch" => {
                batch = value(&mut index)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(batch)
                    .max(1)
            }
            "--csv" => csv = value(&mut index).map(PathBuf::from),
            "-h" | "--help" => {
                println!(
                    "usage: xtask snes-scan --dir ROMS [--dir ROMS...] | --list IMAGES \
                         [--ipl IPL.ROM] [--frames N] [--limit N] [--batch N] [--csv OUT.csv]"
                );
                return 0;
            }
            other => {
                eprintln!("unknown argument: {other}");
                return 2;
            }
        }
    }
    if dirs.is_empty() && list.is_none() {
        eprintln!("snes-scan: at least one --dir or --list is required");
        return 2;
    }

    let ipl = ipl.and_then(|path| fs::read(path).ok()).and_then(|bytes| {
        let array: [u8; 64] = bytes.try_into().ok()?;
        Some(array)
    });

    let mut paths = Vec::new();
    for dir in &dirs {
        collect(dir, &mut paths);
    }
    if let Some(list) = list {
        if let Ok(text) = fs::read_to_string(list) {
            for line in text.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    paths.push(PathBuf::from(line));
                }
            }
        }
    }
    paths.sort();
    paths.dedup();

    let mut seen = HashSet::new();
    let mut rows: Vec<ScanRow> = Vec::new();
    let mut batch_count = 0usize;
    let mut batch_number = 0usize;
    let started = Instant::now();

    'outer: for path in paths {
        if rows.len() >= limit {
            break;
        }
        let (source, wanted) = split_zip_member(&path);
        let is_zip = source
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("zip"))
            .unwrap_or(false);
        if is_zip {
            let Ok(file) = File::open(source) else {
                continue;
            };
            let Ok(mut archive) = ZipArchive::new(file) else {
                continue;
            };
            for index in 0..archive.len() {
                if rows.len() >= limit {
                    break 'outer;
                }
                let Ok(mut member) = archive.by_index(index) else {
                    continue;
                };
                let Ok(name) = member.name() else {
                    continue;
                };
                let name = name.to_string();
                if wanted.is_some_and(|wanted| wanted != name) {
                    continue;
                }
                if !is_rom(Path::new(&name)) {
                    continue;
                }
                let mut bytes = Vec::new();
                if member
                    .by_ref()
                    .take(64 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .is_err()
                    || bytes.len() > 64 * 1024 * 1024
                {
                    continue;
                }
                let display = format!("{}!{}", source.display(), name);
                if let Some(row) = make_row(&display, bytes, &mut seen, ipl, frames) {
                    batch_count += 1;
                    if batch_count >= batch || rows.len() + 1 == limit {
                        batch_number += 1;
                        println!(
                            "batch {batch_number}: {} roms ({} scanned, {} failures so far)",
                            batch_count,
                            rows.len() + 1,
                            rows.iter().filter(|row| row.status != "ok").count()
                                + usize::from(row.status != "ok")
                        );
                        batch_count = 0;
                    }
                    rows.push(row);
                }
            }
        } else {
            let Ok(bytes) = fs::read(source) else {
                continue;
            };
            if let Some(row) =
                make_row(&source.display().to_string(), bytes, &mut seen, ipl, frames)
            {
                batch_count += 1;
                if batch_count >= batch || rows.len() + 1 == limit {
                    batch_number += 1;
                    println!(
                        "batch {batch_number}: {} roms ({} scanned, {} failures so far)",
                        batch_count,
                        rows.len() + 1,
                        rows.iter().filter(|row| row.status != "ok").count()
                            + usize::from(row.status != "ok")
                    );
                    batch_count = 0;
                }
                rows.push(row);
            }
        }
    }

    let failures: Vec<&ScanRow> = rows.iter().filter(|row| row.status != "ok").collect();
    let by_region = |name: &str| rows.iter().filter(|row| row.region == name).count();
    let by_mapping = |name: &str| rows.iter().filter(|row| row.mapping == name).count();
    let strict = rows.iter().filter(|row| row.strict).count();
    let lenient = rows.len().saturating_sub(strict);
    let mut countries: BTreeMap<&str, usize> = BTreeMap::new();
    for row in &rows {
        *countries.entry(row.country.as_str()).or_default() += 1;
    }
    println!(
        "snes-scan: {} unique roms scanned in {} batches, {} failures, {:?}",
        rows.len(),
        batch_number,
        failures.len(),
        started.elapsed()
    );
    println!(
        "regions: NTSC={} PAL={} unknown={}",
        by_region("Ntsc"),
        by_region("Pal"),
        by_region("?")
    );
    println!(
        "mappings: LoRom={} HiRom={} ExLoRom={} ExHiRom={} unknown={}",
        by_mapping("LoRom"),
        by_mapping("HiRom"),
        by_mapping("ExLoRom"),
        by_mapping("ExHiRom"),
        by_mapping("?")
    );
    println!(
        "headers: strict={strict} lenient-fallback={lenient}; countries: {}",
        countries
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let video_hashes = rows
        .iter()
        .map(|row| row.video_fnv)
        .collect::<HashSet<_>>()
        .len();
    let audio_hashes = rows
        .iter()
        .map(|row| row.audio_fnv)
        .collect::<HashSet<_>>()
        .len();
    println!("hashes: video={video_hashes} audio={audio_hashes}");
    for row in &failures {
        println!("FAIL {}: {}", row.path, row.error);
    }
    if let Some(path) = csv {
        if let Err(error) = write_csv(&path, &rows) {
            eprintln!("snes-scan: writing {}: {error}", path.display());
            return 1;
        }
        println!("wrote {}", path.display());
    }
    0
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if is_rom(&path)
            || path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        {
            out.push(path);
        }
    }
}

fn is_rom(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "sfc" | "smc" | "swc" | "fig"
            )
        })
        .unwrap_or(false)
}

fn split_zip_member(path: &Path) -> (&Path, Option<&str>) {
    let Some(text) = path.to_str() else {
        return (path, None);
    };
    let Some(index) = text.find(".zip!") else {
        return (path, None);
    };
    (Path::new(&text[..index + 4]), Some(&text[index + 5..]))
}

fn make_row(
    display: &str,
    bytes: Vec<u8>,
    seen: &mut HashSet<String>,
    ipl: Option<[u8; 64]>,
    frames: u64,
) -> Option<ScanRow> {
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = format!("{:x}", hasher.finalize());
    if !seen.insert(digest.clone()) {
        return None;
    }
    Some(scan_one(Path::new(display), &bytes, digest, ipl, frames))
}

fn scan_one(
    path: &Path,
    bytes: &[u8],
    sha256: String,
    ipl: Option<[u8; 64]>,
    frames: u64,
) -> ScanRow {
    let mut row = ScanRow {
        path: path.display().to_string(),
        sha256,
        size: bytes.len() as u64,
        status: "error".into(),
        strict: true,
        mapping: "?".into(),
        region: "?".into(),
        country: "?".into(),
        title: String::new(),
        cart_type: "?".into(),
        header_offset: 0,
        header_score: 0,
        rom_size: "?".into(),
        sram_size: "?".into(),
        checksum: "?".into(),
        reset_vector: "?".into(),
        frames: 0,
        video_fnv: 0,
        audio_fnv: 0,
        error: String::new(),
    };
    let cart = match Cartridge::from_bytes(bytes) {
        Ok(cart) => cart,
        Err(strict_error) => match Cartridge::from_bytes_lenient(bytes) {
            Ok(cart) => {
                row.strict = false;
                cart
            }
            Err(error) => {
                row.error = format!("{strict_error}; lenient: {error}");
                return row;
            }
        },
    };
    let header = cart.header().clone();
    row.mapping = format!("{:?}", header.mapping);
    row.region = format!("{:?}", header.region());
    row.country = header.country_name().to_string();
    row.title = header.title.clone();
    row.cart_type = header.cartridge_type_name().to_string();
    row.header_offset = header.header_offset;
    row.header_score = header.header_score;
    row.rom_size = header
        .declared_rom_size
        .map(|size| size.to_string())
        .unwrap_or_else(|| "?".into());
    row.sram_size = header
        .declared_sram_size
        .map(|size| size.to_string())
        .unwrap_or_else(|| "?".into());
    row.checksum = format!("{:04X}/{:04X}", header.checksum, header.checksum_complement);
    row.reset_vector = format!("{:04X}", header.reset_vector);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut snes = Snes::new(cart);
        if let Some(ipl) = ipl {
            snes.set_ipl(ipl);
        }
        snes.reset();
        let mut video_hash = 0xcbf2_9ce4_8422_2325u64;
        let mut audio_hash = 0xcbf2_9ce4_8422_2325u64;
        for _ in 0..frames {
            let mut guard = 0u64;
            while !snes.ppu.frame_ready() {
                snes.step();
                guard += 1;
                if guard >= 5_000_000 {
                    return (
                        Err("frame did not complete within the step budget".to_string()),
                        0,
                        0,
                        0,
                    );
                }
            }
            row.frames += 1;
            for byte in snes.ppu.frame_buffer() {
                video_hash ^= u64::from(*byte);
                video_hash = video_hash.wrapping_mul(0x100_0000_01B3);
            }
            for (left, right) in snes.apu.drain_samples() {
                for value in [left, right] {
                    for byte in value.to_le_bytes() {
                        audio_hash ^= u64::from(byte);
                        audio_hash = audio_hash.wrapping_mul(0x100_0000_01B3);
                    }
                }
            }
            snes.ppu.acknowledge_frame();
        }
        (Ok(()), video_hash, audio_hash, 0)
    }));
    match result {
        Ok((Ok(()), video_hash, audio_hash, _)) => {
            row.status = "ok".into();
            row.video_fnv = video_hash;
            row.audio_fnv = audio_hash;
        }
        Ok((Err(error), _, _, _)) => row.error = error,
        Err(_) => row.error = "panic while loading/running".into(),
    }
    row
}

fn write_csv(path: &Path, rows: &[ScanRow]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    writeln!(
        file,
        "path,sha256,size,status,strict,mapping,region,country,title,cart_type,header_offset,header_score,rom_size,sram_size,checksum,reset_vector,frames,video_fnv,audio_fnv,error"
    )?;
    for row in rows {
        writeln!(
            file,
            "\"{}\",{},{},{},{},{},{},{},\"{}\",\"{}\",{},{},{},{},{},{},{},{:016X},{:016X},\"{}\"",
            row.path.replace('"', "\"\""),
            row.sha256,
            row.size,
            row.status,
            row.strict,
            row.mapping,
            row.region,
            row.country,
            row.title.replace('"', "\"\""),
            row.cart_type.replace('"', "\"\""),
            row.header_offset,
            row.header_score,
            row.rom_size,
            row.sram_size,
            row.checksum,
            row.reset_vector,
            row.frames,
            row.video_fnv,
            row.audio_fnv,
            row.error.replace('"', "\"\""),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::split_zip_member;
    use std::path::Path;

    #[test]
    fn splits_archive_and_member() {
        let (archive, member) = split_zip_member(Path::new(
            "/roms/Aaahh!!! Real Monsters.zip!Aaahh!!! Real Monsters.sfc",
        ));
        assert_eq!(archive, Path::new("/roms/Aaahh!!! Real Monsters.zip"));
        assert_eq!(member, Some("Aaahh!!! Real Monsters.sfc"));
    }

    #[test]
    fn leaves_plain_paths_alone() {
        let path = Path::new("/roms/game.sfc");
        let (archive, member) = split_zip_member(path);
        assert_eq!(archive, path);
        assert_eq!(member, None);
    }
}
