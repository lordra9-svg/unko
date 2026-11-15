//! Minimal rollback session loop with per-peer inputs, input delay and
//! prediction.
//!
//! One instance owns one controller port; the other ports are received from
//! peers and merged into the four-port frame the core simulates. Missing
//! remote input is predicted per port with the last known value (or neutral
//! input); a late input restores an older saved state, truncates the
//! post-rollback states and resimulates up to the current frame.
//!
//! An optional fixed input delay shifts both local and remote input by the
//! same number of frames. All peers apply the identical mapping, so the
//! simulations stay deterministic while the remote input for a simulated frame
//! arrives earlier relative to simulation time (fewer rollbacks).

use std::collections::BTreeMap;

use unko_core::{Core, CoreError, InputState};

use crate::rollback::{RollbackBuffer, StoredFrame};

/// How a missing remote input is predicted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PredictionMode {
    /// Repeat the most recent input from that port.
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
    /// Frames of input delay applied to every player (0..=16).
    pub input_delay: u32,
    /// Prediction used while a remote input is missing.
    pub prediction: PredictionMode,
    /// Players in the session (2..=4).
    pub players: u8,
    /// Zero-based port owned by this instance.
    pub local_player: u8,
    /// Hash every state whose inputs are fully known (desync detection).
    pub hash_states: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            buffer_frames: 8,
            input_delay: 0,
            prediction: PredictionMode::LastInput,
            players: 2,
            local_player: 0,
            hash_states: false,
        }
    }
}

