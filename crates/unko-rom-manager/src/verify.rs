//! Read-only verification report for an existing library or rebuilt folder.
//!
//! [`VerifyReport::build`] turns a [`MatchReport`] into a per-game, per-ROM
//! report with the matched source, hash kind, headerless status and size, plus
//! the unknown and duplicate files. Nothing is written to the library; the CLI
//! `verify` mode only prints the report and sets a failing exit status when the
//! set is incomplete.

use crate::catalog::{MatchReport, MatchStatus, Summary};
use crate::dat::DatRom;
use crate::hash::Hashes;

/// Listing limits for the text report.
#[derive(Debug, Clone, Copy)]
pub struct VerifyOptions {
    /// Maximum entries per section; `0` means no limit.
    pub max_list: usize,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self { max_list: 50 }
    }
}

/// One expected ROM's verification state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomVerify {
    /// Expected file name from the DAT.
    pub name: String,
    /// Whether a matching file was found.
    pub found: bool,
    /// Source path or `archive/member` when found.
    pub source: Option<String>,
    /// Hash that produced the match (`sha1`, `md5` or `crc`).
    pub hash: Option<&'static str>,
    /// The match came from the bytes without the 16-byte iNES header.
    pub headerless: bool,
    /// Verified byte length.
    pub size: u64,
    /// Expected byte length from the DAT.
    pub expected_size: u64,
    /// The verified length equals the DAT length.
    pub size_ok: bool,
}

/// One game's verification state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameVerify {
    /// Game name from the DAT.
    pub name: String,
    /// Aggregate status.
    pub status: MatchStatus,
    /// Matched ROM count.
    pub matched: usize,
    /// Expected ROM count.
    pub total: usize,
    /// Per-ROM detail in DAT order.
    pub roms: Vec<RomVerify>,
}

/// Complete verification report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// DAT name.
    pub dat_name: String,
    /// DAT version.
    pub dat_version: String,
    /// Aggregate counts.
    pub summary: Summary,
    /// Per-game detail.
    pub games: Vec<GameVerify>,
    /// Files that matched no DAT entry.
    pub unknown: Vec<String>,
    /// Files that duplicated an earlier match.
    pub duplicates: Vec<String>,
}

impl VerifyReport {
    /// Builds the report from a completed audit.
    #[must_use]
    pub fn build(report: &MatchReport) -> Self {
        let summary = report.summary();
        let mut games = Vec::with_capacity(report.games.len());
        for (game_index, game) in report.dat.games.iter().enumerate() {
            let game_match = report.games[game_index];
            let mut roms = Vec::with_capacity(game.roms.len());
            for (rom_index, dat_rom) in game.roms.iter().enumerate() {
                let flat = report.flat_index(game_index, rom_index);
                let matched = flat.and_then(|flat| report.rom_match[flat].as_ref());
                match matched {
                    Some(matched) => {
                        let scanned = &report.scanned[matched.scanned];
                        let (hashes, size) = if matched.headerless {
                            let headerless = scanned.headerless.unwrap_or(scanned.hashes);
                            (headerless, headerless.size)
                        } else {
                            (scanned.hashes, scanned.hashes.size)
                        };
                        roms.push(RomVerify {
                            name: dat_rom.name.clone(),
                            found: true,
                            source: Some(scanned.origin.display()),
                            hash: Some(hash_kind(dat_rom, &hashes)),
                            headerless: matched.headerless,
                            size,
                            expected_size: dat_rom.size,
                            size_ok: size == dat_rom.size,
                        });
                    }
                    None => roms.push(RomVerify {
                        name: dat_rom.name.clone(),
                        found: false,
                        source: None,
                        hash: None,
                        headerless: false,
                        size: 0,
                        expected_size: dat_rom.size,
                        size_ok: false,
                    }),
                }
            }
            games.push(GameVerify {
                name: game.name.clone(),
                status: game_match.status,
                matched: game_match.matched,
                total: game_match.total,
                roms,
            });
        }
        let unknown = report
            .unknown
            .iter()
            .filter_map(|index| report.scanned.get(*index))
            .map(|rom| rom.origin.display())
            .collect();
        let duplicates = report
            .duplicates
            .iter()
            .filter_map(|index| report.scanned.get(*index))
            .map(|rom| rom.origin.display())
            .collect();
        Self {
            dat_name: report.dat.name.clone(),
            dat_version: report.dat.version.clone(),
            summary,
            games,
            unknown,
            duplicates,
        }
    }

