//! View model for the ROM Manager verification report.
//!
//! The mapping from [`VerifyReport`] to display rows is kept free of `egui` so
//! it can be unit-tested; the window only chooses colours and draws the rows.

use unko_rom_manager::{MatchStatus, VerifyReport};

/// One ROM row in the verification panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyRow {
    /// Game name from the DAT.
    pub game: String,
    /// Expected ROM file name.
    pub rom: String,
    /// Whether a matching file was found.
    pub found: bool,
    /// Matched source (`archive/member` for containers).
    pub source: String,
    /// Hash that produced the match; empty when missing.
    pub hash: &'static str,
    /// The match came from the headerless payload.
    pub headerless: bool,
    /// Verified length equals the DAT length.
    pub size_ok: bool,
}

impl VerifyRow {
    /// Short status cell.
    #[must_use]
    pub fn status_text(&self) -> &'static str {
        if self.found {
            "found"
        } else {
            "missing"
        }
    }

    /// Hash cell, including the headerless and size markers.
    #[must_use]
    pub fn hash_text(&self) -> String {
        if !self.found {
            return String::new();
        }
        let mut text = self.hash.to_string();
        if self.headerless {
            text.push_str(" (headerless)");
        }
        if !self.size_ok {
            text.push_str(" (size mismatch)");
        }
        text
    }

    /// Source cell (`—` for missing).
    #[must_use]
    pub fn source_text(&self) -> &str {
        if self.found {
            &self.source
        } else {
            "—"
        }
    }
}

/// Verification panel data, derived once per Verify action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyView {
    /// DAT name and version.
    pub dat: String,
    /// Every game is complete with no unknown/duplicate files.
    pub clean: bool,
    /// Aggregate one-line summary.
    pub summary: String,
    /// Number of games in the DAT.
    pub games: usize,
    /// Number of complete games.
    pub complete_games: usize,
    /// Number of found ROMs.
    pub found: usize,
    /// Number of expected ROMs.
    pub total: usize,
    /// Per-ROM rows in DAT order.
    pub rows: Vec<VerifyRow>,
    /// Missing `(game, rom)` pairs.
    pub missing: Vec<(String, String)>,
    /// Files that matched no DAT entry.
    pub unknown: Vec<String>,
    /// Files that duplicated an earlier match.
    pub duplicates: Vec<String>,
}

impl VerifyView {
    /// Builds the panel data from a completed verification report.
    #[must_use]
    pub fn from_report(report: &VerifyReport) -> Self {
        let mut rows = Vec::new();
        let mut missing = Vec::new();
        for game in &report.games {
            for rom in &game.roms {
                if rom.found {
                    rows.push(VerifyRow {
                        game: game.name.clone(),
                        rom: rom.name.clone(),
                        found: true,
                        source: rom.source.clone().unwrap_or_default(),
                        hash: rom.hash.unwrap_or(""),
                        headerless: rom.headerless,
                        size_ok: rom.size_ok,
                    });
                } else {
                    missing.push((game.name.clone(), rom.name.clone()));
                    rows.push(VerifyRow {
                        game: game.name.clone(),
                        rom: rom.name.clone(),
                        found: false,
                        source: String::new(),
                        hash: "",
                        headerless: false,
                        size_ok: false,
                    });
                }
            }
        }
        let found = rows.iter().filter(|row| row.found).count();
        let summary = format!(
            "{}/{} games complete · {}/{} ROMs matched · {} unknown · {} duplicate",
            report.summary.complete,
            report.summary.games,
            report.summary.matched_roms,
            report.summary.roms,
            report.unknown.len(),
            report.duplicates.len(),
        );
        Self {
            dat: format!("{} ({})", report.dat_name, report.dat_version),
            clean: report.is_clean(),
            summary,
            games: report.summary.games,
            complete_games: report.summary.complete,
            found,
            total: rows.len(),
            rows,
            missing,
            unknown: report.unknown.clone(),
            duplicates: report.duplicates.clone(),
        }
    }

