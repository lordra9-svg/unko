//! Headless per-archive smoke scanner for the compatibility catalog.
//!
//! Reads a directory or a newline-separated archive list, loads the first ROM
//! member of each ZIP, runs the selected core for a fixed number of frames, and
//! writes one JSON object per archive. It never writes to the input library.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::json;
use sha2::{Digest, Sha256};
use unko_core::{Core, LoadOptions};
use unko_n64_host::N64Core;
use unko_nes_api::NesCore;
use unko_nes_host::AccuracyCore;
use unko_snes_host::{FirmwareKind, SnesCore};
use zip::ZipArchive;

const ROM_BYTES_LIMIT: u64 = 128 * 1024 * 1024;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoreKind {
    Nes,
    Fds,
    Snes,
    N64,
    None,
}

pub fn run(args: &[String]) -> i32 {
    let mut system = "unknown".to_string();
    let mut format = String::new();
    let mut core = CoreKind::Nes;
    let mut input: Option<PathBuf> = None;
    let mut list: Option<PathBuf> = None;
    let mut frames: u32 = 30;
    let mut limit: usize = usize::MAX;
    let mut batch: usize = 250;
    let mut ipl: Option<PathBuf> = None;
    let mut bios: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;

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
            "--system" => system = value(&mut index).unwrap_or(system),
            "--format" => format = value(&mut index).unwrap_or(format),
            "--core" => {
                core = match value(&mut index).unwrap_or_default().as_str() {
                    "nes" => CoreKind::Nes,
                    "fds" => CoreKind::Fds,
                    "snes" => CoreKind::Snes,
                    "n64" => CoreKind::N64,
                    "none" => CoreKind::None,
                    other => {
                        eprintln!("compat-scan: unknown core '{other}'");
                        return 2;
                    }
                }
            }
            "--input" | "--dir" => input = value(&mut index).map(PathBuf::from),
            "--list" => list = value(&mut index).map(PathBuf::from),
            "--frames" => {
                frames = value(&mut index)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(frames)
            }
            "--limit" => {
                limit = value(&mut index)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(limit)
            }
            "--batch" => {
                batch = value(&mut index)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(batch)
                    .max(1)
            }
            "--ipl" => ipl = value(&mut index).map(PathBuf::from),
            "--bios" => bios = value(&mut index).map(PathBuf::from),
            "--out" => out = value(&mut index).map(PathBuf::from),
            "-h" | "--help" => {
                println!(
                    "usage: xtask compat-scan --system ID --format LABEL --core nes|fds|snes|n64|none\n\
                     \x20            (--input DIR | --list FILE) [--frames N] [--limit N]\n\
                     \x20            [--batch N] [--ipl IPL.ROM] [--bios BIOS.ROM] --out FILE.jsonl"
                );
                return 0;
            }
            other => {
                eprintln!("compat-scan: unknown argument '{other}'");
                return 2;
            }
        }
    }

    let Some(out) = out else {
        eprintln!("compat-scan: --out is required");
        return 2;
    };
    let mut paths = Vec::new();
    if let Some(input) = input {
        collect(&input, &mut paths);
    }
    if let Some(list) = list {
        match fs::read_to_string(list) {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.trim();
                    if !line.is_empty() {
                        paths.push(PathBuf::from(line));
                    }
                }
            }
            Err(error) => {
                eprintln!("compat-scan: reading list: {error}");
                return 1;
            }
        }
    }
    if paths.is_empty() {
        eprintln!("compat-scan: --input or --list produced no files");
        return 2;
    }
    paths.sort();
    paths.dedup();
    if let Some(parent) = out.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            eprintln!("compat-scan: create output directory: {error}");
            return 1;
        }
    }
    let mut writer = match File::create(&out) {
        Ok(file) => std::io::BufWriter::new(file),
        Err(error) => {
            eprintln!("compat-scan: create {}: {error}", out.display());
            return 1;
        }
    };

    let ipl = ipl.and_then(|path| match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            eprintln!("compat-scan: reading IPL {}: {error}", path.display());
            None
        }
    });
    let bios = bios.and_then(|path| match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            eprintln!("compat-scan: reading BIOS {}: {error}", path.display());
            None
        }
    });

    let started = Instant::now();
    let mut ok = 0usize;
    let mut unsupported = 0usize;
    let mut failed = 0usize;
    let mut processed = 0usize;
    for path in paths {
        if processed >= limit {
            break;
        }
        processed += 1;
        let row = scan_one(
            &path,
            &system,
            &format,
            core,
            frames,
            ipl.as_deref(),
            bios.as_deref(),
        );
        if row.status == "runs" {
            ok += 1;
        } else if row.status == "unsupported" {
            unsupported += 1;
        } else {
            failed += 1;
        }
        match serde_json::to_string(&json!({
            "system": row.system,
            "format": row.format,
            "path": row.path,
            "title": row.title,
            "sha256": row.sha256,
            "size": row.size,
            "status": row.status,
            "frames": row.frames,
            "video_fnv": row.video_fnv,
            "audio_fnv": row.audio_fnv,
            "error": row.error,
        })) {
            Ok(line) => {
                if writeln!(writer, "{line}").is_err() {
                    eprintln!("compat-scan: write failed");
                    return 1;
                }
            }
            Err(error) => {
                eprintln!("compat-scan: serializing row: {error}");
                return 1;
            }
        }
        if processed.is_multiple_of(batch) {
            println!(
                "{}: {processed} scanned, {ok} ran, {unsupported} unsupported, {failed} failed, {:?}",
                system,
                started.elapsed()
            );
        }
    }
    if let Err(error) = writer.flush() {
        eprintln!("compat-scan: flush: {error}");
        return 1;
    }
    println!(
        "compat-scan {system}: {processed} archives, {ok} ran, {unsupported} unsupported, {failed} failed, {:?} -> {}",
        started.elapsed(),
        out.display()
    );
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
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
            || is_rom(&path)
        {
            out.push(path);
        }
    }
}