    /// Whether every game is complete and there are no unknown or duplicate
    /// files.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.summary.games == self.summary.complete
            && self.summary.matched_roms == self.summary.roms
            && self.unknown.is_empty()
            && self.duplicates.is_empty()
    }

    /// Missing ROMs as `(game, rom)` pairs.
    #[must_use]
    pub fn missing_roms(&self) -> Vec<(&str, &str)> {
        let mut missing = Vec::new();
        for game in &self.games {
            for rom in &game.roms {
                if !rom.found {
                    missing.push((game.name.as_str(), rom.name.as_str()));
                }
            }
        }
        missing
    }

    /// Renders the human-readable report.
    #[must_use]
    pub fn text(&self, options: &VerifyOptions) -> String {
        let limit = if options.max_list == 0 {
            usize::MAX
        } else {
            options.max_list
        };
        let mut out = format!(
            "Unko ROM audit: {} ({})\n{} games: {} complete, {} partial, {} missing; {}/{} ROMs; {} unknown, {} duplicate\nclean: {}\n",
            self.dat_name,
            self.dat_version,
            self.summary.games,
            self.summary.complete,
            self.summary.partial,
            self.summary.missing,
            self.summary.matched_roms,
            self.summary.roms,
            self.unknown.len(),
            self.duplicates.len(),
            if self.is_clean() { "yes" } else { "no" },
        );
        let missing = self.missing_roms();
        if !missing.is_empty() {
            out.push_str(&format!("\nmissing ROMs ({}):\n", missing.len()));
            for (game, rom) in missing.iter().take(limit) {
                out.push_str(&format!("  {game}: {rom}\n"));
            }
            if missing.len() > limit {
                out.push_str(&format!("  ... {} more\n", missing.len() - limit));
            }
        }
        if !self.unknown.is_empty() {
            out.push_str(&format!("\nunknown files ({}):\n", self.unknown.len()));
            for path in self.unknown.iter().take(limit) {
                out.push_str(&format!("  {path}\n"));
            }
            if self.unknown.len() > limit {
                out.push_str(&format!("  ... {} more\n", self.unknown.len() - limit));
            }
        }
        if !self.duplicates.is_empty() {
            out.push_str(&format!("\nduplicate files ({}):\n", self.duplicates.len()));
            for path in self.duplicates.iter().take(limit) {
                out.push_str(&format!("  {path}\n"));
            }
            if self.duplicates.len() > limit {
                out.push_str(&format!("  ... {} more\n", self.duplicates.len() - limit));
            }
        }
        out
    }
}

