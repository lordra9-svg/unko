//! GGRS-backed rollback session.
//!
//! This wraps [`ggrs::P2PSession`] for the NES core's one-port-per-player
//! model. GGRS owns input exchange, prediction, rollback scheduling, time
//! synchronization and checksum desync detection; this module only turns its
//! requests into `Core` save/load/advance calls.
//!
//! Direct UDP, UDP hole punching and the relay transport are supported through
//! [`ggrs::NonBlockingSocket`] adapters. The relay adapter uses explicit member
//! ids assigned by the client (the player slot) rather than public addresses.

use std::collections::HashMap;
use std::net::SocketAddr;

use ggrs::{
    Config, DesyncDetection, Frame, GgrsError, GgrsEvent, GgrsRequest, InputStatus, Message,
    NonBlockingSocket, P2PSession, PlayerHandle, PlayerType, SessionBuilder, SessionState,
    UdpNonBlockingSocket,
};
use unko_core::{Core, InputState};

use crate::holepunch::HolePunchTransport;
use crate::relay::RelayTransport;
use crate::session::{PredictionMode, SessionConfig, SessionStats};
use crate::sync::DesyncInfo;

/// Endpoint identity for a GGRS peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GgrsAddress {
    /// Direct UDP or hole-punched peer address.
    Direct(SocketAddr),
    /// Relay room member id (the player slot).
    Relay(u16),
}

impl std::fmt::Display for GgrsAddress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct(address) => write!(formatter, "{address}"),
            Self::Relay(member) => write!(formatter, "relay#{member}"),
        }
    }
}

/// GGRS game configuration for the NES core.
pub struct GgrsConfig;

impl Config for GgrsConfig {
    /// One controller's button bits for one player.
    type Input = u16;
    /// GGRS predicts the last input by default.
    type InputPredictor = ggrs::PredictRepeatLast;
    /// A serialized `Core` save state.
    type State = Vec<u8>;
    /// Direct address or relay member id.
    type Address = GgrsAddress;
}

/// A running GGRS peer-to-peer session.
pub struct GgrsNetplay {
    session: P2PSession<GgrsConfig>,
    local_handle: PlayerHandle,
    remote_handles: HashMap<GgrsAddress, PlayerHandle>,
    config: SessionConfig,
    stats: SessionStats,
    running: bool,
    desyncs: u64,
    last_desync: Option<DesyncInfo>,
    last_error: Option<String>,
}

impl GgrsNetplay {
    /// Starts a direct-UDP session on `bind_port`.
    ///
    /// `peers` holds one `ip:port` per remote player, in player-handle order
    /// with the local player's own slot omitted. Both peers must build the
    /// same player list; the local slot comes from `config.local_player`.
    pub fn start(bind_port: u16, peers: &[String], config: SessionConfig) -> Result<Self, String> {
        Self::start_with_fps(bind_port, peers, config, 60)
    }

    /// Starts a direct-UDP session with an explicit frame rate.
    pub fn start_with_fps(
        bind_port: u16,
        peers: &[String],
        config: SessionConfig,
        fps: u32,
    ) -> Result<Self, String> {
        Self::start_lockstep_mode(bind_port, peers, config, false, fps)
    }

    /// Starts a lockstep session (no prediction) for deterministic validation.
    pub fn start_lockstep(
        bind_port: u16,
        peers: &[String],
        config: SessionConfig,
    ) -> Result<Self, String> {
        Self::start_lockstep_mode(bind_port, peers, config, true, 60)
    }

    /// Starts GGRS over a hole-punched transport.
    pub fn start_punch(
        transport: HolePunchTransport,
        peers: &[String],
        config: SessionConfig,
    ) -> Result<Self, String> {
        Self::start_punch_mode(transport, peers, config, false, 60)
    }

    /// Starts GGRS over a hole-punched transport with an explicit frame rate.
    pub fn start_punch_with_fps(
        transport: HolePunchTransport,
        peers: &[String],
        config: SessionConfig,
        fps: u32,
    ) -> Result<Self, String> {
        Self::start_punch_mode(transport, peers, config, false, fps)
    }

    /// Lockstep hole-punch session for deterministic validation.
    pub fn start_punch_lockstep(
        transport: HolePunchTransport,
        peers: &[String],
        config: SessionConfig,
    ) -> Result<Self, String> {
        Self::start_punch_mode(transport, peers, config, true, 60)
    }

