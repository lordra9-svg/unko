//! Headless ROM manager CLI used by tests and automation.
//!
//! ```text
//! unko-rom-manager --dat DAT --input ROMS [--1g1r] [--fixdat FIX.xml]
//!                  [--csv REPORT.csv] [--copy OUT] [--zip|--torrentzip]
//!                  [--cache FILE] [--dat-url URL] [--dat-out FILE]
//!                  [--regions USA,Europe] [--languages En] [--all]
//! ```

use std::path::PathBuf;

use crate::{
    audit, fetch_dat, rebuild, summary_line, write_csv, write_fixdat, OneGameOneRom, OutputFormat,
    RebuildOptions, ScanOptions, ScanProgress,
};

/// Runs the headless ROM manager with command-line arguments.
///
/// # Errors
///
/// Returns a human-readable message for bad arguments or failed I/O.
pub fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut dat = None;
    let mut dat_url = None;
    let mut dat_out = None;
    let mut input = None;
    let mut fixdat = None;
    let mut csv = None;
    let mut copy = None;
    let mut one_g1r = false;
    let mut regions = vec![
        "USA".to_string(),
        "Europe".to_string(),
        "World".to_string(),
        "Japan".to_string(),
    ];
    let mut languages = vec!["En".to_string()];
    let mut include_bios = false;
    let mut zip_sets = false;
    let mut torrentzip = false;
    let mut move_files = false;
    let mut cache = None;

    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "scan" => {}
            "--dat" => dat = args.next().map(PathBuf::from),
            "--dat-url" => dat_url = args.next(),
            "--dat-out" => dat_out = args.next().map(PathBuf::from),
            "--input" => input = args.next().map(PathBuf::from),
            "--fixdat" => fixdat = args.next().map(PathBuf::from),
            "--csv" => csv = args.next().map(PathBuf::from),
            "--copy" => copy = args.next().map(PathBuf::from),
            "--1g1r" => one_g1r = true,
            "--regions" => {
                regions = args
                    .next()
                    .ok_or("--regions needs a comma-separated list")?
                    .split(',')
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect();
            }
            "--languages" => {
                languages = args
                    .next()
                    .ok_or("--languages needs a comma-separated list")?
                    .split(',')
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .collect();
            }
            "--all" => include_bios = true,
            "--zip" => zip_sets = true,
            "--torrentzip" => torrentzip = true,
            "--move" => move_files = true,
            "--cache" => cache = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!(
                    "usage: unko-rom-manager --dat DAT --input ROMS [options]\n\
                     \n\
                     options: --1g1r --fixdat FILE --csv FILE --copy DIR\n\
                     \x20        --zip | --torrentzip | --move --cache FILE\n\
                     \x20        --dat-url URL [--dat-out FILE]\n\
                     \x20        --regions USA,Europe,World,Japan --languages En --all\n\
                     \n\
                     --copy writes loose files; --zip/--torrentzip write one archive per game;\n\
                     --move deletes loose sources after a verified copy."
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }

    if dat.is_some() && dat_url.is_some() {
        return Err("use either --dat or --dat-url".into());
    }
    if let Some(url) = dat_url {
        let destination = dat_out.unwrap_or_else(|| PathBuf::from("downloaded.dat"));
        fetch_dat(&url, &destination).map_err(|err| err.to_string())?;
        println!("dat: downloaded {} -> {}", url, destination.display());
        dat = Some(destination);
    }
    let dat = dat.ok_or("--dat or --dat-url is required")?;
    let input = input.ok_or("--input is required")?;
    let options = ScanOptions {
        cache,
        ..ScanOptions::default()
    };
    let mut last = 0usize;
    let mut progress = |stats: &ScanProgress| {
        let count = stats.files + stats.members;
        if count >= last + 1000 {
            last = count;
            eprintln!(
                "scan: {} files, {} members, {:.1} MiB",
                stats.files,
                stats.members,
                stats.bytes as f64 / (1024.0 * 1024.0)
            );
        }
    };
    let report = audit(&dat, &input, &options, &mut progress).map_err(|err| err.to_string())?;
    println!("DAT: {} ({})", report.dat.name, report.dat.version);
    println!("{}", summary_line(&report));

    let selection = if one_g1r {
        let options = OneGameOneRom {
            regions,
            languages,
            exclude_bios: !include_bios,
            ..OneGameOneRom::default()
        };
        let selected = options.select(&report.dat);
        println!("1G1R: {} games selected", selected.len());
        selected
    } else {
        Vec::new()
    };

    if let Some(path) = fixdat {
        write_fixdat(&report, &path).map_err(|err| err.to_string())?;
        println!("fixdat: {}", path.display());
    }
    if let Some(path) = csv {
        write_csv(&report, &path).map_err(|err| err.to_string())?;
        println!("csv: {}", path.display());
    }
    if let Some(output) = copy {
        let selected = if selection.is_empty() {
            (0..report.dat.games.len()).collect::<Vec<_>>()
        } else {
            selection
        };
        let result = rebuild(
            &report,
            &selected,
            &RebuildOptions {
                output: output.clone(),
                move_files,
                format: if torrentzip {
                    OutputFormat::TorrentZip
                } else if zip_sets {
                    OutputFormat::Zip
                } else {
                    OutputFormat::Loose
                },
                ..RebuildOptions::default()
            },
        );
        println!(
            "copy: {} written, {} archives, {} moved, {} skipped, {} errors -> {}",
            result.written,
            result.archives,
            result.moved,
            result.skipped,
            result.errors.len(),
            output.display()
        );
        for error in result.errors.iter().take(10) {
            eprintln!("  {error}");
        }
    }
    Ok(())
}
