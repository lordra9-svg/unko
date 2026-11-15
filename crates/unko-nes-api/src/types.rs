//! NES-specific host vocabulary.
//!
//! These types describe NES controller-port devices, the 2C02/VS display
//! palettes, the NES RF filter model, soft-reset policies, audio-source
//! selection, overscan/mask presentation, cheat codes and the NES-shaped
//! debugger snapshots. They live beside [`crate::NesCore`] so
//! [`unko_core::Core`] stays generic.

/// Device attached to a controller port.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PortDevice {
    /// Standard controller.
    #[default]
    Gamepad,
    /// NES Zapper light gun.
    Zapper,
    /// Arkanoid "Vaus" paddle (8-bit position plus fire).
    Paddle,
    /// Super NES / Subor style mouse: a 32-bit report on `$4016` D0.
    Mouse,
    /// Power Pad / Family Trainer mat (12 membrane buttons).
    PowerPad,
    /// Family Trainer variant of the Power Pad (same serial protocol).
    FamilyTrainer,
    /// Family BASIC keyboard matrix (9 rows x 8 columns).
    FamilyKeyboard,
    /// Subor/Dendy clone keyboard (13 rows x 8 columns, two nibbles).
    SuborKeyboard,
    /// Four Score adapter (four controllers across both ports).
    FourScore,
    /// Famicom controller microphone (`$4016` D2).
    Microphone,
    /// Family BASIC data recorder (cassette loopback on `$4016`).
    Tape,
    /// Miracle Piano MIDI transport (`$4016`, controller port 1).
    MiraclePiano,
}

impl PortDevice {
    /// Stable lowercase name used by configuration files and the CLI.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gamepad => "gamepad",
            Self::Zapper => "zapper",
            Self::Paddle => "paddle",
            Self::Mouse => "mouse",
            Self::PowerPad => "powerpad",
            Self::FamilyTrainer => "familytrainer",
            Self::FamilyKeyboard => "keyboard",
            Self::SuborKeyboard => "subor",
            Self::FourScore => "fourscore",
            Self::Microphone => "microphone",
            Self::Tape => "tape",
            Self::MiraclePiano => "piano",
        }
    }

    /// Parses a device name; unknown names fall back to a gamepad.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim() {
            "zapper" => Self::Zapper,
            "paddle" => Self::Paddle,
            "mouse" | "snesmouse" | "snes-mouse" => Self::Mouse,
            "powerpad" => Self::PowerPad,
            "familytrainer" | "family-trainer" => Self::FamilyTrainer,
            "keyboard" => Self::FamilyKeyboard,
            "subor" | "suborkeyboard" => Self::SuborKeyboard,
            "fourscore" => Self::FourScore,
            "microphone" | "mic" => Self::Microphone,
            "tape" | "recorder" => Self::Tape,
            "piano" | "miraclepiano" | "miracle-piano" => Self::MiraclePiano,
            _ => Self::Gamepad,
        }
    }
}

/// Display palette applied to the core's RGB output.
///
/// The canonical 2C02 table is the reference default (and what differential
/// traces compare against). The other presets are display preferences derived
/// from it, not hardware captures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Palette {
    /// Canonical 2C02 RGB table used for oracle parity.
    #[default]
    Canonical,
    /// Warmer, slightly softer consumer-TV look.
    Consumer,
    /// Saturated, gamma-corrected RGB-monitor look.
    Rgb,
    /// Luminance-only output.
    Monochrome,
    /// User-supplied 64-entry table (`.pal` / `.palette` import).
    Custom,
    /// Generated high-saturation preset.
    Vivid,
    /// Generated low-saturation, warm preset.
    Soft,
    /// Generated PAL-like hue/level preset.
    Pal,
    /// Generated neutral studio-monitor preset.
    Studio,
}

impl Palette {
    /// Stable lowercase name used by settings and the CLI.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Consumer => "consumer",
            Self::Rgb => "rgb",
            Self::Monochrome => "mono",
            Self::Custom => "custom",
            Self::Vivid => "vivid",
            Self::Soft => "soft",
            Self::Pal => "pal",
            Self::Studio => "studio",
        }
    }

    /// Parses a palette name; unknown names fall back to the canonical table.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim() {
            "consumer" | "warm" => Self::Consumer,
            "rgb" => Self::Rgb,
            "mono" | "monochrome" | "grayscale" => Self::Monochrome,
            "custom" => Self::Custom,
            "vivid" => Self::Vivid,
            "soft" => Self::Soft,
            "pal" => Self::Pal,
            "studio" => Self::Studio,
            _ => Self::Canonical,
        }
    }
}

/// Preferred video output style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VideoFilter {
    /// One output pixel per emulated pixel; best for shaders and screenshots.
    #[default]
    Raw,
    /// Core-side composite/NTSC simulation, when the core offers one.
    Composite,
    /// Composite simulation plus stronger scanlines and a slight horizontal
    /// bloom, for a CRT-like presentation.
    Crt,
}

/// Analog output filter model applied after resampling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AudioFilterModel {
    /// The front-loader network: 90 Hz + 440 Hz high-pass, 14 kHz low-pass.
    #[default]
    NesRf,
    /// No filtering (flat output; useful for comparing captures).
    Linear,
    /// Only the 14 kHz low-pass.
    Crisp,
    /// The NES high-passes with an 8 kHz low-pass.
    Warm,
    /// One high-pass and one low-pass at the supplied cutoffs; zero (or a
    /// cutoff at/above Nyquist) disables that stage.
    Custom {
        /// High-pass cutoff in Hz; `0` disables it.
        high_pass_hz: f32,
        /// Low-pass cutoff in Hz; `0` or a value at/above Nyquist disables it.
        low_pass_hz: f32,
    },
}

