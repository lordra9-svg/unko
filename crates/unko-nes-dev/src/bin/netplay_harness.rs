//! Headless two-host NetPlay harness with desync detection.
//!
//! Runs one side of a real UDP session on the Unko NES core, using a built-in
//! synthetic ROM whose RAM accumulates the two controller ports. Each side
//! periodically exchanges save-state hashes; `--inject-desync N` corrupts one
//! side's local input at frame N to prove detection. The host then streams its
//! serialized state in checksummed chunks, the join loads it and both continue
//! from that frame, proving in-session state-transfer recovery.
//!
//! Examples:
//!
//! ```text
//! netplay_harness --role host --bind 192.168.1.10:7901 \
//!     --peer 192.168.1.20:7902 --frames 240
//! netplay_harness --role join --bind 192.168.1.20:7902 \
//!     --peer 192.168.1.10:7901 --frames 240 --inject-desync 100
//! ```

use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use unko_core::{Core, InputState, LoadOptions};
use unko_nes_host::AccuracyCore;
use unko_netplay::holepunch::HolePunchTransport;
use unko_netplay::input::InputFrame;
use unko_netplay::relay::RelayTransport;
use unko_netplay::session::{PredictionMode, RollbackSession, SessionConfig};
use unko_netplay::sync::{
    DesyncInfo, DesyncMonitor, StateAck, StateChunk, StateHash, StateReassembler, StateSender,
};
use unko_netplay::udp::UdpMesh;
use unko_netplay::Transport;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const SYNC: &[u8] = b"UNKOHARN";
const READY: &[u8] = b"UNKORDY1";
const RESUMED: &[u8] = b"UNKORS01";

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Synthetic NROM: strobes the controller ports, accumulates port 1 in
/// `$00/$01` and port 2 in `$02/$03`, and counts loops in `$04`.
fn synthetic_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 16 + 0x8000 + 0x2000];
    rom[0..4].copy_from_slice(b"NES\x1a");
    rom[4] = 2;
    rom[5] = 1;
    let code = [
        0xA9, 0x01, 0x8D, 0x16, 0x40, 0xA9, 0x00, 0x8D, 0x16, 0x40, 0xAD, 0x16, 0x40, 0x29, 0x01,
        0x18, 0x65, 0x00, 0x85, 0x00, 0xA5, 0x01, 0x69, 0x00, 0x85, 0x01, 0xAD, 0x17, 0x40, 0x29,
        0x01, 0x18, 0x65, 0x02, 0x85, 0x02, 0xA5, 0x03, 0x69, 0x00, 0x85, 0x03, 0xE6, 0x04, 0x4C,
        0x00, 0x80,
    ];
    let prg = &mut rom[16..16 + 0x8000];
    prg[..code.len()].copy_from_slice(&code);
    prg[0x7FFA..0x7FFC].copy_from_slice(&[0x00, 0x80]);
    prg[0x7FFC..0x7FFE].copy_from_slice(&[0x00, 0x80]);
    prg[0x7FFE..0x8000].copy_from_slice(&[0x00, 0x80]);
    rom
}

fn host_input(frame: u32) -> u16 {
    match frame % 7 {
        0 => InputState::A,
        2 => InputState::A | InputState::B,
        4 => InputState::B,
        _ => 0,
    }
}

fn join_input(frame: u32) -> u16 {
    match frame % 5 {
        1 => InputState::A,
        3 => InputState::B | InputState::START,
        _ => 0,
    }
}

struct Args {
    role: u8,
    bind: String,
    peer: Option<String>,
    frames: u32,
    transport: String,
    relay: Option<String>,
    room: String,
    delay: u32,
    prediction: PredictionMode,
    hash_interval: u32,
    inject: Option<u32>,
    recover_frames: u32,
}