fn hash_kind(dat_rom: &DatRom, hashes: &Hashes) -> &'static str {
    if dat_rom.sha1.is_some_and(|sha1| sha1 == hashes.sha1) {
        "sha1"
    } else if dat_rom.md5.is_some_and(|md5| md5 == hashes.md5) {
        "md5"
    } else {
        "crc"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::dat::parse_dat_str;
    use crate::hash::hash_bytes;
    use crate::scan::Origin;
    use crate::scan::ScannedRom;

    const DAT: &str = r#"<datafile><header><name>Test</name><version>1</version></header>
      <game name="Alpha (USA)"><rom name="a.nes" size="0"/></game>
      <game name="Bravo (USA)"><rom name="b.nes" size="0"/></game>
      <game name="Charlie (USA)"><rom name="c.nes" size="0"/></game>
    </datafile>"#;

    fn scanned(name: &str, bytes: &[u8]) -> ScannedRom {
        ScannedRom {
            origin: Origin::File(name.into()),
            file_name: name.into(),
            hashes: hash_bytes(bytes),
            headerless: None,
            modified: None,
        }
    }

    fn dat_with_payloads() -> crate::dat::Dat {
        let mut dat = parse_dat_str(DAT).expect("dat");
        for (index, payload) in [
            b"alpha".as_slice(),
            b"bravo".as_slice(),
            b"charlie".as_slice(),
        ]
        .iter()
        .enumerate()
        {
            let hashes = hash_bytes(payload);
            let rom = &mut dat.games[index].roms[0];
            rom.size = payload.len() as u64;
            rom.sha1 = Some(hashes.sha1);
            rom.crc = Some(hashes.crc);
        }
        dat
    }

    #[test]
    fn reports_complete_partial_missing_and_extra_files() {
        let catalog = Catalog::new(dat_with_payloads());
        let report = catalog.match_all(vec![
            scanned("a.nes", b"alpha"),
            scanned("b.nes", b"bravo"),
            scanned("a-copy.nes", b"alpha"),
            scanned("unknown.nes", b"zulu"),
        ]);
        let verify = VerifyReport::build(&report);
        assert_eq!(verify.summary.complete, 2);
        assert_eq!(verify.summary.missing, 1);
        assert_eq!(verify.summary.matched_roms, 2);
        assert_eq!(verify.summary.roms, 3);
        assert!(!verify.is_clean());
        assert_eq!(verify.games[0].status, MatchStatus::Complete);
        assert!(verify.games[0].roms[0].found);
        assert_eq!(verify.games[0].roms[0].hash, Some("sha1"));
        assert!(verify.games[0].roms[0].size_ok);
        assert!(!verify.games[2].roms[0].found);
        assert_eq!(verify.missing_roms(), vec![("Charlie (USA)", "c.nes")]);
        assert_eq!(verify.unknown, vec!["unknown.nes"]);
        assert_eq!(verify.duplicates, vec!["a-copy.nes"]);
        let text = verify.text(&VerifyOptions::default());
        assert!(text.contains("clean: no"));
        assert!(text.contains("missing ROMs (1):"));
        assert!(text.contains("Charlie (USA): c.nes"));
        assert!(text.contains("unknown.nes"));
        assert!(text.contains("a-copy.nes"));
    }

    #[test]
    fn a_complete_set_is_clean_and_headerless_matches_keep_size() {
        let catalog = Catalog::new(dat_with_payloads());
        let report = catalog.match_all(vec![
            scanned("a.nes", b"alpha"),
            scanned("b.nes", b"bravo"),
            scanned("c.nes", b"charlie"),
        ]);
        let verify = VerifyReport::build(&report);
        assert!(verify.is_clean());
        assert_eq!(verify.summary.complete, 3);
        let text = verify.text(&VerifyOptions::default());
        assert!(text.contains("clean: yes"));
        assert!(!text.contains("missing ROMs"));

        // A full .nes file matches the chromeless DAT payload via the
        // headerless hash; the verified size is the payload size.
        let payload = b"alpha";
        let hashes = hash_bytes(payload);
        let mut dat = parse_dat_str(DAT).expect("dat");
        dat.games[0].roms[0].size = payload.len() as u64;
        dat.games[0].roms[0].sha1 = Some(hashes.sha1);
        dat.games[0].roms[0].crc = Some(hashes.crc);
        dat.games.truncate(1);
        let mut full = vec![0u8; 16];
        full[..4].copy_from_slice(b"NES\x1a");
        full.extend_from_slice(payload);
        let entry = ScannedRom {
            origin: Origin::File("a.nes".into()),
            file_name: "a.nes".into(),
            hashes: hash_bytes(&full),
            headerless: Some(hashes),
            modified: None,
        };
        let verify = VerifyReport::build(&Catalog::new(dat).match_all(vec![entry]));
        assert!(verify.is_clean());
        assert!(verify.games[0].roms[0].headerless);
        assert_eq!(verify.games[0].roms[0].size, payload.len() as u64);
        assert!(verify.games[0].roms[0].size_ok);
    }

    #[test]
    fn listing_limits_cap_each_section() {
        let dat = parse_dat_str(
            r#"<datafile><header><name>L</name></header>
               <game name="Big"><rom name="1.nes" size="1"/><rom name="2.nes" size="1"/></game>
            </datafile>"#,
        )
        .expect("dat");
        let verify = VerifyReport::build(&Catalog::new(dat).match_all(Vec::new()));
        let text = verify.text(&VerifyOptions { max_list: 1 });
        assert!(text.contains("missing ROMs (2):"));
        assert!(text.contains("Big: 1.nes"));
        assert!(!text.contains("Big: 2.nes"));
        assert!(text.contains("... 1 more"));
        let unlimited = verify.text(&VerifyOptions { max_list: 0 });
        assert!(unlimited.contains("Big: 2.nes"));
    }
}
