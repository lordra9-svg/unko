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
│ standalone cores (no unko-core/shell dependency):            │
│   unko-nes · unko-snes · unko-n64                            │
│ optional adapters/extensions: unko-nes-host, unko-snes-host, │
│   unko-n64-host, unko-nes-api, unko-snes-api                 │
└──────────────────────────────────────────────────────────────┘
```

The host never depends on a core crate. It depends on `unko-core` (plus
`unko-nes-api` when it wants the NES extension) and builds a `Registry` from
the cores it chooses to link (or, later, load as plugins).
`unko-nes` is the only NES backend: it contains the reference-parity system
itself and has **no** dependency on `unko-core` or the shell. `unko-nes-host`
is a thin adapter that implements `unko_core::Core` and
`unko_nes_api::NesCore` on top of it and is the crate the app and CLI link. The former TetaNES adapter has been retired.

`unko-snes` is the same shape for the SNES: a standalone MIT Rust core with no
`unko-core` or shell dependency. It has ROM parsing, a cycle-stepped 65C816,
the SPC700/APU, the S-DSP, the dot/scanline PPU, DMA/HDMA, the coprocessors and
versioned save states. `unko-snes-host` is its thin `unko_core::Core` adapter
and `unko-snes-api` adds the SNES extension trait.

`unko-n64` is the third standalone core (RDRAM/RSP/RDP/AI/CPU scaffold). It
does not depend on the shell either; `unko-n64-host` is its adapter. Its
current validation status and known approximations are tracked in
`docs/n64-validation.md`.

Every core can be used on its own. Integration guides for building your own
emulator on top of them:

- `docs/nes-core.md`
- `docs/snes-core.md`
- `docs/n64-validation.md` (N64 scaffold status)

## System separation (verified)

The three emulated systems are separate crates with separate adapters:

| Core crate | Depends on | Standalone emulator use |
|---|---|---|
| `unko-nes` | serde, bincode, serde-big-array only | yes — `System::from_ines` + `run_frame` |
| `unko-snes` | serde, bincode, serde-big-array, corosensei only | yes — `Snes::new` / `reset` + PPU frame loop |
| `unko-n64` | no runtime dependencies | yes — `Machine::new` + `run_frame` |

None of the cores depend on `unko-core`, `unko-app`, `unko-rom` or another
system's crate, and none share a system-specific module. The adapters
(`unko-nes-host`, `unko-snes-host`, `unko-n64-host`) are the only crates that
bridge a core into `unko_core::Core`, and the `unko-*-api` crates add extension
traits without being required by the cores. `unko-rom` (archive discovery) and
`unko-rom-manager` are frontend-side input/tooling boundaries and can be
omitted entirely by an embedder.

Dependency check used for this table:

```sh
cargo tree -p unko-nes -e normal --depth 1
cargo tree -p unko-snes -e normal --depth 1
cargo tree -p unko-n64 -e normal --depth 1
```

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
- serializes versioned `UNKOST22` save states, including mapper state and a ROM
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

`unko-netplay` uses **GGRS** (`ggrs::P2PSession`) for direct UDP, hole punching
and the `unko-relay` room. `GgrsNetplay` implements the GGRS `Config` for the
NES input/state shape, maps `SaveGameState`/`LoadGameState`/`AdvanceFrame`
requests onto `Core`, and uses GGRS checksum desync detection. The app resets
the core and keeps frames frozen until the handshake completes. Direct UDP uses
GGRS's UDP socket; hole punching and relay implement `NonBlockingSocket`
adapters. Relay frames are directed by player-slot member ids and the relay
server prepends the source id, so GGRS sees stable per-peer identities without
public addresses.

The legacy engine (framed per-player input messages, `Transport`, a
serialized-state `RollbackBuffer`, and `RollbackSession`) remains as a library
fallback and powers `netplay_harness --engine legacy`; the app no longer starts
it. `SessionConfig` selects the player count, local slot, prediction and input
delay. GGRS uses repeat-last prediction; the legacy engine also offers neutral.
`upnp` adds a std-only UPnP-IGD client (SSDP discovery, device-description
fetch, `AddPortMapping`, `GetExternalIPAddress`) used from a worker thread;
`stun` adds an RFC 5389 binding client that supplies the external IP when the
router has no UPnP; `holepunch` retries `PUNCH`/`PONG` datagrams to public
peers and reports when bidirectional traffic is confirmed; `sync` exchanges
pre-frame state hashes, reports the first mismatch, and carries the chunked
checksummed save-state transfer plus `StateReassembler`/`StateAck` used to
repair a diverged legacy peer; `relay` is a simplified TURN-style server
(`unko-relay`) that assigns clients to a room and forwards `DATA` frames
between members. A manual public-endpoint field covers the rest. Live Internet
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
