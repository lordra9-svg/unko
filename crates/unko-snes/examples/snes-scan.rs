//! Scan a directory of SNES images and report header/checksum detection.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use unko_snes::Cartridge;

fn walk(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sfc") || ext.eq_ignore_ascii_case("smc"))
        {
            out.push(path);
        }
    }
}

fn main() -> ExitCode {
    let Some(root) = env::args().nth(1) else {
        eprintln!("usage: snes-scan <directory>");
        return ExitCode::FAILURE;
    };
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);
    files.sort();

    let mut ok = 0usize;
    let mut checksum_ok = 0usize;
    let mut failed = Vec::new();
    for path in &files {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                failed.push((path.clone(), error.to_string()));
                continue;
            }
        };
        match Cartridge::from_bytes(&bytes) {
            Ok(cartridge) => {
                ok += 1;
                if cartridge.header().checksum_matches(cartridge.rom()) {
                    checksum_ok += 1;
                }
            }
            Err(error) => failed.push((path.clone(), error.to_string())),
        }
    }

    println!(
        "{} images, {ok} parsed, {checksum_ok} checksum-valid, {} failed",
        files.len(),
        failed.len()
    );
    for (path, error) in &failed {
        println!("FAIL {}: {error}", path.display());
    }
    ExitCode::SUCCESS
}
