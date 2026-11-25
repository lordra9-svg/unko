//! Host adapter between the standalone [`unko_snes`] core and [`unko_core`],
//! the system-agnostic emulator interface used by the Unko frontend and CLI.
//!
//! `unko-snes` has no `unko-core` dependency; every conversion between the
//! host-facing configuration types and the native SNES types happens here.
//!
//! # Region
//!
//! The SNES timing region is taken from the cartridge header. `LoadOptions`
//! region overrides are accepted but ignored; a cartridge runs in the region it
//! declares.
//!
//! # Audio
//!
//! The S-DSP produces its native 32040 Hz stereo stream. The adapter reports
//! that rate on [`Core::audio`]; `set_sample_rate` is accepted but does not
//! resample (the driver already emits pre-resampler PCM, and resampling is the
//! host's responsibility).
//!
//! # Coprocessor firmware
//!
//! Firmware is supplied by the user (never bundled) through
//! [`SnesCore::set_firmware`] before the first [`Core::load`]: the S-SMP IPL
//! ROM, the CX4 data ROM, or the NEC DSP program ROM. A cartridge whose
//! firmware is absent runs the base console (as the regression baseline does).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use unko_core::{
    Accuracy, AudioBuffer, AudioQuality, Core, CoreError, CoreInfo, CoreRegistration, Features,
    FrameBuffer, FrameStats, InputState, LoadOptions, PixelFormat, SystemId,
};
use unko_snes::{Cartridge, Region, Snes};

/// Stable id of this core implementation.
pub const CORE_ID: &str = "snes_accuracy_core";

/// Native S-DSP output rate in Hz (`24_606_720 / 768`).
pub const SAMPLE_RATE: f32 = 32_040.0;

/// Frames per second of the NTSC reference timing.
pub const NTSC_FPS: f32 = 60.098_8;

/// Upper bound on `step()` calls for one frame before giving up. A frame is
/// ~357k master clocks and `step()` retires at least one access, so this is
/// several times the worst case.
const FRAME_STEP_GUARD: u64 = 5_000_000;

/// Builds the core.
#[must_use]
pub fn factory() -> Box<dyn Core> {
    Box::new(SnesCore::new())
}

/// Registers this core for discovery (matches `unko_nes_host::registration`).
#[must_use]
pub fn registration() -> CoreRegistration {
    let info = SnesCore::new().info();
    CoreRegistration { info, factory }
}

/// External firmware an SNES cartridge or its coprocessor may require.
///
/// Firmware is supplied by the user (never bundled) and applied by
/// [`SnesCore::set_firmware`] before the first [`Core::load`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareKind {
    /// S-SMP IPL ROM (64 bytes). Required for the SPC700 boot.
    Ipl,
    /// CX4 / Hitachi HG51B data ROM (3072 bytes).
    Cx4,
    /// NEC DSP-1/2/3/4 program ROM.
    NecDsp,
}

/// One emulated SNES.
pub struct SnesCore {
    system: Option<Snes>,
    region: Region,
    audio: Vec<f32>,
    sample_rate: f32,
    ipl: Option<[u8; 64]>,
    cx4: Option<Vec<u8>>,
    nec_dsp: Option<Vec<u8>>,
}

impl SnesCore {
    /// Creates an unloaded core.
    #[must_use]
    pub fn new() -> Self {
        Self {
            system: None,
            region: Region::Ntsc,
            audio: Vec::new(),
            sample_rate: SAMPLE_RATE,
            ipl: None,
            cx4: None,
            nec_dsp: None,
        }
    }

    /// Installs external firmware used by the next [`Core::load`].
    ///
    /// The data is validated when the core is loaded, so a bad size or image is
    /// reported then as [`CoreError::Io`] / [`CoreError::Unsupported`].
    pub fn set_firmware(&mut self, kind: FirmwareKind, bytes: &[u8]) -> Result<(), CoreError> {
        match kind {
            FirmwareKind::Ipl => {
                let ipl: [u8; 64] = bytes.try_into().map_err(|_| {
                    CoreError::Io(format!("IPL ROM must be 64 bytes, got {}", bytes.len()))
                })?;
                self.ipl = Some(ipl);
            }
            FirmwareKind::Cx4 => self.cx4 = Some(bytes.to_vec()),
            FirmwareKind::NecDsp => self.nec_dsp = Some(bytes.to_vec()),
        }
        Ok(())
    }

