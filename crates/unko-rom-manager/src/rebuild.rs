//! Safe copy/rebuild of matched ROMs into an output folder.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::catalog::{MatchReport, MatchStatus};
use crate::hash::hash_bytes;
use crate::scan::{Origin, ScannedRom};

/// How rebuilt sets are written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Loose files, one folder per game unless [`RebuildOptions::per_game`]
    /// is off.
    #[default]
    Loose,
    /// One deterministic ZIP per selected game (`<game>.zip`).
    Zip,
    /// TorrentZip-style archive: sorted entries, the TorrentZip timestamp and
    /// no extra fields. Deflate output is not guaranteed byte-identical to
    /// the reference zlib build.
    TorrentZip,
}

/// Rebuild behaviour.
#[derive(Debug, Clone)]
pub struct RebuildOptions {
    /// Destination folder.
    pub output: PathBuf,
    /// Put each game's ROMs in a folder named after the game.
    pub per_game: bool,
    /// Replace files that already exist.
    pub overwrite: bool,
    /// Skip games that are not fully matched.
    pub only_complete: bool,
    /// Verify each written file against the DAT entry.
    pub verify: bool,
    /// Write loose files or one ZIP archive per game.
    pub format: OutputFormat,
    /// Move loose source files instead of copying them after a verified write.
    ///
    /// Only valid with [`OutputFormat::Loose`]; archive members are never
    /// removed from their container.
    pub move_files: bool,
}

impl Default for RebuildOptions {
    fn default() -> Self {
        Self {
            output: PathBuf::from("rebuilt"),
            per_game: true,
            overwrite: false,
            only_complete: true,
            verify: true,
            format: OutputFormat::Loose,
            move_files: false,
        }
    }
}

/// Result of a rebuild.
#[derive(Debug, Clone, Default)]
pub struct RebuildReport {
    /// Files written (loose files or archive members).
    pub written: usize,
    /// ZIP archives written.
    pub archives: usize,
    /// Files skipped because they already existed or were not matched.
    pub skipped: usize,
    /// Loose source files removed after a verified move.
    pub moved: usize,
    /// Per-file errors.
    pub errors: Vec<String>,
}

struct OutputEntry {
    name: String,
    bytes: Vec<u8>,
    /// Loose source file that may be removed by a move.
    source: Option<PathBuf>,
}

/// Copies the matched ROMs of `selected` games into `options.output`.
///
/// With [`OutputFormat::Zip`] each game becomes one archive named after the
/// game. Archive entries are stored in DAT order with a fixed timestamp and
/// mode, so rebuilding the same set twice produces byte-identical archives.
#[must_use]
pub fn rebuild(
    report: &MatchReport,
    selected: &[usize],
    options: &RebuildOptions,
) -> RebuildReport {
    let mut result = RebuildReport::default();
    if options.move_files && options.format != OutputFormat::Loose {
        result
            .errors
            .push("move requires loose output; archive output keeps the source files".into());
        return result;
    }
    if let Err(err) = fs::create_dir_all(&options.output) {
        result
            .errors
            .push(format!("create {}: {err}", options.output.display()));
        return result;
    }
    for &game_index in selected {
        let Some(game) = report.dat.games.get(game_index) else {
            continue;
        };
        let status = report.games[game_index].status;
        if options.only_complete && status != MatchStatus::Complete {
            result.skipped += game.roms.len();
            continue;
        }
        let mut entries = match prepare_game(report, game_index, options, &mut result.errors) {
            Some(entries) => entries,
            None => {
                result.skipped += game.roms.len();
                continue;
            }
        };
        if entries.is_empty() {
            continue;
        }
        match options.format {
            OutputFormat::Loose => {
                write_loose(&mut result, options, &game.name, &mut entries);
            }
            OutputFormat::Zip | OutputFormat::TorrentZip => {
                write_archive(&mut result, options, &game.name, &entries);
            }
        }
    }
    result
}

