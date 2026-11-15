# Architecture

## Goal

One shell, many systems, and more than one core per system when that is useful.
"Multi-emulator" here means: the application does not know which emulator is
running behind the interface, and it can switch cores without changing the UI,
input, audio, screenshot or save-state code.

## The boundary

```
┌──────────────────────────────────────────────────────────────┐
│ host (CLI today, GUI later)                                  │
│  - pacing, window, audio device, input binding, files        │
└───────────────▲──────────────────────────────┬───────────────┘
                │ FrameBuffer / AudioBuffer     │ run_frame()
                │                               │
┌───────────────┴──────────────────────────────▼───────────────┐
│ unko-core                                                │
│  Core trait · CoreInfo · Registry · InputState · LoadOptions │
└───────────────▲──────────────────────────────┬───────────────┘
                │ implements                    │ factory()
┌───────────────┴──────────────────────────────▼───────────────┐
│ core crates: unko-nes (standalone) · unko-nes-api (NesCore)   │
│  SNES core · GB core · ...                                   │
└──────────────────────────────────────────────────────────────┘
```

The host never depends on a core crate. It depends on `unko-core` (plus
`unko-nes-api` when it wants the NES extension) and builds a `Registry` from
the cores it chooses to link (or, later, load as plugins).
`unko-nes` is the only NES backend: it contains the reference-parity system
itself and has **no** dependency on `unko-core` or the shell. `unko-nes-host`
is a thin adapter that implements `unko_core::Core` and
`unko_nes_api::NesCore` on top of it and is the crate the app and CLI link. The former TetaNES adapter has been retired.

`unko-rom` is a sibling boundary for *input*: it discovers loose ROMs and
members of `.zip`/`.7z`/`.rar` archives, parses iNES/NES 2.0/UNIF metadata and
converts UNIF to iNES/NES 2.0, and returns bytes to whichever core is loading. The UI never sees archive libraries.

## The contract

`unko_core::Core`:

| Method | Responsibility |
|---|---|
| `info()` | static metadata: id, system, accuracy class, features |
| `load(rom, options)` | parse, reset, apply region/sample-rate/filter |
| `reset()` | hard reset, keep the cartridge |
| `run_frame()` | advance exactly one display frame |
| `video()` | borrow the finished frame (RGBA8, 256×240 for NES) |
| `audio()` | borrow this frame's f32 samples |
| `set_input(port, state)` | replace one controller's buttons |
| `set_sample_rate` / `set_audio_quality` | host preferences |
| `save_state` / `load_state` | full machine state, opaque bytes |
| `sram()` / `load_sram` | battery RAM in/out, host owns the files |

`unko_nes_api::NesCore` adds the NES-only controls: port devices, Famicom
microphone/3D glasses, Vs. System input, Miracle Piano, Datach, FDS sides,
core-side palettes/filters, cheats, channel gains and the PPU/APU debugger.

Rules that keep the boundary honest:

1. **The core never owns the window, the audio device, or a file path.** If it
   needs a file, the host passes bytes or a callback.
2. **The core is deterministic.** Same ROM, inputs and options must produce the
   same frames and states. No wall-clock time, no OS RNG, no threads in core
   code.
3. **One display frame per `run_frame`.** A core that runs multiple console
   frames (fast-forward) reports the count in `FrameStats`.
4. **Buffers are borrowed, not copied, at the boundary.** The PPU owns the
   completed frame and exposes it through `System::video`, so the adapter does
   not copy a full frame per display.
5. **Save states are opaque and versioned by the core.** The host stores them,
   never interprets them.

## Registry

`Registry` maps `SystemId` to zero or more `CoreRegistration`s. A system can have
several cores (accuracy-first, speed-first, experimental); the host chooses by
`info.id` or takes the first. Registration is explicit, so a build only contains
the cores you chose to link.

## NES backend

`unko-nes` owns the cycle-stepped CPU, PPU, APU, cartridge and system bus.
The `unko-nes-host` adapter:

