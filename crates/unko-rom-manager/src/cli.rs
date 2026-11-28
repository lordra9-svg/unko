//! Headless ROM manager CLI used by tests and automation.
//!
//! ```text
//! unko-rom-manager --dat DAT --input ROMS [--1g1r] [--fixdat FIX.xml]
//!                  [--csv REPORT.csv] [--copy OUT] [--zip|--torrentzip]
//!                  [--cache FILE] [--dat-url URL] [--dat-out FILE]
//!                  [--dat-sha256 HEX] [--regions USA,Europe] [--languages En] [--all]
//! ```

use std::path::PathBuf;

use crate::hash::{hex, parse_digest};
use crate::{
    audit, fetch_dat, rebuild, summary_line, write_csv, write_fixdat, OneGameOneRom, OutputFormat,
    RebuildOptions, ScanOptions, ScanProgress, VerifyOptions, VerifyReport,
};

/// Parses a 64-character hexadecimal SHA-256 digest.
fn parse_sha256(value: &str) -> Result<[u8; 32], String> {
    parse_digest::<32>(value)
        .ok_or_else(|| "--dat-sha256 needs 64 hexadecimal characters (no separators)".into())
}

/// Computes the SHA-256 digest of a downloaded file for display.
fn sha256_file(path: &std::path::Path) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;

    let mut file = std::fs::File::open(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

/// Runs the headless ROM manager with command-line arguments.
///
/// # Errors
///
/// Returns a human-readable message for bad arguments or failed I/O.
pub fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut dat = None;
    let mut dat_url = None;
    let mut dat_out = None;
    let mut dat_sha256 = None;
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
    let mut sevenz = false;
    let mut move_files = false;
    let mut cache = None;
    let mut verify_mode = false;
    let mut max_list = 50usize;

    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "scan" => {}
            "verify" | "audit" => verify_mode = true,
            "--dat" => dat = args.next().map(PathBuf::from),
            "--dat-url" => dat_url = args.next(),
            "--dat-out" => dat_out = args.next().map(PathBuf::from),
            "--dat-sha256" => {
                let value = args.next().ok_or("--dat-sha256 needs 64 hex characters")?;
                dat_sha256 = Some(parse_sha256(&value)?);
            }
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
            "--7z" => sevenz = true,
            "--move" => move_files = true,
            "--cache" => cache = args.next().map(PathBuf::from),
            "--max-list" => {
                max_list = args
                    .next()
                    .ok_or("--max-list needs a number")?
                    .parse()
                    .map_err(|_| "--max-list needs a number")?;
            }
            "-h" | "--help" => {
                println!(
                    "usage: unko-rom-manager [scan|verify] --dat DAT --input ROMS [options]\n\
                     \n\
                     options: --1g1r --fixdat FILE --csv FILE --copy DIR\n\
                     \x20        --zip | --torrentzip | --7z | --move --cache FILE\n\
                     \x20        --dat-url URL [--dat-out FILE] [--dat-sha256 HEX]\n\
                     \x20        --regions USA,Europe,World,Japan --languages En --all\n\
                     \x20        --max-list N\n\
                     \n\
                     verify (or audit) hashes and reports the library without writing;\n\
                     it exits 1 unless every game is complete and there are no unknown\n\
                     or duplicate files. --max-list caps each listing (0 = unlimited).\n\
                     --copy writes loose files; --zip/--torrentzip/--7z write one archive per game;\n\
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
    if dat_sha256.is_some() && dat_url.is_none() {
        return Err("--dat-sha256 requires --dat-url".into());
    }
    if let Some(url) = dat_url {
        let destination = dat_out.unwrap_or_else(|| PathBuf::from("downloaded.dat"));
        fetch_dat(&url, &destination, dat_sha256.as_ref()).map_err(|err| err.to_string())?;
        println!("dat: downloaded {} -> {}", url, destination.display());
        if dat_sha256.is_none() {
            let digest = sha256_file(&destination)?;
            println!("dat: sha256 {}", hex(&digest));
        }
        dat = Some(destination);
    }
    if verify_mode && (copy.is_some() || zip_sets || torrentzip || sevenz || move_files) {
        return Err(
            "verify is read-only; --copy/--zip/--torrentzip/--7z/--move are not allowed".into(),
        );
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
    if verify_mode {
        let verify = VerifyReport::build(&report);
        println!("{}", verify.text(&VerifyOptions { max_list }));
        if !verify.is_clean() {
            return Err(format!(
                "library is not clean: {}/{} games complete, {} missing ROM(s), {} unknown, {} duplicate",
                verify.summary.complete,
                verify.summary.games,
                verify.missing_roms().len(),
                verify.unknown.len(),
                verify.duplicates.len()
            ));
        }
        return Ok(());
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
                } else if sevenz {
                    OutputFormat::SevenZ
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::{hash_bytes, hex};
    use std::fs;
    use std::path::Path;

    fn write_case(name: &str, extra_game: bool) -> (PathBuf, PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("unko-cli-verify-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let roms = dir.join("roms");
        fs::create_dir_all(&roms).expect("roms");
        let payload = b"verify-payload";
        let hashes = hash_bytes(payload);
        let mut file = vec![0u8; 16];
        file[..4].copy_from_slice(b"NES\x1a");
        file[4] = 1;
        file[5] = 1;
        file.extend_from_slice(payload);
        fs::write(roms.join("a.nes"), file).expect("rom");
        let mut dat = format!(
            "<datafile><header><name>Verify</name><version>1</version></header>\n\
             <game name=\"Alpha (USA)\"><rom name=\"a.nes\" size=\"{}\" crc=\"{:08x}\" sha1=\"{}\"/></game>\n",
            payload.len(),
            hashes.crc,
            hex(&hashes.sha1)
        );
        if extra_game {
            dat.push_str(
                "<game name=\"Beta (USA)\"><rom name=\"b.nes\" size=\"1\" crc=\"00000000\"/></game>\n",
            );
        }
        dat.push_str("</datafile>\n");
        let dat_path = dir.join("test.dat");
        fs::write(&dat_path, dat).expect("dat");
        (dir, dat_path, roms)
    }

    fn verify(dat: &Path, roms: &Path, extra: &[&str]) -> Result<(), String> {
        let mut args = vec![
            "verify".to_string(),
            "--dat".to_string(),
            dat.display().to_string(),
            "--input".to_string(),
            roms.display().to_string(),
        ];
        args.extend(extra.iter().map(|value| (*value).to_string()));
        run(args.into_iter())
    }

    #[test]
    fn verify_accepts_a_complete_headerless_set() {
        let (dir, dat, roms) = write_case("clean", false);
        verify(&dat, &roms, &[]).expect("clean verify");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_rejects_an_incomplete_set() {
        let (dir, dat, roms) = write_case("missing", true);
        let err = verify(&dat, &roms, &[]).expect_err("incomplete verify");
        assert!(err.contains("library is not clean"), "{err}");
        assert!(err.contains("missing ROM(s)"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn verify_refuses_to_write_into_the_library() {
        let (dir, dat, roms) = write_case("readonly", false);
        let output = dir.join("out");
        let err = verify(
            &dat,
            &roms,
            &["--copy", output.to_str().expect("utf8 path")],
        )
        .expect_err("copy in verify mode");
        assert!(err.contains("read-only"), "{err}");
        assert!(!output.exists(), "verify must not create the output");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sha256_argument_parsing() {
        let valid = "0123456789AbCdEf".repeat(4);
        assert_eq!(valid.len(), 64);
        assert!(parse_sha256(&valid).is_ok());
        assert!(parse_sha256(&"a".repeat(63)).is_err());
        assert!(parse_sha256(&"z".repeat(64)).is_err());
        // Correct length and hex digits but with a separator.
        let separated = format!("{}:{}", "a".repeat(32), "b".repeat(31));
        assert_eq!(separated.len(), 64);
        assert!(parse_sha256(&separated).is_err());
        assert!(parse_sha256(&"a".repeat(65)).is_err());
    }
}