    fn start_punch_mode(
        transport: HolePunchTransport,
        peers: &[String],
        config: SessionConfig,
        lockstep: bool,
        fps: u32,
    ) -> Result<Self, String> {
        let addresses = parse_direct_peers(peers, config)?;
        Self::start_with_socket(PunchSocket { transport }, addresses, config, lockstep, fps)
    }

    /// Starts GGRS over a relay room. Member ids are the player slots; the
    /// transport must have joined with `config.local_player`.
    pub fn start_relay(transport: RelayTransport, config: SessionConfig) -> Result<Self, String> {
        Self::start_relay_mode(transport, config, false, 60)
    }

    /// Starts GGRS over a relay room with an explicit frame rate.
    pub fn start_relay_with_fps(
        transport: RelayTransport,
        config: SessionConfig,
        fps: u32,
    ) -> Result<Self, String> {
        Self::start_relay_mode(transport, config, false, fps)
    }

    /// Lockstep relay session for deterministic validation.
    pub fn start_relay_lockstep(
        transport: RelayTransport,
        config: SessionConfig,
    ) -> Result<Self, String> {
        Self::start_relay_mode(transport, config, true, 60)
    }

    fn start_relay_mode(
        transport: RelayTransport,
        config: SessionConfig,
        lockstep: bool,
        fps: u32,
    ) -> Result<Self, String> {
        let config = config.normalized();
        let players = usize::from(config.players);
        let local = usize::from(config.local_player);
        let addresses: Vec<GgrsAddress> = (0..players)
            .filter(|handle| *handle != local)
            .map(|handle| GgrsAddress::Relay(handle as u16))
            .collect();
        Self::start_with_socket(RelaySocket { transport }, addresses, config, lockstep, fps)
    }

    fn start_lockstep_mode(
        bind_port: u16,
        peers: &[String],
        config: SessionConfig,
        lockstep: bool,
        fps: u32,
    ) -> Result<Self, String> {
        if bind_port == 0 {
            return Err("bind port must be non-zero".into());
        }
        let addresses = parse_direct_peers(peers, config)?;
        let socket = UdpNonBlockingSocket::bind_to_port(bind_port)
            .map_err(|err| format!("bind UDP {bind_port}: {err}"))?;
        Self::start_with_socket(
            DirectSocket { inner: socket },
            addresses,
            config,
            lockstep,
            fps,
        )
    }

    fn start_with_socket(
        socket: impl NonBlockingSocket<GgrsAddress> + 'static,
        addresses: Vec<GgrsAddress>,
        config: SessionConfig,
        lockstep: bool,
        fps: u32,
    ) -> Result<Self, String> {
        let mut config = config.normalized();
        // GGRS has one compile-time predictor; the NES adapter uses
        // repeat-last. Direct sessions therefore ignore `PredictionMode`.
        config.prediction = PredictionMode::LastInput;
        let players = usize::from(config.players);
        if addresses.len() != players.saturating_sub(1) {
            return Err(format!(
                "{} players need exactly {} peer address(es), found {}",
                players,
                players.saturating_sub(1),
                addresses.len()
            ));
        }

        let mut builder = SessionBuilder::<GgrsConfig>::new()
            .with_num_players(players)
            .map_err(|err| err.to_string())?
            .with_fps(fps.max(1) as usize)
            .map_err(|err| err.to_string())?
            .with_input_delay(config.input_delay as usize)
            .with_max_prediction_window(if lockstep { 0 } else { config.buffer_frames })
            .with_desync_detection_mode(if config.hash_states {
                DesyncDetection::On { interval: 30 }
            } else {
                DesyncDetection::Off
            });
        let local_handle = usize::from(config.local_player);
        let mut remote_handles = HashMap::new();
        let mut peer_iter = addresses.into_iter();
        for handle in 0..players {
            if handle == local_handle {
                builder = builder
                    .add_player(PlayerType::Local, handle)
                    .map_err(|err| err.to_string())?;
            } else {
                let address = peer_iter
                    .next()
                    .ok_or("missing remote address for player handle")?;
                remote_handles.insert(address, handle);
                builder = builder
                    .add_player(PlayerType::Remote(address), handle)
                    .map_err(|err| err.to_string())?;
            }
        }
        let session = builder
            .start_p2p_session(socket)
            .map_err(|err| err.to_string())?;
        Ok(Self {
            session,
            local_handle,
            remote_handles,
            config,
            stats: SessionStats::default(),
            running: false,
            desyncs: 0,
            last_desync: None,
            last_error: None,
        })
    }

    /// Whether the GGRS handshake has completed.
    #[must_use]
    pub const fn running(&self) -> bool {
        self.running
    }

