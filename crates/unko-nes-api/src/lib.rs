//! NES-specific extension surface for the generic Unko core interface.
//!
//! [`unko_core::Core`] owns only the generic lifecycle: load, run, video,
//! audio, input, sample rate and save states. Everything tied to the NES
//! machine — controller-port devices, the Famicom microphone and 3D glasses,
//! Vs. System input, the Miracle Piano, Datach, FDS, core-side palettes and
//! filters, cheats and the PPU/APU debugger — lives on [`NesCore`].
//!
//! It also owns the NES-specific shared vocabulary: controller-port devices,
//! palettes, video/audio filter models, reset policies, overscan/mask options,
//! cheat codes, the NES button constants and the PPU/APU debugger snapshots.
//!
//! `unko-nes-host` implements both traits. The desktop shell stores a
//! `Box<dyn NesCore>`; future systems implement only [`unko_core::Core`] and
//! their own extension trait.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod debug;
pub mod input;
pub mod types;

pub use debug::{ApuState, CpuState, DebugSpace, Disassembly, PpuState};
pub use input::{Button, NesButtons};
pub use types::{
    AudioFilterModel, AudioSource, Cheat, MaskEdges, Overscan, Palette, PortDevice,
    PpuResetBehavior, RamClear, VideoFilter,
};

use unko_core::{Core, CoreError, LoadOptions, StereoEye};

/// NES-specific host controls layered over [`Core`].
///
/// Methods with a default are optional for a future NES core; the accuracy
/// adapter implements all of them.
pub trait NesCore: Core {
    /// Attaches a device to a controller port (`Gamepad` by default).
    fn set_port_device(&mut self, _port: usize, _device: PortDevice) {}

    /// Attaches or detaches Famicom 3D shutter glasses.
    ///
    /// Cores that support field-sequential output report the eye of each
    /// completed frame from [`NesCore::stereo_eye`].
    fn set_stereo_glasses(&mut self, _enabled: bool) {}

    /// Eye for the most recently completed field, or `None` when no stereo
    /// device is attached.
    ///
    /// Hosts can combine consecutive left and right frames with
    /// [`unko_core::compose_anaglyph`].
    fn stereo_eye(&self) -> Option<StereoEye> {
        None
    }

    /// Reports the Famicom microphone state (`$4016` bit 2).
    fn set_microphone(&mut self, _pressed: bool) {}

    /// Reports the Bandai Karaoke Studio microphone (`$6000-$7FFF` bit 2).
    ///
    /// The A/B buttons come from controller port 1. Cores without a mapper
    /// 188 cartridge ignore it.
    fn set_bandai_microphone(&mut self, _pressed: bool) {}

    /// Whether the loaded image is a Vs. System cabinet board.
    fn is_vs_system(&self) -> bool {
        false
    }

    /// Sets the Vs. System service button, DIP switches and CPU side.
    fn set_vs_input(&mut self, _service: bool, _dips: u8, _primary: bool) {}

    /// Inserts a Vs. System coin into slot 0 (left) or 1 (right).
    fn insert_vs_coin(&mut self, _slot: usize) {}

    /// Sends a Miracle Piano note toward the NES software.
    ///
    /// `velocity` 0 is a Note Off; a non-zero value is a Note On on channel 0.
    fn set_piano_note(&mut self, _note: u8, _velocity: u8) {}

    /// Loads a Datach barcode for a mapper 157 cartridge.
    fn set_datach_barcode(&mut self, _code: &str) -> bool {
        false
    }

