//! Multi-peer sessions exchanging inputs through channel meshes.

use unko_core::{
    Accuracy, AudioBuffer, Core, CoreError, CoreInfo, Features, FrameBuffer, FrameStats,
    InputState, LoadOptions, PixelFormat, SystemId,
};
use unko_netplay::holepunch::HolePunchTransport;
use unko_netplay::input::InputFrame;
use unko_netplay::relay::{RelayServer, RelayTransport};
use unko_netplay::session::{PredictionMode, RollbackSession, SessionConfig};
use unko_netplay::sync::{DesyncMonitor, StateAck, StateChunk, StateReassembler, StateSender};
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

#[test]
fn peers_converge_over_a_hole_punched_transport() {
    let mut a = HolePunchTransport::bind("127.0.0.1:0", &[]).expect("a");
    let a_addr = a.local_addr().expect("a addr");
    let mut b = HolePunchTransport::bind("127.0.0.1:0", &[a_addr]).expect("b");
    let b_addr = b.local_addr().expect("b addr");
    a.add_peer(b_addr);
    for _ in 0..10 {
        a.punch_now();
        b.punch_now();
        if a.connected() && b.connected() {
            break;
        }
    }
    assert!(a.connected() && b.connected(), "punched through");

    let config = SessionConfig {
        buffer_frames: 8,
        players: 2,
        ..SessionConfig::default()
    };
    let mut a_core = TestCore::default();
    let mut b_core = TestCore::default();
    let mut a_session = RollbackSession::with_config(config);
    let mut b_session = RollbackSession::with_config(SessionConfig {
        local_player: 1,
        ..config
    });
    let a_inputs = [10u16, 11, 12, 13];
    let b_inputs = [20u16, 21, 22, 23];

    for frame in 0..4usize {
        let a_frame = a_session
            .step(&mut a_core, [a_inputs[frame], 0, 0, 0])
            .expect("a step");
        a.send(
            &InputFrame {
                frame: a_frame,
                player: 0,
                input: a_inputs[frame],
            }
            .encode(),
        )
        .expect("a send");
        let b_frame = b_session
            .step(&mut b_core, [0, b_inputs[frame], 0, 0])
            .expect("b step");
        b.send(
            &InputFrame {
                frame: b_frame,
                player: 1,
                input: b_inputs[frame],
            }
            .encode(),
        )
        .expect("b send");

        while let Some(bytes) = a.recv().expect("a recv") {
            let message = InputFrame::decode(&bytes).expect("frame");
            a_session
                .receive_remote(&mut a_core, message.player, message.frame, message.input)
                .expect("a remote");
        }
        while let Some(bytes) = b.recv().expect("b recv") {
            let message = InputFrame::decode(&bytes).expect("frame");
            b_session
                .receive_remote(&mut b_core, message.player, message.frame, message.input)
                .expect("b remote");
        }
    }

    let mut expected = TestCore::default();
    let frames = [
        [10, 20, 0, 0],
        [11, 21, 0, 0],
        [12, 22, 0, 0],
        [13, 23, 0, 0],
    ];
    for frame in frames {
        expected.run(frame);
    }
    assert_eq!(a_core.state(), expected.state());
    assert_eq!(b_core.state(), expected.state());
}

