//! One-game-one-ROM (1G1R) selection.

use std::collections::BTreeMap;

use crate::dat::{Dat, Game};

/// 1G1R preferences, applied in this order.
#[derive(Debug, Clone)]
pub struct OneGameOneRom {
    /// Preferred regions, best first (`USA`, `Europe`, `World`, ...).
    pub regions: Vec<String>,
    /// Preferred languages, best first (`En`, `Ja`, ...).
    pub languages: Vec<String>,
    /// Prefer the parent over a clone.
    pub prefer_parent: bool,
    /// Prefer verified dumps.
    pub prefer_verified: bool,
    /// Prefer retail releases over beta/demo/proto/unlicensed dumps.
    pub prefer_retail: bool,
    /// Prefer newer revisions (otherwise older revisions).
    pub prefer_newer: bool,
    /// Exclude `[BIOS]` entries.
    pub exclude_bios: bool,
}

impl Default for OneGameOneRom {
    fn default() -> Self {
        Self {
            regions: vec![
                "USA".into(),
                "Europe".into(),
                "World".into(),
                "Japan".into(),
            ],
            languages: vec!["En".into()],
            prefer_parent: true,
            prefer_verified: true,
            prefer_retail: true,
            prefer_newer: true,
            exclude_bios: true,
        }
    }
}

impl OneGameOneRom {
    /// Selects one game index per parent/clone group.
    #[must_use]
    pub fn select(&self, dat: &Dat) -> Vec<usize> {
        let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, game) in dat.games.iter().enumerate() {
            if self.exclude_bios && is_bios(game) {
                continue;
            }
            groups.entry(game.parent_key()).or_default().push(index);
        }
        let mut selected: Vec<usize> = groups
            .values()
            .filter_map(|candidates| self.choose(dat, candidates))
            .collect();
        selected.sort_unstable();
        selected
    }

    fn choose(&self, dat: &Dat, candidates: &[usize]) -> Option<usize> {
        candidates.iter().copied().min_by(|left, right| {
            let left_key = self.key(dat, *left);
            let right_key = self.key(dat, *right);
            left_key.cmp(&right_key).then_with(|| {
                dat.games[*left]
                    .name
                    .to_lowercase()
                    .cmp(&dat.games[*right].name.to_lowercase())
            })
        })
    }

    fn key(&self, dat: &Dat, index: usize) -> (usize, usize, u8, u8, u8, i64) {
        let game = &dat.games[index];
        let tags = parse_tags(&game.name);
        let region = tags
            .regions
            .iter()
            .filter_map(|region| {
                self.regions
                    .iter()
                    .position(|preferred| preferred.eq_ignore_ascii_case(region))
            })
            .min()
            .unwrap_or(self.regions.len());
        let language = tags
            .languages
            .iter()
            .filter_map(|language| {
                self.languages
                    .iter()
                    .position(|preferred| preferred.eq_ignore_ascii_case(language))
            })
            .min()
            .unwrap_or(self.languages.len());
        let verified = if self.prefer_verified && !game.verified() {
            1
        } else {
            0
        };
        let retail = if self.prefer_retail && !tags.retail {
            1
        } else {
            0
        };
        let parent = if self.prefer_parent && game.is_clone() {
            1
        } else {
            0
        };
        let revision = match (self.prefer_newer, tags.revision) {
            (true, revision) => -revision,
            (false, revision) => revision,
        };
        (region, language, verified, retail, parent, revision)
    }
}

fn is_bios(game: &Game) -> bool {
    game.name.to_ascii_lowercase().starts_with("[bios]")
}

/// Parsed name tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tags {
    /// Region names found in parentheses.
    pub regions: Vec<String>,
    /// Language codes found in parentheses.
    pub languages: Vec<String>,
    /// Highest revision number/letter, `0` when absent.
    pub revision: i64,
    /// Whether the name looks like a retail release.
    pub retail: bool,
}

