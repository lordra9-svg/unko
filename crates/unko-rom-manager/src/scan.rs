//! ROM library scanning: loose files and ZIP members.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use zip::ZipArchive;

use crate::cache::ScanCache;
use crate::hash::{hash_bytes, Hashes};
use crate::{Error, Result};

/// Default extensions treated as ROM candidates.
pub const DEFAULT_EXTENSIONS: &[&str] = &[
    "nes", "fds", "unh", "bin", "rom", "a78", "lnx", "sfc", "smc", "gb", "gbc", "gba", "n64",
    "z64", "v64", "iso", "cue", "chd", "gen", "md", "sms", "gg", "pce", "ws", "wsc",
];

/// Scanning behaviour.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Recursion limit.
    pub max_depth: usize,
    /// Extensions to hash (lowercase, without the dot). Empty means all files.
    pub extensions: Vec<String>,
    /// Read members inside ZIP archives.
    pub include_archives: bool,
    /// Optional persistent hash cache.
    pub cache: Option<PathBuf>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_depth: 8,
            extensions: DEFAULT_EXTENSIONS.iter().map(|ext| (*ext).into()).collect(),
            include_archives: true,
            cache: None,
        }
    }
}

impl ScanOptions {
    /// Whether `path` should be hashed.
    #[must_use]
    pub fn accepts(&self, path: &Path) -> bool {
        if self.extensions.is_empty() {
            return true;
        }
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            return false;
        };
        let extension = extension.to_ascii_lowercase();
        self.extensions
            .iter()
            .any(|candidate| candidate == &extension)
    }
}

/// Progress reported during a scan.
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    /// Loose files visited.
    pub files: usize,
    /// Archive members hashed.
    pub members: usize,
    /// Bytes hashed.
    pub bytes: u64,
    /// Path currently being read.
    pub current: String,
    /// Entries reused from the hash cache.
    pub cache_hits: usize,
}

/// Where a scanned ROM came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A loose file.
    File(PathBuf),
    /// A member of a ZIP archive.
    Zip {
        /// Archive path.
        archive: PathBuf,
        /// Member index.
        index: usize,
        /// Member name.
        member: String,
    },
    /// A member of a 7z or RAR archive.
    Archive {
        /// Archive path.
        archive: PathBuf,
        /// Member index (RAR); 7z members are read by name.
        index: Option<usize>,
        /// Member name.
        member: String,
    },
}

impl Origin {
    /// Human-readable source path.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Zip {
                archive, member, ..
            }
            | Self::Archive {
                archive, member, ..
            } => format!("{}/{}", archive.display(), member),
        }
    }
}

/// One hashed file or archive member.
#[derive(Debug, Clone)]
pub struct ScannedRom {
    /// Source location.
    pub origin: Origin,
    /// File name used for matching/output.
    pub file_name: String,
    /// Hash of the complete bytes.
    pub hashes: Hashes,
    /// Hash of the bytes after removing a 16-byte iNES header.
    pub headerless: Option<Hashes>,
    /// Last modification time, seconds since the Unix epoch.
    pub modified: Option<u64>,
}

impl ScannedRom {
    /// Reads the bytes for this entry.
    pub fn load(&self) -> Result<Vec<u8>> {
        match &self.origin {
            Origin::File(path) => {
                fs::read(path).map_err(|err| Error::Io(format!("{}: {err}", path.display())))
            }
            Origin::Zip { archive, index, .. } => {
                let file = File::open(archive)
                    .map_err(|err| Error::Io(format!("{}: {err}", archive.display())))?;
                let mut zip = ZipArchive::new(file)
                    .map_err(|err| Error::Zip(format!("{}: {err}", archive.display())))?;
                let mut member = zip.by_index(*index).map_err(|err| {
                    Error::Zip(format!("{}: member {index}: {err}", archive.display()))
                })?;
                let member_name = member
                    .name()
                    .map(|name| name.into_owned())
                    .unwrap_or_default();
                let mut bytes = Vec::with_capacity(member.size() as usize);
                member
                    .read_to_end(&mut bytes)
                    .map_err(|err| Error::Io(format!("{member_name}: {err}")))?;
                Ok(bytes)
            }
            Origin::Archive {
                archive,
                index,
                member,
            } => load_archive_member(archive, *index, member),
        }
    }

    /// Returns the bytes that correspond to a headerless DAT entry.
    pub fn load_for(&self, headerless: bool) -> Result<Vec<u8>> {
        let bytes = self.load()?;
        if !headerless {
            return Ok(bytes);
        }
        if bytes.len() >= 16 && &bytes[..4] == b"NES\x1a" {
            Ok(bytes[16..].to_vec())
        } else {
            Ok(bytes)
        }
    }
}