- loads iNES/NES 2.0 and raw FDS images;
- exposes RGBA video and mono f32 audio through borrowed buffers;
- serializes versioned `UNKOST21` save states, including mapper state and a ROM
  fingerprint;
- maps the host input bits to the NES shift-register order;
- decodes Game Genie codes into `unko_nes_api::Cheat`, so the desktop shell links
  `unko-core` and `unko-nes-host` only. `unko-cli` also links `unko-nes`
  directly for its NES-specific analysis commands.

The core is deterministic and has no window, audio-device or filesystem
dependency. The desktop shell owns SDL3 input, cpal output and all files.

`unko_core::Core` is now limited to the generic lifecycle: `load`, `reset`,
`run_frame`, `video`, `audio`, `set_input`, `set_sample_rate`,
`set_audio_quality`, save states and SRAM. All NES-only controls moved to
`unko_nes_api::NesCore`: controller-port devices, Famicom microphone/3D glasses,
Vs. System input, Miracle Piano, Datach, FDS, core-side palettes and filters,
cheats and the PPU/APU debugger. `NesCore: Core` also exposes `as_core` /
`as_core_ref` for generic helpers such as netplay and frame hashing.

The desktop `Player` stores a `Box<dyn NesCore>`; the CLI validates `--core`
through the registry and then builds the adapter. The registry still returns
`Box<dyn Core>` for future non-NES systems. A future system implements only
`Core` plus its own extension trait. The remaining shared types (`PortDevice`,
`Palette`, `VideoFilter`, `InputState` order and the debug snapshots) are still
NES-shaped; moving them is the second half of roadmap **A8**.

## ROM manager core

`unko-netplay` is a host-side foundation for rollback netplay. It depends only
on `unko-core` and provides framed per-player input messages, a `Transport` trait
(in-process channel pair, a connected `UdpTransport`, and a `UdpMesh` /
`MeshTransport` for 2-4 player meshes), a bounded serialized-state
`RollbackBuffer`, and a `RollbackSession` that restores and resimulates when a
late remote input arrives. Each instance owns one controller slot; the session
keeps a per-peer input history and merges the four-port frame every tick.
`SessionConfig` selects the player count, local slot, prediction mode (last
input or neutral) and a fixed input delay applied identically to every player;
the session reports rollback count, predicted frames and the deepest rollback.
The app drives it from `Player::step` and exposes a host/join window with
player/slot selection. `upnp` adds a std-only UPnP-IGD client (SSDP discovery,
device-description fetch, `AddPortMapping`, `GetExternalIPAddress`) used from a
worker thread; `stun` adds an RFC 5389 binding client that supplies the
external IP when the router has no UPnP; `holepunch` retries `PUNCH`/`PONG`
datagrams to public peers and reports when bidirectional traffic is confirmed;
`sync` exchanges pre-frame state hashes for frames that left the rollback
window, reports the first mismatch, and carries the chunked checksummed
save-state transfer plus `StateReassembler`/`StateAck` used to repair a
diverged peer; `RollbackSession::resync` discards the old timeline;
`relay` is a simplified TURN-style server (`unko-relay`) that assigns clients
to a room and forwards `DATA` frames between members, used when punching
stalls. A manual public-endpoint field covers everything else. Live Internet
validation is reproducible with the two-host harness in `docs/netplay.md`
(direct, hole punch and relay paths already pass on a real LAN; a true
Internet run needs two networks).

`unko-rom-manager` is a separate, non-emulator feature core: DAT auditing,
1G1R selection, fixdat/CSV reports and safe rebuilding. It has no dependency
on `unko-core`, `unko-nes` or the app, and CI enforces that boundary. The app
links it only for the Tools → ROM Manager window; the same crate ships a
headless `unko-rom-manager` binary and can be used from any Rust project.

## What is deliberately not here yet

- C ABI / dynamic plugin loading.
- Threading (run-ahead, rollback netplay).
- A GPU frontend, shaders, rewind, debugger UI.
- Multi-instance support beyond "one `Box<dyn Core>` per console".

Those are shell features and should be built on this boundary, not inside the
cores.