    /// Current tuning.
    #[must_use]
    pub const fn config(&self) -> SessionConfig {
        self.config
    }

    /// Changes the local input delay (clamped to 0..=16).
    pub fn set_input_delay(&mut self, delay: u32) {
        let delay = delay.min(16);
        if self
            .session
            .set_input_delay(self.local_handle, delay as usize)
            .is_ok()
        {
            self.config.input_delay = delay;
        }
    }

    /// Highest frame whose inputs are fully confirmed.
    #[must_use]
    pub fn confirmed_frame(&self) -> u32 {
        self.session.confirmed_frame().max(0) as u32
    }

    /// GGRS session counters mapped onto the shared stats shape.
    #[must_use]
    pub const fn stats(&self) -> SessionStats {
        self.stats
    }

    /// `(desyncs, last desync)`.
    #[must_use]
    pub const fn desync_stats(&self) -> (u64, Option<DesyncInfo>) {
        (self.desyncs, self.last_desync)
    }

    /// Takes the most recent desync report.
    pub fn take_desync(&mut self) -> Option<DesyncInfo> {
        self.last_desync.take()
    }

    /// Last transport or protocol error.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// `(ping_ms, frames_behind)` for the first remote player, when GGRS has
    /// enough data.
    #[must_use]
    pub fn network_stats(&self) -> Option<(u128, i32)> {
        let handle = *self.remote_handles.values().next()?;
        self.session
            .network_stats(handle)
            .ok()
            .map(|stats| (stats.ping, stats.local_frames_behind))
    }

    /// Polls the socket and drains GGRS events without advancing the frame.
    pub fn poll(&mut self) {
        self.session.poll_remote_clients();
        for event in self.session.events() {
            match event {
                GgrsEvent::Synchronized { .. } => self.running = true,
                GgrsEvent::Disconnected { addr } => {
                    self.last_error = Some(format!("peer {addr} disconnected"));
                }
                GgrsEvent::NetworkInterrupted {
                    addr,
                    disconnect_timeout,
                } => {
                    self.last_error = Some(format!(
                        "peer {addr} interrupted; disconnecting in {disconnect_timeout} ms"
                    ));
                }
                GgrsEvent::NetworkResumed { addr } => {
                    self.last_error = None;
                    let _ = addr;
                }
                GgrsEvent::DesyncDetected {
                    frame,
                    local_checksum,
                    remote_checksum,
                    addr,
                } => {
                    self.desyncs += 1;
                    self.last_desync = Some(DesyncInfo {
                        frame: frame.max(0) as u32,
                        player: self.remote_handles.get(&addr).copied().unwrap_or_default() as u8,
                        local: local_checksum as u64,
                        remote: remote_checksum as u64,
                    });
                }
                GgrsEvent::Synchronizing { .. } | GgrsEvent::WaitRecommendation { .. } => {}
            }
        }
        if self.session.current_state() == SessionState::Running {
            self.running = true;
        }
    }

    /// Adds this peer's local input and advances or resimulates the core.
    ///
    /// Returns `false` when GGRS is still synchronizing or its prediction
    /// window is full, in which case the caller should keep the last frame.
    pub fn step(&mut self, core: &mut dyn Core, local_input: u16) -> Result<bool, String> {
        self.poll();
        if !self.running || self.session.current_state() != SessionState::Running {
            return Ok(false);
        }
        self.session
            .add_local_input(self.local_handle, local_input)
            .map_err(|err| err.to_string())?;
        let before = self.session.current_frame();
        let requests = match self.session.advance_frame() {
            Ok(requests) => requests,
            Err(GgrsError::PredictionThreshold | GgrsError::NotSynchronized) => {
                return Ok(false);
            }
            Err(err) => return Err(err.to_string()),
        };
        let current = self.session.current_frame().max(0) as u32;
        for request in requests {
            match request {
                GgrsRequest::SaveGameState { cell, frame } => {
                    let mut state = Vec::new();
                    core.save_state(&mut state).map_err(|err| err.to_string())?;
                    let checksum = checksum(&state);
                    cell.save(frame, Some(state), Some(checksum));
                }
                GgrsRequest::LoadGameState { cell, frame } => {
                    let state = cell.load().ok_or_else(|| {
                        format!("GGRS asked for a missing state at frame {frame}")
                    })?;
                    core.load_state(&state).map_err(|err| err.to_string())?;
                    self.stats.rollbacks = self.stats.rollbacks.saturating_add(1);
                    let depth = current.saturating_sub(frame.max(0) as u32);
                    self.stats.max_rollback_depth = self.stats.max_rollback_depth.max(depth);
                }
                GgrsRequest::AdvanceFrame { inputs } => {
                    for (port, (input, status)) in inputs.iter().enumerate() {
                        core.set_input(port, InputState::from_bits(*input));
                        if *status == InputStatus::Predicted {
                            self.stats.predicted = self.stats.predicted.saturating_add(1);
                        }
                    }
                    for port in inputs.len()..4 {
                        core.set_input(port, InputState::empty());
                    }
                    core.run_frame().map_err(|err| err.to_string())?;
                    self.stats.frame = current;
                }
            }
        }
        self.stats.frame = self.session.current_frame().max(0) as u32;
        // A rollback-only pass resimulates earlier frames without advancing the
        // session frame; the caller must not count it as a new display frame.
        Ok(self.session.current_frame() > before)
    }
}

