//! Minimal rollback session loop.
//!
//! Player 1's input is local; player 2's input is received from the peer.
//! Ports 3/4 are cleared. Frames are simulated with the last known remote
//! input until the real one arrives; a late input restores an older saved
//! state, truncates the post-rollback states and resimulates up to the current
//! frame.

use std::collections::BTreeMap;

use unko_core::{Core, CoreError, InputState};

use crate::rollback::{RollbackBuffer, StoredFrame};

/// A rollback session around one core.
pub struct RollbackSession {
    core: Box<dyn Core>,
    buffer: RollbackBuffer,
    known_remote: BTreeMap<u32, [u16; 4]>,
    last_remote: [u16; 4],
    frame: u32,
}

impl RollbackSession {
    /// Creates a session holding `buffer_frames` rollback states.
    #[must_use]
    pub fn new(core: Box<dyn Core>, buffer_frames: usize) -> Self {
        Self {
            core,
            buffer: RollbackBuffer::new(buffer_frames),
            known_remote: BTreeMap::new(),
            last_remote: [0; 4],
            frame: 0,
        }
    }

    /// Current (next) frame number.
    #[must_use]
    pub const fn frame(&self) -> u32 {
        self.frame
    }

    /// Advances one frame with the local player's input.
    ///
    /// # Errors
    ///
    /// Returns a core save/run error.
    pub fn step(&mut self, local: [u16; 4]) -> Result<u32, CoreError> {
        let frame = self.frame;
        let remote = self.known_remote.remove(&frame).unwrap_or(self.last_remote);
        self.last_remote = remote;
        let mut state = Vec::new();
        self.core.save_state(&mut state)?;
        self.buffer.push(StoredFrame {
            frame,
            inputs: local,
            state,
        });
        self.apply(local, remote);
        self.core.run_frame()?;
        self.frame = frame + 1;
        Ok(frame)
    }

    /// Registers a remote input. Returns `true` when the frames were
    /// resimulated.
    ///
    /// # Errors
    ///
    /// Returns a core save/load error.
    pub fn receive_remote(&mut self, frame: u32, inputs: [u16; 4]) -> Result<bool, CoreError> {
        self.known_remote.insert(frame, inputs);
        let oldest = self.buffer.oldest_frame().unwrap_or(u32::MAX);
        if frame >= self.frame || frame < oldest {
            return Ok(false);
        }
        let Some(entry) = self.buffer.find(frame) else {
            return Ok(false);
        };
        let state = entry.state.clone();
        let replay: Vec<(u32, [u16; 4])> = self
            .buffer
            .iter()
            .filter(|entry| entry.frame >= frame)
            .map(|entry| (entry.frame, entry.inputs))
            .collect();
        self.core.load_state(&state)?;
        self.buffer.truncate_from(frame);
        // The last remote input known before the rollback point seeds the
        // prediction for any frame without a real input.
        let mut last = self
            .known_remote
            .range(..frame)
            .next_back()
            .map_or(self.last_remote, |(_, inputs)| *inputs);
        for (number, local) in replay {
            let remote = self.known_remote.get(&number).copied().unwrap_or(last);
            if self.known_remote.contains_key(&number) {
                last = remote;
            }
            let mut state = Vec::new();
            self.core.save_state(&mut state)?;
            self.buffer.push(StoredFrame {
                frame: number,
                inputs: local,
                state,
            });
            self.apply(local, remote);
            self.core.run_frame()?;
        }
        self.last_remote = last;
        Ok(true)
    }