/// Parses No-Intro/TOSEC-style name tags.
#[must_use]
pub fn parse_tags(name: &str) -> Tags {
    const REGIONS: &[&str] = &[
        "USA",
        "Europe",
        "Japan",
        "World",
        "Asia",
        "Australia",
        "Brazil",
        "Canada",
        "China",
        "France",
        "Germany",
        "Italy",
        "Korea",
        "Netherlands",
        "Spain",
        "Sweden",
        "Taiwan",
        "UK",
        "Unknown",
        "Argentina",
        "Belgium",
        "Denmark",
        "Finland",
        "Greece",
        "Hong Kong",
        "Mexico",
        "New Zealand",
        "Norway",
        "Portugal",
        "Russia",
        "Switzerland",
        "Poland",
    ];
    const LANGUAGES: &[&str] = &[
        "En", "Ja", "Fr", "De", "Es", "It", "Nl", "Pt", "Sv", "No", "Da", "Fi", "Zh", "Ko", "Ru",
        "Pl", "Cs", "Hu", "Tr", "Ar", "He",
    ];
    const NON_RETAIL: &[&str] = &[
        "Beta",
        "Proto",
        "Demo",
        "Sample",
        "Unl",
        "Pirate",
        "Aftermarket",
        "Program",
        "Debug",
    ];

    let mut tags = Tags {
        retail: true,
        ..Tags::default()
    };
    for group in name
        .split('(')
        .skip(1)
        .filter_map(|rest| rest.split(')').next())
    {
        for token in group.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if REGIONS
                .iter()
                .any(|region| region.eq_ignore_ascii_case(token))
            {
                tags.regions.push(token.to_string());
            } else if LANGUAGES
                .iter()
                .any(|language| language.eq_ignore_ascii_case(token))
            {
                tags.languages.push(token.to_string());
            } else if NON_RETAIL
                .iter()
                .any(|status| status.eq_ignore_ascii_case(token))
            {
                tags.retail = false;
            } else if let Some(revision) = parse_revision(token) {
                tags.revision = tags.revision.max(revision);
            }
        }
    }
    tags
}

