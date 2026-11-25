//! Adapter between the [`unko_n64`] scaffold and [`unko_core`].
//!
//! `load` validates and normalises the cartridge, then boots its IPL3 on the
//! VR4300 interpreter (PIF HLE) with RDRAM, RSP memory and PI/VI stubs.
//! `run_frame` runs RSP scalar code/DMA, applies pending RDP fill commands and
//! presents the VI framebuffer from RDRAM as RGBA. Textured triangles, the RSP
//! vector unit, audio and precise exceptions are the next milestone.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use unko_core::{
    Accuracy, AudioBuffer, AudioQuality, Core, CoreError, CoreInfo, CoreRegistration, Features,
    FrameBuffer, FrameStats, InputState, LoadOptions, PixelFormat, SystemId,
};
use unko_n64::machine::Machine;
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
    machine: Option<Machine>,
    sample_rate: f32,
}

impl N64Core {
    /// Creates an unloaded core.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cart: None,
            machine: None,
            sample_rate: 48_000.0,
        }
    }

    /// Parsed cartridge, when one is loaded.
    #[must_use]
    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.cart.as_ref()
    }

    /// Booted machine, when one is loaded.
    #[must_use]
    pub fn machine(&self) -> Option<&Machine> {
        self.machine.as_ref()
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
        self.machine = Some(Machine::new(&cart));
        self.cart = Some(cart);
        Ok(())
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        let cart = self
            .cart
            .as_ref()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".to_owned()))?;
        self.machine = Some(Machine::new(cart));
        Ok(())
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        let machine = self
            .machine
            .as_mut()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".to_owned()))?;
        machine
            .run_frame()
            .map_err(|error| CoreError::Unsupported(error.to_string()))?;
        Ok(FrameStats {
            frames: 1,
            samples: 0,
        })
    }

    fn video(&self) -> FrameBuffer<'_> {
        let data = self
            .machine
            .as_ref()
            .map_or(&[][..], |machine| machine.frame());
        FrameBuffer {
            data,
            width: WIDTH,
            height: HEIGHT,
            stride: WIDTH as usize * 4,
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
        // IPL3: beq zero,zero,-1 ; nop (a valid endless loop).
        rom[0x80..0x84].copy_from_slice(&0x1000_FFFFu32.to_be_bytes());
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
    fn boots_and_presents_frames() {
        let mut core = N64Core::new();
        core.load(&z64(), &LoadOptions::default()).expect("load");
        let stats = core.run_frame().expect("placeholder frame");
        assert_eq!(stats.frames, 1);
        assert_eq!(core.video().width, WIDTH);
        assert_eq!(core.video().stride, WIDTH as usize * 4);
        assert_eq!(core.machine().expect("machine").frames, 1);
        assert!(core.reset().is_ok());
        assert_eq!(core.machine().expect("machine").frames, 0);
    }

    #[test]
    fn reports_an_unimplemented_opcode() {
        let mut rom = z64();
        // COP1 (floating point) is not implemented yet.
        rom[0x80..0x84].copy_from_slice(&[0x46, 0x20, 0x00, 0x00]);
        let mut core = N64Core::new();
        core.load(&rom, &LoadOptions::default()).expect("load");
        let error = core.run_frame().expect_err("unimplemented opcode");
        assert!(error.to_string().contains("unimplemented"), "{error}");
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
