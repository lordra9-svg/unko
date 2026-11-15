//! Per-game play-time counters.
//!
//! Stored in `<save_dir>/playtime.conf`, one tab-separated line per game.
//! The game stem is the last field, so a stem containing spaces or punctuation
//! cannot shift the numeric fields. Tabs and newlines in a stem are folded to
//! spaces on write, keeping the format unambiguous.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// Accumulated play time for one game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayRecord {
    /// Total tracked seconds.
    pub seconds: u64,
    /// Number of times the game was started.
    pub sessions: u32,
}

/// All per-game play-time records, keyed by save stem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayStats {
    records: BTreeMap<String, PlayRecord>,
}

impl PlayStats {
    /// File name inside the save directory.
    pub const FILE_NAME: &'static str = "playtime.conf";

    /// Loads counters from `path`; an unreadable or absent file is empty.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .map(|text| Self::from_text(&text))
            .unwrap_or_default()
    }

    /// Parses the on-disk form, ignoring comments, malformed lines and
    /// duplicate stems (the last one wins).
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut stats = Self::default();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(3, '\t');
            let (Some(seconds), Some(sessions), Some(stem)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Some(seconds) = seconds
                .strip_prefix("seconds=")
                .and_then(|value| value.parse::<u64>().ok())
            else {
                continue;
            };
            let Some(sessions) = sessions
                .strip_prefix("sessions=")
                .and_then(|value| value.parse::<u32>().ok())
            else {
                continue;
            };
            let stem = normalise(stem);
            if stem.is_empty() {
                continue;
            }
            stats.records.insert(stem, PlayRecord { seconds, sessions });
        }
        stats
    }

    /// Serializes to the on-disk form, sorted by stem for stable output.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from("# Unko play time (seconds, sessions, game stem)\n");
        for (stem, record) in &self.records {
            out.push_str(&format!(
                "seconds={}\tsessions={}\t{stem}\n",
                record.seconds, record.sessions
            ));
        }
        out
    }

    /// Writes the counters, creating the parent directory when needed.
    ///
    /// # Errors
    ///
    /// Returns the filesystem error when the file cannot be written.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_text())
    }

    /// Record for one game.
    #[must_use]
    pub fn record(&self, stem: &str) -> Option<PlayRecord> {
        self.records.get(&normalise(stem)).copied()
    }

    /// Total seconds for one game (`0` when it was never started).
    #[must_use]
    pub fn seconds(&self, stem: &str) -> u64 {
        self.record(stem).map_or(0, |record| record.seconds)
    }

    /// Adds elapsed seconds to one game.
    pub fn add_seconds(&mut self, stem: &str, seconds: u64) {
        if seconds == 0 {
            return;
        }
        self.records.entry(normalise(stem)).or_default().seconds += seconds;
    }

    /// Counts one start of a game.
    pub fn begin_session(&mut self, stem: &str) {
        let record = self.records.entry(normalise(stem)).or_default();
        record.sessions = record.sessions.saturating_add(1);
    }

    /// Clears one game's time and session count.
    pub fn reset(&mut self, stem: &str) {
        self.records.remove(&normalise(stem));
    }

    /// Sum of every game's seconds.
    #[must_use]
    pub fn total_seconds(&self) -> u64 {
        self.records.values().map(|record| record.seconds).sum()
    }

    /// Number of tracked games.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no game has a record.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Formats seconds for the library and properties window.
#[must_use]
pub fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    if seconds < 3600 {
        return format!("{}m {:02}s", seconds / 60, seconds % 60);
    }
    format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60)
}

fn normalise(stem: &str) -> String {
    stem.replace(['\t', '\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_records_in_stable_order() {
        let mut stats = PlayStats::default();
        stats.add_seconds("Zelda", 3725);
        stats.begin_session("Zelda");
        stats.begin_session("Zelda");
        stats.add_seconds("Contra (USA)", 59);
        let text = stats.to_text();
        assert!(text.starts_with("# Unko play time"));
        // Sorted by stem (BTreeMap order), not insertion order.
        let contra = text.find("Contra (USA)").expect("contra");
        let zelda = text.find("Zelda").expect("zelda");
        assert!(contra < zelda);
        let loaded = PlayStats::from_text(&text);
        assert_eq!(loaded, stats);
        assert_eq!(loaded.seconds("Zelda"), 3725);
        assert_eq!(loaded.record("Zelda").expect("record").sessions, 2);
        assert_eq!(loaded.total_seconds(), 3725 + 59);
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn folds_tabs_and_newlines_in_a_stem() {
        let mut stats = PlayStats::default();
        stats.begin_session("odd\tname\nhere");
        let text = stats.to_text();
        assert_eq!(text.lines().count(), 2, "one header plus one record");
        let loaded = PlayStats::from_text(&text);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.seconds("odd name here"), 0);
        assert_eq!(loaded.record("odd name here").expect("record").sessions, 1);
    }

    #[test]
    fn ignores_malformed_lines_and_comments() {
        let text = "# comment\n\
                    garbage\n\
                    seconds=12\tsessions=2\n\
                    seconds=nope\tsessions=1\tBad\n\
                    seconds=5\tsessions=nope\tBad\n\
                    seconds=7\tsessions=3\tGood\n";
        let stats = PlayStats::from_text(text);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats.seconds("Good"), 7);
        assert_eq!(stats.record("Good").expect("record").sessions, 3);
    }

    #[test]
    fn adds_clamps_and_resets() {
        let mut stats = PlayStats::default();
        stats.add_seconds("game", 0);
        assert!(stats.is_empty());
        stats.add_seconds("game", 10);
        stats.add_seconds("game", 5);
        assert_eq!(stats.seconds("game"), 15);
        stats.reset("game");
        assert!(stats.is_empty());
        assert_eq!(stats.seconds("game"), 0);
    }

    #[test]
    fn saves_and_loads_through_a_file() {
        let dir = std::env::temp_dir().join(format!("unko-playtime-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(PlayStats::FILE_NAME);
        let mut stats = PlayStats::default();
        stats.add_seconds("Contra (USA)", 90);
        stats.save(&path).expect("save");
        let loaded = PlayStats::load(&path);
        assert_eq!(loaded.seconds("Contra (USA)"), 90);
        let missing = PlayStats::load(&dir.join("absent.conf"));
        assert!(missing.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn formats_short_and_long_durations() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(59), "59s");
        assert_eq!(format_duration(60), "1m 00s");
        assert_eq!(format_duration(125), "2m 05s");
        assert_eq!(format_duration(3600), "1h 00m");
        assert_eq!(format_duration(3725), "1h 02m");
        assert_eq!(format_duration(90061), "25h 01m");
    }
}
