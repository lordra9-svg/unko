//! SNES-specific extension surface for the generic Unko core interface.
//!
//! [`unko_core::Core`] owns only the generic lifecycle: load, run, video,
//! audio, input, sample rate and save states. The SNES vocabulary — the
//! controller auto-poll bit layout, the timing region, video-mode metadata and
//! the extension trait methods the frontend calls — lives here.
//!
//! The bit layout matches the S-CPU auto-poll result registers `$4218-$421F`
//! and the serial shift order, i.e. `unko_snes::io::button`. A host that
//! presents a SNES title must therefore build [`unko_core::InputState`] with
//! these constants, not the NES `$4016` ones.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod debug;

pub use debug::{CpuState, DebugSpace, Disassembly, DspState, PpuState};

use unko_core::Core;

/// SNES controller bit for each button (`$4218-$421F` order).
///
/// Import this trait to use `InputState::B`, `InputState::START`, and the
/// other SNES button constants.
pub trait SnesButtons {
    /// B button.
    const B: u16 = 1 << 15;
    /// Y button.
    const Y: u16 = 1 << 14;
    /// Select button.
    const SELECT: u16 = 1 << 13;
    /// Start button.
    const START: u16 = 1 << 12;
    /// D-pad up.
    const UP: u16 = 1 << 11;
    /// D-pad down.
    const DOWN: u16 = 1 << 10;
    /// D-pad left.
    const LEFT: u16 = 1 << 9;
    /// D-pad right.
    const RIGHT: u16 = 1 << 8;
    /// A button.
    const A: u16 = 1 << 7;
    /// X button.
    const X: u16 = 1 << 6;
    /// L shoulder button.
    const L: u16 = 1 << 5;
    /// R shoulder button.
    const R: u16 = 1 << 4;
}

impl SnesButtons for unko_core::InputState {}

/// A named SNES button, for mapping UI actions to input bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// A button.
    A,
    /// B button.
    B,
    /// X button.
    X,
    /// Y button.
    Y,
    /// L shoulder.
    L,
    /// R shoulder.
    R,
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
    /// The [`unko_core::InputState`] bit this button corresponds to.
    #[must_use]
    pub const fn bit(self) -> u16 {
        match self {
            Self::A => 1 << 7,
            Self::B => 1 << 15,
            Self::X => 1 << 6,
            Self::Y => 1 << 14,
            Self::L => 1 << 5,
            Self::R => 1 << 4,
            Self::Select => 1 << 13,
            Self::Start => 1 << 12,
            Self::Up => 1 << 11,
            Self::Down => 1 << 10,
            Self::Left => 1 << 9,
            Self::Right => 1 << 8,
        }
    }
}

/// Console timing region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// 60 Hz NTSC.
    Ntsc,
    /// 50 Hz PAL.
    Pal,
}

impl Region {
    /// Stable identifier used by the settings file.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ntsc => "ntsc",
            Self::Pal => "pal",
        }
    }

    /// Frames per second of the region's reference timing.
    #[must_use]
    pub const fn frame_rate(self) -> f64 {
        match self {
            Self::Ntsc => 60.098_8,
            Self::Pal => 50.007_0,
        }
    }
}

/// Display geometry of the most recently completed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMode {
    /// 256-pixel output.
    Normal,
    /// 512-pixel hi-res output.
    HiRes,
    /// 256-pixel output with the 240-line overscan geometry.
    Overscan,
}

/// SNES-specific host controls layered over [`Core`].
pub trait SnesCore: Core {
    /// The core as the generic trait object (for hosts that store both).
    fn as_core(&mut self) -> &mut dyn Core;

    /// The core as the generic trait object.
    fn as_core_ref(&self) -> &dyn Core;

    /// Timing region of the loaded cartridge.
    fn region(&self) -> Region {
        Region::Ntsc
    }

    /// Frames per second of the loaded cartridge.
    fn frame_rate(&self) -> f64 {
        self.region().frame_rate()
    }

    /// Whether this core implements the debugger hooks.
    fn debug_supported(&self) -> bool {
        false
    }

    /// Snapshot of the 65C816 registers.
    fn debug_cpu(&self) -> Option<CpuState> {
        None
    }

    /// Snapshot of the S-PPU position and display state.
    fn debug_ppu(&self) -> Option<PpuState> {
        None
    }

    /// Snapshot of the S-DSP register file.
    fn debug_dsp(&self) -> Option<DspState> {
        None
    }

    /// Reads one byte from a debug address space.
    fn debug_peek(&self, _space: DebugSpace, _address: u32) -> Option<u8> {
        None
    }

    /// Decodes one instruction for a disassembly listing.
    fn debug_disassemble(&mut self, _address: u32) -> Option<Disassembly> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_bits_are_unique() {
        let bits: Vec<u16> = [
            Button::A,
            Button::B,
            Button::X,
            Button::Y,
            Button::L,
            Button::R,
            Button::Select,
            Button::Start,
            Button::Up,
            Button::Down,
            Button::Left,
            Button::Right,
        ]
        .iter()
        .map(|button| button.bit())
        .collect();
        let mut sorted = bits.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), bits.len());
        assert_eq!(Button::B.bit(), 1 << 15);
        assert_eq!(Button::A.bit(), 1 << 7);
    }

    #[test]
    fn region_timing() {
        assert!((Region::Ntsc.frame_rate() - 60.098_8).abs() < 1e-6);
        assert!((Region::Pal.frame_rate() - 50.007).abs() < 1e-6);
        assert_eq!(Region::Ntsc.name(), "ntsc");
    }
}
