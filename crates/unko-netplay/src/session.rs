//! Minimal rollback session loop with input-delay and prediction options.
//!
//! Player 1's input is local; player 2's input is received from the peer.
//! Ports 3/4 are cleared. Missing remote input is predicted with the last
//! known value (or neutral input); a late input restores an older saved state,
//! truncates the post-rollback states and resimulates up to the current frame.
//!
//! An optional fixed input delay shifts both the local and the remote input by
//! the same number of frames. Both peers apply the identical mapping, so the
//! simulations stay deterministic while the remote input for a simulated frame
//! arrives earlier relative to simulation time (fewer rollbacks).

use std::collections::BTreeMap;

use unko_core::{Core, CoreError, InputState};

use crate::rollback::{RollbackBuffer, StoredFrame};

/// How a missing remote input is predicted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PredictionMode {
    /// Repeat the most recent remote input.
    #[default]
    LastInput,
    /// Use released inputs (all zero) until the real value arrives.
    Neutral,
}

impl PredictionMode {
    /// Stable lowercase name used by configuration and the UI.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LastInput => "last-input",
            Self::Neutral => "neutral",
        }
    }
}

/// Session tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// Rollback states retained.
    pub buffer_frames: usize,
    /// Frames of input delay applied to both peers (0..=16).
    pub input_delay: u32,
    /// Prediction used while a remote input is missing.
    pub prediction: PredictionMode,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            buffer_frames: 8,
            input_delay: 0,
            prediction: PredictionMode::LastInput,
        }
    }
}

/// Live session counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionStats {
    /// Next simulation frame.
    pub frame: u32,
    /// Frames that required resimulation.
    pub rollbacks: u32,
    /// Frames simulated with a predicted remote input.
    pub predicted: u32,
    /// Deepest rollback (in frames) seen so far.
    pub max_rollback_depth: u32,
}

const NEUTRAL: [u16; 4] = [0; 4];

/// A rollback session around one core.
pub struct RollbackSession {
    buffer: RollbackBuffer,
    known_remote: BTreeMap<u32, [u16; 4]>,
    local_inputs: BTreeMap<u32, [u16; 4]>,
    config: SessionConfig,
    stats: SessionStats,
}

impl RollbackSession {
    /// Creates a session holding `buffer_frames` rollback states with the
    /// default prediction and no input delay.
    #[must_use]
    pub fn new(buffer_frames: usize) -> Self {
        Self::with_config(SessionConfig {
            buffer_frames,
            ..SessionConfig::default()
        })
    }

    /// Creates a session with explicit tuning.
    #[must_use]
    pub fn with_config(config: SessionConfig) -> Self {
        let config = SessionConfig {
            buffer_frames: config.buffer_frames.max(1),
            input_delay: config.input_delay.min(16),
            prediction: config.prediction,
        };
        Self {
            buffer: RollbackBuffer::new(config.buffer_frames),
            known_remote: BTreeMap::new(),
            local_inputs: BTreeMap::new(),
            config,
            stats: SessionStats::default(),
        }
    }

    /// Current tuning.
    #[must_use]
    pub const fn config(&self) -> SessionConfig {
        self.config
    }

    /// Changes the prediction mode (live).
    pub fn set_prediction(&mut self, prediction: PredictionMode) {
        self.config.prediction = prediction;
    }

    /// Changes the input delay (live, clamped to 0..=16).
    ///
    /// Changing the delay mid-session is allowed: the next frame uses the new
    /// mapping and any missing input is predicted.
    pub fn set_input_delay(&mut self, delay: u32) {
        self.config.input_delay = delay.min(16);
    }

    /// Current (next) frame number.
    #[must_use]
    pub const fn frame(&self) -> u32 {
        self.stats.frame
    }

    /// Session counters.
    #[must_use]
    pub const fn stats(&self) -> SessionStats {
        self.stats
    }

    /// Advances one frame with the local player's input.
    ///
    /// # Errors
    ///
    /// Returns a core save/run error.
    pub fn step(&mut self, core: &mut dyn Core, local: [u16; 4]) -> Result<u32, CoreError> {
        let frame = self.stats.frame;
        self.local_inputs.insert(frame, local);
        let (local, remote) = self.effective_inputs(frame, true);
        let mut state = Vec::new();
        core.save_state(&mut state)?;
        self.buffer.push(StoredFrame {
            frame,
            inputs: local,
            state,
        });
        Self::apply(core, local, remote);
        core.run_frame()?;
        self.stats.frame = frame + 1;
        Ok(frame)
    }

