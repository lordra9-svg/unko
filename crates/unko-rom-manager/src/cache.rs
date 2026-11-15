//! Persistent hash cache for library scans.
//!
//! Keys are a path (loose files) or `archive::index::member` (ZIP members)
//! plus the file size and modification time. A matching entry lets the
//! scanner reuse the hashes without reading the bytes again. The file is a
//! small, human-readable TSV so it can be deleted or inspected by hand.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hash::Hashes;
use crate::{Error, Result};

const HEADER: &str = "# unko-rom-manager cache v1";

/// One cached hash set.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    size: u64,
    modified: Option<u64>,
    hashes: Hashes,
    headerless: Option<Hashes>,
}

/// Hash cache loaded from disk.
#[derive(Debug, Default)]
pub struct ScanCache {
    path: PathBuf,
    entries: HashMap<String, Entry>,
    hits: usize,
    misses: usize,
    dirty: bool,
}

impl ScanCache {
    /// Loads a cache, or starts empty when the file is absent or unreadable.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let mut cache = Self {
            path: path.to_path_buf(),
            ..Self::default()
        };
        let Ok(text) = fs::read_to_string(path) else {
            return cache;
        };
        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let Some(entry) = parse_line(line) else {
                continue;
            };
            cache.entries.insert(entry.0, entry.1);
        }
        cache
    }

    /// Number of stored entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cache file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Empties the in-memory cache; the next [`Self::save`] rewrites the file
    /// with only its header.
    pub fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.dirty = true;
        }
    }

    /// Number of entries served from the cache.
    #[must_use]
    pub const fn hits(&self) -> usize {
        self.hits
    }

    /// Number of entries that had to be hashed again.
    #[must_use]
    pub const fn misses(&self) -> usize {
        self.misses
    }

    /// Returns cached hashes when size and modification time still match.
    pub fn lookup(
        &mut self,
        key: &str,
        size: u64,
        modified: Option<u64>,
    ) -> Option<(Hashes, Option<Hashes>)> {
        let hit = self
            .entries
            .get(key)
            .filter(|entry| entry.size == size && entry.modified == modified)
            .map(|entry| (entry.hashes, entry.headerless));
        if hit.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        hit
    }

    /// Stores or replaces one entry.
    pub fn store(
        &mut self,
        key: String,
        size: u64,
        modified: Option<u64>,
        hashes: Hashes,
        headerless: Option<Hashes>,
    ) {
        let entry = Entry {
            size,
            modified,
            hashes,
            headerless,
        };
        if self.entries.get(&key) != Some(&entry) {
            self.entries.insert(key, entry);
            self.dirty = true;
        }
    }

    /// Writes the cache when anything changed.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the cache directory or file cannot be
    /// written.
    pub fn save(&self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .map_err(|err| Error::Io(format!("{}: {err}", parent.display())))?;
            }
        }
        let mut text = String::from(HEADER);
        text.push('\n');
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        for key in keys {
            let entry = &self.entries[key];
            text.push_str(&format_line(key, entry));
            text.push('\n');
        }
        fs::write(&self.path, text)
            .map_err(|err| Error::Io(format!("{}: {err}", self.path.display())))
    }
}

fn format_line(key: &str, entry: &Entry) -> String {
    let modified = entry
        .modified
        .map_or_else(|| "-".to_string(), |value| value.to_string());
    let mut fields = vec![
        escape(key),
        entry.size.to_string(),
        modified,
        entry.hashes.crc_hex(),
        entry.hashes.md5_hex(),
        entry.hashes.sha1_hex(),
    ];
    match &entry.headerless {
        Some(hashes) => {
            fields.push(hashes.size.to_string());
            fields.push(hashes.crc_hex());
            fields.push(hashes.md5_hex());
            fields.push(hashes.sha1_hex());
        }
        None => fields.extend([
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
            "-".to_string(),
        ]),
    }
    fields.join("\t")
}

/// Deletes a cache file, treating an absent file as already cleared.
///
/// # Errors
///
/// Returns the filesystem error when an existing file cannot be removed.
pub fn clear_cache_file(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn parse_line(line: &str) -> Option<(String, Entry)> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 10 {
        return None;
    }
    let size = fields[1].parse().ok()?;
    let modified = if fields[2] == "-" {
        None
    } else {
        Some(fields[2].parse().ok()?)
    };
    let hashes = Hashes {
        size,
        crc: u32::from_str_radix(fields[3], 16).ok()?,
        md5: crate::hash::parse_digest::<16>(fields[4])?,
        sha1: crate::hash::parse_digest::<20>(fields[5])?,
    };
    let headerless = if fields[6] == "-" {
        None
    } else {
        Some(Hashes {
            size: fields[6].parse().ok()?,
            crc: u32::from_str_radix(fields[7], 16).ok()?,
            md5: crate::hash::parse_digest::<16>(fields[8])?,
            sha1: crate::hash::parse_digest::<20>(fields[9])?,
        })
    };
    Some((
        unescape(fields[0]),
        Entry {
            size,
            modified,
            hashes,
            headerless,
        },
    ))
}

fn escape(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for ch in key.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\t' => out.push_str("%09"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut chars = key.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let code: String = chars.by_ref().take(2).collect();
        match u8::from_str_radix(&code, 16).ok() {
            Some(byte) => out.push(char::from(byte)),
            None => {
                out.push('%');
                out.push_str(&code);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_bytes;

    #[test]
    fn round_trips_entries_and_misses_on_change() {
        let path = std::env::temp_dir().join(format!(
            "unko-cache-{}-{}.tsv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos())
        ));
        let hashes = hash_bytes(b"payload");
        let mut cache = ScanCache::load(&path);
        assert!(cache.lookup("a\tb%c", hashes.size, Some(7)).is_none());
        cache.store("a\tb%c".into(), hashes.size, Some(7), hashes, Some(hashes));
        cache.save().expect("save");

        let mut loaded = ScanCache::load(&path);
        assert_eq!(
            loaded.lookup("a\tb%c", hashes.size, Some(7)),
            Some((hashes, Some(hashes)))
        );
        assert_eq!(loaded.hits(), 1);
        assert!(loaded.lookup("a\tb%c", hashes.size, Some(8)).is_none());
        assert_eq!(loaded.misses(), 1);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn len_clear_and_clear_file_manage_the_cache() {
        let path = std::env::temp_dir().join(format!(
            "unko-cache-manage-{}-{}.tsv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos())
        ));
        let hashes = hash_bytes(b"payload");
        let mut cache = ScanCache::load(&path);
        assert!(cache.is_empty());
        cache.store("a".into(), hashes.size, None, hashes, None);
        cache.store("b".into(), hashes.size, None, hashes, Some(hashes));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.path(), path.as_path());
        cache.save().expect("save");
        assert_eq!(ScanCache::load(&path).len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        cache.save().expect("save empty");
        assert!(path.exists(), "clear rewrites the file");
        assert!(ScanCache::load(&path).is_empty());

        clear_cache_file(&path).expect("remove");
        assert!(!path.exists());
        clear_cache_file(&path).expect("absent file is already clear");
    }
}