    fn apply_firmware(&self, system: &mut Snes) -> Result<(), CoreError> {
        if let Some(ipl) = self.ipl {
            system.set_ipl(ipl);
        }
        if let Some(cx4) = self.cx4.as_ref() {
            system
                .set_cx4_firmware(cx4)
                .map_err(|error| CoreError::Unsupported(error.to_string()))?;
        }
        if let Some(dsp) = self.nec_dsp.as_ref() {
            system
                .set_dsp1_firmware(dsp)
                .map_err(|error| CoreError::Unsupported(error.to_string()))?;
        }
        Ok(())
    }

    fn machine(&mut self) -> Result<&mut Snes, CoreError> {
        self.system
            .as_mut()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".to_owned()))
    }
}

impl Default for SnesCore {
    fn default() -> Self {
        Self::new()
    }
}

impl Core for SnesCore {
    fn info(&self) -> CoreInfo {
        CoreInfo {
            id: CORE_ID,
            name: "Unko SNES (accuracy)",
            version: env!("CARGO_PKG_VERSION"),
            system: SystemId::Snes,
            accuracy: Accuracy::CycleAccurate,
            features: Features::SAVE_STATES.union(Features::BATTERY),
        }
    }

    fn load(&mut self, rom: &[u8], _options: &LoadOptions) -> Result<(), CoreError> {
        let cart =
            Cartridge::from_bytes(rom).map_err(|error| CoreError::InvalidRom(error.to_string()))?;
        let region = cart.header().region();
        let mut system = Snes::new(cart);
        // Firmware must be in place before the reset that boots the APU.
        self.apply_firmware(&mut system)?;
        system.reset();
        self.system = Some(system);
        self.region = region;
        self.audio.clear();
        Ok(())
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        self.machine()?.reset();
        self.audio.clear();
        Ok(())
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        let system = self.machine()?;
        let mut guard = 0u64;
        while !system.ppu.frame_ready() {
            system.step();
            guard += 1;
            if guard >= FRAME_STEP_GUARD {
                return Err(CoreError::Internal(
                    "frame did not complete within the step budget".to_owned(),
                ));
            }
        }
        let mut audio = Vec::with_capacity(1_200);
        for (left, right) in system.apu.drain_samples() {
            audio.push(f32::from(left) / 32_768.0);
            audio.push(f32::from(right) / 32_768.0);
        }
        system.ppu.acknowledge_frame();
        self.audio = audio;
        Ok(FrameStats {
            frames: 1,
            samples: self.audio.len() / 2,
        })
    }

    fn video(&self) -> FrameBuffer<'_> {
        let Some(system) = self.system.as_ref() else {
            return FrameBuffer {
                data: &[],
                width: 0,
                height: 0,
                stride: 0,
                format: PixelFormat::Rgba8,
            };
        };
        let width = system.ppu.frame_width() as u32;
        let height = system.ppu.frame_height() as u32;
        FrameBuffer {
            data: system.ppu.frame_buffer(),
            width,
            height,
            stride: width as usize * 4,
            format: PixelFormat::Rgba8,
        }
    }

    fn audio(&self) -> AudioBuffer<'_> {
        AudioBuffer {
            samples: &self.audio,
            sample_rate: self.sample_rate,
            channels: 2,
        }
    }

    fn set_input(&mut self, port: usize, input: InputState) {
        // `InputState` bits use the S-SMP auto-poll order; see
        // `unko_snes::io::button`.
        if let Some(system) = self.system.as_mut() {
            system.set_joypad(port, input.bits());
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        // The core emits native 32040 Hz PCM; keep reporting the native rate.
        let _ = sample_rate;
    }

    fn set_audio_quality(&mut self, _quality: AudioQuality) {}

    fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
        let state = self.machine()?.save_state();
        out.clear();
        out.extend_from_slice(&state);
        Ok(())
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
        self.machine()?
            .load_state(state)
            .map_err(|error| CoreError::State(error.to_string()))
    }

    fn sram(&mut self) -> &[u8] {
        match self.system.as_ref() {
            Some(system) => system.cart.sram(),
            None => &[],
        }
    }

    fn load_sram(&mut self, sram: &[u8]) -> Result<(), CoreError> {
        if let Some(system) = self.system.as_mut() {
            system.cart.load_sram(sram);
        }
        Ok(())
    }
}

impl unko_snes_api::SnesCore for SnesCore {
    fn as_core(&mut self) -> &mut dyn Core {
        self
    }

    fn as_core_ref(&self) -> &dyn Core {
        self
    }