fn parse_direct_peers(peers: &[String], config: SessionConfig) -> Result<Vec<GgrsAddress>, String> {
    let players = usize::from(config.normalized().players);
    if peers.len() != players.saturating_sub(1) {
        return Err(format!(
            "{} players need exactly {} peer address(es), found {}",
            players,
            players.saturating_sub(1),
            peers.len()
        ));
    }
    let mut addresses = Vec::with_capacity(peers.len());
    for peer in peers {
        let address: SocketAddr = peer.parse().map_err(|err| format!("{peer}: {err}"))?;
        if address.is_ipv6() {
            return Err(format!("{address}: GGRS direct UDP supports IPv4 only"));
        }
        addresses.push(GgrsAddress::Direct(address));
    }
    Ok(addresses)
}

/// GGRS socket over the built-in non-blocking UDP socket.
struct DirectSocket {
    inner: UdpNonBlockingSocket,
}

impl NonBlockingSocket<GgrsAddress> for DirectSocket {
    fn send_to(&mut self, msg: &Message, addr: &GgrsAddress) {
        if let GgrsAddress::Direct(address) = addr {
            self.inner.send_to(msg, address);
        }
    }

    fn receive_all_messages(&mut self) -> Vec<(GgrsAddress, Message)> {
        let mut messages: Vec<_> = self
            .inner
            .receive_all_messages()
            .into_iter()
            .map(|(address, message)| (GgrsAddress::Direct(address), message))
            .collect();
        messages.truncate(MAX_POLL_MESSAGES);
        messages
    }
}

/// GGRS socket over a hole-punched UDP transport.
struct PunchSocket {
    transport: HolePunchTransport,
}

impl NonBlockingSocket<GgrsAddress> for PunchSocket {
    fn send_to(&mut self, msg: &Message, addr: &GgrsAddress) {
        if let GgrsAddress::Direct(address) = addr {
            if let Ok(bytes) = bincode::serialize(msg) {
                let _ = self.transport.send_to(*address, &bytes);
            }
        }
    }

    fn receive_all_messages(&mut self) -> Vec<(GgrsAddress, Message)> {
        let mut messages = Vec::new();
        while messages.len() < MAX_POLL_MESSAGES {
            let Ok(Some((address, bytes))) = self.transport.recv_from() else {
                break;
            };
            if let Ok(message) = bincode::deserialize(&bytes) {
                messages.push((GgrsAddress::Direct(address), message));
            }
        }
        messages
    }
}

/// GGRS socket over the relay room.
struct RelaySocket {
    transport: RelayTransport,
}

impl NonBlockingSocket<GgrsAddress> for RelaySocket {
    fn send_to(&mut self, msg: &Message, addr: &GgrsAddress) {
        if let GgrsAddress::Relay(member) = addr {
            if let Ok(bytes) = bincode::serialize(msg) {
                let _ = self.transport.send_to(*member, &bytes);
            }
        }
    }

    fn receive_all_messages(&mut self) -> Vec<(GgrsAddress, Message)> {
        let mut messages = Vec::new();
        while messages.len() < MAX_POLL_MESSAGES {
            let Ok(Some((member, bytes))) = self.transport.recv_from() else {
                break;
            };
            if let Ok(message) = bincode::deserialize(&bytes) {
                messages.push((GgrsAddress::Relay(member), message));
            }
        }
        messages
    }
}

/// Maximum datagrams consumed from one socket poll.
const MAX_POLL_MESSAGES: usize = 256;

