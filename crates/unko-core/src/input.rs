//! Controller state shared by the host and every core.

/// One controller's button mask.
///
/// The host and core agree on a 16-bit mask; the NES adapter defines the
/// `A`/`B`/`Select`/`Start`/D-pad bit assignments on top of it through
/// `unko_nes_api::NesButtons`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputState(u16);

impl InputState {
    /// No buttons held.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Builds state from a raw bit pattern.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Raw bit pattern.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Returns a copy with `button` held or released.
    #[must_use]
    pub const fn with(self, button: u16, pressed: bool) -> Self {
        if pressed {
            Self(self.0 | button)
        } else {
            Self(self.0 & !button)
        }
    }

    /// Whether a button bit is held.
    #[must_use]
    pub const fn is_pressed(self, button: u16) -> bool {
        self.0 & button != 0
    }
}