    fn region(&self) -> unko_snes_api::Region {
        match self.region {
            Region::Ntsc => unko_snes_api::Region::Ntsc,
            Region::Pal => unko_snes_api::Region::Pal,
        }
    }

    fn frame_rate(&self) -> f64 {
        self.region().frame_rate()
    }
}

/// Whether `bytes` parses as an SNES cartridge image.
#[must_use]
pub fn is_snes_rom(bytes: &[u8]) -> bool {
    Cartridge::from_bytes(bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unko_snes::{Mapping, RomHeader};

    /// Minimal valid LoROM image, mirroring the `unko-snes` cartridge tests.
    fn build_lorom(len: usize) -> Vec<u8> {
        let mut rom: Vec<u8> = (0..len)
            .map(|index| (index.wrapping_mul(37) & 0xFF) as u8)
            .collect();
        let offset = Mapping::LoRom.header_offset();
        rom[offset..offset + 21].fill(0);
        rom[offset..offset + 21].copy_from_slice(b"TEST ROM             ");
        rom[offset + 0x15] = 0x20; // LoROM
        rom[offset + 0x16] = 0x02; // ROM + RAM + battery
        rom[offset + 0x17] = (len / 1024).trailing_zeros() as u8;
        rom[offset + 0x18] = 0x03; // 8 KiB SRAM
        rom[offset + 0x19] = 1;
        rom[offset + 0x1A] = 0x01;
        rom[offset + 0x3C..offset + 0x3E].copy_from_slice(&0x8000u16.to_le_bytes());

        let mut checksum_source = rom.clone();
        checksum_source[offset + 0x1C..offset + 0x20].fill(0);
        let base = checksum_source.iter().map(|&b| u32::from(b)).sum::<u32>();
        let checksum = ((base + 0x01FE) & 0xFFFF) as u16;
        rom[offset + 0x1C..offset + 0x1E].copy_from_slice(&(!checksum).to_le_bytes());
        rom[offset + 0x1E..offset + 0x20].copy_from_slice(&checksum.to_le_bytes());
        rom
    }

    #[test]
    fn loads_a_lorom_cartridge() {
        let mut core = SnesCore::new();
        assert_eq!(core.info().system, SystemId::Snes);
        let rom = build_lorom(0x8000);
        core.load(&rom, &LoadOptions::default()).expect("load");
        assert!(core.system.is_some());
        assert_eq!(core.region, Region::Ntsc);
        // Nothing has run yet.
        assert!(core.audio().samples.is_empty());
        assert_eq!(core.audio().channels, 2);
        assert_eq!(core.audio().sample_rate, SAMPLE_RATE);
    }

    #[test]
    fn parses_the_built_header() {
        let rom = build_lorom(0x8000);
        let cart = Cartridge::from_bytes(&rom).expect("cart");
        let header: &RomHeader = cart.header();
        assert_eq!(header.title.trim(), "TEST ROM");
        assert!(header.declared_sram_size.unwrap_or(0) > 0);
    }

    #[test]
    fn state_round_trips() {
        let mut core = SnesCore::new();
        core.load(&build_lorom(0x8000), &LoadOptions::default())
            .expect("load");
        core.run_frame().expect("first frame");
        let mut a = Vec::new();
        core.save_state(&mut a).expect("save");
        assert!(!a.is_empty());
        core.run_frame().expect("second frame");
        core.load_state(&a).expect("load state");
        let mut b = Vec::new();
        core.save_state(&mut b).expect("save again");
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_malformed_firmware_sizes() {
        let mut core = SnesCore::new();
        assert!(core.set_firmware(FirmwareKind::Ipl, &[0u8; 32]).is_err());

        // A wrong-sized CX4 image is only detected when the core is loaded.
        core.set_firmware(FirmwareKind::Cx4, &[0u8; 16])
            .expect("staged");
        let error = core
            .load(&build_lorom(0x8000), &LoadOptions::default())
            .expect_err("bad CX4 firmware must be rejected");
        assert!(matches!(error, CoreError::Unsupported(_)), "{error:?}");
    }

    #[test]
    fn applies_ipl_before_reset() {
        let mut core = SnesCore::new();
        core.set_firmware(FirmwareKind::Ipl, &[0u8; 64])
            .expect("stage IPL");
        core.load(&build_lorom(0x8000), &LoadOptions::default())
            .expect("load with IPL");
        assert!(core.run_frame().is_ok());
    }
}
