//! Per-game property overrides (region, controller ports and mapper).
//!
//! Stored next to the game's other save data as `<stem>.game`, so the file
//! survives moving the library and stays keyed by the ROM name. A missing or
//! empty file means "use the global setting and the iNES header".

use std::fs;
use std::path::Path;

use unko_core::{PortDevice, Region};

/// Overrides that apply to one game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GameProperties {
    /// Timing region; `None` uses the global setting.
    pub region: Option<Region>,
    /// Mapper number; `None` uses the iNES/NES 2.0 header.
    pub mapper: Option<u16>,
    /// Controller device on port 1; `None` uses the global setting.
    pub port1: Option<PortDevice>,
    /// Controller device on port 2; `None` uses the global setting.
    pub port2: Option<PortDevice>,
}

impl GameProperties {
    /// Whether no override is set.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.region.is_none()
            && self.mapper.is_none()
            && self.port1.is_none()
            && self.port2.is_none()
    }

    /// Loads overrides from `path`; an unreadable or absent file is empty.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .map(|text| Self::from_text(&text))
            .unwrap_or_default()
    }

    /// Writes the overrides, or removes the file when nothing is set.
    ///
    /// # Errors
    ///
    /// Returns the filesystem error when the file cannot be written or
    /// removed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if self.is_empty() {
            return match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            };
        }
        fs::write(path, self.to_text())
    }

    /// Serializes to the on-disk key/value form.
    #[must_use]
    pub fn to_text(self) -> String {
        let region = self.region.map_or("global", region_key);
        let mapper = self
            .mapper
            .map_or_else(|| "header".to_string(), |mapper| mapper.to_string());
        let port1 = self.port1.map_or("global", PortDevice::name);
        let port2 = self.port2.map_or("global", PortDevice::name);
        format!(
            "# Unko per-game properties\nregion={region}\nmapper={mapper}\nport1={port1}\nport2={port2}\n"
        )
    }

    /// Parses the on-disk key/value form, ignoring unknown keys and comments.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        let mut properties = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "region" => properties.region = region_from_key(value),
                "mapper" => properties.mapper = parse_mapper(value),
                "port1" => properties.port1 = parse_port(value),
                "port2" => properties.port2 = parse_port(value),
                _ => {}
            }
        }
        properties
    }
}

fn parse_mapper(value: &str) -> Option<u16> {
    if value.eq_ignore_ascii_case("header") || value.eq_ignore_ascii_case("auto") {
        return None;
    }
    value.parse::<u16>().ok().filter(|mapper| *mapper <= 0x0FFF)
}

fn parse_port(value: &str) -> Option<PortDevice> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "global" | "auto" | "none" | "-") {
        return None;
    }
    [
        PortDevice::Gamepad,
        PortDevice::Zapper,
        PortDevice::Paddle,
        PortDevice::Mouse,
        PortDevice::PowerPad,
        PortDevice::FamilyTrainer,
        PortDevice::FamilyKeyboard,
        PortDevice::SuborKeyboard,
        PortDevice::FourScore,
        PortDevice::Microphone,
        PortDevice::Tape,
        PortDevice::MiraclePiano,
    ]
    .into_iter()
    .find(|device| device.name() == value)
}

fn region_key(region: Region) -> &'static str {
    match region {
        Region::Auto => "global",
        Region::Ntsc => "ntsc",
        Region::Pal => "pal",
        Region::Dendy => "dendy",
    }
}

fn region_from_key(value: &str) -> Option<Region> {
    match value.to_ascii_lowercase().as_str() {
        "ntsc" => Some(Region::Ntsc),
        "pal" => Some(Region::Pal),
        "dendy" => Some(Region::Dendy),
        _ => None,
    }
}

/// Rewrites the mapper field of an iNES or NES 2.0 header in place.
///
/// # Errors
///
/// Returns an error when the image is not headered iNES/NES 2.0, or when a
/// mapper above 255 is requested for an iNES 1.0 header (which cannot express
/// it).
pub fn apply_mapper_override(rom: &mut [u8], mapper: u16) -> Result<(), String> {
    if rom.len() < 16 || &rom[0..4] != b"NES\x1a" {
        return Err("mapper override needs an iNES or NES 2.0 image".into());
    }
    if mapper > 0x0FFF {
        return Err(format!("mapper {mapper} is outside the 12-bit range"));
    }
    let nes2 = rom[7] & 0x0C == 0x08;
    if !nes2 && mapper > 0x00FF {
        return Err(format!(
            "mapper {mapper} needs a NES 2.0 header; this image is iNES"
        ));
    }
    rom[6] = (rom[6] & 0x0F) | ((mapper & 0x0F) as u8) << 4;
    rom[7] = (rom[7] & 0x0F) | (mapper & 0xF0) as u8;
    if nes2 {
        rom[8] = (rom[8] & 0xF0) | ((mapper >> 8) & 0x0F) as u8;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_round_trip() {
        let properties = GameProperties {
            region: Some(Region::Pal),
            mapper: Some(5),
            port1: Some(PortDevice::Zapper),
            port2: Some(PortDevice::FourScore),
        };
        assert_eq!(GameProperties::from_text(&properties.to_text()), properties);
        assert!(GameProperties::from_text("region=global\nmapper=header\n").is_empty());
        assert_eq!(
            GameProperties::from_text("region=dendy\nmapper=header\n"),
            GameProperties {
                region: Some(Region::Dendy),
                ..GameProperties::default()
            }
        );
    }

    #[test]
    fn empty_properties_remove_the_file() {
        let path = std::env::temp_dir().join(format!(
            "unko-game-props-{}-{}.game",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos())
        ));
        let properties = GameProperties {
            region: Some(Region::Ntsc),
            mapper: Some(4),
            ..GameProperties::default()
        };
        properties.save(&path).expect("save");
        assert_eq!(GameProperties::load(&path), properties);
        GameProperties::default().save(&path).expect("remove");
        assert!(!path.exists());
        assert!(GameProperties::default().is_empty());
    }

    #[test]
    fn mapper_override_rewrites_ines_bytes() {
        let mut rom = vec![0u8; 16];
        rom[..4].copy_from_slice(b"NES\x1a");
        rom[6] = 0x10;
        rom[7] = 0x00;
        apply_mapper_override(&mut rom, 0x123).expect_err("12-bit needs NES 2.0");
        apply_mapper_override(&mut rom, 0x21).expect("ines");
        assert_eq!(rom[6] & 0xF0, 0x10);
        assert_eq!(rom[7] & 0xF0, 0x20);
        rom[7] = 0x28;
        apply_mapper_override(&mut rom, 0x123).expect("nes 2.0");
        assert_eq!(rom[6] & 0xF0, 0x30);
        assert_eq!(rom[7] & 0xF0, 0x20);
        assert_eq!(rom[8] & 0x0F, 0x01);
        assert_eq!(rom[7] & 0x0C, 0x08, "NES 2.0 marker preserved");
    }

    #[test]
    fn mapper_override_rejects_fds() {
        let mut fds = vec![b'F', b'D', b'S', 0x1A, 0, 0];
        assert!(apply_mapper_override(&mut fds, 5).is_err());
    }
}
