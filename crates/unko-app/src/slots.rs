//! Save-state slot naming, metadata and pruning.
//!
//! A game's states already live at `<stem>.<core>.slotN.state` with a
//! `64x60` RGBA `.thumb` sidecar. This module adds a `.meta` sidecar with a
//! user-visible name, the emulated frame and the save time, and keeps the
//! three files of a slot in sync.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Number of slots offered by the frontend (0-9).
pub const SLOT_COUNT: u8 = 10;
/// Longest accepted slot name, in characters.
const NAME_LIMIT: usize = 40;

/// Metadata stored beside one state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMeta {
    /// User-visible name; empty means the default.
    pub name: String,
    /// Emulated frame at the save point.
    pub frame: u64,
    /// Unix seconds when the state was written.
    pub saved_unix: u64,
}

impl SlotMeta {
    /// Serializes the on-disk key/value form.
    #[must_use]
    pub fn to_text(&self) -> String {
        format!(
            "# Unko save-state metadata\nname={}\nframe={}\nsaved={}\n",
            self.name, self.frame, self.saved_unix
        )
    }

    /// Parses the on-disk form, ignoring unknown keys and comments.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut meta = Self {
            name: String::new(),
            frame: 0,
            saved_unix: 0,
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "name" => meta.name = sanitize_slot_name(value).unwrap_or_default(),
                "frame" => meta.frame = value.trim().parse().unwrap_or(0),
                "saved" => meta.saved_unix = value.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
        meta
    }
}

/// Default display name for a slot.
#[must_use]
pub fn default_slot_name(slot: u8) -> String {
    format!("Slot {slot}")
}

/// Trims, collapses whitespace, drops control characters and limits the
/// length. Returns `None` when nothing meaningful remains.
#[must_use]
pub fn sanitize_slot_name(name: &str) -> Option<String> {
    let mut cleaned = String::new();
    let mut pending_space = false;
    for ch in name.chars() {
        // Tabs and newlines are whitespace and fold to one space; other
        // control characters are dropped.
        if ch.is_whitespace() {
            pending_space = !cleaned.is_empty();
            continue;
        }
        if ch.is_control() {
            continue;
        }
        if pending_space {
            cleaned.push(' ');
            pending_space = false;
        }
        cleaned.push(ch);
        if cleaned.chars().count() >= NAME_LIMIT {
            break;
        }
    }
    let trimmed = cleaned.trim_end();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// One slot's current on-disk state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotInfo {
    /// Slot index.
    pub slot: u8,
    /// Whether a state file exists.
    pub exists: bool,
    /// User-visible name (or the default for an empty slot).
    pub name: String,
    /// Emulated frame at the save point; `0` when unknown.
    pub frame: u64,
    /// Unix seconds of the save; `0` when unknown.
    pub saved_unix: u64,
    /// State file size in bytes.
    pub bytes: u64,
    /// Whether a thumbnail sidecar exists.
    pub has_thumb: bool,
}

/// Paths and sidecars for one game's slots.
#[derive(Debug, Clone)]
pub struct SlotStore {
    directory: PathBuf,
    prefix: String,
}

impl SlotStore {
    /// Creates a store for `stem`/`core_id` inside `directory`.
    #[must_use]
    pub fn new(directory: &Path, stem: &str, core_id: &str) -> Self {
        Self {
            directory: directory.to_path_buf(),
            prefix: format!("{stem}.{core_id}"),
        }
    }

    /// State file path.
    #[must_use]
    pub fn state_path(&self, slot: u8) -> PathBuf {
        self.directory
            .join(format!("{}.slot{slot}.state", self.prefix))
    }

    /// Thumbnail sidecar path.
    #[must_use]
    pub fn thumb_path(&self, slot: u8) -> PathBuf {
        self.directory
            .join(format!("{}.slot{slot}.thumb", self.prefix))
    }

    /// Metadata sidecar path.
    #[must_use]
    pub fn meta_path(&self, slot: u8) -> PathBuf {
        self.directory
            .join(format!("{}.slot{slot}.meta", self.prefix))
    }