/// Loads and verifies every ROM for one game.
///
/// Returns `None` when a ROM is unmatched, unreadable or fails verification;
/// the reason is pushed to `errors` and the caller records the skip.
fn prepare_game(
    report: &MatchReport,
    game_index: usize,
    options: &RebuildOptions,
    errors: &mut Vec<String>,
) -> Option<Vec<OutputEntry>> {
    let game = report.dat.games.get(game_index)?;
    let mut entries = Vec::with_capacity(game.roms.len());
    for (rom_index, rom) in game.roms.iter().enumerate() {
        let flat = report.flat_index(game_index, rom_index)?;
        let matched = report.rom_match.get(flat).and_then(|entry| *entry)?;
        let scanned = report.scanned.get(matched.scanned)?;
        let bytes = match scanned.load_for(matched.headerless) {
            Ok(bytes) => bytes,
            Err(err) => {
                errors.push(err.to_string());
                return None;
            }
        };
        if options.verify {
            if let Some(error) = verify_rom(rom, &bytes, scanned) {
                errors.push(error);
                return None;
            }
        }
        let source = match &scanned.origin {
            Origin::File(path) => Some(path.clone()),
            _ => None,
        };
        entries.push(OutputEntry {
            name: sanitize(&rom.name),
            bytes,
            source,
        });
    }
    Some(entries)
}

fn verify_rom(rom: &crate::dat::DatRom, bytes: &[u8], scanned: &ScannedRom) -> Option<String> {
    let hashes = hash_bytes(bytes);
    if hashes.size != rom.size {
        return Some(format!(
            "{}: size {} does not match DAT size {}",
            scanned.origin.display(),
            hashes.size,
            rom.size
        ));
    }
    if let Some(expected) = rom.sha1 {
        if hashes.sha1 != expected {
            return Some(format!(
                "{}: SHA-1 does not match the DAT",
                scanned.origin.display()
            ));
        }
    } else if let Some(expected) = rom.crc {
        if hashes.crc != expected {
            return Some(format!(
                "{}: CRC-32 does not match the DAT",
                scanned.origin.display()
            ));
        }
    } else if let Some(expected) = rom.md5 {
        if hashes.md5 != expected {
            return Some(format!(
                "{}: MD5 does not match the DAT",
                scanned.origin.display()
            ));
        }
    }
    None
}

fn write_loose(
    result: &mut RebuildReport,
    options: &RebuildOptions,
    game: &str,
    entries: &mut Vec<OutputEntry>,
) {
    let directory = if options.per_game {
        options.output.join(sanitize(game))
    } else {
        options.output.clone()
    };
    if let Err(err) = fs::create_dir_all(&directory) {
        result
            .errors
            .push(format!("create {}: {err}", directory.display()));
        return;
    }
    for entry in entries.drain(..) {
        let destination = directory.join(&entry.name);
        if destination.exists() && !options.overwrite {
            result.skipped += 1;
            continue;
        }
        match fs::write(&destination, &entry.bytes) {
            Ok(()) => {
                result.written += 1;
                if options.move_files {
                    if let Some(source) = entry.source {
                        if !same_file(&source, &destination) {
                            match fs::remove_file(&source) {
                                Ok(()) => result.moved += 1,
                                Err(err) => result.errors.push(format!(
                                    "moved copy written but source {} could not be removed: {err}",
                                    source.display()
                                )),
                            }
                        }
                    }
                }
            }
            Err(err) => result
                .errors
                .push(format!("write {}: {err}", destination.display())),
        }
    }
}

fn write_archive(
    result: &mut RebuildReport,
    options: &RebuildOptions,
    game: &str,
    entries: &[OutputEntry],
) {
    let destination = options.output.join(format!("{}.zip", sanitize(game)));
    if destination.exists() && !options.overwrite {
        result.skipped += entries.len();
        return;
    }
    let torrentzip = matches!(options.format, OutputFormat::TorrentZip);
    let payload: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|entry| (entry.name.clone(), entry.bytes.clone()))
        .collect();
    match archive(destination.as_path(), &payload, torrentzip) {
        Ok(()) => {
            result.written += entries.len();
            result.archives += 1;
        }
        Err(err) => result.errors.push(err),
    }
}

fn archive(
    destination: &Path,
    entries: &[(String, Vec<u8>)],
    torrentzip: bool,
) -> Result<(), String> {
    let part = destination.with_extension("zip.part");
    let result = write_archive_file(&part, entries, torrentzip);
    match result {
        Ok(()) => fs::rename(&part, destination)
            .map_err(|err| format!("rename {}: {err}", destination.display())),
        Err(err) => {
            let _ = fs::remove_file(&part);
            Err(err)
        }
    }
}