/// Scans `root` and returns matching candidates plus progress.
pub fn scan_path(
    root: &Path,
    options: &ScanOptions,
    progress: &mut dyn FnMut(&ScanProgress),
) -> Result<(Vec<ScannedRom>, ScanProgress)> {
    let mut roms = Vec::new();
    let mut stats = ScanProgress::default();
    let mut cache = options.cache.as_ref().map(|path| ScanCache::load(path));
    walk(
        root, options, &mut roms, &mut stats, 0, progress, &mut cache,
    )?;
    if let Some(cache) = &mut cache {
        stats.cache_hits = cache.hits();
        let _ = cache.save();
    }
    Ok((roms, stats))
}

fn walk(
    dir: &Path,
    options: &ScanOptions,
    out: &mut Vec<ScannedRom>,
    stats: &mut ScanProgress,
    depth: usize,
    progress: &mut dyn FnMut(&ScanProgress),
    cache: &mut Option<ScanCache>,
) -> Result<()> {
    if depth > options.max_depth {
        return Ok(());
    }
    let entries =
        fs::read_dir(dir).map_err(|err| Error::Io(format!("{}: {err}", dir.display())))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let hidden = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.'));
        if hidden {
            continue;
        }
        if file_type.is_dir() {
            walk(&path, options, out, stats, depth + 1, progress, cache)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        stats.files += 1;
        stats.current = path.display().to_string();
        progress(stats);
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if options.include_archives && extension == "zip" {
            scan_zip(&path, options, out, stats, progress, cache)?;
            continue;
        }
        if options.include_archives && extension == "7z" {
            scan_7z(&path, options, out, stats, progress, cache)?;
            continue;
        }
        if options.include_archives && extension == "rar" {
            scan_rar(&path, options, out, stats, progress, cache)?;
            continue;
        }
        if extension == "7z" || extension == "rar" {
            continue;
        }
        if !options.accepts(&path) {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let metadata = fs::metadata(&path).ok();
        let size = metadata.as_ref().map_or(0, fs::Metadata::len);
        let modified = modified_secs(&path);
        let key = path.to_string_lossy().into_owned();
        let cached = cache
            .as_mut()
            .and_then(|cache| cache.lookup(&key, size, modified));
        let (hashes, headerless) = if let Some(cached) = cached {
            cached
        } else {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) => {
                    stats.current = format!("{}: {err}", path.display());
                    progress(stats);
                    continue;
                }
            };
            let hashes = hash_bytes(&bytes);
            let headerless = headerless_hashes(&bytes, &extension);
            stats.bytes += bytes.len() as u64;
            if let Some(cache) = cache.as_mut() {
                cache.store(key, hashes.size, modified, hashes, headerless);
            }
            (hashes, headerless)
        };
        out.push(ScannedRom {
            origin: Origin::File(path.clone()),
            file_name,
            hashes,
            headerless,
            modified,
        });
    }
    Ok(())
}

fn scan_zip(
    archive: &Path,
    options: &ScanOptions,
    out: &mut Vec<ScannedRom>,
    stats: &mut ScanProgress,
    progress: &mut dyn FnMut(&ScanProgress),
    cache: &mut Option<ScanCache>,
) -> Result<()> {
    let file = match File::open(archive) {
        Ok(file) => file,
        Err(err) => {
            stats.current = format!("{}: {err}", archive.display());
            progress(stats);
            return Ok(());
        }
    };
    let mut zip = match ZipArchive::new(file) {
        Ok(zip) => zip,
        Err(err) => {
            stats.current = format!("{}: {err}", archive.display());
            progress(stats);
            return Ok(());
        }
    };
    let modified = modified_secs(archive);
    for index in 0..zip.len() {
        let Ok(mut member) = zip.by_index(index) else {
            continue;
        };
        if member.is_dir() {
            continue;
        }
        let Ok(name) = member.name() else {
            continue;
        };
        let name = name.into_owned();
        let member_path = Path::new(&name);
        if !options.accepts(member_path) {
            continue;
        }
        let file_name = member_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        let extension = member_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let size = member.size();
        let key = format!("{}::{index}::{name}", archive.display());
        let cached = cache
            .as_mut()
            .and_then(|cache| cache.lookup(&key, size, modified));
        let (hashes, headerless) = if let Some(cached) = cached {
            stats.members += 1;
            cached
        } else {
            let mut bytes = Vec::with_capacity(size as usize);
            if member.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let hashes = hash_bytes(&bytes);
            let headerless = headerless_hashes(&bytes, &extension);
            stats.members += 1;
            stats.bytes += bytes.len() as u64;
            if let Some(cache) = cache.as_mut() {
                cache.store(key, hashes.size, modified, hashes, headerless);
            }
            (hashes, headerless)
        };
        stats.current = format!("{}/{}", archive.display(), name);
        progress(stats);
        out.push(ScannedRom {
            origin: Origin::Zip {
                archive: archive.to_path_buf(),
                index,
                member: name,
            },
            file_name,
            hashes,
            headerless,
            modified,
        });
    }
    Ok(())
}