    /// Reads a slot's metadata, falling back to the default name.
    #[must_use]
    pub fn read_meta(&self, slot: u8) -> SlotMeta {
        let mut meta = fs::read_to_string(self.meta_path(slot))
            .map(|text| SlotMeta::from_text(&text))
            .unwrap_or(SlotMeta {
                name: String::new(),
                frame: 0,
                saved_unix: 0,
            });
        if meta.name.is_empty() {
            meta.name = default_slot_name(slot);
        }
        meta
    }

    /// Writes a slot's metadata, normalising the name.
    ///
    /// # Errors
    ///
    /// Returns the filesystem error when the sidecar cannot be written.
    pub fn write_meta(&self, slot: u8, meta: &SlotMeta) -> io::Result<()> {
        let mut meta = meta.clone();
        meta.name = sanitize_slot_name(&meta.name).unwrap_or_else(|| default_slot_name(slot));
        fs::write(self.meta_path(slot), meta.to_text())
    }

    /// Current information for one slot.
    #[must_use]
    pub fn info(&self, slot: u8) -> SlotInfo {
        let state = self.state_path(slot);
        let exists = state.is_file();
        let bytes = fs::metadata(&state).map_or(0, |meta| meta.len());
        let meta = self.read_meta(slot);
        let saved_unix = if meta.saved_unix > 0 {
            meta.saved_unix
        } else {
            fs::metadata(&state)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_secs())
        };
        SlotInfo {
            slot,
            exists,
            name: if exists {
                meta.name
            } else {
                default_slot_name(slot)
            },
            frame: meta.frame,
            saved_unix,
            bytes,
            has_thumb: self.thumb_path(slot).is_file(),
        }
    }

    /// Information for every frontend slot.
    #[must_use]
    pub fn list(&self) -> Vec<SlotInfo> {
        (0..SLOT_COUNT).map(|slot| self.info(slot)).collect()
    }

    /// Removes a state and both sidecars. Missing files are ignored.
    ///
    /// # Errors
    ///
    /// Returns any filesystem error other than "not found".
    pub fn delete(&self, slot: u8) -> io::Result<()> {
        for path in [
            self.state_path(slot),
            self.thumb_path(slot),
            self.meta_path(slot),
        ] {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    /// Deletes thumbnail and metadata sidecars whose state file is missing.
    ///
    /// Returns the number of removed files. Slots with a state are untouched.
    ///
    /// # Errors
    ///
    /// Returns any filesystem error other than "not found".
    pub fn prune_orphans(&self) -> io::Result<usize> {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return Ok(0);
        };
        let prefix = format!("{}.slot", self.prefix);
        let mut pruned = 0;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let Some((slot, kind)) = rest.split_once('.') else {
                continue;
            };
            if kind != "thumb" && kind != "meta" {
                continue;
            }
            let Ok(slot) = slot.parse::<u8>() else {
                continue;
            };
            if !self.state_path(slot).is_file() {
                match fs::remove_file(entry.path()) {
                    Ok(()) => pruned += 1,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
            }
        }
        Ok(pruned)
    }

    /// Deletes every frontend slot.
    ///
    /// # Errors
    ///
    /// Returns the first filesystem error.
    pub fn clear(&self) -> io::Result<()> {
        for slot in 0..SLOT_COUNT {
            self.delete(slot)?;
        }
        Ok(())
    }
}

