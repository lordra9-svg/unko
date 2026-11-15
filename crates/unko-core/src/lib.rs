//! Host-facing emulator core interface.
//!
//! `unko-core` deliberately contains no emulation code and no console
//! vocabulary. It defines the generic lifecycle (`load`, `run_frame`, video,
//! audio, input masks, save states), the registry and presentation types.
//! NES-only controls and types live in `unko-nes-api`; frontends and cores
//! meet here.
//!
//! ```no_run
//! use unko_core::{Core, LoadOptions, Registry};
//! # fn run(registry: &Registry, rom: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! let mut core = registry
//!     .create_default(unko_core::SystemId::Nes)
//!     .ok_or("no NES core")?;
//! core.load(rom, &LoadOptions::default())?;
//! core.run_frame()?;
//! let frame = core.video();
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod core;
pub mod input;
pub mod registry;
pub mod stereo;
pub mod types;

pub use core::{Core, CoreError};
pub use input::InputState;
pub use registry::{CoreRegistration, Factory, Registry};
pub use stereo::{compose_anaglyph, StereoEye};
pub use types::{
    Accuracy, AudioBuffer, AudioQuality, CoreInfo, Features, FrameBuffer, FrameStats, LoadOptions,
    PixelFormat, Region, SystemId,
};