fn load_archive_member(archive: &Path, index: Option<usize>, member: &str) -> Result<Vec<u8>> {
    let extension = archive
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "7z" => {
            let mut reader =
                sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
                    .map_err(|err| Error::Zip(format!("{}: {err}", archive.display())))?;
            reader
                .read_file(member)
                .map_err(|err| Error::Zip(format!("{}: {err}", archive.display())))
        }
        "rar" => {
            let reader = rars::ArchiveReader::read_path(archive)
                .map_err(|err| Error::Zip(format!("{}: {err}", archive.display())))?;
            let bytes = match index {
                Some(index) => reader.read_member_at(index, None),
                None => reader.read_member(member.as_bytes(), None),
            }
            .map_err(|err| Error::Zip(format!("{}: {err}", archive.display())))?;
            bytes.ok_or_else(|| {
                Error::Zip(format!("{}: member {member} missing", archive.display()))
            })
        }
        _ => Err(Error::Zip(format!(
            "{}: unsupported archive format",
            archive.display()
        ))),
    }
}

fn scan_7z(
    archive: &Path,
    options: &ScanOptions,
    out: &mut Vec<ScannedRom>,
    stats: &mut ScanProgress,
    progress: &mut dyn FnMut(&ScanProgress),
    cache: &mut Option<ScanCache>,
) -> Result<()> {
    let Ok(reader) = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
    else {
        stats.current = format!("{}: cannot open 7z", archive.display());
        progress(stats);
        return Ok(());
    };
    let files: Vec<(String, u64)> = reader
        .archive()
        .files
        .iter()
        .filter(|file| !file.is_directory())
        .map(|file| (file.name().to_string(), file.size()))
        .collect();
    drop(reader);
    let modified = modified_secs(archive);
    for (name, size) in files {
        let member_path = Path::new(&name);
        if !options.accepts(member_path) {
            continue;
        }
        let Some(file_name) = member_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let key = format!("{}::7z::{name}", archive.display());
        let cached = cache
            .as_mut()
            .and_then(|cache| cache.lookup(&key, size, modified));
        let (hashes, headerless) = if let Some(cached) = cached {
            stats.members += 1;
            cached
        } else {
            let Ok(bytes) = load_archive_member(archive, None, &name) else {
                continue;
            };
            let hashes = hash_bytes(&bytes);
            let extension = member_path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let headerless = headerless_hashes(&bytes, &extension);
            stats.members += 1;
            stats.bytes += bytes.len() as u64;
            if let Some(cache) = cache.as_mut() {
                cache.store(key, hashes.size, modified, hashes, headerless);
            }
            (hashes, headerless)
        };
        stats.current = format!("{}/{}", archive.display(), name);
        progress(stats);
        out.push(ScannedRom {
            origin: Origin::Archive {
                archive: archive.to_path_buf(),
                index: None,
                member: name,
            },
            file_name,
            hashes,
            headerless,
            modified,
        });
    }
    Ok(())
}

fn scan_rar(
    archive: &Path,
    options: &ScanOptions,
    out: &mut Vec<ScannedRom>,
    stats: &mut ScanProgress,
    progress: &mut dyn FnMut(&ScanProgress),
    cache: &mut Option<ScanCache>,
) -> Result<()> {
    let Ok(reader) = rars::ArchiveReader::read_path(archive) else {
        stats.current = format!("{}: cannot open RAR", archive.display());
        progress(stats);
        return Ok(());
    };
    let members: Vec<(usize, String, u64, bool)> = reader
        .members()
        .enumerate()
        .map(|(index, member)| {
            (
                index,
                String::from_utf8_lossy(&member.meta.name).into_owned(),
                member.meta.unpacked_size,
                member.meta.is_directory,
            )
        })
        .collect();
    drop(reader);
    let modified = modified_secs(archive);
    for (index, name, size, is_directory) in members {
        if is_directory {
            continue;
        }
        let member_path = Path::new(&name);
        if !options.accepts(member_path) {
            continue;
        }
        let Some(file_name) = member_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let key = format!("{}::rar::{index}::{name}", archive.display());
        let cached = cache
            .as_mut()
            .and_then(|cache| cache.lookup(&key, size, modified));
        let (hashes, headerless) = if let Some(cached) = cached {
            stats.members += 1;
            cached
        } else {
            let Ok(bytes) = load_archive_member(archive, Some(index), &name) else {
                continue;
            };
            let hashes = hash_bytes(&bytes);
            let extension = member_path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let headerless = headerless_hashes(&bytes, &extension);
            stats.members += 1;
            stats.bytes += bytes.len() as u64;
            if let Some(cache) = cache.as_mut() {
                cache.store(key, hashes.size, modified, hashes, headerless);
            }
            (hashes, headerless)
        };
        stats.current = format!("{}/{}", archive.display(), name);
        progress(stats);
        out.push(ScannedRom {
            origin: Origin::Archive {
                archive: archive.to_path_buf(),
                index: Some(index),
                member: name,
            },
            file_name,
            hashes,
            headerless,
            modified,
        });
    }
    Ok(())
}