/// Human-readable file size for the slot list.
#[must_use]
pub fn size_label(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (KIB * KIB))
    } else if bytes >= 1024 {
        format!("{:.0} KiB", bytes as f64 / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// Unix seconds now, used for new metadata.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (PathBuf, SlotStore) {
        let dir = std::env::temp_dir().join(format!("unko-slots-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let store = SlotStore::new(&dir, "Contra (USA)-deadbeef", "nes_accuracy_core");
        (dir, store)
    }

    #[test]
    fn names_are_defaulted_trimmed_and_limited() {
        assert_eq!(default_slot_name(3), "Slot 3");
        assert_eq!(
            sanitize_slot_name("  before   boss  "),
            Some("before boss".into())
        );
        assert_eq!(sanitize_slot_name("a\tb\nc"), Some("a b c".into()));
        assert_eq!(sanitize_slot_name("  \t "), None);
        assert_eq!(
            sanitize_slot_name(&"x".repeat(60))
                .expect("name")
                .chars()
                .count(),
            40
        );
        assert_eq!(
            sanitize_slot_name("ends   ").as_deref(),
            Some("ends"),
            "trailing whitespace must not survive"
        );
    }

    #[test]
    fn metadata_round_trips_and_tolerates_junk() {
        let meta = SlotMeta {
            name: "Before boss".into(),
            frame: 12_345,
            saved_unix: 1_760_000_000,
        };
        assert_eq!(SlotMeta::from_text(&meta.to_text()), meta);
        let parsed = SlotMeta::from_text(
            "# comment\ngarbage\nname=  Two   words \nframe=nope\nsaved=42\nunknown=1\n",
        );
        assert_eq!(parsed.name, "Two words");
        assert_eq!(parsed.frame, 0);
        assert_eq!(parsed.saved_unix, 42);
    }

    #[test]
    fn list_reports_state_metadata_and_thumbnail() {
        let (dir, store) = temp_store("list");
        fs::write(store.state_path(2), vec![0u8; 1234]).expect("state");
        fs::write(store.thumb_path(2), vec![0u8; 64 * 60 * 4]).expect("thumb");
        store
            .write_meta(
                2,
                &SlotMeta {
                    name: "  Mid-game  ".into(),
                    frame: 99,
                    saved_unix: 1_700_000_000,
                },
            )
            .expect("meta");
        let slots = store.list();
        assert_eq!(slots.len(), usize::from(SLOT_COUNT));
        let slot = &slots[2];
        assert!(slot.exists);
        assert_eq!(slot.name, "Mid-game");
        assert_eq!(slot.frame, 99);
        assert_eq!(slot.saved_unix, 1_700_000_000);
        assert_eq!(slot.bytes, 1234);
        assert!(slot.has_thumb);
        assert!(!slots[0].exists);
        assert_eq!(slots[0].name, "Slot 0");
        assert_eq!(slots[0].bytes, 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn delete_removes_all_three_files() {
        let (dir, store) = temp_store("delete");
        fs::write(store.state_path(1), b"state").expect("state");
        fs::write(store.thumb_path(1), b"thumb").expect("thumb");
        fs::write(store.meta_path(1), b"meta").expect("meta");
        store.delete(1).expect("delete");
        assert!(!store.state_path(1).exists());
        assert!(!store.thumb_path(1).exists());
        assert!(!store.meta_path(1).exists());
        // A second delete is a no-op.
        store.delete(1).expect("repeat delete");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_removes_only_orphaned_sidecars() {
        let (dir, store) = temp_store("prune");
        // Complete slot 1.
        fs::write(store.state_path(1), b"state").expect("state1");
        fs::write(store.thumb_path(1), b"thumb").expect("thumb1");
        fs::write(store.meta_path(1), b"meta").expect("meta1");
        // Slot 2 has sidecars but no state.
        fs::write(store.thumb_path(2), b"thumb").expect("thumb2");
        fs::write(store.meta_path(2), b"meta").expect("meta2");
        // Slot 3 has a state but no sidecars (not an orphan).
        fs::write(store.state_path(3), b"state").expect("state3");
        assert_eq!(store.prune_orphans().expect("prune"), 2);
        assert!(store.state_path(1).exists());
        assert!(store.thumb_path(1).exists());
        assert!(store.meta_path(1).exists());
        assert!(!store.thumb_path(2).exists());
        assert!(!store.meta_path(2).exists());
        assert!(store.state_path(3).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn size_labels_use_readable_units() {
        assert_eq!(size_label(0), "0 B");
        assert_eq!(size_label(999), "999 B");
        assert_eq!(size_label(2048), "2 KiB");
        assert_eq!(size_label(417_054), "407 KiB");
        assert_eq!(size_label(2 * 1024 * 1024), "2.0 MiB");
    }

    #[test]
    fn clear_removes_every_frontend_slot() {
        let (dir, store) = temp_store("clear");
        for slot in [0u8, 5, 9] {
            fs::write(store.state_path(slot), b"state").expect("state");
            fs::write(store.thumb_path(slot), b"thumb").expect("thumb");
        }
        store.clear().expect("clear");
        for slot in 0..SLOT_COUNT {
            assert!(!store.state_path(slot).exists());
            assert!(!store.thumb_path(slot).exists());
        }
        let _ = fs::remove_dir_all(dir);
    }
}