fn write_archive_file(
    part: &Path,
    entries: &[(String, Vec<u8>)],
    torrentzip: bool,
) -> Result<(), String> {
    let file = fs::File::create(part).map_err(|err| format!("create {}: {err}", part.display()))?;
    let mut writer = ZipWriter::new(file);
    let mut ordered: Vec<&(String, Vec<u8>)> = entries.iter().collect();
    if torrentzip {
        ordered.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let mut options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(if torrentzip {
            DateTime::from_date_and_time(1996, 12, 24, 23, 32, 0)
                .expect("the TorrentZip timestamp is a valid DOS date")
        } else {
            DateTime::default()
        });
    options = if torrentzip {
        options.compression_level(Some(9)).unix_permissions(0)
    } else {
        options.unix_permissions(0o644)
    };
    let mut names = HashSet::with_capacity(entries.len());
    for (name, bytes) in ordered {
        if !names.insert(name.as_str()) {
            return Err(format!("{name}: duplicate entry in the game"));
        }
        writer
            .start_file(name.as_str(), options)
            .map_err(|err| format!("{}: {err}", part.display()))?;
        writer
            .write_all(bytes)
            .map_err(|err| format!("{}: {err}", part.display()))?;
    }
    writer
        .finish()
        .map(|_| ())
        .map_err(|err| format!("{}: {err}", part.display()))
}

/// Whether two paths refer to the same file (or the same lexical path).
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn sanitize(name: &str) -> String {
    let file_name = Path::new(name)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    let mut out: String = file_name
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                ch
            }
        })
        .collect();
    while out.ends_with([' ', '.']) {
        out.pop();
    }
    if out.is_empty() {
        "rom".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::dat::parse_dat_str;
    use crate::hash::hash_bytes;
    use crate::scan::{Origin, ScannedRom};
    use std::io::Read;

    fn fixture() -> (MatchReport, std::path::PathBuf) {
        let payload = b"rom bytes";
        let second = b"second rom";
        let hashes = hash_bytes(payload);
        let second_hashes = hash_bytes(second);
        let dat = parse_dat_str(&format!(
            r#"<datafile><header><name>T</name></header>
              <game name="Game (USA)" id="1">
                <rom name="Game (USA).bin" size="{}" crc="{:08x}" sha1="{}"/>
                <rom name="Game (USA) (Track 2).bin" size="{}" crc="{:08x}" sha1="{}"/>
              </game>
            </datafile>"#,
            hashes.size,
            hashes.crc,
            hashes.sha1_hex(),
            second_hashes.size,
            second_hashes.crc,
            second_hashes.sha1_hex()
        ))
        .expect("dat");
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let output = std::env::temp_dir().join(format!(
            "unko-rebuild-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos())
        ));
        let _ = fs::remove_dir_all(&output);
        fs::create_dir_all(&output).expect("dir");
        let source = output.join("source.bin");
        fs::write(&source, payload).expect("source");
        let source2 = output.join("source2.bin");
        fs::write(&source2, second).expect("source2");
        let report = Catalog::new(dat).match_all(vec![
            ScannedRom {
                origin: Origin::File(source),
                file_name: "source.bin".into(),
                hashes,
                headerless: None,
                modified: None,
            },
            ScannedRom {
                origin: Origin::File(source2),
                file_name: "source2.bin".into(),
                hashes: second_hashes,
                headerless: None,
                modified: None,
            },
        ]);
        (report, output)
    }

    #[test]
    fn copies_and_verifies_matched_files() {
        let (report, output) = fixture();
        let result = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: true,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(result.written, 2);
        assert!(result.errors.is_empty());
        assert_eq!(
            fs::read(output.join("Game (USA)").join("Game (USA).bin")).expect("read"),
            b"rom bytes"
        );
        let _ = fs::remove_dir_all(&output);
    }

    #[test]
    fn writes_one_verified_zip_per_game() {
        let (report, output) = fixture();
        let result = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: true,
                format: OutputFormat::Zip,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(result.archives, 1);
        assert_eq!(result.written, 2);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let file = fs::File::open(output.join("Game (USA).zip")).expect("archive");
        let mut archive = zip::ZipArchive::new(file).expect("zip");
        assert_eq!(archive.len(), 2);
        {
            let mut first = archive.by_index(0).expect("member");
            assert_eq!(first.name().expect("name"), "Game (USA).bin");
            let mut bytes = Vec::new();
            first.read_to_end(&mut bytes).expect("read member");
            assert_eq!(bytes, b"rom bytes");
        }
        let second = archive.by_index(1).expect("member");
        assert_eq!(second.name().expect("name"), "Game (USA) (Track 2).bin");
        let _ = fs::remove_dir_all(&output);
    }

    #[test]
    fn zip_output_is_deterministic() {
        let (report, output) = fixture();
        let first = output.join("first");
        let second = output.join("second");
        for destination in [&first, &second] {
            let result = rebuild(
                &report,
                &[0],
                &RebuildOptions {
                    output: destination.clone(),
                    overwrite: true,
                    format: OutputFormat::Zip,
                    ..RebuildOptions::default()
                },
            );
            assert_eq!(result.archives, 1);
        }
        assert_eq!(
            fs::read(first.join("Game (USA).zip")).expect("first"),
            fs::read(second.join("Game (USA).zip")).expect("second")
        );
        let _ = fs::remove_dir_all(&output);
    }

    #[test]
    fn torrentzip_style_sorts_entries_and_uses_the_fixed_timestamp() {
        let (report, output) = fixture();
        let result = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: true,
                format: OutputFormat::TorrentZip,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(result.archives, 1);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let file = fs::File::open(output.join("Game (USA).zip")).expect("archive");
        let mut archive = zip::ZipArchive::new(file).expect("zip");
        // The DAT order is Track 2 first, Game second; TorrentZip sorts them.
        let names: Vec<String> = (0..archive.len())
            .map(|index| {
                archive
                    .by_index(index)
                    .expect("member")
                    .name()
                    .expect("name")
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec!["Game (USA) (Track 2).bin", "Game (USA).bin"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        let member = archive.by_index(0).expect("member");
        assert_eq!(
            member.last_modified(),
            Some(
                zip::DateTime::from_date_and_time(1996, 12, 24, 23, 32, 0).expect("valid DOS date")
            )
        );
        let _ = fs::remove_dir_all(&output);
    }

    #[test]
    fn zip_skips_existing_archive_without_overwrite() {
        let (report, output) = fixture();
        let options = RebuildOptions {
            output: output.clone(),
            overwrite: true,
            format: OutputFormat::Zip,
            ..RebuildOptions::default()
        };
        assert_eq!(rebuild(&report, &[0], &options).archives, 1);
        let again = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                overwrite: false,
                ..options
            },
        );
        assert_eq!(again.archives, 0);
        assert_eq!(again.skipped, 2);
        let _ = fs::remove_dir_all(&output);
    }

    #[test]
    fn moves_loose_sources_after_a_verified_write() {
        let (report, output) = fixture();
        let source = output.join("source.bin");
        let second = output.join("source2.bin");
        let result = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: true,
                move_files: true,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(result.moved, 2);
        assert_eq!(result.written, 2);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(!source.exists(), "loose source removed");
        assert!(!second.exists());
        assert_eq!(
            fs::read(output.join("Game (USA)").join("Game (USA).bin")).expect("read"),
            b"rom bytes"
        );
        let _ = fs::remove_dir_all(&output);
    }

    #[test]
    fn move_keeps_sources_when_the_destination_exists() {
        let (report, output) = fixture();
        let source = output.join("source.bin");
        let copy = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: true,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(copy.written, 2);
        let result = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: false,
                move_files: true,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(result.moved, 0);
        assert_eq!(result.skipped, 2);
        assert!(source.exists(), "a skipped file must not be deleted");
        let _ = fs::remove_dir_all(&output);
    }

    #[test]
    fn move_rejects_archive_output() {
        let (report, output) = fixture();
        let result = rebuild(
            &report,
            &[0],
            &RebuildOptions {
                output: output.clone(),
                overwrite: true,
                format: OutputFormat::Zip,
                move_files: true,
                ..RebuildOptions::default()
            },
        );
        assert_eq!(result.written, 0);
        assert_eq!(result.archives, 0);
        assert!(result.errors[0].contains("move requires loose"));
        assert!(output.join("source.bin").exists());
        let _ = fs::remove_dir_all(&output);
    }
}
