//! Adapter between the [`unko_n64`] scaffold and [`unko_core`].
//!
//! The image layer is real: `load` validates and normalises the cartridge and
//! reports its metadata. Emulation is not implemented yet, so `run_frame`
//! returns [`CoreError::Unsupported`] with a precise message. Registering the
//! core now lets the host route N64 images to it and surface that status.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use unko_core::{
    Accuracy, AudioBuffer, AudioQuality, Core, CoreError, CoreInfo, CoreRegistration, Features,
    FrameBuffer, FrameStats, InputState, LoadOptions, PixelFormat, SystemId,
};
use unko_n64::Cartridge;

/// Stable id of this core implementation.
pub const CORE_ID: &str = "n64_accuracy_core";

/// Native output width once rendering exists (480i is presented as 640x480).
pub const WIDTH: u32 = 640;
/// Native output height.
pub const HEIGHT: u32 = 480;

/// Builds the core.
#[must_use]
pub fn factory() -> Box<dyn Core> {
    Box::new(N64Core::new())
}

/// Registers this core for discovery.
#[must_use]
pub fn registration() -> CoreRegistration {
    let info = N64Core::new().info();
    CoreRegistration { info, factory }
}

/// One N64 image. Emulation is not implemented yet.
pub struct N64Core {
    cart: Option<Cartridge>,
    sample_rate: f32,
}

impl N64Core {
    /// Creates an unloaded core.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cart: None,
            sample_rate: 48_000.0,
        }
    }

    /// Parsed cartridge, when one is loaded.
    #[must_use]
    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.cart.as_ref()
    }
}

impl Default for N64Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core for N64Core {
    fn info(&self) -> CoreInfo {
        CoreInfo {
            id: CORE_ID,
            name: "Unko N64 (scaffold)",
            version: env!("CARGO_PKG_VERSION"),
            system: SystemId::Other("n64"),
            accuracy: Accuracy::Compatible,
            features: Features::empty(),
        }
    }

    fn load(&mut self, rom: &[u8], _options: &LoadOptions) -> Result<(), CoreError> {
        let cart =
            Cartridge::from_bytes(rom).map_err(|error| CoreError::InvalidRom(error.to_string()))?;
        self.cart = Some(cart);
        Ok(())
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        if self.cart.is_none() {
            return Err(CoreError::Internal("no ROM loaded".to_owned()));
        }
        Ok(())
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        Err(CoreError::Unsupported(
            "N64 CPU/RCP emulation is not implemented yet".to_owned(),
        ))
    }

    fn video(&self) -> FrameBuffer<'_> {
        FrameBuffer {
            data: &[],
            width: WIDTH,
            height: HEIGHT,
            stride: 0,
            format: PixelFormat::Rgba8,
        }
    }

    fn audio(&self) -> AudioBuffer<'_> {
        AudioBuffer {
            samples: &[],
            sample_rate: self.sample_rate,
            channels: 2,
        }
    }

    fn set_input(&mut self, _port: usize, _input: InputState) {}

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn set_audio_quality(&mut self, _quality: AudioQuality) {}

    fn save_state(&mut self, _out: &mut Vec<u8>) -> Result<(), CoreError> {
        Err(CoreError::Unsupported(
            "N64 save states are not implemented yet".to_owned(),
        ))
    }

    fn load_state(&mut self, _state: &[u8]) -> Result<(), CoreError> {
        Err(CoreError::Unsupported(
            "N64 save states are not implemented yet".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z64() -> Vec<u8> {
        let mut rom = vec![0u8; 0x100];
        rom[0x00..0x04].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        rom[0x20..0x20 + 9].copy_from_slice(b"TESTGAME ");
        rom[0x3C] = b'N';
        rom[0x3D] = b'D';
        rom[0x3E] = b'E';
        rom
    }

    #[test]
    fn loads_and_describes_the_cartridge() {
        let mut core = N64Core::new();
        core.load(&z64(), &LoadOptions::default()).expect("load");
        assert_eq!(core.info().system, SystemId::Other("n64"));
        let cart = core.cartridge().expect("cartridge");
        assert_eq!(cart.header().title, "TESTGAME");
        assert_eq!(cart.header().cart_id, [b'N', b'D']);
        let mut out = Vec::new();
        assert!(matches!(
            core.save_state(&mut out),
            Err(CoreError::Unsupported(_))
        ));
    }

    #[test]
    fn reports_unimplemented_emulation() {
        let mut core = N64Core::new();
        core.load(&z64(), &LoadOptions::default()).expect("load");
        let error = core.run_frame().expect_err("emulation is not implemented");
        assert!(error.to_string().contains("not implemented"), "{error}");
        assert!(core.reset().is_ok());
    }

    #[test]
    #[ignore = "requires UNKO_N64_ROM to point at a real N64 image"]
    fn real_image_loads_from_env() {
        let Ok(path) = std::env::var("UNKO_N64_ROM") else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read image");
        let Ok(cart) = Cartridge::from_bytes(&bytes) else {
            println!("skipping: {path} is not a standard N64 cartridge image");
            return;
        };
        println!(
            "{:?} title={:?} id={:?} country={} version={} crc={:08X}/{:08X}",
            cart.byte_order(),
            cart.header().title,
            cart.header().cart_id,
            cart.header().country as char,
            cart.header().version,
            cart.header().crc1,
            cart.header().crc2
        );
        let mut core = N64Core::new();
        core.load(&bytes, &LoadOptions::default()).expect("load");
        assert!(core.run_frame().is_err());
    }

    #[test]
    fn rejects_non_n64_images() {
        let mut core = N64Core::new();
        let mut rom = z64();
        rom[0..4].copy_from_slice(&[b'N', b'E', b'S', 0x1A]);
        assert!(matches!(
            core.load(&rom, &LoadOptions::default()),
            Err(CoreError::InvalidRom(_))
        ));
    }
}