    /// Registers a remote input addressed to input frame `frame`. Returns
    /// `true` when the frames were resimulated.
    ///
    /// # Errors
    ///
    /// Returns a core save/load error.
    pub fn receive_remote(
        &mut self,
        core: &mut dyn Core,
        frame: u32,
        inputs: [u16; 4],
    ) -> Result<bool, CoreError> {
        if self.known_remote.get(&frame) == Some(&inputs) {
            return Ok(false);
        }
        self.known_remote.insert(frame, inputs);
        let Some(target) = frame.checked_add(self.config.input_delay) else {
            return Ok(false);
        };
        let oldest = self.buffer.oldest_frame().unwrap_or(u32::MAX);
        if target >= self.stats.frame || target < oldest {
            return Ok(false);
        }
        let Some(entry) = self.buffer.find(target) else {
            return Ok(false);
        };
        let state = entry.state.clone();
        let replay: Vec<u32> = self
            .buffer
            .iter()
            .filter(|entry| entry.frame >= target)
            .map(|entry| entry.frame)
            .collect();
        core.load_state(&state)?;
        self.buffer.truncate_from(target);
        self.stats.rollbacks = self.stats.rollbacks.saturating_add(1);
        self.stats.max_rollback_depth = self
            .stats
            .max_rollback_depth
            .max(self.stats.frame.saturating_sub(target));
        for number in replay {
            let (local, remote) = self.effective_inputs(number, false);
            let mut state = Vec::new();
            core.save_state(&mut state)?;
            self.buffer.push(StoredFrame {
                frame: number,
                inputs: local,
                state,
            });
            Self::apply(core, local, remote);
            core.run_frame()?;
        }
        Ok(true)
    }

    /// Inputs for simulation frame `frame`, applying the input delay.
    ///
    /// Both peers map a simulated frame to input frame `frame - delay`, so a
    /// missing remote input is predicted from the newest known earlier input.
    fn effective_inputs(&mut self, frame: u32, count_prediction: bool) -> ([u16; 4], [u16; 4]) {
        let Some(input_frame) = frame.checked_sub(self.config.input_delay) else {
            return (NEUTRAL, NEUTRAL);
        };
        let local = self
            .local_inputs
            .get(&input_frame)
            .copied()
            .unwrap_or(NEUTRAL);
        let remote = match self.known_remote.get(&input_frame) {
            Some(inputs) => *inputs,
            None => {
                if count_prediction {
                    self.stats.predicted = self.stats.predicted.saturating_add(1);
                }
                match self.config.prediction {
                    PredictionMode::LastInput => self
                        .known_remote
                        .range(..input_frame)
                        .next_back()
                        .map_or(NEUTRAL, |(_, inputs)| *inputs),
                    PredictionMode::Neutral => NEUTRAL,
                }
            }
        };
        (local, remote)
    }

    fn apply(core: &mut dyn Core, local: [u16; 4], remote: [u16; 4]) {
        core.set_input(0, InputState::from_bits(local[0]));
        core.set_input(1, InputState::from_bits(remote[0]));
        for port in 2..4 {
            core.set_input(port, InputState::empty());
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
            let mut out = self.frame.to_le_bytes().to_vec();
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
        let mut core = FakeCore::default();
        let mut session = RollbackSession::new(8);
        let locals = [[10, 0, 0, 0], [11, 0, 0, 0], [12, 0, 0, 0], [13, 0, 0, 0]];
        for local in locals {
            session.step(&mut core, local).expect("step");
        }
        assert_eq!(session.frame(), 4);
        assert!(session
            .receive_remote(&mut core, 2, [20, 0, 0, 0])
            .expect("rollback"));

        let state = core.state();
        let expected = expected_state(&locals, &[[0; 4], [0; 4], [20, 0, 0, 0], [20, 0, 0, 0]]);
        assert_eq!(state, expected);
        assert_eq!(session.frame(), 4);
        assert_eq!(session.stats().rollbacks, 1);
        assert_eq!(session.stats().max_rollback_depth, 2);
    }

    #[test]
    fn old_or_future_input_does_not_rollback() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::new(2);
        for frame in 0..6 {
            session
                .step(&mut core, [frame as u16, 0, 0, 0])
                .expect("step");
        }
        // Frame 0 has fallen out of the two-frame buffer.
        assert!(!session.receive_remote(&mut core, 0, [1; 4]).expect("late"));
        assert!(!session
            .receive_remote(&mut core, 9, [1; 4])
            .expect("future"));
        assert_eq!(session.frame(), 6);
    }