fn parse_revision(token: &str) -> Option<i64> {
    let token = token.trim();
    if let Some(rest) = token.strip_prefix("Rev ") {
        if let Ok(number) = rest.parse::<i64>() {
            return Some(number);
        }
        if rest.len() == 1 {
            let letter = rest.as_bytes()[0];
            if letter.is_ascii_uppercase() {
                return Some(i64::from(letter - b'A' + 1));
            }
        }
    }
    if let Some(rest) = token.strip_prefix('v') {
        // `vX.Y` (or the `_`-separated `vX_Y` form) is packed as
        // `(major << 32) | minor`, so integer ordering follows the components:
        // `v1.2 < v1.10 < v2.0`. `vX` and `vX.` mean a zero minor. The major
        // component is limited to 31 bits so the packed value stays
        // non-negative; anything larger is not a real revision tag.
        let rest = rest.replace('_', ".");
        let (major, minor) = rest.split_once('.').unwrap_or((rest.as_str(), "0"));
        let major = major.parse::<u32>().ok()?;
        if major > i32::MAX as u32 {
            return None;
        }
        let minor = if minor.is_empty() {
            0
        } else {
            minor.parse::<u32>().ok()?
        };
        return Some((i64::from(major) << 32) | i64::from(minor));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::parse_dat_str;

    const DAT: &str = r#"<datafile><header><name>T</name></header>
      <game name="Game (USA)" id="1"><rom name="a.nes" size="1"/></game>
      <game name="Game (Japan)" id="2" cloneofid="1"><rom name="b.nes" size="1"/></game>
      <game name="Game (Europe) (En,Fr,De)" id="3" cloneofid="1"><rom name="c.nes" size="1"/></game>
      <game name="Game (USA) (Beta)" id="4" cloneofid="1"><rom name="d.nes" size="1"/></game>
      <game name="Game (USA) (Rev 1)" id="5" cloneofid="1"><rom name="e.nes" size="1"/></game>
    </datafile>"#;

    #[test]
    fn prefers_region_language_and_retail() {
        let dat = parse_dat_str(DAT).expect("dat");
        let selected = OneGameOneRom::default().select(&dat);
        assert_eq!(selected, vec![0]);
        assert_eq!(dat.games[0].name, "Game (USA)");
    }

    #[test]
    fn prefers_newer_revision_when_parent_bias_is_off() {
        let dat = parse_dat_str(DAT).expect("dat");
        let options = OneGameOneRom {
            prefer_parent: false,
            ..OneGameOneRom::default()
        };
        let selected = options.select(&dat);
        assert_eq!(selected, vec![4]);
        assert_eq!(dat.games[4].name, "Game (USA) (Rev 1)");
    }

    #[test]
    fn tags_parse_regions_languages_and_revisions() {
        let tags = parse_tags("Game (Europe) (En,Fr,De) (Rev 2)");
        assert_eq!(tags.regions, vec!["Europe"]);
        assert_eq!(tags.languages, vec!["En", "Fr", "De"]);
        assert_eq!(tags.revision, 2);
        assert!(tags.retail);
        assert!(!parse_tags("Game (USA) (Proto)").retail);
    }

    #[test]
    fn v_revision_tags_order_numerically_by_component() {
        let revision = |name: &str| parse_tags(name).revision;
        assert!(revision("Game (v1.0)") < revision("Game (v1.1)"));
        assert!(revision("Game (v1.1)") < revision("Game (v1.2)"));
        assert!(revision("Game (v1.2)") < revision("Game (v1.10)"));
        assert!(revision("Game (v1.10)") < revision("Game (v2.0)"));
    }

    #[test]
    fn v_revision_forms_and_rejections() {
        assert_eq!(parse_revision("v1"), Some(1 << 32));
        assert_eq!(parse_revision("v1.0"), Some(1 << 32));
        assert_eq!(parse_revision("v1_10"), Some((1 << 32) | 10));
        assert_eq!(parse_revision("v1."), Some(1 << 32));
        for malformed in [
            "v1.2.3",
            "v",
            "v.",
            "vx",
            "v1.x",
            "v-1",
            "v4294967296.0",
            "",
        ] {
            assert_eq!(parse_revision(malformed), None, "{malformed}");
        }
        assert_eq!(parse_revision("Rev 3"), Some(3));
        assert_eq!(parse_revision("Rev A"), Some(1));
        assert_eq!(parse_revision("Rev Z"), Some(26));
    }

    const V_REVISION_CLONES: &str = r#"<datafile><header><name>T</name></header>
      <game name="Game (USA) (v1.2)" id="2" cloneofid="1"><rom name="a.nes" size="1"/></game>
      <game name="Game (USA) (v1.10)" id="3" cloneofid="1"><rom name="b.nes" size="1"/></game>
    </datafile>"#;

    #[test]
    fn v_revision_prefers_numerically_newer_clone() {
        let dat = parse_dat_str(V_REVISION_CLONES).expect("dat");
        let options = OneGameOneRom {
            prefer_parent: false,
            prefer_newer: true,
            ..OneGameOneRom::default()
        };
        let selected = options.select(&dat);
        assert_eq!(selected, vec![1]);
        assert_eq!(dat.games[1].name, "Game (USA) (v1.10)");
    }

    const V_REVISION_RANGE: &str = r#"<datafile><header><name>T</name></header>
      <game name="Game (USA) (v1.0)" id="2" cloneofid="1"><rom name="a.nes" size="1"/></game>
      <game name="Game (USA) (v1.1)" id="3" cloneofid="1"><rom name="b.nes" size="1"/></game>
      <game name="Game (USA) (v1.2)" id="4" cloneofid="1"><rom name="c.nes" size="1"/></game>
      <game name="Game (USA) (v1.10)" id="5" cloneofid="1"><rom name="d.nes" size="1"/></game>
      <game name="Game (USA) (v2.0)" id="6" cloneofid="1"><rom name="e.nes" size="1"/></game>
    </datafile>"#;

    #[test]
    fn v_revision_selects_expected_extreme() {
        let dat = parse_dat_str(V_REVISION_RANGE).expect("dat");
        let newest = OneGameOneRom {
            prefer_parent: false,
            prefer_newer: true,
            ..OneGameOneRom::default()
        };
        assert_eq!(newest.select(&dat), vec![4]);
        assert_eq!(dat.games[4].name, "Game (USA) (v2.0)");

        let oldest = OneGameOneRom {
            prefer_parent: false,
            prefer_newer: false,
            ..OneGameOneRom::default()
        };
        assert_eq!(oldest.select(&dat), vec![0]);
        assert_eq!(dat.games[0].name, "Game (USA) (v1.0)");
    }
}