fn is_rom(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "nes"
                    | "unh"
                    | "fds"
                    | "qd"
                    | "sfc"
                    | "smc"
                    | "swc"
                    | "fig"
                    | "z64"
                    | "v64"
                    | "n64"
                    | "ndd"
                    | "bin"
                    | "rom"
            )
        })
}

struct Row {
    system: String,
    format: String,
    path: String,
    title: String,
    sha256: String,
    size: u64,
    status: String,
    frames: u32,
    video_fnv: String,
    audio_fnv: String,
    error: String,
}

#[allow(clippy::too_many_arguments)]
fn scan_one(
    path: &Path,
    system: &str,
    format: &str,
    core_kind: CoreKind,
    frames: u32,
    ipl: Option<&[u8]>,
    bios: Option<&[u8]>,
) -> Row {
    let mut row = Row {
        system: system.to_string(),
        format: format.to_string(),
        path: path.display().to_string(),
        title: title_of(path),
        sha256: String::new(),
        size: 0,
        status: "nothing".to_string(),
        frames: 0,
        video_fnv: String::new(),
        audio_fnv: String::new(),
        error: String::new(),
    };
    let (member, bytes) = match load_first(path) {
        Ok(value) => value,
        Err(error) => {
            row.error = error;
            return row;
        }
    };
    if let Some(member) = member {
        row.path = format!("{}!{member}", path.display());
    }
    row.size = bytes.len() as u64;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    row.sha256 = format!("{:x}", hasher.finalize());

    if core_kind == CoreKind::None {
        row.status = "unsupported".to_string();
        row.error = "No core implementation for this image format yet".to_string();
        return row;
    }
    if core_kind == CoreKind::Fds && bios.is_none() {
        row.error = "FDS BIOS not supplied (--bios)".to_string();
        return row;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        run_core(core_kind, &bytes, frames, ipl, bios)
    }));
    match result {
        Ok(Ok((video_fnv, audio_fnv))) => {
            row.status = "runs".to_string();
            row.frames = frames;
            row.video_fnv = format!("{video_fnv:016x}");
            row.audio_fnv = format!("{audio_fnv:016x}");
        }
        Ok(Err(error)) => row.error = error,
        Err(_) => row.error = "panic while loading or running".to_string(),
    }
    row
}