    #[test]
    fn input_delay_shifts_both_input_streams() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 8,
            input_delay: 1,
            prediction: PredictionMode::Neutral,
        });
        session.step(&mut core, [10, 0, 0, 0]).expect("step");
        session.step(&mut core, [11, 0, 0, 0]).expect("step");
        session.step(&mut core, [12, 0, 0, 0]).expect("step");
        assert_eq!(
            core.state(),
            expected_state(
                &[[0; 4], [10, 0, 0, 0], [11, 0, 0, 0]],
                &[[0; 4], [0; 4], [0; 4]]
            )
        );
    }

    #[test]
    fn late_input_rolls_back_at_the_delayed_frame() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 8,
            input_delay: 2,
            prediction: PredictionMode::Neutral,
        });
        let locals = [[1, 0, 0, 0], [2, 0, 0, 0], [3, 0, 0, 0], [4, 0, 0, 0]];
        for local in locals {
            session.step(&mut core, local).expect("step");
        }
        // Input frame 0 belongs to simulation frame 2.
        assert!(session
            .receive_remote(&mut core, 0, [9, 0, 0, 0])
            .expect("rollback"));
        assert_eq!(session.stats().rollbacks, 1);
        assert_eq!(session.stats().max_rollback_depth, 2);
        assert_eq!(session.frame(), 4);
        assert_eq!(
            core.state(),
            expected_state(
                &[[0; 4], [0; 4], [1, 0, 0, 0], [2, 0, 0, 0]],
                &[[0; 4], [0; 4], [9, 0, 0, 0], [0; 4]]
            )
        );
    }

    #[test]
    fn neutral_prediction_releases_missing_input() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 4,
            input_delay: 0,
            prediction: PredictionMode::Neutral,
        });
        session.step(&mut core, [5, 0, 0, 0]).expect("step");
        session.step(&mut core, [6, 0, 0, 0]).expect("step");
        assert_eq!(
            core.state(),
            expected_state(&[[5, 0, 0, 0], [6, 0, 0, 0]], &[[0; 4], [0; 4]])
        );
        assert_eq!(session.stats().predicted, 2);
        assert_eq!(session.stats().rollbacks, 0);
    }

    #[test]
    fn last_input_prediction_repeats_the_previous_remote() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::new(4);
        session
            .receive_remote(&mut core, 0, [8, 0, 0, 0])
            .expect("remote");
        session.step(&mut core, [1, 0, 0, 0]).expect("step");
        session.step(&mut core, [2, 0, 0, 0]).expect("step");
        // Frame 1 was predicted from frame 0's remote (8), not neutral.
        assert_eq!(
            core.state(),
            expected_state(&[[1, 0, 0, 0], [2, 0, 0, 0]], &[[8, 0, 0, 0], [8, 0, 0, 0]])
        );
        assert_eq!(session.stats().predicted, 1);
        assert_eq!(session.stats().rollbacks, 0);
    }

    #[test]
    fn input_delay_avoids_rollbacks_for_one_frame_latency() {
        let inputs = [[1u16, 0, 0, 0], [2, 0, 0, 0], [3, 0, 0, 0], [4, 0, 0, 0]];
        let run = |delay: u32| {
            let mut core = FakeCore::default();
            let mut session = RollbackSession::with_config(SessionConfig {
                buffer_frames: 8,
                input_delay: delay,
                prediction: PredictionMode::LastInput,
            });
            for (frame, local) in inputs.iter().enumerate() {
                session.step(&mut core, *local).expect("step");
                // One frame of one-way latency: the peer's input for frame i
                // is only available after our own step i.
                session
                    .receive_remote(&mut core, frame as u32, *local)
                    .expect("remote");
            }
            session.stats()
        };
        let delayed = run(1);
        let immediate = run(0);
        assert_eq!(delayed.rollbacks, 0, "delay hides the one-frame latency");
        assert!(immediate.rollbacks > 0, "immediate mode rolls back");
        assert_eq!(delayed.predicted, 0);
        assert!(immediate.predicted > 0);
    }
}
