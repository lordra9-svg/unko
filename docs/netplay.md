# NetPlay

NetPlay lives in `unko-netplay` and depends on `unko-core` plus the GGRS
rollback library, so it works with any core that implements `Core` save
states.

## Engines

| Path | Rollback engine |
|---|---|
| Direct UDP | **GGRS** (`gschup/ggrs` 0.13, `ggrs::P2PSession`) |
| UDP hole punching | legacy `RollbackSession` |
| `unko-relay` room | legacy `RollbackSession` |

`GgrsNetplay` implements `ggrs::Config` (`Input = u16`, `State = Vec<u8>`,
`Address = SocketAddr`) and translates GGRS requests into `Core` calls:

- `SaveGameState` serializes the core state and passes a 128-bit FNV-1a
  checksum to GGRS for desync detection.
- `LoadGameState` restores the requested state before GGRS resimulates.
- `AdvanceFrame` applies every player's input to its controller port and runs
  one core frame.

The app resets the core when a GGRS session starts and does not advance frames
until the handshake reaches `Running`, so both peers begin at frame 0 from the
same power-on state. GGRS handles input exchange, repeat-last prediction,
rollback scheduling, time synchronization and checksum comparison. The window
reports the active engine, frame, confirmed frame, rollbacks, predicted
port-frames, deepest rollback and desync reports. GGRS cannot repair a desync:
the session pauses and must be restarted.

The legacy engine is retained for the transports that do not expose GGRS's
per-peer source address yet:

- One instance owns one controller slot; 2-4 players are supported. Input
  messages carry the sender's player tag (`UNKOI2`), and each session keeps a
  per-peer history and merges the four-port frame every tick.
- Missing remote input is predicted with **last input** or **neutral**.
- A fixed **input delay** (0-16 frames) maps simulation frame `s` to input
  frame `s-delay` on every peer.
- Late inputs restore a saved state, truncate the newer states and resimulate.
- **Desync detection**: peers hash the pre-frame save state and exchange hashes
  for frames that fell out of the rollback buffer.
- **In-session repair**: on a mismatch the lowest-numbered player streams a
  save state in 1024-byte chunks (`StateChunk`, frame + length + FNV-1a
  checksum), sends the newest fully-known buffered state
  (`transfer_snapshot`), and the receiver reassembles it, loads it, discards
  its history (`RollbackSession::resync`) and acknowledges (`StateAck`).

## Transports

| Transport | Use |
|---|---|
| GGRS UDP socket | Direct UDP to one peer per player (LAN or forwarded port) |
| `UdpMesh` | Legacy direct/mesh transport used by the legacy engine and tests |
| UPnP-IGD | Ask the router to open the UDP port and report the public endpoint |
| STUN (RFC 5389) | Discover the external IP when UPnP is unavailable |
| `HolePunchTransport` | Exchange `PUNCH`/`PONG` datagrams; start only after bidirectional traffic |
| `RelayTransport` + `unko-relay` | Room-based fallback when direct paths fail |
| Manual endpoint | User-supplied public `ip:port` |

The window is **Tools → NetPlay…**: role, players, slot, ports, peers, public
endpoint, UPnP/STUN detection, hole punch, relay server/room, prediction and
input delay. Direct UDP starts GGRS and resets the core; the console is paused
during a hole-punch handshake so both legacy peers start from the same frame.

## Two-host harness

`crates/unko-nes-dev/src/bin/netplay_harness.rs` runs one side of a real UDP
session on the actual NES core using the legacy transport engine. The GGRS
path has a loopback integration test in `unko-netplay` (two GGRS sessions, a
`Core` test double with save/load/input semantics, 90 simulated frames and an
identical final state). It builds a synthetic NROM whose RAM
accumulates both controller ports, runs a deterministic input sequence,
prints the final save-state hash, and compares it with an independent local
run of the true input sequence. No commercial ROM is needed.

