//! Multi-peer sessions exchanging inputs through channel meshes.

use unko_core::{
    Accuracy, AudioBuffer, Core, CoreError, CoreInfo, Features, FrameBuffer, FrameStats,
    InputState, LoadOptions, PixelFormat, SystemId, VideoFilter,
};
use unko_netplay::input::InputFrame;
use unko_netplay::session::{PredictionMode, RollbackSession, SessionConfig};
use unko_netplay::transport::{channel_pair, ChannelTransport, MeshTransport};
use unko_netplay::Transport;

#[derive(Default)]
struct TestCore {
    frame: u32,
    hash: u64,
    inputs: [u16; 4],
}

impl TestCore {
    fn state(&self) -> Vec<u8> {
        let mut out = self.frame.to_le_bytes().to_vec();
        out.extend_from_slice(&self.hash.to_le_bytes());
        out
    }

    fn run(&mut self, frame: [u16; 4]) {
        self.inputs = frame;
        self.run_frame().unwrap();
    }
}

impl Core for TestCore {
    fn info(&self) -> CoreInfo {
        CoreInfo {
            id: "test",
            name: "Test",
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
        for input in self.inputs {
            self.hash = self.hash.wrapping_mul(31).wrapping_add(u64::from(input));
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
            return Err(CoreError::State("bad test state".into()));
        }
        self.frame = u32::from_le_bytes(state[0..4].try_into().unwrap());
        self.hash = u64::from_le_bytes(state[4..12].try_into().unwrap());
        Ok(())
    }
}

/// One peer in a full-mesh test.
struct Peer {
    session: RollbackSession,
    core: TestCore,
    out: MeshTransport,
    inbox: Vec<ChannelTransport>,
}

fn peers(count: usize, config: SessionConfig) -> Vec<Peer> {
    let mut outgoing: Vec<Vec<Box<dyn Transport>>> = (0..count).map(|_| Vec::new()).collect();
    let mut incoming: Vec<Vec<ChannelTransport>> = (0..count).map(|_| Vec::new()).collect();
    for (sender, out) in outgoing.iter_mut().enumerate() {
        for (receiver, inbox) in incoming.iter_mut().enumerate() {
            if sender == receiver {
                continue;
            }
            let (tx, rx) = channel_pair();
            out.push(Box::new(tx));
            inbox.push(rx);
        }
    }
    (0..count)
        .map(|index| Peer {
            session: RollbackSession::with_config(SessionConfig {
                local_player: index as u8,
                ..config
            }),
            core: TestCore::default(),
            out: MeshTransport::new(std::mem::take(&mut outgoing[index])),
            inbox: std::mem::take(&mut incoming[index]),
        })
        .collect()
}

/// Sends this peer's frame with its own port set.
fn send(session: &mut RollbackSession, core: &mut TestCore, out: &mut MeshTransport, local: u16) {
    let player = session.config().local_player;
    let mut ports = [0u16; 4];
    ports[usize::from(player)] = local;
    let frame = session.step(core, ports).expect("step");
    let message = InputFrame {
        frame,
        player,
        input: local,
    };
    out.send(&message.encode()).expect("send");
}

/// Drains every queued message from every peer's inboxes.
fn drain(all: &mut [Peer]) -> Vec<InputFrame> {
    let mut queued = Vec::new();
    for peer in all.iter_mut() {
        for inbox in &mut peer.inbox {
            while let Some(bytes) = inbox.recv().expect("recv") {
                queued.push(InputFrame::decode(&bytes).expect("frame"));
            }
        }
    }
    queued
}

/// Applies messages to every peer (each session ignores its own port and
/// out-of-range players).
fn apply(all: &mut [Peer], messages: &[InputFrame]) {
    for message in messages {
        for peer in all.iter_mut() {
            peer.session
                .receive_remote(&mut peer.core, message.player, message.frame, message.input)
                .expect("rollback");
        }
    }
}

/// Delivers every queued message.
fn deliver(all: &mut [Peer]) {
    let messages = drain(all);
    apply(all, &messages);
}

#[test]
fn peers_converge_after_a_late_input() {
    let config = SessionConfig {
        buffer_frames: 8,
        players: 2,
        ..SessionConfig::default()
    };
    let mut all = peers(2, config);
    let inputs = [[10u16, 11, 12, 13, 14, 15], [20u16, 21, 22, 23, 24, 25]];
    let mut held = None;

    for frame in 0..6usize {
        for (index, player) in inputs.iter().enumerate() {
            let local = player[frame];
            let peer = &mut all[index];
            send(&mut peer.session, &mut peer.core, &mut peer.out, local);
        }
        let messages = drain(&mut all);
        let mut apply_now = Vec::new();
        for message in messages {
            // Hold player 2's frame-2 input until the end.
            if frame == 2 && message.player == 1 && held.is_none() {
                held = Some(message);
            } else {
                apply_now.push(message);
            }
        }
        apply(&mut all, &apply_now);
    }
    let late = held.expect("held input");
    apply(&mut all, &[late]);

    let mut expected = TestCore::default();
    for frame in 0..6usize {
        expected.run([inputs[0][frame], inputs[1][frame], 0, 0]);
    }
    for peer in &all {
        assert_eq!(peer.core.state(), expected.state(), "peer converged");
    }
    assert!(all[0].session.stats().rollbacks > 0, "peer 1 rolled back");
}

#[test]
fn three_players_converge_with_mesh() {
    let config = SessionConfig {
        buffer_frames: 8,
        players: 3,
        ..SessionConfig::default()
    };
    let mut all = peers(3, config);
    let inputs = [
        [10u16, 11, 12, 13, 14, 15],
        [20u16, 21, 22, 23, 24, 25],
        [30u16, 31, 32, 33, 34, 35],
    ];
    for frame in 0..6usize {
        for (index, player) in inputs.iter().enumerate() {
            let local = player[frame];
            let peer = &mut all[index];
            send(&mut peer.session, &mut peer.core, &mut peer.out, local);
        }
        deliver(&mut all);
    }
    let mut expected = TestCore::default();
    for frame in 0..6usize {
        expected.run([inputs[0][frame], inputs[1][frame], inputs[2][frame], 0]);
    }
    for peer in &all {
        assert_eq!(peer.core.state(), expected.state(), "3-player convergence");
        // Delivery happens after the step, so each remote port arrives one
        // frame late and exercises rollback.
        assert!(peer.session.stats().rollbacks > 0);
    }
}

#[test]
fn four_players_converge_with_mesh_and_delay() {
    let config = SessionConfig {
        buffer_frames: 8,
        input_delay: 1,
        prediction: PredictionMode::LastInput,
        players: 4,
        ..SessionConfig::default()
    };
    let mut all = peers(4, config);
    let inputs = [
        [10u16, 11, 12, 13, 14, 15],
        [20u16, 21, 22, 23, 24, 25],
        [30u16, 31, 32, 33, 34, 35],
        [40u16, 41, 42, 43, 44, 45],
    ];
    for frame in 0..6usize {
        for (index, player) in inputs.iter().enumerate() {
            let local = player[frame];
            let peer = &mut all[index];
            send(&mut peer.session, &mut peer.core, &mut peer.out, local);
        }
        deliver(&mut all);
    }
    // Delay 1: simulation frame s uses input frame s-1 for every port.
    let mut expected = TestCore::default();
    for frame in 0..6usize {
        let mut merged = [0u16; 4];
        if frame > 0 {
            for (port, player) in inputs.iter().enumerate() {
                merged[port] = player[frame - 1];
            }
        }
        expected.run(merged);
    }
    for peer in &all {
        assert_eq!(peer.core.state(), expected.state(), "4-player convergence");
        assert_eq!(peer.session.stats().predicted, 0);
    }
}