/// Stable 128-bit FNV-1a checksum for GGRS desync detection.
fn checksum(bytes: &[u8]) -> u128 {
    let mut hash: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    hash
}

/// GGRS frame number is an `i32`; expose the mapping for tests.
#[allow(dead_code)]
fn frame_number(frame: Frame) -> u32 {
    frame.max(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::RelayServer;
    use std::time::{Duration, Instant};
    use unko_core::{AudioBuffer, CoreError, FrameBuffer, FrameStats, LoadOptions, PixelFormat};

    #[derive(Default)]
    struct TestCore {
        inputs: [u16; 4],
        frame: u32,
        hash: u64,
    }

    impl TestCore {
        fn state(&self) -> Vec<u8> {
            let mut out = Vec::with_capacity(12);
            out.extend_from_slice(&self.frame.to_le_bytes());
            out.extend_from_slice(&self.hash.to_le_bytes());
            out
        }
    }

    impl Core for TestCore {
        fn info(&self) -> unko_core::CoreInfo {
            unko_core::CoreInfo {
                id: "ggrs-test",
                name: "GGRS test core",
                version: "0",
                system: unko_core::SystemId::Nes,
                accuracy: unko_core::Accuracy::Compatible,
                features: unko_core::Features::empty(),
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

        fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
            out.extend_from_slice(&self.state());
            Ok(())
        }

        fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
            if state.len() != 12 {
                return Err(CoreError::State("bad test state".into()));
            }
            self.frame = u32::from_le_bytes(state[0..4].try_into().unwrap());
            self.hash = u64::from_le_bytes(state[4..12].try_into().unwrap());
            Ok(())
        }
    }

    #[test]
    fn two_ggrs_sessions_converge_on_the_same_state() {
        // Distinct from GGRS's own test ports (7700-7731).
        let host_port = 7741;
        let join_port = 7742;
        let config = SessionConfig {
            players: 2,
            local_player: 0,
            input_delay: 0,
            buffer_frames: 8,
            prediction: PredictionMode::LastInput,
            hash_states: true,
        };
        let mut host = GgrsNetplay::start(host_port, &[format!("127.0.0.1:{join_port}")], config)
            .expect("host session");
        let mut join = GgrsNetplay::start(
            join_port,
            &[format!("127.0.0.1:{host_port}")],
            SessionConfig {
                local_player: 1,
                ..config
            },
        )
        .expect("join session");

        let mut core_a = TestCore::default();
        let mut core_b = TestCore::default();
        for _ in 0..4000 {
            let _ = host.step(&mut core_a, 0x01);
            let _ = join.step(&mut core_b, 0x02);
            if host.running() && join.running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(host.running() && join.running(), "GGRS handshake");

        let mut a_frames = 0;
        let mut b_frames = 0;
        for _ in 0..6000 {
            if a_frames < 90 && host.step(&mut core_a, 0x01).unwrap_or(false) {
                a_frames += 1;
            }
            if b_frames < 90 && join.step(&mut core_b, 0x02).unwrap_or(false) {
                b_frames += 1;
            }
            if a_frames >= 90 && b_frames >= 90 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(a_frames >= 90 && b_frames >= 90, "both peers advanced");
        assert_eq!(core_a.state(), core_b.state());
        assert_eq!(core_a.frame, core_b.frame);
    }

    #[test]
    fn lockstep_sessions_reach_the_same_state() {
        let host_port = 7744;
        let join_port = 7745;
        let config = SessionConfig {
            players: 2,
            local_player: 0,
            input_delay: 0,
            buffer_frames: 8,
            prediction: PredictionMode::LastInput,
            hash_states: false,
        };
        let mut host =
            GgrsNetplay::start_lockstep(host_port, &[format!("127.0.0.1:{join_port}")], config)
                .expect("host session");
        let mut join = GgrsNetplay::start_lockstep(
            join_port,
            &[format!("127.0.0.1:{host_port}")],
            SessionConfig {
                local_player: 1,
                ..config
            },
        )
        .expect("join session");

        let mut core_a = TestCore::default();
        let mut core_b = TestCore::default();
        let mut a_frames = 0;
        let mut b_frames = 0;
        for _ in 0..8000 {
            if a_frames < 40 && host.step(&mut core_a, 0x01).unwrap_or(false) {
                a_frames += 1;
            }
            if b_frames < 40 && join.step(&mut core_b, 0x02).unwrap_or(false) {
                b_frames += 1;
            }
            if a_frames >= 40 && b_frames >= 40 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(a_frames >= 40 && b_frames >= 40, "both peers advanced");
        assert_eq!(core_a.state(), core_b.state());
    }

    fn test_config(local_player: u8) -> SessionConfig {
        SessionConfig {
            players: 2,
            local_player,
            input_delay: 0,
            buffer_frames: 8,
            prediction: PredictionMode::LastInput,
            hash_states: false,
        }
    }

    fn run_lockstep_pair(
        mut host: GgrsNetplay,
        mut join: GgrsNetplay,
        frames: u32,
    ) -> (TestCore, TestCore) {
        let mut core_a = TestCore::default();
        let mut core_b = TestCore::default();
        let mut a_frames = 0;
        let mut b_frames = 0;
        for _ in 0..8000 {
            // Keep both peers polling every iteration, even after one has
            // reached the target. Stopping a peer early leaves its last queued
            // input unsent while the other is still catching up, so the two
            // cores can apply different inputs for the same frame.
            if host.step(&mut core_a, 0x01).unwrap_or(false) {
                a_frames += 1;
            }
            if join.step(&mut core_b, 0x02).unwrap_or(false) {
                b_frames += 1;
            }
            if a_frames >= frames && b_frames >= frames {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            a_frames >= frames && b_frames >= frames,
            "both peers advanced"
        );

        // Reaching the target frame only means each peer reported it locally. A
        // final input packet may still be in flight, so pump both sessions until
        // the states agree. Without this the returned states would depend on how
        // promptly the transport delivered the last packets, which is flaky on
        // loaded CI runners.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state_a = core_a.state();
            let state_b = core_b.state();
            if state_a == state_b {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "lockstep peers did not settle after {frames} frames: \
                 host reached frame {}, join reached frame {}, \
                 host state {}, join state {}",
                core_a.frame,
                core_b.frame,
                hex_state(&state_a),
                hex_state(&state_b),
            );
            // Keep polling both peers; a stalled lockstep advance does not
            // change the core state, so this is safe even past the target frame.
            let _ = host.step(&mut core_a, 0x01).unwrap_or(false);
            let _ = join.step(&mut core_b, 0x02).unwrap_or(false);
            std::thread::sleep(Duration::from_millis(1));
        }
        (core_a, core_b)
    }

    fn hex_state(state: &[u8]) -> String {
        state.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn relay_ggrs_sessions_reach_the_same_state() {
        let relay = RelayServer::spawn("127.0.0.1:0").expect("relay");
        let endpoint = relay.local_addr().to_string();
        let mut a =
            RelayTransport::bind("127.0.0.1:0", &endpoint, "ggrs", "secret", 0).expect("relay a");
        let mut b =
            RelayTransport::bind("127.0.0.1:0", &endpoint, "ggrs", "secret", 1).expect("relay b");
        for _ in 0..100 {
            a.pump();
            b.pump();
            if a.joined() && b.joined() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(a.joined() && b.joined(), "relay joined");
        let host = GgrsNetplay::start_relay_lockstep(a, test_config(0)).expect("host");
        let join = GgrsNetplay::start_relay_lockstep(b, test_config(1)).expect("join");
        let (core_a, core_b) = run_lockstep_pair(host, join, 30);
        assert_eq!(core_a.state(), core_b.state());
    }

    #[test]
    fn punch_ggrs_sessions_reach_the_same_state() {
        let host_addr: SocketAddr = "127.0.0.1:7760".parse().unwrap();
        let join_addr: SocketAddr = "127.0.0.1:7761".parse().unwrap();
        let mut a = HolePunchTransport::bind("127.0.0.1:7760", &[join_addr]).expect("punch a");
        let mut b = HolePunchTransport::bind("127.0.0.1:7761", &[host_addr]).expect("punch b");
        for _ in 0..200 {
            a.punch();
            b.punch();
            if a.connected() && b.connected() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(a.connected() && b.connected(), "punch connected");
        let host = GgrsNetplay::start_punch_lockstep(a, &[join_addr.to_string()], test_config(0))
            .expect("host");
        let join = GgrsNetplay::start_punch_lockstep(b, &[host_addr.to_string()], test_config(1))
            .expect("join");
        let (core_a, core_b) = run_lockstep_pair(host, join, 30);
        assert_eq!(core_a.state(), core_b.state());
    }

    #[test]
    fn rejects_a_mismatched_peer_list() {
        let err = match GgrsNetplay::start(7743, &[], SessionConfig::default()) {
            Ok(_) => panic!("expected a peer-count error"),
            Err(err) => err,
        };
        assert!(err.contains("peer address"));
    }
}