```sh
cargo build --release -p unko-nes-dev --bin netplay_harness
cargo build --release -p unko-netplay --bin unko-relay

# Direct UDP (replace the addresses with two hosts on the same LAN)
netplay_harness --role host --bind 192.168.12.252:7901 \
    --peer 192.168.12.252:7902 --frames 240
netplay_harness --role join --bind 192.168.12.252:7902 \
    --peer 192.168.12.252:7901 --frames 240

# Hole punching
netplay_harness --role host --transport punch --bind 192.168.12.252:7911 \
    --peer 192.168.12.252:7912 --frames 240
netplay_harness --role join --transport punch --bind 192.168.12.252:7912 \
    --peer 192.168.12.252:7911 --frames 240

# Relay
unko-relay --bind 192.168.12.252:7900
netplay_harness --role host --transport relay --relay 192.168.12.252:7900 \
    --room demo --bind 192.168.12.252:7901 --frames 240
netplay_harness --role join --transport relay --relay 192.168.12.252:7900 \
    --room demo --bind 192.168.12.252:7902 --frames 240
```

Start both sides within about a second. A two-phase `SYNC`/`READY` handshake
then aligns the frame loops; frames that arrive during the handshake are
stored for their frame numbers.

`--inject-desync N` deliberately applies a different local input on the join
side at frame `N` (the wire input stays correct) to prove detection. After a
mismatch the host streams its state in chunks; the join reassembles it,
verifies the checksum, loads it, resynchronises and acknowledges. Both then
continue for `--recover-frames` from the transferred frame to prove
in-session recovery.

Expected output (both sides):

```text
role=host frames=240 received=240 rollbacks=235 predicted=235 depth=2 \
  hash=0c6f6706deada347 reference=0c6f6706deada347 match=true
```

### Local validation (2026-11-14)

Run on one Linux host over its LAN address (`192.168.12.252`), two separate
processes and the real `AccuracyCore`:

| Transport | Host hash | Join hash | Match |
|---|---|---|---|
| Direct UDP mesh | `0c6f6706deada347` | `0c6f6706deada347` | yes (depth 2) |
| Hole punch | `0c6f6706deada347` | `0c6f6706deada347` | yes (depth 6) |
| Relay (`unko-relay`) | `0c6f6706deada347` | `0c6f6706deada347` | yes (depth 70/112) |

All runs delivered 240/240 remote inputs and converged to the independent
reference hash.

### Desync detection and recovery (2026-11-14)

Two processes, direct UDP, 300 frames, hashes exchanged every 30 frames:

```text
normal run   host/join: 300/300 inputs, 143 hashes, 0 mismatches, match=true
injected     join applies input^1 at frame 100 (wire input stays correct),
             detected once frame 112 settled:
             desync=detected frame=112 player=1 local=26d761f09280ef68 remote=2df7722e654c5586
             state-transfer ack frame=241 checksum=095ff293e97c4ed5
             recovered=state-transfer frame=441 hashes=52 mismatches=0 \
               hash=b8bb578e43a68454 reference=b8bb578e43a68454 match=true
```

Both processes printed the same detection, transfer and recovery lines. The
same run over the `unko-relay` room transport (port 7900, room `demo3`)
produced the identical detection, transfer (`frame=241`,
`checksum=095ff293e97c4ed5`) and recovery (`frame=441`, 52 hashes, 0
mismatches, `b8bb578e43a68454`). Three consecutive direct runs also all
matched.

A true Internet run still needs two networks/hosts; the commands above are the
reproducible procedure.

## Limitations

- The relay is deliberately simpler than RFC 5766 TURN: one shared relay
  endpoint, room membership by source address, no permissions, channels,
  authentication or encryption.
- Hole punching cannot traverse symmetric NATs; use the relay there.
- Live Internet latency/loss has not been measured from two networks yet.
- State hashing reuses the rollback save states, but it still hashes the full
  serialized state at the configured cadence and costs some CPU on large
  boards (FDS/MMC5); it is off for local play and only enabled in netplay.
- A repair transfer needs a fully-settled buffered state. If the buffer was
  just cleared, the authority waits (and keeps retrying) until one exists;
  chunks are retransmitted until the acknowledgement arrives.
