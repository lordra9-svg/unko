# NetPlay

NetPlay is a peer-to-peer rollback layer in `unko-netplay`. It depends only on
`unko-core`, so it works with any core that implements `Core` save states.

## Model

- One instance owns one controller slot; 2-4 players are supported. Input
  messages carry the sender's player tag (`UNKOI2`), and each session keeps a
  per-peer history and merges the four-port frame every tick.
- Missing remote input is predicted with **last input** or **neutral**.
- A fixed **input delay** (0-16 frames) maps simulation frame `s` to input
  frame `s-delay` on every peer, trading responsiveness for fewer rollbacks.
- Late inputs restore a saved state, truncate the newer states and resimulate.
  Counters: rollbacks, predicted port-frames, deepest rollback.

## Transports

| Transport | Use |
|---|---|
| `UdpMesh` | Direct UDP to one peer per player (LAN or forwarded port) |
| UPnP-IGD | Ask the router to open the UDP port and report the public endpoint |
| STUN (RFC 5389) | Discover the external IP when UPnP is unavailable |
| `HolePunchTransport` | Exchange `PUNCH`/`PONG` datagrams; start only after bidirectional traffic |
| `RelayTransport` + `unko-relay` | Room-based fallback when direct paths fail |
| Manual endpoint | User-supplied public `ip:port` |

The window is **Tools → NetPlay…**: role, players, slot, ports, peers, public
endpoint, UPnP/STUN detection, hole punch, relay server/room, prediction and
input delay. The console is paused during a hole-punch handshake so both peers
start from the same frame.

## Two-host harness

`crates/unko-nes-dev/src/bin/netplay_harness.rs` runs one side of a real UDP
session on the actual NES core. It builds a synthetic NROM whose RAM
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
reference hash. A true Internet run still needs two networks/hosts; the
commands above are the reproducible procedure.

## Limitations

- The relay is deliberately simpler than RFC 5766 TURN: one shared relay
  endpoint, room membership by source address, no permissions, channels,
  authentication or encryption.
- Hole punching cannot traverse symmetric NATs; use the relay there.
- Live Internet latency/loss has not been measured from two networks yet.