    /// Writes the underlying core state (for tests and diagnostics).
    ///
    /// # Errors
    ///
    /// Returns the core's save-state error.
    pub fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
        self.core.save_state(out)
    }

    fn apply(&mut self, local: [u16; 4], remote: [u16; 4]) {
        self.core.set_input(0, InputState::from_bits(local[0]));
        self.core.set_input(1, InputState::from_bits(remote[0]));
        for port in 2..4 {
            self.core.set_input(port, InputState::empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unko_core::{
        Accuracy, AudioBuffer, CoreInfo, Features, FrameBuffer, FrameStats, LoadOptions,
        PixelFormat, SystemId, VideoFilter,
    };

    #[derive(Default)]
    struct FakeCore {
        frame: u32,
        hash: u64,
        inputs: [u16; 4],
    }

    impl FakeCore {
        fn state(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&self.frame.to_le_bytes());
            out.extend_from_slice(&self.hash.to_le_bytes());
            out
        }
    }

    impl Core for FakeCore {
        fn info(&self) -> CoreInfo {
            CoreInfo {
                id: "fake",
                name: "Fake",
                version: "0",
                system: SystemId::Nes,
                accuracy: Accuracy::High,
                features: Features::SAVE_STATES,
            }
        }

        fn load(&mut self, _rom: &[u8], _options: &LoadOptions) -> Result<(), CoreError> {
            Ok(())
        }

        fn reset(&mut self) -> Result<(), CoreError> {
            self.frame = 0;
            self.hash = 0;
            Ok(())
        }

        fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
            for port in self.inputs {
                self.hash = self.hash.wrapping_mul(31).wrapping_add(u64::from(port));
            }
            self.frame += 1;
            Ok(FrameStats {
                frames: 1,
                samples: 0,
            })
        }

        fn video(&self) -> FrameBuffer<'_> {
            FrameBuffer {
                data: &[],
                width: 0,
                height: 0,
                stride: 0,
                format: PixelFormat::Rgba8,
            }
        }

        fn audio(&self) -> AudioBuffer<'_> {
            AudioBuffer {
                samples: &[],
                sample_rate: 44_100.0,
                channels: 1,
            }
        }

        fn set_input(&mut self, port: usize, input: InputState) {
            if let Some(slot) = self.inputs.get_mut(port) {
                *slot = input.bits();
            }
        }

        fn set_sample_rate(&mut self, _sample_rate: f32) {}

        fn set_video_filter(&mut self, _filter: VideoFilter) {}

        fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
            out.extend_from_slice(&self.state());
            Ok(())
        }

        fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
            if state.len() != 12 {
                return Err(CoreError::State("bad fake state".into()));
            }
            self.frame = u32::from_le_bytes(state[0..4].try_into().unwrap());
            self.hash = u64::from_le_bytes(state[4..12].try_into().unwrap());
            Ok(())
        }
    }

    fn expected_state(locals: &[[u16; 4]], remotes: &[[u16; 4]]) -> Vec<u8> {
        let mut core = FakeCore::default();
        for (local, remote) in locals.iter().zip(remotes) {
            core.inputs = [local[0], remote[0], 0, 0];
            core.run_frame().unwrap();
        }
        core.state()
    }

    #[test]
    fn late_remote_input_rolls_back_and_resimulates() {
        let mut session = RollbackSession::new(Box::new(FakeCore::default()), 8);
        let locals = [[10, 0, 0, 0], [11, 0, 0, 0], [12, 0, 0, 0], [13, 0, 0, 0]];
        for local in locals {
            session.step(local).expect("step");
        }
        assert_eq!(session.frame(), 4);
        assert!(session.receive_remote(2, [20, 0, 0, 0]).expect("rollback"));

        let mut state = Vec::new();
        session.save_state(&mut state).expect("state");
        let expected = expected_state(&locals, &[[0; 4], [0; 4], [20, 0, 0, 0], [20, 0, 0, 0]]);
        assert_eq!(state, expected);
        assert_eq!(session.frame(), 4);
    }

    #[test]
    fn old_or_future_input_does_not_rollback() {
        let mut session = RollbackSession::new(Box::new(FakeCore::default()), 2);
        for frame in 0..6 {
            session.step([frame as u16, 0, 0, 0]).expect("step");
        }
        // Frame 0 has fallen out of the two-frame buffer.
        assert!(!session.receive_remote(0, [1; 4]).expect("late"));
        assert!(!session.receive_remote(9, [1; 4]).expect("future"));
        assert_eq!(session.frame(), 6);
    }
}
