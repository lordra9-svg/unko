//! Runs the local public mapper test-ROM suites and records per-mapper results.
//!
//! Usage:
//!
//! ```text
//! cargo run --release -p unko-nes-dev --example mapper_suite -- \
//!     /var/home/Lordra/ai/refs/rustynes/tests/roms
//! ```
//!
//! The ROMs are not committed. Missing suite directories are reported and
//! skipped, so the harness also works on a partial local drop.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use unko_nes::System;
use unko_nes_dev::mapper_suite::{
    classify_blargg, distinct_colors, mapper_number, parse_holy_mapperel_result,
};

#[derive(Clone, Copy)]
enum Oracle {
    Blargg,
    HolyMapperel,
    Smoke,
}

struct Case {
    suite: &'static str,
    path: PathBuf,
    frames: u32,
    alt_mmc3: bool,
    oracle: Oracle,
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: mapper_suite <rom-root> [--frames N]");
        return ExitCode::FAILURE;
    };
    let mut override_frames = None;
    while let Some(flag) = args.next() {
        if flag == "--frames" {
            match args.next().and_then(|value| value.parse::<u32>().ok()) {
                Some(frames) => override_frames = Some(frames),
                None => {
                    eprintln!("--frames needs a number");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            eprintln!("unknown argument {flag}");
            return ExitCode::FAILURE;
        }
    }
    let root = Path::new(&root);
    if !root.is_dir() {
        eprintln!(
            "{} is not a directory; public test ROMs are not committed",
            root.display()
        );
        return ExitCode::FAILURE;
    }

    let mut cases = Vec::new();
    add_suite(
        root,
        "holy_mapperel",
        "holy_mapperel",
        2400,
        Oracle::HolyMapperel,
        &mut cases,
    );
    add_suite(
        root,
        "extra/mappers",
        "holy_mapperel",
        2400,
        Oracle::HolyMapperel,
        &mut cases,
    );
    for suite in [
        "blargg/mmc3_test",
        "blargg/mmc3_test_2",
        "blargg/mmc3_irq_tests",
    ] {
        add_suite(root, suite, suite, 1200, Oracle::Blargg, &mut cases);
    }
    add_suite(root, "mmc5", "mmc5", 900, Oracle::Smoke, &mut cases);
    add_suite(root, "m22", "m22 (VRC2a)", 900, Oracle::Smoke, &mut cases);
    add_suite(root, "mmc1_a12", "mmc1_a12", 900, Oracle::Smoke, &mut cases);
    if let Some(frames) = override_frames {
        for case in &mut cases {
            case.frames = frames;
        }
    }

    let mut by_mapper: BTreeMap<u16, (u32, u32)> = BTreeMap::new();
    let mut failures = 0u32;
    let mut no_verdict = 0u32;
    for case in &cases {
        let rom = match fs::read(&case.path) {
            Ok(rom) => rom,
            Err(err) => {
                println!(
                    "FAIL suite={} file={} read: {err}",
                    case.suite,
                    case.path.display()
                );
                failures += 1;
                continue;
            }
        };
        let mapper = mapper_number(&rom).unwrap_or(0xFFFF);
        let outcome = run_case(case, &rom);
        let entry = by_mapper.entry(mapper).or_insert((0, 0));
        entry.1 += 1;
        let name = case
            .path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        match outcome {
            Ok((true, detail)) => {
                entry.0 += 1;
                println!(
                    "pass mapper={mapper:03} suite={} file={name} {detail}",
                    case.suite
                );
            }
            Ok((false, detail)) => {
                failures += 1;
                if detail.starts_with("oracle=no-verdict") {
                    no_verdict += 1;
                }
                println!(
                    "FAIL mapper={mapper:03} suite={} file={name} {detail}",
                    case.suite
                );
            }
            Err(err) => {
                failures += 1;
                println!(
                    "FAIL mapper={mapper:03} suite={} file={name} error={err}",
                    case.suite
                );
            }
        }
    }

    println!();
    for (mapper, (pass, total)) in &by_mapper {
        println!("mapper {mapper:03}: {pass}/{total}");
    }
    println!(
        "SUMMARY: {} cases, {} failed, {} without a verdict",
        cases.len(),
        failures,
        no_verdict
    );
    if failures == 0 && no_verdict == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn add_suite(
    root: &Path,
    relative: &str,
    suite: &'static str,
    frames: u32,
    oracle: Oracle,
    cases: &mut Vec<Case>,
) {
    let directory = root.join(relative);
    let Ok(entries) = fs::read_dir(&directory) else {
        println!("skip suite={suite} ({} not present)", directory.display());
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "nes"))
        .collect();
    files.sort();
    for path in files {
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let alt_mmc3 = name.contains("rev_A") || name.contains("MMC3_alt") || name.contains("MMC6");
        cases.push(Case {
            suite,
            path,
            frames,
            alt_mmc3,
            oracle,
        });
    }
}

/// Runs one ROM and returns `(passed, detail)`.
fn run_case(case: &Case, rom: &[u8]) -> Result<(bool, String), String> {
    let mut system = System::from_ines(rom).map_err(|err| err.to_string())?;
    system.set_mmc3_alternate(case.alt_mmc3);
    for _ in 0..case.frames {
        system.run_frame();
    }
    let rows = screen_rows(&system);
    let ram = ram_text(&system);
    match case.oracle {
        Oracle::Blargg => {
            let text = format!("{}\n{ram}", rows.join("\n"));
            let verdict = classify_blargg(&text);
            Ok((
                verdict.unwrap_or(false),
                match verdict {
                    Some(true) => "oracle=text result=passed".to_string(),
                    Some(false) => "oracle=text result=failed".to_string(),
                    None => "oracle=no-verdict".to_string(),
                },
            ))
        }
        Oracle::HolyMapperel => match parse_holy_mapperel_result(&rows) {
            Some(code) => Ok((code == 0, format!("oracle=holy-mapperel code={code:04X}"))),
            None => Ok((false, "oracle=no-verdict".into())),
        },
        Oracle::Smoke => {
            let colors = distinct_colors(system.video());
            Ok((colors > 1, format!("oracle=smoke distinct_colors={colors}")))
        }
    }
}

fn screen_rows(system: &System) -> Vec<String> {
    (0..30u16)
        .map(|row| {
            (0..32u16)
                .map(
                    |column| match system.vram_peek(0x2000 + row * 32 + column) {
                        0 => ' ',
                        0x20..=0x7E => char::from(system.vram_peek(0x2000 + row * 32 + column)),
                        _ => '?',
                    },
                )
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn ram_text(system: &System) -> String {
    let mut text = String::new();
    for address in 0x6001..0x6100u16 {
        let byte = system.read_ram(address);
        if byte == 0 {
            break;
        }
        text.push(if (0x20..=0x7E).contains(&byte) {
            char::from(byte)
        } else {
            '.'
        });
    }
    text
}
