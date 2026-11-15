//! Two simulated peers exchanging inputs through the channel transport.

use unko_core::{
    Accuracy, AudioBuffer, Core, CoreError, CoreInfo, Features, FrameBuffer, FrameStats,
    InputState, LoadOptions, PixelFormat, SystemId, VideoFilter,
};
use unko_netplay::input::InputFrame;
use unko_netplay::session::RollbackSession;
use unko_netplay::transport::{channel_pair, ChannelTransport};
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

    fn run(&mut self, local: u16, remote: u16) {
        self.inputs = [local, remote, 0, 0];
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

fn deliver(
    transport: &mut ChannelTransport,
    session: &mut RollbackSession,
    core: &mut TestCore,
) -> usize {
    let mut count = 0;
    while let Some(bytes) = transport.recv().expect("recv") {
        let frame = InputFrame::decode(&bytes).expect("frame");
        if session
            .receive_remote(core, frame.frame, frame.ports)
            .expect("rollback")
        {
            count += 1;
        }
    }
    count
}

fn local_frame(frame: u32, bits: u16) -> Vec<u8> {
    InputFrame {
        frame,
        ports: [bits, 0, 0, 0],
    }
    .encode()
}

#[test]
fn peers_converge_after_a_late_input() {
    let (mut a_link, mut b_link) = channel_pair();
    let mut a_core = TestCore::default();
    let mut b_core = TestCore::default();
    let mut a = RollbackSession::new(8);
    let mut b = RollbackSession::new(8);
    let a_inputs = [10u16, 11, 12, 13, 14, 15];
    let b_inputs = [20u16, 21, 22, 23, 24, 25];
    let mut rolled = 0usize;

    for frame in 0..6u32 {
        let a_frame = a
            .step(&mut a_core, [a_inputs[frame as usize], 0, 0, 0])
            .unwrap();
        a_link
            .send(&local_frame(a_frame, a_inputs[frame as usize]))
            .unwrap();
        // Frame 2 is held back to exercise rollback.
        if frame != 2 {
            rolled += deliver(&mut b_link, &mut b, &mut b_core);
        }

        let b_frame = b
            .step(&mut b_core, [b_inputs[frame as usize], 0, 0, 0])
            .unwrap();
        b_link
            .send(&local_frame(b_frame, b_inputs[frame as usize]))
            .unwrap();
        rolled += deliver(&mut a_link, &mut a, &mut a_core);
    }

    // Deliver any still-queued input (including the held frame) and count the
    // rollbacks it triggers.
    while let Some(bytes) = b_link.recv().expect("recv") {
        let frame = InputFrame::decode(&bytes).expect("frame");
        if b.receive_remote(&mut b_core, frame.frame, frame.ports)
            .expect("rollback")
        {
            rolled += 1;
        }
    }
    assert!(rolled > 0, "at least one rollback expected");

    // Reference runs apply the true remote sequence with no prediction.
    let mut expected_b = TestCore::default();
    for (index, local) in b_inputs.iter().enumerate() {
        expected_b.run(*local, a_inputs[index]);
    }
    let mut expected_a = TestCore::default();
    for (index, local) in a_inputs.iter().enumerate() {
        expected_a.run(*local, b_inputs[index]);
    }
    assert_eq!(b_core.state(), expected_b.state(), "B converged");
    assert_eq!(a_core.state(), expected_a.state(), "A converged");
    assert_eq!(b.frame(), 6);
}
