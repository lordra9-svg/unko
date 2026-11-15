//! Per-game Vs. System cabinet settings.
//!
//! Stored beside the other per-game files as `<stem>.vs`. Only the DIP
//! switches and the primary-CPU flag persist; coin and service are momentary
//! cabinet inputs and always start released.

use std::fs;
use std::path::{Path, PathBuf};

/// Persisted Vs. System operator settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VsConfig {
    /// DIP switches 1-8, bit 0 = switch 1.
    pub dips: u8,
    /// True when the game runs on the primary CPU.
    pub primary: bool,
}

impl Default for VsConfig {
    fn default() -> Self {
        Self {
            dips: 0,
            primary: true,
        }
    }
}

impl VsConfig {
    /// Path of the per-game file for `stem`.
    #[must_use]
    pub fn path(save_dir: &Path, stem: &str) -> PathBuf {
        save_dir.join(format!("{stem}.vs"))
    }

    /// Loads a file, falling back to the defaults on any error.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path).map_or_else(|_| Self::default(), |text| Self::parse(&text))
    }

    /// Parses `key=value` lines, ignoring unknown keys and comments.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut config = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "dips" => {
                    if let Ok(dips) = value.trim().parse::<u8>() {
                        config.dips = dips;
                    }
                }
                "primary" => match value.trim() {
                    "0" => config.primary = false,
                    "1" => config.primary = true,
                    _ => {}
                },
                _ => {}
            }
        }
        config
    }

    /// Serializes the settings as stable `key=value` text.
    #[must_use]
    pub fn to_text(self) -> String {
        format!("dips={}\nprimary={}\n", self.dips, u8::from(self.primary))
    }

    /// Writes the settings, creating the parent directory.
    pub fn save(self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_keys_and_ignores_the_rest() {
        let config = VsConfig::parse("# Vs. settings\nunknown=7\ndips = 165\nprimary=0\n");
        assert_eq!(
            config,
            VsConfig {
                dips: 165,
                primary: false
            }
        );
    }

    #[test]
    fn malformed_values_keep_the_defaults() {
        assert_eq!(
            VsConfig::parse("dips=999\nprimary=x\n"),
            VsConfig::default()
        );
        assert_eq!(VsConfig::parse("not a pair"), VsConfig::default());
    }

    #[test]
    fn path_uses_the_rom_stem() {
        let path = VsConfig::path(Path::new("/tmp/saves"), "Vs. Super Mario Bros");
        assert_eq!(path, Path::new("/tmp/saves/Vs. Super Mario Bros.vs"));
    }

    #[test]
    fn text_round_trips() {
        let config = VsConfig {
            dips: 0xA5,
            primary: false,
        };
        assert_eq!(VsConfig::parse(&config.to_text()), config);
        assert_eq!(
            VsConfig::default(),
            VsConfig {
                dips: 0,
                primary: true
            }
        );
    }
}