    /// Datach adapter X24C02 contents (256 bytes; empty when absent).
    fn datach_eeprom(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Restores the Datach adapter X24C02 contents.
    fn load_datach_eeprom(&mut self, _bytes: &[u8]) {}

    /// Aims the Zapper attached to `port` and reports the trigger state.
    fn set_zapper(&mut self, _port: usize, _x: u8, _y: u8, _trigger: bool) {}

    /// Sets a paddle's knob position (`0..=255`) and fire button.
    fn set_paddle(&mut self, _port: usize, _position: u8, _fire: bool) {}

    /// Adds mouse motion since the last report (`dx` right, `dy` up, in
    /// mickeys) and sets the two buttons.
    fn set_mouse(&mut self, _port: usize, _dx: i32, _dy: i32, _left: bool, _right: bool) {}

    /// Sets the pressed Power Pad buttons (bit `i` = button `i + 1`).
    fn set_power_pad(&mut self, _port: usize, _buttons: u16) {}

    /// Replaces the Family BASIC keyboard matrix.
    ///
    /// One byte per matrix row: low nibble = column half 0, high nibble =
    /// column half 1, and a set bit means the key is held.
    fn set_family_keyboard(&mut self, _keys: [u8; 9]) {}

    /// Replaces the Subor keyboard matrix (13 rows, active-high).
    fn set_subor_keyboard(&mut self, _rows: [u8; 13]) {}

    /// Changes the core-side video filter.
    fn set_video_filter(&mut self, filter: VideoFilter);

    /// Changes the display palette.
    fn set_palette(&mut self, _palette: Palette) {}

    /// Supplies the 64-entry RGB table used by [`Palette::Custom`].
    fn set_palette_table(&mut self, _table: [[u8; 3]; 64]) {}

    /// Selects the analog output filter model. May take effect on the next
    /// frame.
    fn set_audio_filters(&mut self, _filters: AudioFilterModel) {}

    /// Selects which audio sources reach the mixer.
    fn set_audio_source(&mut self, _source: AudioSource) {}

    /// Selects the work-RAM fill policy used at power-on. A soft reset leaves
    /// work RAM unchanged.
    fn set_ram_clear(&mut self, _mode: RamClear) {}

    /// Selects the PPU register behaviour on a soft reset.
    fn set_ppu_reset_behavior(&mut self, _behavior: PpuResetBehavior) {}

    /// Selects the alternate MMC3A/NEC IRQ revision for MMC3-family boards.
    fn set_mmc3_alternate(&mut self, _alternate: bool) {}

    /// Replaces the active cheat overlay. The core applies it as host
    /// presentation state; it is never serialized.
    fn set_cheats(&mut self, _cheats: &[Cheat]) {}

    /// Loads a raw Famicom Disk System image with its user-supplied BIOS.
    fn load_fds(
        &mut self,
        _disk: &[u8],
        _bios: &[u8],
        _options: &LoadOptions,
    ) -> Result<(), CoreError> {
        Err(CoreError::Unsupported(
            "FDS is not supported by this core".into(),
        ))
    }

    /// Number of disk sides in the loaded FDS image (0 for cartridges).
    fn disk_sides(&self) -> usize {
        0
    }

    /// Currently inserted FDS side.
    fn current_disk_side(&self) -> usize {
        0
    }

    /// Inserts an FDS side, or ejects it with `None`.
    fn insert_disk_side(&mut self, _side: Option<usize>) {}

    /// Raw FDS image with pending writes flushed (empty for cartridges).
    fn disk_image(&mut self) -> Vec<u8> {
        Vec::new()
    }

    /// Sets the FDS transfer-speed multiplier (1/2/4/8).
    fn set_fds_speed(&mut self, _speed: u8) {}

    /// Enables or disables automatic FDS side swapping.
    fn set_fds_auto_side_swap(&mut self, _enabled: bool) {}

    /// Sets the per-channel mixer gains in
    /// `[pulse1, pulse2, triangle, noise, dmc, expansion]` order.
    ///
    /// A gain of `1.0` is the unmodified hardware mix; the host uses this for
    /// the per-channel volume/mute controls. The values are presentation
    /// state, never part of a save state.
    fn set_channel_gains(&mut self, _gains: [f32; 6]) {}

    /// Whether this core implements the debugger surface.
    ///
    /// Frontends check this once instead of probing every accessor.
    fn debug_supported(&self) -> bool {
        false
    }

    /// Snapshot of the CPU registers, or `None` when unsupported.
    fn debug_cpu(&self) -> Option<CpuState> {
        None
    }

    /// Snapshot of the PPU timing and registers, or `None` when unsupported.
    fn debug_ppu(&self) -> Option<PpuState> {
        None
    }

    /// Snapshot of the APU channels, or `None` when unsupported.
    fn debug_apu(&self) -> Option<ApuState> {
        None
    }

    /// Side-effect-free read of one byte in `space`.
    fn debug_peek(&self, _space: DebugSpace, _address: u16) -> Option<u8> {
        None
    }

    /// Writes one byte in `space`; returns whether the write was accepted.
    ///
    /// ROM and register ranges should be rejected rather than silently
    /// ignored, so the frontend can report read-only addresses.
    fn debug_poke(&mut self, _space: DebugSpace, _address: u16, _value: u8) -> bool {
        false
    }

    /// Executes exactly one CPU instruction, advancing the rest of the
    /// machine with it. Returns the cycles consumed.
    fn debug_step(&mut self) -> Result<u32, CoreError> {
        Err(CoreError::Unsupported("debug single-step".into()))
    }

    /// Decodes the instruction at a CPU address without executing it.
    fn debug_disassemble(&self, _address: u16) -> Option<Disassembly> {
        None
    }

    /// The 32 palette entries as RGB, after emphasis, for the debugger.
    fn debug_palette(&self) -> Option<[[u8; 3]; 32]> {
        None
    }

    /// Frames per second of the loaded machine (NTSC/PAL/Dendy).
    ///
    /// Netplay uses this for GGRS time synchronization; cores may override it.
    fn frame_rate(&self) -> f64 {
        60.0988
    }

    /// Downcast hook for system-specific extension surfaces.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Mutable downcast hook for system-specific extension surfaces.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Reborrows this machine through the generic [`Core`] interface.
    ///
    /// Used when a generic helper (netplay, tracing, hashing) expects
    /// `&mut dyn Core`.
    fn as_core(&mut self) -> &mut dyn Core;

    /// Shared reborrow through the generic [`Core`] interface.
    fn as_core_ref(&self) -> &dyn Core;
}
