//! [`NesCore`] adapter over the N64 scaffold.
//!
//! The image loads and its metadata is available, but emulation is not
//! implemented; the first `run_frame` returns the precise error that the player
//! shows and pauses on.

use unko_core::{
    AudioBuffer, Core, CoreError, CoreInfo, FrameBuffer, FrameStats, InputState, LoadOptions,
};
use unko_n64_host::N64Core;
use unko_nes_api::{NesCore, VideoFilter};

/// Adapter so the existing player loop can open an N64 image.
pub struct N64Adapter {
    inner: N64Core,
}

impl N64Adapter {
    /// Creates an unloaded N64 core.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: N64Core::new(),
        }
    }

    /// The wrapped scaffold core.
    #[must_use]
    pub fn inner(&self) -> &N64Core {
        &self.inner
    }
}

impl Default for N64Adapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Core for N64Adapter {
    fn info(&self) -> CoreInfo {
        self.inner.info()
    }

    fn load(&mut self, rom: &[u8], options: &LoadOptions) -> Result<(), CoreError> {
        self.inner.load(rom, options)
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        self.inner.reset()
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        self.inner.run_frame()
    }

    fn video(&self) -> FrameBuffer<'_> {
        self.inner.video()
    }

    fn audio(&self) -> AudioBuffer<'_> {
        self.inner.audio()
    }

    fn set_input(&mut self, port: usize, input: InputState) {
        self.inner.set_input(port, input);
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.inner.set_sample_rate(sample_rate);
    }

    fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
        self.inner.save_state(out)
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
        self.inner.load_state(state)
    }
}

impl NesCore for N64Adapter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_core(&mut self) -> &mut dyn Core {
        self
    }

    fn as_core_ref(&self) -> &dyn Core {
        self
    }

    fn set_video_filter(&mut self, _filter: VideoFilter) {}

    fn frame_rate(&self) -> f64 {
        60.0
    }
}
