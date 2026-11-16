# Unko

A multi-platform, multi-core emulator shell in Rust. The first system is the
NES, powered by the standalone `unko-nes` core. The base
`unko_core::Core` contract is system-generic; NES-only controls live in the
`unko-nes-api` `NesCore` extension trait. A second isolated core,
`unko-snes`, is in progress.

> Formerly `multi-emu` / `multiemu`. Host crates: `unko-core`, `unko-rom`,
> `unko-cli`, `unko-app`. NES core: `unko-nes`. Adapter: `unko-nes-host`
> (registry id `nes_accuracy_core`). SNES core: `unko-snes`.

**Repository:** <https://github.com/lordra9-svg/unko>

[![CI](https://github.com/lordra9-svg/unko/actions/workflows/ci.yml/badge.svg)](https://github.com/lordra9-svg/unko/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/lordra9-svg/unko)](https://github.com/lordra9-svg/unko/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Status

The shell is usable end-to-end: library scanning, a Dolphin-inspired desktop
UI, GPU shaders, speed controls, cheats, ten named save-state slots with
thumbnails, exact rewind, 0-4 frame run-ahead and input-movie recording. It
runs the NES core headlessly or in a window, outputs RGBA video and f32 audio,
plays through cpal, accepts keyboard/SDL3 gamepad/touch/peripheral input,
records WAV/PNG/GIF and verifies determinism. FDS images use a user-supplied
`disksys.rom`. The same UI builds for Windows, macOS, Linux, iOS and Android.
An offscreen wgpu/Vulkan renderer and opt-in wgpu presenter are available
behind `--features vulkan`; the glow path remains the default presenter.

Accuracy milestone **A5** is complete. **A6/A7** NES breadth and capture work is
frozen as done for now (2026-11-16); VS. System and PlayChoice-10 remain paused
by the owner pending a 2C05 title, a good DualSystem dump and PC10 images. The
SNES track: the isolated `unko-snes` crate now has ROM parsing and checksum
validation, LoROM/HiROM/ExLoROM/ExHiROM mapping, a cycle-stepped 65C816, the
full SPC700, an APU wrapper with IPL/ports, the S-DSP voice pipeline, a
line-based PPU with modes 0-7, sprites, subscreen/colour math and HDMA, plus
S-CPU DMA and APU timers. Retail Super Mario World is now **99.5% pixel-identical
to ares at 600 frames** with bit-identical VRAM/CGRAM/OAM; only a one-sprite
sub-frame scheduling difference remains.

Reference gates:

- `nestest` golden log: **8,980 / 8,980** lines exact
- blargg `instr_timing`, `instr_basics`, `cpu_exec_space`, `cpu_dummy_reads`
  and the five `cpu_interrupts_v2` singles: pass
- `ppu_vbl_nmi` **12/12**, `ppu_sprites` **19/19**, blargg APU/DMC **13/13**
- mapper matrix (Holy Mapperel, blargg MMC3, MMC5, VRC2a, MMC1-A12):
  **65/65**
- AccuracyCoin v3.1.0 isolated: **146/146 non-DRAW**, 0 failed, 5 DRAW skips
- `cargo test --workspace`: **833 passed, 0 failed**; clippy/fmt clean
- VS. System: mapper 99, cabinet input, UniSystem/DualSystem parsing,
  2C03/2C04/2C05 palettes and 2C05 security; 13 repaired local images boot
  (including Gumshoe's outer bank)

Licensed-library audit (7,101 images):

| Scanned | Loaded | Unsupported mapper IDs | Load/run failures | Draw @ 120 |
|---:|---:|---:|---:|---:|
| 7,101 | 7,073 (99.6%) | 0 | 28 (truncated dumps) | 6,503 |

Details: [`docs/accuracy-status.md`](docs/accuracy-status.md),
[`docs/accuracy-divergences.md`](docs/accuracy-divergences.md),
[`docs/snes-accuracy-divergences.md`](docs/snes-accuracy-divergences.md),
[`docs/compatibility.md`](docs/compatibility.md),
[`docs/mapper-audit.md`](docs/mapper-audit.md),
[`docs/test-rom-inventory.md`](docs/test-rom-inventory.md).

## NetPlay

`unko-netplay` uses **GGRS** (`gschup/ggrs`) for rollback on direct UDP, UDP
hole punching and the `unko-relay` room server. GGRS owns input exchange,
repeat-last prediction, rollback scheduling, time synchronization and checksum
desync detection; the adapter maps its save/load/advance requests onto `Core`.
2-4 players, a unique slot per instance, a fixed 0-16 frame input delay, UPnP-
IGD mapping, STUN fallback and a manual public endpoint are supported. The
window reports the active engine, frame, confirmed frame, rollbacks, predicted
frames, deepest rollback and desync reports. Netplay disables rewind, movies
and run-ahead. The legacy input-framing session remains for tests and
`netplay_harness --engine legacy`. See [`docs/netplay.md`](docs/netplay.md).

## Project stats

| Metric | Value |
|---|---:|
| Workspace crates | 12 |
| First-party Rust lines | ~105,200 |
| Workspace tests | 833 pass, 0 failed |
| AccuracyCoin non-DRAW rows | 146 / 146 |
| Licensed library images | 7,101 |

Measured on the local checkout, 2026-11-16. Test/accuracy figures are re-run in
CI; performance budgets are in [`docs/performance.md`](docs/performance.md).

## Layout

```
crates/
  unko-core/         # generic Core, SystemId, registry, frame/audio/state types
  unko-rom/          # discovery, .zip/.7z/.rar, iNES/NES 2.0/UNIF metadata
  unko-nes/          # standalone reference-parity NES core
  unko-nes-api/      # NesCore: devices, FDS, Vs., debugger, filters
  unko-nes-host/     # Core + NesCore adapter
  unko-nes-dev/      # AccuracyCoin battery, mapper suite, harnesses
  unko-snes/         # standalone SNES core (ROM, 65C816, CPU bus; PPU next)
  unko-rom-manager/  # standalone DAT audit, 1G1R, fixdat, rebuild
  unko-netplay/      # GGRS rollback, hole-punch/relay adapters, legacy session
  unko-cli/          # info/scan/run/text/verify/trace/diff/bench/corpus
  unko-app/          # desktop shell (Dolphin-inspired UX)
  xtask/             # Rust repository automation (former scripts)
```

Architecture, systems, roadmap, frontend, platform and licensing notes are in
[`docs/`](docs/); start with
[`docs/architecture.md`](docs/architecture.md) and
[`docs/roadmap.md`](docs/roadmap.md).

## Build and run

```sh
cargo build --release
cargo run -p unko-cli -- scan roms
cargo run -p unko-cli -- info "roms/.../game.zip"
cargo run -p unko-cli -- run "roms/.../game.zip" --frames 120 --out shot.ppm
cargo run -p unko-cli -- text "roms/.../test.nes" --frames 600
```

ROMs may be loose `.nes`/`.fds` files or inside ZIP/7z/RAR archives; `run`
writes the last frame as a binary PPM. `cargo run --release -p xtask -- pgo <rom-dir>` produces a
profile-guided build (about 10-13% faster).

## Desktop app

```sh
cargo run -p unko-app --release
cargo run -p unko-app --release -- --rom "roms/.../game.zip"
```

Library defaults to `roms/` under the portable data folder (`--roms DIR` to
change). Settings covers video/audio/palette/filters, controller ports, hotkeys,
paths and advanced core options. Start+Select opens the controller menu; single
Start/Select stay with the game. Keyboard: arrows, X=A, Z=B, Enter=Start,
Shift=Select, Space=pause, F5/F8=save/load, F9/F10=step, Backspace=rewind,
Tab=fast-forward, F11=fullscreen.

Port devices: NES Zapper, Arkanoid paddle, SNES/Subor mouse, Power Pad/Family
Trainer, Family BASIC/Subor keyboards, Famicom microphone, tape loopback,
Miracle Piano (SysEx replies and 49-key on-screen keyboard), Datach scanner
(X24C02/X24C01) and Four Score. The debugger shows CPU/PPU/APU state,
disassembly, palette/OAM and memory editors plus PC breakpoints.

The **ROM Manager** audits Logiqx/No-Intro DATs, selects 1G1R, exports
fixdat/CSV, verifies per-ROM matches, writes deterministic ZIP/7z sets and
rebuilds output. It is backed by the standalone `unko-rom-manager` crate and
has no dependency on the NES core or shell. See
[`docs/rom-manager-comparison.md`](docs/rom-manager-comparison.md).

The portable data root holds `unko.conf`, `roms/`, `saves/` and the FDS BIOS.
Resolution order: `--data-dir`/`--home`, `UNKO_HOME`/`UNKO_DATA_DIR`,
`--portable`, `unko.conf` beside the executable, CWD, writable executable
folder, then the platform data directory. Settings → Paths shows the active
root. Mobile entry points and packaging are in
[`docs/platforms.md`](docs/platforms.md).

## Testing, accuracy and benchmarks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p xtask -- license-audit
cargo run --release -p unko-cli -- bench --synthetic 3000 --min-fps 300
```

Tests needing public ROMs look under `test-roms/` and skip cleanly when absent;
no ROMs are committed. Save states are checked functionally across all 294
constructing mapper boards, plus byte-exact re-save for the common set.

AccuracyCoin (supply the ROM/catalog):

```sh
cargo run -p unko-nes-dev --release --example accuracycoin -- \
  test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated
```

Mapper suite (local RustyNES ROM tree):

```sh
cargo run --release -p unko-nes-dev --example mapper_suite -- \
  /path/to/rustynes/tests/roms
```

Current development-machine performance: synthetic NROM **~785 fps**
(13x real-time); the CI floor is 300 fps. See
[`docs/performance.md`](docs/performance.md).

## Embedding the NES core

`unko-nes` does not depend on `unko-core`, the CLI, the desktop shell or any
Unko state.

```toml
[dependencies]
unko-nes = { git = "https://github.com/lordra9-svg/unko", branch = "main" }
```

```rust
let mut nes = unko_nes::System::from_ines(&std::fs::read("game.nes")?)?;
nes.set_sample_rate(48_000.0);
nes.set_controller(0, 0);
nes.run_frame();
let rgba = nes.video();   // 256 × 240 RGBA8
let audio = nes.audio();  // mono f32 for the frame
# Ok::<(), Box<dyn std::error::Error>>(())
```

Save states, cheats and debugger helpers are default features;
`default-features = false` gives a lean core. For the host interface use
`unko-nes-host`, which implements `unko_core::Core` and
`unko_nes_api::NesCore`.

## Licensing and security

Project code is MIT. `unko-nes` includes ISC-licensed adaptations from ares;
see [`NOTICE`](NOTICE), [`licenses/`](licenses/) and
`crates/unko-nes/PROVENANCE.md`. Third-party crates are listed in `NOTICE`;
`xtask license-audit` checks the dependency license allowlist. Security
notes and the audit report are in [`docs/security.md`](docs/security.md).

This repository contains **no ROMs, BIOS images or copyrighted firmware**.
Bring legally obtained dumps.
