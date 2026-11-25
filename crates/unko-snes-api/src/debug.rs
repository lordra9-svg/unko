//! Optional SNES debugger snapshots shared by frontends and the host adapter.
//!
//! Every accessor is optional: a core without debug hooks returns `None`, so
//! the frontend can hide the panels. The types deliberately mirror
//! `unko_nes_api::debug` where the concept is shared (register/PPU snapshots,
//! a borrowed disassembly row) and otherwise describe the SNES machine.

/// Address space used by SNES debugger reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebugSpace {
    /// Work RAM, 128 KiB.
    Wram,
    /// Video RAM, 16-bit words addressed as bytes (64 KiB).
    Vram,
    /// Object attribute memory (544 bytes).
    Oam,
    /// Colour RAM, 15-bit words addressed as bytes (512 bytes).
    Cgram,
    /// S-SMP APU RAM (64 KiB).
    ApuRam,
    /// S-DSP register file (`$00-$7F`).
    DspRegisters,
}

impl DebugSpace {
    /// Human label for the space.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Wram => "WRAM",
            Self::Vram => "VRAM",
            Self::Oam => "OAM",
            Self::Cgram => "CGRAM",
            Self::ApuRam => "APU RAM",
            Self::DspRegisters => "DSP",
        }
    }

    /// Number of addressable bytes in the space.
    #[must_use]
    pub const fn size(self) -> u32 {
        match self {
            Self::Wram => 0x2_0000,
            Self::Vram => 0x1_0000,
            Self::Oam => 0x220,
            Self::Cgram => 0x200,
            Self::ApuRam => 0x1_0000,
            Self::DspRegisters => 0x80,
        }
    }

    /// Clamps an address into the space.
    #[must_use]
    pub const fn clamp(self, address: u32) -> u32 {
        let mask = self.size() - 1;
        address & mask
    }
}

/// Snapshot of the 65C816 registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuState {
    /// Accumulator (16-bit).
    pub a: u16,
    /// X index (16-bit).
    pub x: u16,
    /// Y index (16-bit).
    pub y: u16,
    /// Stack pointer.
    pub s: u16,
    /// Direct page register.
    pub d: u16,
    /// Program bank.
    pub pbr: u8,
    /// Data bank.
    pub dbr: u8,
    /// Program counter within the program bank.
    pub pc: u16,
    /// Packed processor status.
    pub p: u8,
    /// Emulation mode.
    pub emulation: bool,
    /// Master clocks retired since reset.
    pub cycles: u64,
}

/// Snapshot of the S-PPU position and display state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuState {
    /// Completed frames.
    pub frame: u64,
    /// Current scanline.
    pub line: u16,
    /// Current dot within the scanline.
    pub dot: u16,
    /// Output width in pixels (256 or 512).
    pub width: u32,
    /// Output height in pixels (224 or 240).
    pub height: u32,
    /// Hi-res mode.
    pub hires: bool,
    /// Currently in vblank.
    pub vblank: bool,
    /// INIDISP brightness (`0-15`).
    pub brightness: u8,
    /// INIDISP forced blank.
    pub forced_blank: bool,
}

/// Snapshot of the S-DSP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DspState {
    /// Output sample rate.
    pub sample_rate: u32,
    /// Finished output samples since reset.
    pub samples: u64,
    /// Raw register file `$00-$7F`.
    pub registers: [u8; 128],
}

/// One decoded instruction for a disassembly listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
    /// Address of the opcode (24-bit).
    pub address: u32,
    /// Instruction bytes.
    pub bytes: Vec<u8>,
    /// Formatted instruction.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_clamp_and_size() {
        assert_eq!(DebugSpace::Wram.size(), 0x2_0000);
        assert_eq!(DebugSpace::Wram.clamp(0x3_FFFF), 0x1_FFFF);
        assert_eq!(DebugSpace::Oam.clamp(0x1_0000), 0);
        assert_eq!(DebugSpace::DspRegisters.size(), 0x80);
        assert_eq!(DebugSpace::Vram.label(), "VRAM");
    }
}
