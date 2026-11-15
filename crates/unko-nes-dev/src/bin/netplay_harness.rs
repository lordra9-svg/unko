//! Headless two-host NetPlay harness with desync detection.
//!
//! Runs one side of a real UDP session on the Unko NES core, using a built-in
//! synthetic ROM whose RAM accumulates the two controller ports. Each side
//! periodically exchanges save-state hashes; `--inject-desync N` corrupts one
//! side's local input at frame N to prove detection, then both peers restart
//! from frame 0 after a `RESYNC` barrier to prove recovery.
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
use unko_netplay::sync::{DesyncInfo, DesyncMonitor, StateHash};
use unko_netplay::udp::UdpMesh;
use unko_netplay::Transport;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const SYNC: &[u8] = b"UNKOHARN";
const READY: &[u8] = b"UNKORDY1";
const RESYNC: &[u8] = b"UNKORSY1";

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

struct DrainResult {
    desync: Option<DesyncInfo>,
}

fn drain(
    transport: &mut dyn Transport,
    session: &mut RollbackSession,
    core: &mut AccuracyCore,
    seen: &mut HashSet<u32>,
    monitor: &mut DesyncMonitor,
) -> Result<DrainResult, String> {
    let mut desync = None;
    while let Some(bytes) = transport.recv().map_err(|err| err.to_string())? {
        if let Some(hash) = StateHash::decode(&bytes) {
            if let Some(info) = monitor.record_remote(hash) {
                desync.get_or_insert(info);
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
    Ok(DrainResult { desync })
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
    for frame in 0..args.frames {
        let local = if args.role == 0 {
            host_input(frame)
        } else {
            join_input(frame)
        };
        let injected = args.inject == Some(frame) && args.role == 1;
        let applied = if injected { local ^ 1 } else { local };
        let mut ports = [0u16; 4];
        ports[usize::from(args.role)] = applied;
        let number = session
            .step(&mut core, ports)
            .map_err(|err| err.to_string())?;
        let message = InputFrame {
            frame: number,
            player: args.role,
            input: local,
        };
        transport
            .send(&message.encode())
            .map_err(|err| err.to_string())?;

        if frame.is_multiple_of(args.hash_interval) {
            for (number, hash) in session.settled_hashes(last_hash) {
                last_hash = Some(number);
                if let Some(info) = monitor.record_local(number, hash) {
                    detected.get_or_insert(info);
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
                hashes += 1;
            }
        }

        if let Some(info) = drain(
            transport.as_mut(),
            &mut session,
            &mut core,
            &mut seen,
            &mut monitor,
        )?
        .desync
        {
            detected.get_or_insert(info);
        }
        std::thread::sleep(Duration::from_millis(8));
    }

    // Wait for the remaining inputs and hashes to settle.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = Instant::now();
    loop {
        let result = drain(
            transport.as_mut(),
            &mut session,
            &mut core,
            &mut seen,
            &mut monitor,
        )?;
        if let Some(info) = result.desync {
            detected.get_or_insert(info);
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

    if let Some(inject_frame) = args.inject {
        let Some(info) = detected else {
            return Err(format!(
                "injected divergence at frame {inject_frame} was not detected"
            ));
        };
        println!(
            "desync=detected frame={} player={} local={:016x} remote={:016x}",
            info.frame, info.player, info.local, info.remote
        );

        // Coordinated restart: both peers exchange RESYNC, drop the old
        // session and replay from frame 0.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut peer_ready = false;
        while !peer_ready {
            if Instant::now() >= deadline {
                return Err("resync barrier timed out".into());
            }
            transport.send(RESYNC).map_err(|err| err.to_string())?;
            while let Some(bytes) = transport.recv().map_err(|err| err.to_string())? {
                if bytes == RESYNC {
                    peer_ready = true;
                }
            }
            if !peer_ready {
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        let mut core = AccuracyCore::new();
        core.load(&rom, &LoadOptions::default())
            .map_err(|err| err.to_string())?;
        let mut session = RollbackSession::with_config(config);
        let mut monitor = DesyncMonitor::default();
        let mut seen = HashSet::new();
        last_hash = None;
        for frame in 0..args.recover_frames {
            let local = if args.role == 0 {
                host_input(frame)
            } else {
                join_input(frame)
            };
            let mut ports = [0u16; 4];
            ports[usize::from(args.role)] = local;
            let number = session
                .step(&mut core, ports)
                .map_err(|err| err.to_string())?;
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
                for (number, hash) in session.settled_hashes(last_hash) {
                    last_hash = Some(number);
                    if let Some(info) = monitor.record_local(number, hash) {
                        detected.get_or_insert(info);
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
                }
            }
            if let Some(info) = drain(
                transport.as_mut(),
                &mut session,
                &mut core,
                &mut seen,
                &mut monitor,
            )?
            .desync
            {
                detected.get_or_insert(info);
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.len() < args.recover_frames as usize && Instant::now() < deadline {
            let _ = drain(
                transport.as_mut(),
                &mut session,
                &mut core,
                &mut seen,
                &mut monitor,
            )?;
            std::thread::sleep(Duration::from_millis(8));
        }
        let hash = snapshot_hash(&mut core)?;
        let reference = reference_hash(&rom, args.recover_frames)?;
        let recovered = monitor.mismatches() == 0 && hash == reference;
        println!(
            "recovered={recovered} hashes={} mismatches={} hash={hash:016x} reference={reference:016x}",
            monitor.checked(),
            monitor.mismatches()
        );
        if !recovered {
            return Err("recovery did not converge".into());
        }
        return Ok(());
    }

    if let Some(info) = detected {
        return Err(format!("unexpected desync: {info}"));
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