fn run_core(
    kind: CoreKind,
    bytes: &[u8],
    frames: u32,
    ipl: Option<&[u8]>,
    bios: Option<&[u8]>,
) -> Result<(u64, u64), String> {
    let options = LoadOptions::default();
    let mut core: Box<dyn Core> = match kind {
        CoreKind::Nes => {
            let mut core = AccuracyCore::new();
            core.load(bytes, &options)
                .map_err(|error| error.to_string())?;
            Box::new(core)
        }
        CoreKind::Fds => {
            let mut core = AccuracyCore::new();
            let bios = bios.ok_or_else(|| "FDS BIOS not supplied".to_string())?;
            core.load_fds(bytes, bios, &options)
                .map_err(|error| error.to_string())?;
            Box::new(core)
        }
        CoreKind::Snes => {
            let mut core = SnesCore::new();
            if let Some(ipl) = ipl {
                core.set_firmware(FirmwareKind::Ipl, ipl)
                    .map_err(|error| error.to_string())?;
            }
            core.load(bytes, &options)
                .map_err(|error| error.to_string())?;
            Box::new(core)
        }
        CoreKind::N64 => {
            let mut core = N64Core::new();
            core.load(bytes, &options)
                .map_err(|error| error.to_string())?;
            Box::new(core)
        }
        CoreKind::None => unreachable!(),
    };
    let mut video_hash = FNV_OFFSET;
    let mut audio_hash = FNV_OFFSET;
    for _ in 0..frames {
        core.run_frame().map_err(|error| error.to_string())?;
        let video = core.video();
        for byte in video.data {
            video_hash ^= u64::from(*byte);
            video_hash = video_hash.wrapping_mul(FNV_PRIME);
        }
        let audio = core.audio();
        for sample in audio.samples {
            for byte in sample.to_le_bytes() {
                audio_hash ^= u64::from(byte);
                audio_hash = audio_hash.wrapping_mul(FNV_PRIME);
            }
        }
    }
    Ok((video_hash, audio_hash))
}

fn load_first(path: &Path) -> Result<(Option<String>, Vec<u8>), String> {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        let file = File::open(path).map_err(|error| format!("open: {error}"))?;
        let mut archive = ZipArchive::new(file).map_err(|error| format!("zip: {error}"))?;
        for index in 0..archive.len() {
            let mut member = match archive.by_index(index) {
                Ok(member) => member,
                Err(_) => continue,
            };
            if member.is_dir() {
                continue;
            }
            let member_name = member.name();
            let Ok(name) = member_name else {
                continue;
            };
            let name = name.to_string();
            if !is_rom(Path::new(&name)) {
                continue;
            }
            let mut bytes = Vec::new();
            member
                .by_ref()
                .take(ROM_BYTES_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("read {name}: {error}"))?;
            if bytes.len() as u64 > ROM_BYTES_LIMIT {
                return Err(format!("{name}: member exceeds {ROM_BYTES_LIMIT} bytes"));
            }
            return Ok((Some(name), bytes));
        }
        Err("no recognized ROM member".to_string())
    } else {
        let bytes = fs::read(path).map_err(|error| format!("read: {error}"))?;
        if bytes.len() as u64 > ROM_BYTES_LIMIT {
            return Err(format!("image exceeds {ROM_BYTES_LIMIT} bytes"));
        }
        Ok((None, bytes))
    }
}

fn title_of(path: &Path) -> String {
    path.file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