impl AudioFilterModel {
    /// Stable identifier used by the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::NesRf => "nes-rf",
            Self::Linear => "linear",
            Self::Crisp => "crisp",
            Self::Warm => "warm",
            Self::Custom { .. } => "custom",
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NesRf => "NES RF",
            Self::Linear => "Linear",
            Self::Crisp => "Crisp (14 kHz low-pass)",
            Self::Warm => "Warm (8 kHz low-pass)",
            Self::Custom { .. } => "Custom",
        }
    }
}

/// Power-on / soft-reset RAM fill policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RamClear {
    /// Keep the current contents (a soft reset does not clear work RAM).
    Keep,
    /// Fill with `$00` (the previous emulator default).
    #[default]
    Zero,
    /// Fill with `$FF`.
    Fill,
    /// Fill with a deterministic pseudo-random pattern.
    Random,
}

impl RamClear {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Zero => "zero",
            Self::Fill => "fill",
            Self::Random => "random",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "keep" | "none" => Self::Keep,
            "fill" | "ff" => Self::Fill,
            "random" => Self::Random,
            _ => Self::Zero,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Keep => "Keep contents",
            Self::Zero => "Fill $00",
            Self::Fill => "Fill $FF",
            Self::Random => "Random",
        }
    }
}

/// PPU register behaviour on a soft reset (the MiSTer "PPU Reset Behavior"
/// option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PpuResetBehavior {
    /// Famicom-style: a soft reset leaves the PPU registers alone.
    #[default]
    Famicom,
    /// NES-style: clear the `$2000`/`$2001` derived registers.
    Nes,
}

impl PpuResetBehavior {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Famicom => "famicom",
            Self::Nes => "nes",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "nes" => Self::Nes,
            _ => Self::Famicom,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Famicom => "Famicom (keep registers)",
            Self::Nes => "NES (clear registers)",
        }
    }
}

/// Which audio sources reach the mixer (the MiSTer "Audio Enable" option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioSource {
    /// The 2A03 and cartridge expansion audio.
    #[default]
    Both,
    /// Only the 2A03 APU.
    Internal,
    /// Only cartridge expansion audio.
    Expansion,
    /// Silence (the machine still runs).
    None,
}

impl AudioSource {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Internal => "internal",
            Self::Expansion => "expansion",
            Self::None => "none",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "internal" | "apu" => Self::Internal,
            "expansion" | "cart" => Self::Expansion,
            "none" | "off" => Self::None,
            _ => Self::Both,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Both => "Both",
            Self::Internal => "Internal (2A03)",
            Self::Expansion => "Cartridge expansion",
            Self::None => "None",
        }
    }
}

/// Analog overscan crop (the MiSTer "Overscan" option).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overscan {
    /// All 256x240 pixels.
    #[default]
    Normal,
    /// Hide 8 scanlines top and bottom.
    Vertical,
    /// Hide 8 pixels left and right.
    Borders,
    /// Hide 8 pixels on every side.
    Everything,
}

impl Overscan {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Vertical => "vertical",
            Self::Borders => "borders",
            Self::Everything => "everything",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "vertical" => Self::Vertical,
            "borders" => Self::Borders,
            "everything" | "all" => Self::Everything,
            _ => Self::Normal,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Vertical => "Vertical overscan",
            Self::Borders => "Borders",
            Self::Everything => "Everything",
        }
    }

    /// Visible source rectangle in pixels: `(x, y, width, height)`.
    #[must_use]
    pub const fn visible(self) -> (u16, u16, u16, u16) {
        match self {
            Self::Normal => (0, 0, 256, 240),
            Self::Vertical => (0, 8, 256, 224),
            Self::Borders => (8, 0, 240, 240),
            Self::Everything => (8, 8, 240, 224),
        }
    }
}

/// Display mask over the left/right software-mask columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskEdges {
    /// Show every column.
    #[default]
    Off,
    /// Mask the leftmost 8 pixels.
    Left,
    /// Mask both 8-pixel edges.
    Both,
    /// Mask the left edge unless the game already clips it.
    Auto,
}

impl MaskEdges {
    /// Stable name for the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Left => "left",
            Self::Both => "both",
            Self::Auto => "auto",
        }
    }

    /// Parses a persisted name.
    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "left" => Self::Left,
            "both" => Self::Both,
            "auto" => Self::Auto,
            _ => Self::Off,
        }
    }

    /// Human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Left => "Left",
            Self::Both => "Both",
            Self::Auto => "Auto",
        }
    }
}

/// One decoded cheat code. Host preferences, never part of a save state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cheat {
    /// A Game Genie substitution on a PRG address (`$8000-$FFFF`).
    GameGenie {
        /// Original user-entered code, kept for editing/display.
        code: String,
        /// CPU address to intercept.
        address: u16,
        /// Value to return instead of the ROM byte.
        value: u8,
        /// Optional compare byte (8-character codes only).
        compare: Option<u8>,
    },
    /// A raw RAM write applied before each frame.
    Ram {
        /// CPU address (`$0000-$07FF` for work RAM).
        address: u16,
        /// Value to write.
        value: u8,
    },
}
