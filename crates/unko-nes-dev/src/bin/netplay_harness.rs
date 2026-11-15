//! Headless two-host NetPlay harness.
//!
//! Runs one side of a real UDP session on the Unko NES core, using a built-in
//! synthetic ROM whose RAM accumulates the two controller ports. Each side
//! prints the final save-state hash and the hash of an independent local run
//! with the true input sequence, so two machines (or two processes) can be
//! compared without a commercial ROM.
//!
//! Examples:
//!
//! ```text
//! # Direct UDP (LAN/public mesh)
//! netplay_harness --role host --bind 192.168.1.10:7901 \
//!     --peer 192.168.1.20:7902 --frames 240
//! netplay_harness --role join --bind 192.168.1.20:7902 \
//!     --peer 192.168.1.10:7901 --frames 240
//!
//! # Hole punching (same command, --transport punch)
//! # Relay
//! unko-relay --bind 192.168.1.10:7900
//! netplay_harness --role host --transport relay --relay 192.168.1.10:7900 \
//!     --room demo --bind 192.168.1.10:7901 --frames 240
//! netplay_harness --role join --transport relay --relay 192.168.1.10:7900 \
//!     --room demo --bind 192.168.1.20:7902 --frames 240
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
use unko_netplay::udp::UdpMesh;
use unko_netplay::Transport;

/// Handshake datagram; both sides must be bound before frames start.
const SYNC: &[u8] = b"UNKOHARN";
const READY: &[u8] = b"UNKORDY1";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

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
    rom[4] = 2; // 32 KiB PRG
    rom[5] = 1; // 8 KiB CHR
    let code = [
        0xA9, 0x01, 0x8D, 0x16, 0x40, // LDA #1; STA $4016 (strobe)
        0xA9, 0x00, 0x8D, 0x16, 0x40, // LDA #0; STA $4016
        0xAD, 0x16, 0x40, // LDA $4016
        0x29, 0x01, // AND #1
        0x18, 0x65, 0x00, 0x85, 0x00, // CLC; ADC $00; STA $00
        0xA5, 0x01, 0x69, 0x00, 0x85, 0x01, // LDA $01; ADC #0; STA $01
        0xAD, 0x17, 0x40, // LDA $4017
        0x29, 0x01, // AND #1
        0x18, 0x65, 0x02, 0x85, 0x02, // CLC; ADC $02; STA $02
        0xA5, 0x03, 0x69, 0x00, 0x85, 0x03, // LDA $03; ADC #0; STA $03
        0xE6, 0x04, // INC $04
        0x4C, 0x00, 0x80, // JMP $8000
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
            "-h" | "--help" => {
                println!(
                    "usage: netplay_harness --role host|join --frames N [--bind ADDR] \\
                     [--peer ADDR] [--transport mesh|punch|relay] [--relay ADDR] \\
                     [--room NAME] [--delay N] [--prediction last|neutral]"
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

fn drain(
    transport: &mut dyn Transport,
    session: &mut RollbackSession,
    core: &mut AccuracyCore,
    seen: &mut HashSet<u32>,
) -> Result<usize, String> {
    let mut count = 0;
    while let Some(bytes) = transport.recv().map_err(|err| err.to_string())? {
        let Some(frame) = InputFrame::decode(&bytes) else {
            continue;
        };
        seen.insert(frame.frame);
        session
            .receive_remote(core, frame.player, frame.frame, frame.input)
            .map_err(|err| err.to_string())?;
        count += 1;
    }
    Ok(count)
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
    let mut state = Vec::new();
    core.save_state(&mut state).map_err(|err| err.to_string())?;
    Ok(fnv1a(&state))
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    let rom = synthetic_rom();
    let mut core = AccuracyCore::new();
    core.load(&rom, &LoadOptions::default())
        .map_err(|err| err.to_string())?;

    let mut transport = build_transport(&args)?;
    let mut session = RollbackSession::with_config(SessionConfig {
        buffer_frames: 128,
        input_delay: args.delay,
        prediction: args.prediction,
        players: 2,
        local_player: args.role,
    });
    let mut seen = HashSet::new();

    // Two-phase handshake: a SYNC round trip proves both sockets are bound,
    // and the READY round trip makes both sides finish within one RTT so the
    // frame loops start together. Input frames that arrive early are stored by
    // the session for their frame numbers.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seen_sync = false;
    while !seen_sync {
        if Instant::now() >= deadline {
            return Err("peer handshake timed out".into());
        }
        transport.send(SYNC).map_err(|err| err.to_string())?;
        while let Some(bytes) = transport.recv().map_err(|err| err.to_string())? {
            if bytes == SYNC {
                seen_sync = true;
            } else if let Some(frame) = InputFrame::decode(&bytes) {
                seen.insert(frame.frame);
                session
                    .receive_remote(&mut core, frame.player, frame.frame, frame.input)
                    .map_err(|err| err.to_string())?;
            }
        }
        if !seen_sync {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let mut seen_ready = false;
    while !seen_ready {
        if Instant::now() >= deadline {
            return Err("peer start barrier timed out".into());
        }
        transport.send(READY).map_err(|err| err.to_string())?;
        while let Some(bytes) = transport.recv().map_err(|err| err.to_string())? {
            if bytes == READY {
                seen_ready = true;
            } else if let Some(frame) = InputFrame::decode(&bytes) {
                seen.insert(frame.frame);
                session
                    .receive_remote(&mut core, frame.player, frame.frame, frame.input)
                    .map_err(|err| err.to_string())?;
            }
        }
        if !seen_ready {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    for frame in 0..args.frames {
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
        let message = InputFrame {
            frame: number,
            player: args.role,
            input: local,
        };
        transport
            .send(&message.encode())
            .map_err(|err| err.to_string())?;
        drain(transport.as_mut(), &mut session, &mut core, &mut seen)?;
        // Pace the session so a late packet stays inside the rollback
        // window on a real link.
        std::thread::sleep(Duration::from_millis(2));
    }

    // Keep draining until every remote input arrived and the link is quiet,
    // so late inputs have been rolled back before hashing.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut quiet = Instant::now();
    loop {
        if drain(transport.as_mut(), &mut session, &mut core, &mut seen)? > 0 {
            quiet = Instant::now();
        }
        if seen.len() >= args.frames as usize && quiet.elapsed() > Duration::from_millis(400) {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    let mut state = Vec::new();
    core.save_state(&mut state).map_err(|err| err.to_string())?;
    let hash = fnv1a(&state);
    let reference = reference_hash(&rom, args.frames)?;
    let stats = session.stats();
    println!(
        "role={} frames={} received={} rollbacks={} predicted={} depth={} hash={hash:016x} reference={reference:016x} match={}",
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