fn parse_args() -> Result<Args, String> {
    let mut role = None;
    let mut bind = String::new();
    let mut peer = None;
    let mut frames = 240;
    let mut transport = "mesh".to_string();
    let mut relay = None;
    let mut room = "unko".to_string();
    let mut delay = 0;
    let mut prediction = PredictionMode::LastInput;
    let mut hash_interval = 30;
    let mut inject = None;
    let mut recover_frames = 160;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--role" => {
                let value = args.next().ok_or("--role needs host|join")?;
                role = Some(match value.as_str() {
                    "host" => 0,
                    "join" => 1,
                    _ => return Err("--role needs host|join".into()),
                });
            }
            "--bind" => bind = args.next().ok_or("--bind needs an address")?,
            "--peer" => peer = Some(args.next().ok_or("--peer needs an address")?),
            "--frames" => {
                frames = args
                    .next()
                    .ok_or("--frames needs a number")?
                    .parse()
                    .map_err(|_| "--frames needs a number")?
            }
            "--transport" => transport = args.next().ok_or("--transport needs a value")?,
            "--relay" => relay = Some(args.next().ok_or("--relay needs an address")?),
            "--room" => room = args.next().ok_or("--room needs a name")?,
            "--delay" => {
                delay = args
                    .next()
                    .ok_or("--delay needs a number")?
                    .parse()
                    .map_err(|_| "--delay needs a number")?
            }
            "--prediction" => {
                prediction = match args
                    .next()
                    .ok_or("--prediction needs last|neutral")?
                    .as_str()
                {
                    "neutral" => PredictionMode::Neutral,
                    _ => PredictionMode::LastInput,
                }
            }
            "--hash-interval" => {
                let value: u32 = args
                    .next()
                    .ok_or("--hash-interval needs a number")?
                    .parse()
                    .map_err(|_| "--hash-interval needs a number")?;
                hash_interval = value.max(1);
            }
            "--inject-desync" => {
                inject = Some(
                    args.next()
                        .ok_or("--inject-desync needs a frame")?
                        .parse()
                        .map_err(|_| "--inject-desync needs a frame")?,
                )
            }
            "--recover-frames" => {
                recover_frames = args
                    .next()
                    .ok_or("--recover-frames needs a number")?
                    .parse()
                    .map_err(|_| "--recover-frames needs a number")?
            }
            "-h" | "--help" => {
                println!(
                    "usage: netplay_harness --role host|join --frames N [--bind ADDR] \
                     [--peer ADDR] [--transport mesh|punch|relay] [--relay ADDR] \
                     [--room NAME] [--delay N] [--prediction last|neutral] \
                     [--hash-interval N] [--inject-desync N] [--recover-frames N]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        role: role.ok_or("--role is required")?,
        bind,
        peer,
        frames,
        transport,
        relay,
        room,
        delay,
        prediction,
        hash_interval,
        inject,
        recover_frames,
    })
}