#[test]
fn peers_converge_over_a_relay() {
    let mut server = RelayServer::spawn("127.0.0.1:0").expect("relay");
    let relay = server.local_addr().to_string();
    let mut a = RelayTransport::bind("127.0.0.1:0", &relay, "test").expect("a");
    let mut b = RelayTransport::bind("127.0.0.1:0", &relay, "test").expect("b");
    for _ in 0..100 {
        a.pump();
        b.pump();
        if a.joined() && b.joined() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(a.joined() && b.joined(), "relay join");

    let config = SessionConfig {
        buffer_frames: 8,
        players: 2,
        ..SessionConfig::default()
    };
    let mut a_core = TestCore::default();
    let mut b_core = TestCore::default();
    let mut a_session = RollbackSession::with_config(config);
    let mut b_session = RollbackSession::with_config(SessionConfig {
        local_player: 1,
        ..config
    });
    let a_inputs = [10u16, 11, 12, 13];
    let b_inputs = [20u16, 21, 22, 23];
    let mut received_a = 0usize;
    let mut received_b = 0usize;

    fn drain(
        transport: &mut RelayTransport,
        session: &mut RollbackSession,
        core: &mut TestCore,
        received: &mut usize,
    ) {
        while let Some(bytes) = transport.recv().expect("recv") {
            let message = InputFrame::decode(&bytes).expect("frame");
            session
                .receive_remote(core, message.player, message.frame, message.input)
                .expect("remote");
            *received += 1;
        }
    }

    for frame in 0..4usize {
        let a_frame = a_session
            .step(&mut a_core, [a_inputs[frame], 0, 0, 0])
            .expect("a step");
        a.send(
            &InputFrame {
                frame: a_frame,
                player: 0,
                input: a_inputs[frame],
            }
            .encode(),
        )
        .expect("a send");
        let b_frame = b_session
            .step(&mut b_core, [0, b_inputs[frame], 0, 0])
            .expect("b step");
        b.send(
            &InputFrame {
                frame: b_frame,
                player: 1,
                input: b_inputs[frame],
            }
            .encode(),
        )
        .expect("b send");
        drain(&mut a, &mut a_session, &mut a_core, &mut received_a);
        drain(&mut b, &mut b_session, &mut b_core, &mut received_b);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // The relay hop can deliver after the frame loop; wait for every input and
    // let the resulting rollbacks settle before hashing the state.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while received_a < 4 || received_b < 4 {
        if std::time::Instant::now() >= deadline {
            break;
        }
        drain(&mut a, &mut a_session, &mut a_core, &mut received_a);
        drain(&mut b, &mut b_session, &mut b_core, &mut received_b);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(
        received_a >= 4 && received_b >= 4,
        "all relayed inputs arrived"
    );

    let mut expected = TestCore::default();
    for frame in [
        [10, 20, 0, 0],
        [11, 21, 0, 0],
        [12, 22, 0, 0],
        [13, 23, 0, 0],
    ] {
        expected.run(frame);
    }
    assert_eq!(a_core.state(), expected.state());
    assert_eq!(b_core.state(), expected.state());
    server.shutdown();
}

#[test]
fn peers_repair_a_desync_with_a_state_transfer() {
    let (mut a_link, mut b_link) = channel_pair();
    let config = SessionConfig {
        buffer_frames: 4,
        players: 2,
        hash_states: true,
        ..SessionConfig::default()
    };
    let mut a_core = TestCore::default();
    let mut b_core = TestCore::default();
    let mut a = RollbackSession::with_config(config);
    let mut b = RollbackSession::with_config(SessionConfig {
        local_player: 1,
        ..config
    });
    let mut a_monitor = DesyncMonitor::default();
    let mut b_monitor = DesyncMonitor::default();
    let mut a_last: Option<u32> = None;
    let mut b_last: Option<u32> = None;

    // Play 24 frames; B applies a wrong input at frame 12 only.
    for frame in 0..24u32 {
        let a_local = 10 + frame as u16;
        let b_local = 20 + frame as u16;
        let a_frame = a.step(&mut a_core, [a_local, 0, 0, 0]).expect("a");
        a_link
            .send(
                &InputFrame {
                    frame: a_frame,
                    player: 0,
                    input: a_local,
                }
                .encode(),
            )
            .expect("a send");
        let applied = if frame == 12 { b_local + 100 } else { b_local };
        let b_frame = b.step(&mut b_core, [0, applied, 0, 0]).expect("b");
        b_link
            .send(
                &InputFrame {
                    frame: b_frame,
                    player: 1,
                    input: b_local,
                }
                .encode(),
            )
            .expect("b send");

        // Exchange inputs and settled hashes.
        while let Some(bytes) = a_link.recv().expect("a recv") {
            if let Some(hash) = unko_netplay::StateHash::decode(&bytes) {
                a_monitor.record_remote(hash);
            } else if let Some(frame) = InputFrame::decode(&bytes) {
                a.receive_remote(&mut a_core, frame.player, frame.frame, frame.input)
                    .expect("a remote");
            }
        }
        while let Some(bytes) = b_link.recv().expect("b recv") {
            if let Some(hash) = unko_netplay::StateHash::decode(&bytes) {
                b_monitor.record_remote(hash);
            } else if let Some(frame) = InputFrame::decode(&bytes) {
                b.receive_remote(&mut b_core, frame.player, frame.frame, frame.input)
                    .expect("b remote");
            }
        }
        for (session, monitor, last, link) in [
            (&a, &mut a_monitor, &mut a_last, &mut a_link),
            (&b, &mut b_monitor, &mut b_last, &mut b_link),
        ] {
            let player = session.config().local_player;
            for (number, hash) in session.settled_hashes(*last) {
                *last = Some(number);
                monitor.record_local(number, hash);
                link.send(
                    &unko_netplay::StateHash {
                        frame: number,
                        player,
                        hash,
                    }
                    .encode(),
                )
                .expect("hash send");
            }
        }
        // Drain the hashes just sent so both monitors compare them.
        while let Some(bytes) = a_link.recv().expect("a recv") {
            if let Some(hash) = unko_netplay::StateHash::decode(&bytes) {
                a_monitor.record_remote(hash);
            }
        }
        while let Some(bytes) = b_link.recv().expect("b recv") {
            if let Some(hash) = unko_netplay::StateHash::decode(&bytes) {
                b_monitor.record_remote(hash);
            }
        }
    }
    assert!(
        a_monitor.mismatches() > 0 || b_monitor.mismatches() > 0,
        "the injected input must be detected"
    );
    assert_ne!(a_core.state(), b_core.state(), "timelines diverged");

    // The authority streams its snapshot in chunks; the peer reassembles it,
    // loads it, resyncs its session and acknowledges.
    let resume_frame = a.frame();
    let mut state = Vec::new();
    a_core.save_state(&mut state).expect("save");
    let sender = StateSender::new(resume_frame, &state);
    let mut reassembler = StateReassembler::default();
    let mut chunks = Vec::new();
    let mut sender = sender;
    while let Some(chunk) = sender.next_chunk() {
        chunks.push(chunk.to_vec());
    }
    for chunk in &chunks {
        a_link.send(chunk).expect("chunk send");
    }
    let mut ack = None;
    while ack.is_none() {
        while let Some(bytes) = b_link.recv().expect("b recv") {
            if let Some(chunk) = StateChunk::decode(&bytes) {
                if let Some(state) = reassembler.accept(chunk).expect("accept") {
                    b_core.load_state(&state).expect("load");
                    b.resync(resume_frame);
                    b_monitor.reset();
                    ack = Some(StateAck {
                        frame: resume_frame,
                        checksum: sender.checksum(),
                        player: 1,
                    });
                }
            }
        }
    }
    let ack = ack.expect("ack");
    b_link.send(&ack.encode()).expect("ack send");
    assert!(sender.acknowledged(ack));
    a.resync(resume_frame);
    a_monitor.reset();
    assert_eq!(a_core.state(), b_core.state(), "snapshot adopted");

    // Continue from the shared frame; both must converge again.
    for frame in resume_frame..resume_frame + 12 {
        let a_local = 10 + frame as u16;
        let b_local = 20 + frame as u16;
        let a_frame = a.step(&mut a_core, [a_local, 0, 0, 0]).expect("a");
        a_link
            .send(
                &InputFrame {
                    frame: a_frame,
                    player: 0,
                    input: a_local,
                }
                .encode(),
            )
            .expect("a send");
        let b_frame = b.step(&mut b_core, [0, b_local, 0, 0]).expect("b");
        b_link
            .send(
                &InputFrame {
                    frame: b_frame,
                    player: 1,
                    input: b_local,
                }
                .encode(),
            )
            .expect("b send");
        while let Some(bytes) = a_link.recv().expect("a recv") {
            if let Some(frame) = InputFrame::decode(&bytes) {
                a.receive_remote(&mut a_core, frame.player, frame.frame, frame.input)
                    .expect("a remote");
            }
        }
        while let Some(bytes) = b_link.recv().expect("b recv") {
            if let Some(frame) = InputFrame::decode(&bytes) {
                b.receive_remote(&mut b_core, frame.player, frame.frame, frame.input)
                    .expect("b remote");
            }
        }
    }
    assert_eq!(
        a_core.state(),
        b_core.state(),
        "peers converge after the transfer"
    );
}
