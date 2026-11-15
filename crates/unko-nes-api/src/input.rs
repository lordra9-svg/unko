//! NES button assignments for the generic [`InputState`] mask.

use unko_core::InputState;

/// NES `$4016` shift-register bit for each controller button.
///
/// Import this trait to use `InputState::A`, `InputState::START`, and the
/// other NES button constants.
pub trait NesButtons {
    /// A button.
    const A: u16 = 1 << 0;
    /// B button.
    const B: u16 = 1 << 1;
    /// Select button.
    const SELECT: u16 = 1 << 2;
    /// Start button.
    const START: u16 = 1 << 3;
    /// D-pad up.
    const UP: u16 = 1 << 4;
    /// D-pad down.
    const DOWN: u16 = 1 << 5;
    /// D-pad left.
    const LEFT: u16 = 1 << 6;
    /// D-pad right.
    const RIGHT: u16 = 1 << 7;
}

impl NesButtons for InputState {}

/// A named button, for mapping UI actions to [`InputState`] bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// A button.
    A,
    /// B button.
    B,
    /// Select.
    Select,
    /// Start.
    Start,
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// D-pad left.
    Left,
    /// D-pad right.
    Right,
}

impl Button {
    /// The [`InputState`] bit this button corresponds to.
    #[must_use]
    pub const fn bit(self) -> u16 {
        match self {
            Self::A => InputState::A,
            Self::B => InputState::B,
            Self::Select => InputState::SELECT,
            Self::Start => InputState::START,
            Self::Up => InputState::UP,
            Self::Down => InputState::DOWN,
            Self::Left => InputState::LEFT,
            Self::Right => InputState::RIGHT,
        }
    }
}