    /// Whether the game is complete (used for the row colour).
    #[must_use]
    pub fn status_label(&self) -> &'static str {
        if self.clean {
            "CLEAN"
        } else {
            "NOT CLEAN"
        }
    }

    /// Colour hint for a row: `Some(true)` found, `Some(false)` missing.
    #[must_use]
    pub fn row_status(&self, row: &VerifyRow) -> MatchStatus {
        if row.found {
            MatchStatus::Complete
        } else {
            MatchStatus::Missing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unko_rom_manager::{GameVerify, RomVerify, Summary};

    fn found_rom() -> RomVerify {
        RomVerify {
            name: "Contra (USA).nes".into(),
            found: true,
            source: Some("/roms/Contra (USA).zip/Contra (USA).nes".into()),
            hash: Some("sha1"),
            headerless: true,
            size: 131_072,
            expected_size: 131_072,
            size_ok: true,
        }
    }

    fn missing_rom() -> RomVerify {
        RomVerify {
            name: "Missing (USA).nes".into(),
            found: false,
            source: None,
            hash: None,
            headerless: false,
            size: 0,
            expected_size: 1,
            size_ok: false,
        }
    }

    fn report(clean: bool) -> VerifyReport {
        let mut games = vec![GameVerify {
            name: "Contra (USA)".into(),
            status: MatchStatus::Complete,
            matched: 1,
            total: 1,
            roms: vec![found_rom()],
        }];
        if !clean {
            games.push(GameVerify {
                name: "Missing (USA)".into(),
                status: MatchStatus::Missing,
                matched: 0,
                total: 1,
                roms: vec![missing_rom()],
            });
        }
        VerifyReport {
            dat_name: "No-Intro".into(),
            dat_version: "2026".into(),
            summary: Summary {
                games: games.len(),
                complete: 1,
                partial: 0,
                missing: if clean { 0 } else { 1 },
                roms: if clean { 1 } else { 2 },
                matched_roms: 1,
                unknown: if clean { 0 } else { 1 },
                duplicates: if clean { 0 } else { 1 },
            },
            games,
            unknown: if clean {
                Vec::new()
            } else {
                vec!["/roms/unknown.nes".into()]
            },
            duplicates: if clean {
                Vec::new()
            } else {
                vec!["/roms/copy.nes".into()]
            },
        }
    }

    #[test]
    fn found_rows_keep_source_hash_and_flags() {
        let view = VerifyView::from_report(&report(true));
        assert!(view.clean);
        assert_eq!(view.status_label(), "CLEAN");
        assert_eq!(view.dat, "No-Intro (2026)");
        assert_eq!(view.games, 1);
        assert_eq!(view.complete_games, 1);
        assert_eq!(view.found, 1);
        assert_eq!(view.total, 1);
        let row = &view.rows[0];
        assert_eq!(row.game, "Contra (USA)");
        assert_eq!(row.status_text(), "found");
        assert_eq!(row.hash_text(), "sha1 (headerless)");
        assert_eq!(row.source_text(), "/roms/Contra (USA).zip/Contra (USA).nes");
        assert_eq!(view.row_status(row), MatchStatus::Complete);
        assert!(view.missing.is_empty());
    }

    #[test]
    fn missing_and_extra_files_are_mapped_and_marked_unclean() {
        let view = VerifyView::from_report(&report(false));
        assert!(!view.clean);
        assert_eq!(view.status_label(), "NOT CLEAN");
        assert_eq!(
            view.summary,
            "1/2 games complete · 1/2 ROMs matched · 1 unknown · 1 duplicate"
        );
        assert_eq!(view.games, 2);
        assert_eq!(view.complete_games, 1);
        assert_eq!(view.unknown, vec!["/roms/unknown.nes"]);
        assert_eq!(view.duplicates, vec!["/roms/copy.nes"]);
        assert_eq!(view.found, 1);
        assert_eq!(view.total, 2);
        assert_eq!(
            view.missing,
            vec![("Missing (USA)".into(), "Missing (USA).nes".into())]
        );
        // Size mismatches are surfaced in the hash cell.
        let mut rom = found_rom();
        rom.size_ok = false;
        let mut broken = report(true);
        broken.games[0].roms[0] = rom;
        let view = VerifyView::from_report(&broken);
        assert_eq!(
            view.rows[0].hash_text(),
            "sha1 (headerless) (size mismatch)"
        );
        assert_eq!(view.rows[0].status_text(), "found");
    }

    #[test]
    fn missing_rows_use_an_empty_source() {
        let view = VerifyView::from_report(&report(false));
        let row = view
            .rows
            .iter()
            .find(|row| !row.found)
            .expect("missing row");
        assert_eq!(row.game, "Missing (USA)");
        assert_eq!(row.status_text(), "missing");
        assert_eq!(row.source_text(), "—");
        assert_eq!(row.hash_text(), "");
        assert_eq!(view.row_status(row), MatchStatus::Missing);
        assert_eq!(
            view.missing,
            vec![("Missing (USA)".into(), "Missing (USA).nes".into())]
        );
        assert_eq!(view.found, 1);
        assert_eq!(view.total, 2);
    }
}