impl SessionConfig {
    /// Clamps the player count and local slot into a valid session.
    #[must_use]
    pub fn normalized(self) -> Self {
        let players = self.players.clamp(2, 4);
        Self {
            buffer_frames: self.buffer_frames.max(1),
            input_delay: self.input_delay.min(16),
            prediction: self.prediction,
            players,
            local_player: self.local_player.min(players - 1),
            hash_states: self.hash_states,
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
    /// Port-frames simulated with a predicted input.
    pub predicted: u32,
    /// Deepest rollback (in frames) seen so far.
    pub max_rollback_depth: u32,
}

const NEUTRAL: [u16; 4] = [0; 4];

/// A rollback session around one core.
pub struct RollbackSession {
    buffer: RollbackBuffer,
    /// Per-peer input history: port -> (input frame -> value).
    peers: BTreeMap<u8, BTreeMap<u32, u16>>,
    local_inputs: BTreeMap<u32, [u16; 4]>,
    config: SessionConfig,
    stats: SessionStats,
    /// Hashes of fully-settled frames, for desync detection.
    hashes: BTreeMap<u32, u64>,
}

impl RollbackSession {
    /// Creates a two-player session (this instance owns port 1) with the
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
        let config = config.normalized();
        Self {
            buffer: RollbackBuffer::new(config.buffer_frames),
            peers: BTreeMap::new(),
            local_inputs: BTreeMap::new(),
            config,
            stats: SessionStats::default(),
            hashes: BTreeMap::new(),
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

    /// Whether every remote port has supplied the input for simulation frame
    /// `frame` (after applying the input delay).
    ///
    /// Once this is true, the state after `frame` is final: no later input can
    /// roll it back, so it is safe to publish a state hash for comparison.
    #[must_use]
    pub fn remote_inputs_through(&self, frame: u32) -> bool {
        let Some(input_frame) = frame.checked_sub(self.config.input_delay) else {
            return true;
        };
        (0..self.config.players)
            .filter(|player| *player != self.config.local_player)
            .all(|player| {
                self.peers
                    .get(&player)
                    .is_some_and(|inputs| inputs.contains_key(&input_frame))
            })
    }

    /// Advances one frame with the local player's four-port input view.
    ///
    /// Only `local_player`'s port is taken from `local`; the rest are merged
    /// from peers or predicted.
    ///
    /// # Errors
    ///
    /// Returns a core save/run error.
    pub fn step(&mut self, core: &mut dyn Core, local: [u16; 4]) -> Result<u32, CoreError> {
        let frame = self.stats.frame;
        self.local_inputs.insert(frame, local);
        let (local, merged, _complete) = self.effective_inputs(frame, true);
        let mut state = Vec::new();
        core.save_state(&mut state)?;
        self.note_hash(frame, &state);
        self.buffer.push(StoredFrame {
            frame,
            inputs: local,
            state,
        });
        Self::apply(core, merged);
        core.run_frame()?;
        self.stats.frame = frame + 1;
        Ok(frame)
    }

    /// Registers one port's input from `player` for input frame `frame`.
    /// Returns `true` when frames were resimulated.
    ///
    /// Inputs from this instance's own port and from slots outside
    /// `config.players` are ignored.
    ///
    /// # Errors
    ///
    /// Returns a core save/load error.
    pub fn receive_remote(
        &mut self,
        core: &mut dyn Core,
        player: u8,
        frame: u32,
        input: u16,
    ) -> Result<bool, CoreError> {
        if player >= self.config.players || player == self.config.local_player {
            return Ok(false);
        }
        let known = self.peers.entry(player).or_default();
        if known.get(&frame) == Some(&input) {
            return Ok(false);
        }
        known.insert(frame, input);
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
            let (local, merged, _complete) = self.effective_inputs(number, false);
            let mut state = Vec::new();
            core.save_state(&mut state)?;
            self.note_hash(number, &state);
            self.buffer.push(StoredFrame {
                frame: number,
                inputs: local,
                state,
            });
            Self::apply(core, merged);
            core.run_frame()?;
        }
        Ok(true)
    }

    /// Builds the four-port frame for simulation frame `frame`.
    ///
    /// Both peers map a simulated frame to input frame `frame - delay`.
    fn effective_inputs(
        &mut self,
        frame: u32,
        count_prediction: bool,
    ) -> ([u16; 4], [u16; 4], bool) {
        let Some(input_frame) = frame.checked_sub(self.config.input_delay) else {
            return (NEUTRAL, NEUTRAL, true);
        };
        let local = self
            .local_inputs
            .get(&input_frame)
            .copied()
            .unwrap_or(NEUTRAL);
        let mut merged = [0u16; 4];
        let mut complete = true;
        for port in 0..usize::from(self.config.players) {
            let player = port as u8;
            if player == self.config.local_player {
                merged[port] = local[port];
                continue;
            }
            let known = self.peers.get(&player);
            match known.and_then(|inputs| inputs.get(&input_frame)) {
                Some(input) => merged[port] = *input,
                None => {
                    complete = false;
                    if count_prediction {
                        self.stats.predicted = self.stats.predicted.saturating_add(1);
                    }
                    merged[port] = match self.config.prediction {
                        PredictionMode::LastInput => known
                            .and_then(|inputs| inputs.range(..input_frame).next_back())
                            .map_or(0, |(_, input)| *input),
                        PredictionMode::Neutral => 0,
                    };
                }
            }
        }
        (local, merged, complete)
    }

    /// Hashes the pre-frame save state that is already needed for rollback.
    fn note_hash(&mut self, frame: u32, state: &[u8]) {
        if !self.config.hash_states {
            return;
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in state {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.hashes.insert(frame, hash);
        while self.hashes.len() > 1024 {
            let oldest = *self.hashes.keys().next().expect("non-empty");
            self.hashes.remove(&oldest);
        }
    }

    /// Hashes of frames that can no longer be rolled back (older than the
    /// state buffer), after `after` when given.
    ///
    /// These are the safe frames to exchange: a late input can still change a
    /// buffered frame, but not one that already fell out of the buffer.
    #[must_use]
    pub fn settled_hashes(&self, after: Option<u32>) -> Vec<(u32, u64)> {
        let oldest = self.buffer.oldest_frame().unwrap_or(u32::MAX);
        let start = after.map_or(0, |frame| frame.saturating_add(1));
        if start >= oldest {
            return Vec::new();
        }
        self.hashes
            .range(start..oldest)
            .map(|(frame, hash)| (*frame, *hash))
            .collect()
    }

    fn apply(core: &mut dyn Core, merged: [u16; 4]) {
        for (port, input) in merged.iter().enumerate() {
            core.set_input(port, InputState::from_bits(*input));
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

    /// Reference state for a list of four-port frames.
    fn expected(frames: &[[u16; 4]]) -> Vec<u8> {
        let mut core = FakeCore::default();
        for frame in frames {
            core.inputs = *frame;
            core.run_frame().unwrap();
        }
        core.state()
    }

    /// Two-player frame from a local port-0 value and a remote port-1 value.
    fn two(player0: u16, player1: u16) -> [u16; 4] {
        [player0, player1, 0, 0]
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
            .receive_remote(&mut core, 1, 2, 20)
            .expect("rollback"));

        assert_eq!(
            core.state(),
            expected(&[two(10, 0), two(11, 0), two(12, 20), two(13, 20),])
        );
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
        assert!(!session.receive_remote(&mut core, 1, 0, 1).expect("late"));
        assert!(!session.receive_remote(&mut core, 1, 9, 1).expect("future"));
        assert_eq!(session.frame(), 6);
    }

    #[test]
    fn hashes_wait_for_every_remote_input() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::new(4);
        session.step(&mut core, [1, 0, 0, 0]).expect("step");
        assert!(!session.remote_inputs_through(0));
        session.receive_remote(&mut core, 1, 0, 7).expect("remote");
        assert!(session.remote_inputs_through(0));
        assert!(!session.remote_inputs_through(1));
    }

    #[test]
    fn settled_hashes_cover_only_rollback_final_frames() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 4,
            hash_states: true,
            ..SessionConfig::default()
        });
        for frame in 0..8u32 {
            session
                .receive_remote(&mut core, 1, frame, 20 + frame as u16)
                .expect("remote");
            session
                .step(&mut core, [10 + frame as u16, 0, 0, 0])
                .expect("step");
        }
        let hashes = session.settled_hashes(None);
        assert!(!hashes.is_empty());
        assert!(
            hashes.iter().all(|(frame, _)| *frame < 4),
            "only frames older than the 4-frame buffer settle"
        );
        assert_eq!(session.settled_hashes(Some(0)), hashes[1..]);
    }

    #[test]
    fn equal_sessions_hash_equal_and_divergence_changes_the_hash() {
        let config = SessionConfig {
            buffer_frames: 4,
            hash_states: true,
            ..SessionConfig::default()
        };
        let mut a = RollbackSession::with_config(config);
        let mut b = RollbackSession::with_config(config);
        let mut c = RollbackSession::with_config(config);
        let mut core_a = FakeCore::default();
        let mut core_b = FakeCore::default();
        let mut core_c = FakeCore::default();
        for frame in 0..8u32 {
            a.receive_remote(&mut core_a, 1, frame, 30)
                .expect("a remote");
            b.receive_remote(&mut core_b, 1, frame, 30)
                .expect("b remote");
            c.receive_remote(&mut core_c, 1, frame, 30)
                .expect("c remote");
            a.step(&mut core_a, [1, 0, 0, 0]).expect("a");
            b.step(&mut core_b, [1, 0, 0, 0]).expect("b");
            c.step(&mut core_c, [1, 0, 0, 0]).expect("c");
        }
        assert_eq!(a.settled_hashes(None), b.settled_hashes(None));
        // Differing remote input at frame 0 changes states after frame 0.
        let mut d = RollbackSession::with_config(config);
        let mut core_d = FakeCore::default();
        for frame in 0..8u32 {
            let input = if frame == 0 { 31 } else { 30 };
            d.receive_remote(&mut core_d, 1, frame, input)
                .expect("d remote");
            d.step(&mut core_d, [1, 0, 0, 0]).expect("d");
        }
        let base = a.settled_hashes(None);
        let changed = d.settled_hashes(None);
        assert_eq!(base.first(), changed.first(), "pre-state 0 is unchanged");
        assert_ne!(base, changed, "later pre-states include the divergence");
    }

    #[test]
    fn local_and_out_of_range_inputs_are_ignored() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            players: 3,
            local_player: 1,
            ..SessionConfig::default()
        });
        session.step(&mut core, [0, 5, 0, 0]).expect("step");
        assert!(!session
            .receive_remote(&mut core, 1, 0, 9)
            .expect("own port"));
        assert!(!session
            .receive_remote(&mut core, 3, 0, 9)
            .expect("out of range"));
        assert_eq!(session.stats().rollbacks, 0);
    }

    #[test]
    fn input_delay_shifts_both_input_streams() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 8,
            input_delay: 1,
            prediction: PredictionMode::Neutral,
            ..SessionConfig::default()
        });
        session.step(&mut core, [10, 0, 0, 0]).expect("step");
        session.step(&mut core, [11, 0, 0, 0]).expect("step");
        session.step(&mut core, [12, 0, 0, 0]).expect("step");
        assert_eq!(core.state(), expected(&[two(0, 0), two(10, 0), two(11, 0)]));
    }

    #[test]
    fn late_input_rolls_back_at_the_delayed_frame() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 8,
            input_delay: 2,
            prediction: PredictionMode::Neutral,
            ..SessionConfig::default()
        });
        let locals = [[1, 0, 0, 0], [2, 0, 0, 0], [3, 0, 0, 0], [4, 0, 0, 0]];
        for local in locals {
            session.step(&mut core, local).expect("step");
        }
        // Input frame 0 belongs to simulation frame 2.
        assert!(session
            .receive_remote(&mut core, 1, 0, 9)
            .expect("rollback"));
        assert_eq!(session.stats().rollbacks, 1);
        assert_eq!(session.stats().max_rollback_depth, 2);
        assert_eq!(session.frame(), 4);
        assert_eq!(
            core.state(),
            expected(&[two(0, 0), two(0, 0), two(1, 9), two(2, 0)])
        );
    }

    #[test]
    fn neutral_prediction_releases_missing_input() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 4,
            prediction: PredictionMode::Neutral,
            ..SessionConfig::default()
        });
        session.step(&mut core, [5, 0, 0, 0]).expect("step");
        session.step(&mut core, [6, 0, 0, 0]).expect("step");
        assert_eq!(core.state(), expected(&[two(5, 0), two(6, 0)]));
        assert_eq!(session.stats().predicted, 2);
        assert_eq!(session.stats().rollbacks, 0);
    }

    #[test]
    fn last_input_prediction_repeats_the_previous_remote() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::new(4);
        session.receive_remote(&mut core, 1, 0, 8).expect("remote");
        session.step(&mut core, [1, 0, 0, 0]).expect("step");
        session.step(&mut core, [2, 0, 0, 0]).expect("step");
        // Frame 1 predicted port 1 from frame 0's remote (8).
        assert_eq!(core.state(), expected(&[two(1, 8), two(2, 8)]));
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
                ..SessionConfig::default()
            });
            for (frame, local) in inputs.iter().enumerate() {
                session.step(&mut core, *local).expect("step");
                // One frame of one-way latency: the peer's input for frame i
                // is only available after our own step i.
                session
                    .receive_remote(&mut core, 1, frame as u32, local[0])
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

    #[test]
    fn three_players_merge_into_one_frame() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 8,
            players: 3,
            local_player: 0,
            ..SessionConfig::default()
        });
        session.receive_remote(&mut core, 1, 0, 20).expect("p2");
        session.receive_remote(&mut core, 2, 0, 30).expect("p3");
        session.step(&mut core, [10, 0, 0, 0]).expect("step");
        assert_eq!(core.state(), expected(&[[10, 20, 30, 0]]));
        assert_eq!(session.stats().predicted, 0);
    }

    #[test]
    fn four_players_converge_after_a_late_input() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 8,
            players: 4,
            local_player: 0,
            ..SessionConfig::default()
        });
        for frame in 0..4u32 {
            for (player, value) in [(1u8, 20u16), (2, 30), (3, 40)] {
                session
                    .receive_remote(&mut core, player, frame, value + frame as u16)
                    .expect("remote");
            }
            session
                .step(&mut core, [10 + frame as u16, 0, 0, 0])
                .expect("step");
        }
        // Player 4's input for frame 1 arrives late.
        assert!(session
            .receive_remote(&mut core, 3, 1, 99)
            .expect("rollback"));
        let mut frames = Vec::new();
        for frame in 0..4u16 {
            frames.push([
                10 + frame,
                20 + frame,
                30 + frame,
                if frame == 1 { 99 } else { 40 + frame },
            ]);
        }
        assert_eq!(core.state(), expected(&frames));
        assert_eq!(session.stats().rollbacks, 1);
        assert_eq!(session.stats().max_rollback_depth, 3);
    }

    #[test]
    fn prediction_is_per_peer() {
        let mut core = FakeCore::default();
        let mut session = RollbackSession::with_config(SessionConfig {
            buffer_frames: 4,
            players: 3,
            local_player: 0,
            prediction: PredictionMode::LastInput,
            ..SessionConfig::default()
        });
        session.receive_remote(&mut core, 1, 0, 7).expect("p2");
        session.receive_remote(&mut core, 2, 0, 9).expect("p3");
        session.step(&mut core, [1, 0, 0, 0]).expect("f0");
        session.step(&mut core, [2, 0, 0, 0]).expect("f1");
        // Frame 1 predicts each missing port from that same port's history.
        assert_eq!(core.state(), expected(&[[1, 7, 9, 0], [2, 7, 9, 0]]));
        assert_eq!(session.stats().predicted, 2);
    }
}