fn headerless_hashes(bytes: &[u8], extension: &str) -> Option<Hashes> {
    if extension != "nes" {
        return None;
    }
    if bytes.len() < 16 || &bytes[..4] != b"NES\x1a" {
        return None;
    }
    Some(hash_bytes(&bytes[16..]))
}

fn modified_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn scans_loose_and_zip_members_with_headerless_hash() {
        let dir = std::env::temp_dir().join(format!("unko-rm-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");

        let mut nes = vec![0u8; 16];
        nes[..4].copy_from_slice(b"NES\x1a");
        nes.extend_from_slice(b"payload");
        fs::write(dir.join("game.nes"), &nes).expect("write");

        let zip_path = dir.join("set.zip");
        let file = File::create(&zip_path).expect("zip");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("inside.nes", SimpleFileOptions::default())
            .expect("member");
        zip.write_all(&nes).expect("write member");
        zip.finish().expect("finish");
        fs::write(dir.join("readme.txt"), b"ignored").expect("txt");

        let mut progress = |_: &ScanProgress| {};
        let (roms, stats) = scan_path(&dir, &ScanOptions::default(), &mut progress).expect("scan");
        assert_eq!(roms.len(), 2);
        assert_eq!(stats.members, 1);
        let loose = roms
            .iter()
            .find(|rom| rom.file_name == "game.nes")
            .expect("loose");
        assert_eq!(loose.hashes.size, 23);
        assert_eq!(loose.headerless.expect("headerless").size, 7);
        let packed = roms
            .iter()
            .find(|rom| rom.file_name == "inside.nes")
            .expect("zip");
        assert_eq!(packed.headerless.expect("headerless").size, 7);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_7z_members_and_reads_them_back() {
        let dir = std::env::temp_dir().join(format!("unko-rm-7z-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let src = dir.join("src");
        fs::create_dir_all(&src).expect("src");
        let mut nes = vec![0u8; 16];
        nes[..4].copy_from_slice(b"NES\x1a");
        nes.extend_from_slice(b"payload");
        fs::write(src.join("game.nes"), &nes).expect("rom");
        let archives = dir.join("archives");
        fs::create_dir_all(&archives).expect("archives");
        let archive = archives.join("set.7z");
        sevenz_rust2::compress_to_path(&src, &archive).expect("compress");

        let mut progress = |_: &ScanProgress| {};
        let (roms, _) = scan_path(&archives, &ScanOptions::default(), &mut progress).expect("scan");
        let rom = roms
            .iter()
            .find(|rom| rom.file_name == "game.nes")
            .expect("7z member");
        assert!(matches!(rom.origin, Origin::Archive { .. }));
        assert_eq!(rom.load().expect("load"), nes);
        assert_eq!(rom.hashes, hash_bytes(&nes));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_reuses_unchanged_files_and_misses_after_a_change() {
        let dir = std::env::temp_dir().join(format!("unko-rm-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let rom = dir.join("game.nes");
        fs::write(&rom, b"NES\x1apayload").expect("write");
        let cache = dir.join("cache.tsv");
        let options = ScanOptions {
            cache: Some(cache.clone()),
            ..ScanOptions::default()
        };
        let mut progress = |_: &ScanProgress| {};

        let (first, stats) = scan_path(&dir, &options, &mut progress).expect("first");
        assert_eq!(stats.cache_hits, 0);
        let (second, stats) = scan_path(&dir, &options, &mut progress).expect("second");
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(first[0].hashes, second[0].hashes);
        assert_eq!(first[0].headerless, second[0].headerless);

        // A size change invalidates the entry even within the same second.
        fs::write(&rom, b"NES\x1apayload plus more").expect("rewrite");
        let (third, stats) = scan_path(&dir, &options, &mut progress).expect("third");
        assert_eq!(stats.cache_hits, 0);
        assert_ne!(first[0].hashes, third[0].hashes);
        let _ = fs::remove_dir_all(&dir);
    }
}
