# Unko

A multi-platform, multi-core emulator shell written in Rust. The first system
is the NES, powered by the reusable `unko-nes` core behind a
system-agnostic interface.

> **Rename:** this project was formerly `multi-emu` / `multiemu`. The emulator is
> now **Unko**, the host crates are `unko-core`, `unko-rom`, `unko-cli` and
> `unko-app`, the reusable NES core is `unko-nes`, and the host adapter is
> `unko-nes-host` (registry id `nes_accuracy_core`).

**Repository:** <https://github.com/lordra9-svg/unko>

[![CI](https://github.com/lordra9-svg/unko/actions/workflows/ci.yml/badge.svg)](https://github.com/lordra9-svg/unko/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Status

The shell is usable end-to-end: library scanning, a Dolphin-inspired desktop
UI, GPU output shaders, speed controls, cheats, save states (10 named slots
per game with thumbnails and metadata), exact rewind, 0-4 frame run-ahead and
input-movie (TAS) record/playback. It runs the NES core
headlessly or in a desktop window, produces RGBA video and f32 audio, plays
audio through cpal, accepts keyboard, SDL3 gamepad, touch and peripheral input,
records WAV/PNG/GIF, and verifies determinism. FDS images load with a user-supplied `disksys.rom`
(**Settings → Paths → FDS BIOS**), with disk-side selection and persisted
disk writes. The same UI builds for Windows, macOS, Linux, iOS and Android.

Milestone **A5** (accuracy gate) is complete; **A6** (breadth: remaining
peripherals, FDS audio parity and validation) is in progress. The gates are
verified against the public reference suites:

- `nestest` golden log: **8,980 / 8,980 lines** exact (PC, A/X/Y, SP, P and cycle count)
- blargg `instr_timing` and `instr_basics`: pass
- `ppu_vbl_nmi` **12/12**, `ppu_sprites` **19/19**, blargg APU/DMC **13/13**
- `cpu_exec_space` (PPU-I/O and APU variants), `cpu_dummy_reads` and all five
  `cpu_interrupts_v2` singles: pass
- AccuracyCoin v3.1.0 isolated: **146/146 non-DRAW rows pass**, 0 failed,
  5 DRAW skips, 0 unrun (199,320 frames)
- `cargo test --workspace`: **633 passed, 0 failed**; clippy `-D warnings` clean
- NESdev archive baseline: **137 pass / 11 fail / 115 visual-or-input-only** (PAL APU timing leaves 4 failures; the alternate MMC3 selector leaves only `read_joy3/count_errors`)
- RustyNES test tree: **159 pass / 11 fail / 171 visual-or-input-only** (166 / 4 with PAL APU in PAL mode; 1 with the alternate MMC3 selector too)

Compatibility from the local licensed 7,101-image NES audit:

| Metric | Result |
|---|---:|
| Scanned | 7,101 |
| Loaded (mapper implemented) | 7,073 (**99.6%**) |
| Unsupported mapper IDs | **0** |
| Load/run failures | 28 (all truncated dumps) |
| Rendering at 20 frames | 3,897 |
| Rendering at 120 frames | 6,503 |
| Distinct mapper IDs in the set | 285 |

## Netplay foundations

Netplay lives in `unko-netplay` and is wired into the shell: framed four-port
input messages, in-process and non-blocking UDP transports, a serialized-state
rollback buffer and a session loop that resimulates after a late remote input.
The library tracks **per-game play time** (accumulated only while a game is
running, persisted in the save folder, shown in the Played column and Game
properties). Tools → **NetPlay…** hosts or joins a **2-4 player** mesh over UDP. Each
instance picks a unique player slot and lists the other players' addresses;
input messages are tagged with the owning port and merged per frame. Missing
remote input is predicted with **last input** or **neutral**, and a **fixed
input delay** (0-16 frames, applied identically to every player) trades
responsiveness for fewer rollbacks. For internet play the window can ask the
router for a **UPnP-IGD** UDP mapping (non-blocking, with the mapped public
endpoint filled in), fall back to a **STUN** binding request for the external
IP when UPnP is unavailable, or take a **manual public endpoint**. A **Hole
punch** mode exchanges punch packets with public peers, keeps the console
paused during the handshake, and only starts the session after bidirectional
traffic is confirmed. If punching stalls, a **relay** server (included as
`unko-relay`) forwards the same input messages between room members. Peers
also exchange **state hashes** for frames that have left the rollback window;
a mismatch pauses the session, reports the frame and both hashes, and the
authoritative peer streams a checksummed save-state transfer of the newest
fully-settled buffered state so play resumes from the shared frame. The window shows frame, sent/received/rollbacks,
predicted frames, the deepest rollback and hash-check counters. Netplay disables rewind, movies and run-ahead. Direct UDP, hole
punching and the relay were validated over a real LAN with the two-host
harness; a true Internet run needs two networks. See
[`docs/netplay.md`](docs/netplay.md) for the harness commands and results.

## Project stats

| Metric | Value |
|---|---:|
| Workspace crates | 9 |
| First-party Rust lines | ~78,900 |
| Workspace tests | 633 pass, 0 failed |
| AccuracyCoin non-DRAW rows | 146 / 146 |
| Licensed library images scanned | 7,101 |

Numbers are from the local checkout on 2026-11-15; the test and
accuracy figures are re-measured on every CI run or release audit.

Only mapper 308 (NTDEC Batman) does not draw headless without Start input.
"Rendering" in that audit is a boot smoke signal, not a completion claim; the
20-frame window mostly measures how long a game stays on its boot screen.
Current figures, dated entries and the remaining limits are in
[`docs/compatibility.md`](docs/compatibility.md),
[`docs/accuracy-status.md`](docs/accuracy-status.md) and
[`docs/mapper-audit.md`](docs/mapper-audit.md); known divergences are in
[`docs/accuracy-divergences.md`](docs/accuracy-divergences.md).

## Layout

```
crates/
  unko-core/         # SystemId, Core trait, video/audio/input/state types, registry
  unko-rom/          # discovery + .zip/.7z/.rar containers + iNES metadata
  unko-nes/          # standalone reference-parity NES core (no unko-core dependency)
  unko-nes-host/     # Core ABI adapter between unko-nes and unko-core
  unko-nes-dev/      # AccuracyCoin battery and development harnesses
  unko-rom-manager/  # standalone DAT audit, 1G1R, fixdat and rebuild core
  unko-netplay/      # input framing, transport and rollback session (foundation)
  unko-cli/          # headless runner: info, scan, run, verify, trace, diff, bench, corpus
  unko-app/          # desktop shell (Dolphin-inspired UX)
docs/
  accuracy.md            # accuracy charter, gates and source rules
  accuracy-status.md     # measured results and dated experiment log
  accuracy-divergences.md # known divergences and the open failure list
  architecture.md        # core boundary and why it looks like this
  systems.md             # system/core entries, licenses, integration order
  roadmap.md             # A0..A7 and M0..M4
  implementation-plan.md # current easy-to-hard feature queue
  mapper-audit.md        # 7,101-image compatibility and mapper coverage
  test-rom-inventory.md  # NESdev suite coverage and missing BIOS/ROMs
  compatibility.md       # systems, mappers, peripherals and feature support
  performance.md         # benchmark, state costs and efficiency budgets
  oracle-feature-matrix.md # comparison against the NES emulator field
  decisions.md           # license/stack/scope decisions
  emulator-tooling.md    # installed reference emulators and toolchains
  frontend.md            # desktop UX blueprint and the GPL boundary
  netplay.md             # transports, session model and two-host harness
```

Target order: **NES → SNES (ares) → N64 (ares) → optional external Dolphin**.
The NES track uses one core: `unko-nes`, a standalone Rust core built to
reference parity. It is MIT with documented ISC-licensed adaptations from ares;
MesenCE, RustyNES, gopher64 and Dolphin are GPL references or separate products.
Details in `docs/systems.md`.

## Build and run

A profile-guided build (about 10-13% faster) is available with
`scripts/pgo.sh <rom-directory>`; see `docs/performance.md`.

```sh
cargo build --release
cargo run -p unko-cli -- scan roms
cargo run -p unko-cli -- info "roms/.../game.zip"
cargo run -p unko-cli -- run "roms/.../game.zip" --frames 120 --out shot.ppm
cargo run -p unko-cli -- text "roms/.../test.nes" --frames 600
```

`text` prints the nametable as ASCII, which is how the test-ROM suites in
`docs/test-rom-inventory.md` are scored.

ROMs may be loose `.nes` files or inside `.zip`, `.7z`, or `.rar` archives;
the CLI and GUI use the same reader. `scan` reports the library and mapper
metadata without extracting anything to disk.

`run` writes the last display frame as a binary PPM (`P6`) so you can inspect
output without a window.

## Desktop app

```sh
cargo run -p unko-app --release
cargo run -p unko-app --release -- --rom "roms/.../game.zip"
```

The library defaults to `roms/` under the portable data folder; pass
`--roms DIR` to point elsewhere. The shell follows Dolphin's desktop
UX: a game library as the home screen, menu bar, toolbar, player viewport,
status bar, and one settings window with a category rail. See
`docs/frontend.md` for the blueprint and the GPL boundary.

Gamepads are hot-plugged through SDL3; bindings, auto-fire, dead zone,
per-game profiles and keyboard shortcuts are configurable in Settings.
**Open ROM** has a native file picker and accepts `.nes`, `.fds`, `.zip`, `.7z`
and `.rar`; dropping a ROM or archive on the window opens it and adds its
folder to the library.

Tools → **Game properties…** overrides the timing region, the controller
ports and the header mapper for one game. The override is stored as `<stem>.game` in the save folder and
applies on the next load; **Save and reload** applies it immediately (or
launch with `--properties`). Mapper
overrides rewrite the iNES/NES 2.0 header, so a mislabelled dump can boot
without editing the file on disk.

Settings → Controllers has a **Connected** dropdown per port: Auto, None,
Keyboard, Gamepad 1-4, Mouse (Zapper), Mouse (Paddle) or Mouse (SNES/Subor),
plus the NES protocol dropdown and **Restore default controls**. Explicit port
choices override the gamepad matrix for that port.

A connected gamepad can navigate the UI: D-pad selects in the library, A plays
the selected game, B clears the selection, Select toggles grid/list, and
**Start+Select** opens the controller menu (and pauses a running game). The
menu covers Resume/Pause, Stop, Open ROM, Settings, ROM Manager, Rescan, view
toggle and Quit. Start and Select alone stay with the game when one is running.

Every OS window starts at its built-in default geometry on first use. The app
does not resize windows automatically; after you move or resize one, its
position and size are remembered in `unko.conf` (`layout_main`,
`layout_settings`, `layout_devices`, `layout_debugger`, `layout_cheats`,
`layout_rom_manager`) and restored on the next launch.

### ROM Manager

The Tools → ROM Manager window opens as a separate OS window on desktop. It
audits a ROM folder against one or more Logiqx XML/No-Intro DATs, shows
complete/partial/missing games (the full list, virtualized for large DATs),
selects a 1G1R set, exports fixdats/CSV and safely copies matched ROMs to an
output folder. A **ZIP sets** toggle writes one deterministic archive per
game instead of loose files, and a persistent scan cache skips unchanged
files on later audits. Its position and size are remembered in `unko.conf`; the
default size is used the first time. It is backed by the standalone `unko-rom-manager` crate, which has no
dependency on the NES core or the shell.

```sh
# Open the manager with a DAT and source folder preloaded
unko-app --rom-manager --dat "No-Intro NES.dat" --roms /path/to/roms

# Headless equivalent
cargo run -p unko-rom-manager -- \
  --dat "No-Intro NES.dat" --input /path/to/roms --1g1r \
  --fixdat missing.xml --csv report.csv --copy rebuilt --zip
# --move deletes loose sources after a verified copy (loose output only)
```

It scans loose files and ZIP, 7z or RAR containers, can download/update a DAT
over HTTP with `--dat-url`, can write deterministic ZIP or TorrentZip-style
sets, and has a read-only `verify`/`audit` mode that reports per-ROM
missing/unknown/duplicate detail and exits nonzero unless the set is complete.
The same report is available in the window behind the **Verify** button (or
`--rom-verify` after the initial scan), with the CLEAN/NOT CLEAN status,
matched source/hash/headerless flags and missing/unknown/duplicate sections.
The scan cache is visible and controllable in the window: entry count, file
size and path, a **Use scan cache** toggle, and **Clear cache** to force a
full re-hash. Rebuild and its post-rebuild audit run on the manager's
background worker, showing per-game write counts and then verification file
counts while the UI stays responsive. A rebuild that finishes without errors
re-audits its output folder and opens the panel automatically:

```sh
cargo run -p unko-rom-manager -- verify --dat DAT --input rebuilt
``` The feature comparison of the bundled managers (igir, RomVault, ClrMamePro,
RomCenter and Romulus) is in
[`docs/rom-manager-comparison.md`](docs/rom-manager-comparison.md). Current
limits: ZIP input only, no 7z/RAR/CHD, no TorrentZip output and no patching
yet; rebuild only copies and never deletes.

### Portable data folder

Unko keeps every mutable file under one data root: `unko.conf`, `roms/`,
`saves/` (states with thumbnails and metadata, SRAM, cheats, per-game
profiles, FDS writes, movies, screenshots and recordings) and the user-supplied `disksys.rom`.
Copying that folder to another machine keeps the whole setup intact.

The root is resolved in this order:

1. `--data-dir DIR` (or `--home DIR`);
2. `UNKO_HOME` / `UNKO_DATA_DIR`;
3. `--portable`, which selects the folder beside the executable (or beside the
   AppImage / macOS `.app`);
4. an `unko.conf` beside the executable;
5. an `unko.conf` in the current directory (the legacy layout);
6. the executable folder when it is writable;
7. the platform data directory for read-only installs and mobile.

Paths written to `unko.conf` are stored relative to the data folder whenever
they live inside it, so moving the folder does not leave stale absolute
paths. Settings → Paths shows the active data folder and offers **Use portable
defaults**.

```sh
# Explicit portable folder
unko-app --data-dir /media/usb/Unko

# Portable beside the executable
unko-app --portable
```

The CLI is already stateless for this purpose: `--bios`, `--sram`, `--out` and
`--audio-dir` take explicit paths, so scripts can point at the same portable
folder.

### Mobile and packaging

The same shell builds for iOS and Android: the crate is a library
(`unko_app`) plus a desktop binary, with `android_main` and `start_winit_app`
entry points, a touch overlay, and platform settings/save paths. Packaging
scaffolds live in `crates/unko-app/android` (Gradle + `cargo-ndk`,
`NativeActivity`) and `crates/unko-app/ios` (XcodeGen + static library, wgpu
renderer). Desktop packages are produced on tag pushes by
`.github/workflows/release.yml`: a Linux tarball and AppImage, a Windows zip
and a macOS `Unko.app` zip, each with `SHA256SUMS`. Archives are byte
reproducible from the same commit and toolchain, and CI audits dependency
licenses and verifies the reproducibility claim. Requirements and current
gaps are in [`docs/platforms.md`](docs/platforms.md).
Keyboard: arrows = D-pad, X = A, Z = B, Enter = Start, Shift = Select,
Space = pause, F5/F8 = save/load state, F9/F10 = step frame/instruction,
Backspace = rewind while paused, Tab = hold-to-fast-forward, F11 = fullscreen.

Playback and capture: 25-300% speed presets, frame advance, exact rewind
(64-256 MiB ring), 0-4 frame run-ahead, input-movie (TAS) recording/playback,
Game Genie and raw-RAM cheats, WAV/PNG/GIF capture, and screenshots.

Per-port devices (Settings -> Controllers): NES Zapper, Arkanoid paddle,
SNES/Subor mouse, Power Pad / Family Trainer, Family BASIC keyboard,
Subor/Dendy keyboard, Famicom microphone, data-recorder tape loopback, Miracle
Piano transport, Datach scanner with a persisted X24C02 EEPROM, and the Four
Score adapter.

The debugger window (`Emulation -> Debugger`, or `--debug` on the command
line) shows CPU registers and disassembly, PPU timing/palette/OAM, APU channel
outputs, a hex viewer/editor for CPU, PPU and OAM memory, and PC breakpoints
that pause before the instruction executes.

## Testing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/license_audit.py
```

Tests that need public test ROMs look for them under `test-roms/` and skip
cleanly when the folder is absent. No ROMs are tracked in this repository.
Save states are checked functionally across every mapper that constructs
(294 boards: two frames of video and audio must be identical after restore),
plus byte-exact re-save for the common mapper set.

The public mapper test-ROM matrix is reproducible with the local RustyNES tree
(ROMs are not committed):

```sh
cargo run --release -p unko-nes-dev --example mapper_suite -- \
  /path/to/rustynes/tests/roms
```

It scores Holy Mapperel (42 variants), the three blargg MMC3 suites, MMC5,
VRC2a and MMC1-A12, and prints per-mapper totals. Latest run: **65/65 pass,
0 failed, 0 without a verdict**.

The synthetic performance floor is checked in CI; the current development
machine measures about **785 fps** (13.1x real time):

```sh
cargo run --release -p unko-cli -- bench --synthetic 3000 --min-fps 300
```

The compatibility smoke test needs a local library:

```sh
cargo run --release -p unko-cli -- corpus <library-dir> --frames 20
```

To run the AccuracyCoin battery, supply the ROM and its catalog yourself:

```sh
cargo run -p unko-nes-dev --release --example accuracycoin -- \
  test-roms/AccuracyCoin.nes test-roms/AccuracyCoin.tsv --isolated
```

## Using the NES core in another project

The `unko-nes` crate is standalone: it does not depend on `unko-core`, the
CLI, the desktop shell, or any Unko state.

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
let audio = nes.audio();  // mono f32 samples for the frame
# Ok::<(), Box<dyn std::error::Error>>(())
```

Save states, cheats and debugger helpers are default features; build with
`default-features = false` for a lean core. If you need the system-agnostic
`unko_core::Core` interface, use the separate `unko-nes-host` adapter.

## Licensing

Project code is MIT. Parts of the `unko-nes` NES core are adapted from
ares under its ISC licence; attribution is in `NOTICE`,
`licenses/ares-LICENSE.txt` and the crate's `PROVENANCE.md`. The desktop shell
uses SDL3 (zlib), cpal, egui/eframe and the Rust audio/image crates listed in
`NOTICE`. See `NOTICE` and `licenses/` for all third-party terms.

The retired TetaNES header-correction database was not carried over; the
accuracy core uses iNES/NES 2.0 headers directly. DAT-based bad-dump detection
and header repair are the next ROM-management feature.

This repository contains **no ROMs** and no copyrighted console firmware. Bring
your own legally obtained dumps and BIOS images.