fn build_transport(args: &Args) -> Result<Box<dyn Transport>, String> {
    match args.transport.as_str() {
        "mesh" => {
            let peer = args.peer.as_ref().ok_or("mesh needs --peer")?;
            let mut mesh = UdpMesh::bind(&args.bind).map_err(|err| err.to_string())?;
            mesh.add_peer(peer).map_err(|err| err.to_string())?;
            Ok(Box::new(mesh))
        }
        "punch" => {
            let peer: SocketAddr = args
                .peer
                .as_ref()
                .ok_or("punch needs --peer")?
                .parse()
                .map_err(|err| format!("--peer: {err}"))?;
            let mut punch =
                HolePunchTransport::bind(&args.bind, &[peer]).map_err(|err| err.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while !punch.connected() {
                if Instant::now() >= deadline {
                    return Err("hole punch timed out".into());
                }
                punch.punch_now();
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(Box::new(punch))
        }
        "relay" => {
            let relay = args.relay.as_ref().ok_or("relay needs --relay")?;
            let mut transport = RelayTransport::bind(&args.bind, relay, &args.room)
                .map_err(|err| err.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while !transport.joined() {
                if Instant::now() >= deadline {
                    return Err("relay join timed out".into());
                }
                transport.pump();
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(Box::new(transport))
        }
        other => Err(format!("unknown transport {other}")),
    }
}

/// Waits for both sides to be bound, then synchronizes the frame loops.
fn handshake(
    transport: &mut dyn Transport,
    session: &mut RollbackSession,
    core: &mut AccuracyCore,
    seen: &mut HashSet<u32>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    for phase in [SYNC, READY] {
        let mut complete = false;
        while !complete {
            if Instant::now() >= deadline {
                return Err("peer handshake timed out".into());
            }
            transport.send(phase).map_err(|err| err.to_string())?;
            while let Some(bytes) = transport.recv().map_err(|err| err.to_string())? {
                if bytes == phase {
                    complete = true;
                } else if let Some(frame) = InputFrame::decode(&bytes) {
                    seen.insert(frame.frame);
                    session
                        .receive_remote(core, frame.player, frame.frame, frame.input)
                        .map_err(|err| err.to_string())?;
                }
            }
            if !complete {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    Ok(())
}

fn snapshot_hash(core: &mut AccuracyCore) -> Result<u64, String> {
    let mut state = Vec::new();
    core.save_state(&mut state).map_err(|err| err.to_string())?;
    Ok(fnv1a(&state))
}

#[derive(Default)]
struct SyncEvents {
    desync: Option<DesyncInfo>,
    state: Option<(Vec<u8>, u32, u64)>,
    ack: Option<StateAck>,
    resumed: bool,
}

/// Routes every transport message to its handler.
fn exchange(
    transport: &mut dyn Transport,
    session: &mut RollbackSession,
    core: &mut AccuracyCore,
    seen: &mut HashSet<u32>,
    monitor: &mut DesyncMonitor,
    reassembler: &mut StateReassembler,
) -> Result<SyncEvents, String> {
    let mut events = SyncEvents::default();
    while let Some(bytes) = transport.recv().map_err(|err| err.to_string())? {
        if bytes == RESUMED {
            events.resumed = true;
            continue;
        }
        if let Some(ack) = StateAck::decode(&bytes) {
            events.ack = Some(ack);
            continue;
        }
        if let Some(chunk) = StateChunk::decode(&bytes) {
            let (frame, checksum) = (chunk.frame, chunk.checksum);
            if let Some(state) = reassembler.accept(chunk)? {
                events.state = Some((state, frame, checksum));
            }
            continue;
        }
        if let Some(hash) = StateHash::decode(&bytes) {
            if let Some(info) = monitor.record_remote(hash) {
                events.desync = Some(info);
            }
            continue;
        }
        let Some(frame) = InputFrame::decode(&bytes) else {
            continue;
        };
        seen.insert(frame.frame);
        session
            .receive_remote(core, frame.player, frame.frame, frame.input)
            .map_err(|err| err.to_string())?;
    }
    Ok(events)
}

/// Steps one frame, sends its input, and exchanges any settled hashes.
#[allow(clippy::too_many_arguments)]
fn play_frame(
    args: &Args,
    transport: &mut dyn Transport,
    session: &mut RollbackSession,
    core: &mut AccuracyCore,
    monitor: &mut DesyncMonitor,
    last_hash: &mut Option<u32>,
    hashes: &mut u32,
    frame: u32,
    inject: bool,
) -> Result<(), String> {
    let local = if args.role == 0 {
        host_input(frame)
    } else {
        join_input(frame)
    };
    let applied = if inject { local ^ 1 } else { local };
    let mut ports = [0u16; 4];
    ports[usize::from(args.role)] = applied;
    let number = session.step(core, ports).map_err(|err| err.to_string())?;
    transport
        .send(
            &InputFrame {
                frame: number,
                player: args.role,
                input: local,
            }
            .encode(),
        )
        .map_err(|err| err.to_string())?;
    if frame.is_multiple_of(args.hash_interval) {
        for (number, hash) in session.settled_hashes(*last_hash) {
            *last_hash = Some(number);
            if let Some(info) = monitor.record_local(number, hash) {
                eprintln!("local mismatch at frame {number}: {info}");
            }
            transport
                .send(
                    &StateHash {
                        frame: number,
                        player: args.role,
                        hash,
                    }
                    .encode(),
                )
                .map_err(|err| err.to_string())?;
            *hashes += 1;
        }
    }
    Ok(())
}

fn reference_hash(rom: &[u8], frames: u32) -> Result<u64, String> {
    let mut core = AccuracyCore::new();
    core.load(rom, &LoadOptions::default())
        .map_err(|err| err.to_string())?;
    for frame in 0..frames {
        core.set_input(0, InputState::from_bits(host_input(frame)));
        core.set_input(1, InputState::from_bits(join_input(frame)));
        core.run_frame().map_err(|err| err.to_string())?;
    }
    snapshot_hash(&mut core)
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    let rom = synthetic_rom();
    let mut transport = build_transport(&args)?;
    let config = SessionConfig {
        buffer_frames: 128,
        input_delay: args.delay,
        prediction: args.prediction,
        players: 2,
        local_player: args.role,
        hash_states: true,
    };

    let mut core = AccuracyCore::new();
    core.load(&rom, &LoadOptions::default())
        .map_err(|err| err.to_string())?;
    let mut session = RollbackSession::with_config(config);
    let mut monitor = DesyncMonitor::default();
    let mut seen = HashSet::new();
    handshake(transport.as_mut(), &mut session, &mut core, &mut seen)?;

    let mut detected = None;
    let mut last_hash: Option<u32> = None;
    let mut hashes = 0u32;
    let mut reassembler = StateReassembler::default();
    let mut sender: Option<StateSender> = None;
    let mut frame = 0u32;

    // Phase 1: normal play, stopping as soon as a divergence is noticed.
    while frame < args.frames && detected.is_none() {
        let inject = args.inject == Some(frame) && args.role == 1;
        play_frame(
            &args,
            transport.as_mut(),
            &mut session,
            &mut core,
            &mut monitor,
            &mut last_hash,
            &mut hashes,
            frame,
            inject,
        )?;
        let events = exchange(
            transport.as_mut(),
            &mut session,
            &mut core,
            &mut seen,
            &mut monitor,
            &mut reassembler,
        )?;
        detected = events.desync;
        frame += 1;
        std::thread::sleep(Duration::from_millis(8));
    }

    // Phase 2: repair in place when a divergence was detected.
    if let Some(info) = detected {
        if args.inject.is_none() {
            return Err(format!("unexpected desync: {info}"));
        }
        println!(
            "desync=detected frame={} player={} local={:016x} remote={:016x}",
            info.frame, info.player, info.local, info.remote
        );

        let authority = args.role == 0;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut repaired = false;
        while !repaired {
            if Instant::now() >= deadline {
                return Err("state transfer timed out".into());
            }
            if authority && sender.is_none() {
                // Wait for a buffered state whose inputs are all known: the
                // live state can still embed a predicted remote input.
                if let Some((resume_frame, state)) = session.transfer_snapshot() {
                    core.load_state(&state).map_err(|err| err.to_string())?;
                    session.resync(resume_frame);
                    monitor.reset();
                    last_hash = None;
                    sender = Some(StateSender::new(resume_frame, &state));
                }
            }
            if let Some(out) = sender.as_mut() {
                for _ in 0..24 {
                    match out.next_chunk() {
                        Some(chunk) => transport.send(chunk).map_err(|err| err.to_string())?,
                        None => {
                            out.restart();
                            break;
                        }
                    }
                }
            }
            let events = exchange(
                transport.as_mut(),
                &mut session,
                &mut core,
                &mut seen,
                &mut monitor,
                &mut reassembler,
            )?;
            if let Some(ack) = events.ack {
                if sender.as_ref().is_some_and(|out| out.acknowledged(ack)) {
                    transport.send(RESUMED).map_err(|err| err.to_string())?;
                    sender = None;
                    repaired = true;
                    println!(
                        "state-transfer ack frame={} checksum={:016x}",
                        ack.frame, ack.checksum
                    );
                }
            }
            if let Some((state, resume_frame, checksum)) = events.state {
                core.load_state(&state).map_err(|err| err.to_string())?;
                session.resync(resume_frame);
                monitor.reset();
                last_hash = None;
                transport
                    .send(
                        &StateAck {
                            frame: resume_frame,
                            checksum,
                            player: args.role,
                        }
                        .encode(),
                    )
                    .map_err(|err| err.to_string())?;
                let wait = Instant::now() + Duration::from_secs(10);
                loop {
                    if Instant::now() >= wait {
                        return Err("resume barrier timed out".into());
                    }
                    if exchange(
                        transport.as_mut(),
                        &mut session,
                        &mut core,
                        &mut seen,
                        &mut monitor,
                        &mut reassembler,
                    )?
                    .resumed
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                repaired = true;
                println!("state-transfer loaded frame={resume_frame} checksum={checksum:016x}");
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        // Phase 3: continue from the shared snapshot with the true inputs.
        let start = session.frame();
        let end = start + args.recover_frames;
        while session.frame() < end {
            let number = session.frame();
            play_frame(
                &args,
                transport.as_mut(),
                &mut session,
                &mut core,
                &mut monitor,
                &mut last_hash,
                &mut hashes,
                number,
                false,
            )?;
            let events = exchange(
                transport.as_mut(),
                &mut session,
                &mut core,
                &mut seen,
                &mut monitor,
                &mut reassembler,
            )?;
            if let Some(info) = events.desync {
                return Err(format!("desync after state transfer: {info}"));
            }
            std::thread::sleep(Duration::from_millis(8));
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut quiet = Instant::now();
        while quiet.elapsed() < Duration::from_millis(400) {
            if Instant::now() >= deadline {
                break;
            }
            let events = exchange(
                transport.as_mut(),
                &mut session,
                &mut core,
                &mut seen,
                &mut monitor,
                &mut reassembler,
            )?;
            if events.desync.is_some() {
                return Err("desync after state transfer".into());
            }
            quiet = Instant::now();
            std::thread::sleep(Duration::from_millis(5));
        }

        let hash = snapshot_hash(&mut core)?;
        let final_frame = session.frame();
        let reference = reference_hash(&rom, final_frame)?;
        let recovered = monitor.mismatches() == 0 && hash == reference && monitor.checked() > 0;
        println!(
            "recovered=state-transfer frame={final_frame} hashes={} mismatches={} hash={hash:016x} reference={reference:016x} match={}",
            monitor.checked(),
            monitor.mismatches(),
            hash == reference
        );
        if !recovered {
            return Err("state transfer did not converge".into());
        }
        return Ok(());
    }

    // Wait for the remaining inputs and hashes to settle.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = Instant::now();
    loop {
        let events = exchange(
            transport.as_mut(),
            &mut session,
            &mut core,
            &mut seen,
            &mut monitor,
            &mut reassembler,
        )?;
        if let Some(info) = events.desync {
            return Err(format!("unexpected desync: {info}"));
        }
        if seen.len() >= args.frames as usize && quiet.elapsed() > Duration::from_millis(400) {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        if seen.len() >= args.frames as usize {
            quiet = quiet.min(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(8));
    }

    let hash = snapshot_hash(&mut core)?;
    let reference = reference_hash(&rom, args.frames)?;
    let stats = session.stats();
    println!(
        "role={} frames={} received={} rollbacks={} predicted={} depth={} hashes={hashes} hash={hash:016x} reference={reference:016x} match={}",
        if args.role == 0 { "host" } else { "join" },
        args.frames,
        seen.len(),
        stats.rollbacks,
        stats.predicted,
        stats.max_rollback_depth,
        hash == reference
    );
    if seen.len() < args.frames as usize || hash != reference {
        return Err("session did not converge to the reference state".into());
    }
    Ok(())
}
