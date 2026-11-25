# Roadmap

## Accuracy track (parallel, NES)

**NES is frozen as done for now (2026-11-16).** The public suites are green and
the remaining items below are deferred to a later date: hardware captures,
partial mapper boards, FDS display-only oracles, NSF corpus validation, real
peripheral validation, and VS. System/PlayChoice-10. No new NES work is planned
on this track until it is explicitly reopened.

The main effort was the reference-parity accuracy core. Details in
`docs/accuracy.md`.

- [x] **A0** accuracy crate, deterministic frame/audio/state hashing, test-ROM
      runner, provenance firewall.
- [x] **A1** cycle-stepped 6502: `nestest` 0 diff, `instr_timing` and
      `instr_basics` pass.
- [x] **A2a** `vbl_nmi` 12/12; penultimate-cycle NMI polling.
- [x] **A2b** sprite suite 19/19: all `spr_hit_*`, all overflow ROMs,
      `oam_read`, `oam_stress` and `sprite_ram` pass.
- [x] **A3** APU + DMC: all 13 blargg APU/DMC ROMs in the suite pass
      (length/IRQ/jitter/reset/halt/reload, DMC basics/rates). The nonlinear
      mixer curves are implemented and unit-tested. DMC DMA cycle stealing is
      validated by a synthetic reference benchmark: over 300 frames the same
      10,126 loop iterations are stolen with and without DMC (the only
      difference is a two-iteration start-up offset, present with DMC off too).
      The abort/interleave edge cases remain.
- [ ] **A4** top 30 mapper families + commercial oracle corpus. Progress:
      NROM/MMC1/UxROM/CNROM/MMC3(+IRQ)/AxROM/VRC(+IRQ)/Color Dreams/CPROM/
      GxROM/Camerica/UNROM-512/NINA/DxROM/Nanjing-163/Sunsoft-FME7/Sachen/
      Jaleco-SS88006/Namco-163/Namco-175-340/Action-53/J.Y.-ASIC/Waixing-FS305/
      Bandai-FCG/TXC/CoolBoy-MMC3-8025/MMC3-215/GTROM/Cony-83/RAMBO-1/VRC6/
      Taito-TC0190-TC0690/X1-005/Irem-G-101/JF-13/multicart latches/partial
      MMC5, MMC2/MMC4 PPU-latched CHR, Sunsoft-4 (including CHR-ROM
      nametables), the MMC3-8025/FS005 protection path (176), a corrected
      VRC2/VRC4 core (21/22/23/25), Sachen JV001 (136), Bandai LZ93D50 (159),
      MMC3 512 KiB CHR wiring (197), Waixing FS303 (195) with its `$5000`
      PRG-RAM, the MMC2/MMC3 hybrid (165), Super Mega JV001 (172), NTDEC
      TC-112 (193), full PPU mirroring modes, the 143
      protected NROM and the 212/235/286 multicarts, `System` bus, Core ABI
      adapter and a `corpus` runner. Full-library audit: 7,073/7,101 load,
      3,897 rendering at 20 frames, 0 unsupported, 0 panics; 120/240-frame
      passes 6,503/6,427 rendering, with only input-gated 308 never drawing
      headless;
      136/193/197 render 7/7,
      159/165 8/8, 172 4/6, 195 8/8 (Tiexue needs Start on its title screen)
      (`docs/mapper-audit.md`).
      FDS wavetable/modulation audio is implemented from the documented model
      and bit-matches RustyNES for 10.9 s with the APU muted; the residual is
      a table-upload note change. The raw
      loader (`System::from_fds`) and mapper 20 board now map the 8 KiB BIOS,
      32 KiB work RAM, the `$4020-$4032` disk registers/timer IRQ and the
      `$4040-$4097` audio window. The disk is delivered as a synthesized wire
      image (lead-in/inter-block gaps, `$80` start marks, per-block CRC-16) at
      the ~149-cycle byte cadence, and the real BIOS boots a local
      commercial `.fds` title to its licence screen and game title. Remaining
      FDS work: the frame-653 table-upload
      audio residual. MMC5 now covers the full CHR range (the extended-attribute
      path carries physical offsets, so >256 KiB CHR works). The Datach board
      (157) and the previously open six-image ASIC tail are implemented; the
      remaining hardware-dependent checks are tracked in
      `docs/mapper-audit.md`. VRC6 expansion audio (two
      pulses + sawtooth), Sunsoft 5B / YM2149 (three tones + noise + envelope)
      Namco 163 (up to eight wave channels, shared $40-$7F sound RAM), MMC5
      (two pulses + PCM DAC) and the VRC7/YM2413 two-operator FM engine are
      implemented with unit tests; the VRC7 built-in patch table is the
      documented instrument ROM dump and the custom patch registers are exact.
- [x] **A5** AccuracyCoin gate and divergence ledger. Isolated runner exercises
      all 151 rows: **146 pass, 0 fail, 5 DRAW skips, 0 unrun** — every
      non-DRAW test passes. `$2007 Stress Test` was closed with the PPU data
      state machine and per-dot fetch-buffer feed (170/170 answer-key bytes,
      confirmed against both RustyNES and MesenCE). The ledger
      (`docs/accuracy-divergences.md`) has no open failures and the dated
      measured log is `docs/accuracy-status.md`.
- [ ] **A6** FDS, VS, PAL/Dendy, expansion audio, peripherals, NSF.
      Progress: the accuracy core supports a per-port device type and the
      **NES Zapper** (`$4016`/`$4017` bit 4 trigger, bit 3 light, active-low;
      the sensor samples bright pixels in its 8x8 area as the beam passes),
      with unit coverage, and the desktop app maps the mouse to port 2 with a
      Controllers-tab toggle. **PAL/Dendy timing** is wired through
      `System::set_region` (312-line PPU, fractional 3.2-dot CPU/PPU ratio,
      NTSC-only odd-frame skip), with unit-test verification. An **NSF/NSFE
      parser and player** builds a synthetic NROM image with a player stub
      (INIT once, PLAY from the NMI) and is covered by header, image-layout and
      init/play unit tests. Expansion audio (VRC6, Sunsoft 5B, Namco 163,
      MMC5 and VRC7) is implemented and unit-tested. The **Famicom 3D System**
      is implemented from the NESdev description: `$4016` bit 1 (OUT1)
      selects the eye, the core latches it at the field boundary and the
      frontend combines consecutive left/right fields into a red/cyan
      anaglyph. **Karaoke Studio** (mapper 188) reads its A/B buttons and
      active-low 1-bit microphone at `$6000-$7FFF`. **Miracle Piano** replies
      to SysEx 4 and SysEx 11 with the documented version/memory-test messages,
      and the **Datach** adapter's external 128-byte X24C01 (Battle Rush) is
      modelled on `$8000-$8003` bit 3 with a 384-byte save. The Devices window
      adds a clickable 49-key piano keyboard with optional computer-key input.
      The **VS. System** now loads mapper 99 (banking, four-screen VRAM,
      OUT1-gated work RAM), reads coin/service/DIP input on `$4016`/`$4017`,
      models the `$4020` coin counter and uses the documented 2C03/2C04/2C05
      RGB palettes, RGB emphasis and 2C05 register/security quirks, with the
      DIP/primary choices remembered per game. Remaining: VS BIOS/ROMs,
      per-title DIP defaults, the DualSystem sub-CPU/watchdog, PlayChoice-10
      input and validation against real PAL/NSF content. A local Vs. drop was
      repaired and staged on 2026-11-16: 13 images (mapper 99/2C03/2C04,
      including Gumshoe's 40 KiB outer bank) boot in `test-roms/vs/`. VS. and
      PlayChoice-10 work is **paused by the owner** for now: a 2C05 title, a
      good DualSystem dump and PC10 iNES images are still missing, and PC10
      needs the second-CPU/INST-ROM layer. The FDS `$4030.D1`
      DRAM-refresh watchdog matches the hardware capture (`67FF`/`6E25`).
- [ ] **A7** differential harness, hardware captures, performance pass.
      The differential harness is complete: per-frame RGBA, audio and
      palette-independent geometry hashes are compared against RustyNES, and
      ares/MesenCE captures triage disagreements (`unko trace` / `diff` /
      `parity` / `corpus --parity`). Performance pass done: 813-819 fps
      (1.22 ms/frame) on the synthetic rendering NROM, 375-416 fps on the
      MMC2 title, 1029-1040 fps with PGO, and >= 375 fps with 15 competing
      CPU hogs. CI enforces a 300 fps synthetic budget, and the Windows/macOS
      check jobs plus the Android/iOS core checks cover the other targets.
      Hardware captures remain outstanding.
- [x] **A8** generalise `unko-core`. Every NES method lives on
      `unko_nes_api::NesCore` (`unko-nes-api`), implemented by `unko-nes-host`,
      and the NES-specific shared types moved with them: `PortDevice`,
      `Palette`, `VideoFilter`, `AudioFilterModel`, `AudioSource`, `RamClear`,
      `PpuResetBehavior`, `Overscan`, `MaskEdges`, `Cheat`, the PPU/APU/CPU
      debug snapshots and the `NesButtons` bit assignments. The base `Core`
      keeps only load/run/video/audio/generic input mask/sample-rate,
      save-state and SRAM. The GUI stores a `Box<dyn NesCore>` and no longer
      links `unko-nes`; `unko-cli` keeps its direct dependency for NES-specific
      analysis commands.

## M0 — core boundary (done)

- [x] Cargo workspace, MIT license, third-party notices.
- [x] `unko-core`: `Core`, `CoreInfo`, `Registry`, video/audio/input/state types.
- [x] `unko-nes`: load, frames, RGBA video, f32 audio, input, SRAM
      and versioned save states. The TetaNES adapter has been retired.
- [x] `unko-cli`: `info`, `run` (PPM screenshots), `bench`.
- [x] App shell for five targets: shared `unko_app` library with desktop,
  Android (`android_main`) and iOS (`start_winit_app`) entry points; SDL3
  gamepads on desktop and a mobile input stub; desktop release workflow;
  Android Gradle and iOS XcodeGen scaffolds. Mobile packaging/signing and
  device validation still need Xcode and the Android SDK/NDK
  (`docs/platforms.md`).
- [x] Synthetic NROM boot test; state round-trip test; registry tests.
- [x] Verified locally: 256×240 RGBA, ~937 fps / 15.6× realtime in release on a
      test ROM.

## M1 — NES product slice

- [x] Desktop shell with Dolphin-inspired UX (`unko-app`): menu bar, toolbar,
      game library (list + grid), search, player viewport, status bar, settings
      window with category rail, about dialog.
- [x] Library reads loose `.nes` and `.zip`/`.7z`/`.rar` members without
      extracting; default folder `roms/`.
- [x] Keyboard input (arrows, X/Z, Enter, Shift), pause/reset/stop, save/load
      state, screenshot, fullscreen, deterministic `verify`.
- [x] Native file dialog (`rfd`) for the ROM/save folders, integer scaling and
      a nearest/linear texture-filter option; the composite/NTSC display filter
      is offered through the core (`VideoFilter::Composite`).
- [x] Gamepad support (SDL3), rebinding UI, hot-plug, four players, a
      gamepad-to-player matrix, per-port and per-pad disables, and a Devices
      window showing port devices and every pad's assignment/connection.
- [x] Detachable Settings/Devices/Debugger OS windows on desktop (embedded
      panels on mobile), remembered layout and a confirmed reset-all that
      restores config/profiles/thumbnails while keeping saves, SRAM and
      recordings.
- [x] Library grid columns (1-8), configurable list columns and per-system
      library roots (NES scanned; SNES/N64 tags are placeholders).
- [x] Display aspect Fit/4:3/8:7, fixed 1-6x integer scale, 1-3x output
      supersampling, built-in output shader presets and window-size presets.
- [x] MiSTer accuracy options: PPU reset behaviour, RAM clear, audio source,
      overscan and edge masking, and the iNES/NES 2.0 warning.
- [x] Palette `.pal`/`.palette` import (192/768-byte binary and text) with a
      custom table, plus Vivid/Soft/PAL-like/Studio display models generated
      from the ares NTSC colorimetry (ISC). Exact hardware tables remain
      import-only; the canonical table is unchanged for oracle parity.
- [x] Audio device via `cpal`; 15-250 ms target buffer with startup prefill, stall trimming and decayed underrun recovery; Fast/Standard/High resampler kernels and NES RF/Crisp/Warm/Linear/custom output filters.
- [x] Local capture: WAV, PNG sequence and GIF.
- [x] Input display, touch controller, auto-fire and per-game gamepad profiles.
- [x] SRAM persistence keyed by ROM content hash; save-state slots with
      thumbnails.
  - [x] Core exposes `sram()` / `load_sram`; the app writes `saves/<stem>.sav`.
- [x] Frame advance, hold-to-fast-forward and 25-300% playback presets in
      the host.
- [x] Rewind: 30-frame keyframes plus per-frame inputs, 64/128/256 MiB ring,
      hold-Backspace stepping with exact state replay, plus 0-4 frame
      run-ahead (commit, draw prediction frames, restore).
- [x] Input movies (TAS): full start state plus per-frame 4-port inputs in the
      `UNKOMOV1` format, with Emulation -> Movie record/play/stop and status
      bar progress.
- [x] CLI regression mode: hash frames/audio, fresh-run vs state-resume, assert
      byte-identical tails (`unko verify`).
- [x] Versioned accuracy-core save states and retirement of the TetaNES adapter.
- [x] Test-ROM harness: `nestest`, blargg CPU/PPU/APU, `ppu_vbl_nmi`, MMC3 A12,
      and the AccuracyCoin isolated battery. Pass counts are recorded in
      `docs/accuracy-status.md`; the core tests run in `crates/unko-nes/tests/`
      and the AccuracyCoin harness in `crates/unko-nes-dev/`.
- [x] Mapper audit against a licensed ROM library: 7,101 images in four
      batches, results and ranked gaps in `docs/mapper-audit.md`. 7,073 load,
      0 unsupported, 28 failures; MMC2/MMC4 followed from it.

## M2 — NES depth

- [x] Zapper (core sensor + app mouse aiming/trigger).
- [x] Other input devices: Four Score (4-player chain with adapter signature),
      Arkanoid paddle (inverted MSB-first position + fire, mouse steering),
      SNES/Subor mouse (32-bit MSB-first report, buttons, sensitivity cycling
      and sign-magnitude motion accumulated between latches),
      Power Pad and the Family Trainer variant (12-button two-bit serial read
      with an on-screen mat), the Family BASIC keyboard (9x8 matrix with the
      documented host-key mapping), the Subor/Dendy clone keyboard (13x8
      two-nibble scan), the Famicom microphone (`$4016` D2) and
      the Family BASIC data recorder (cassette loopback on `$4016`), the
      Miracle Piano MIDI transport (port 1) and the Datach barcode scanner
      (mapper 157, UPC-A/EAN-13 modules at the documented 1000-cycle rate).
      The piano's MIDI replies and the Datach EEPROM need their real-ROM
      validation; both transports have synthetic tests. Device state is part
      of `UNKOST21` save states; the Controllers tab selects a device per
      port.
- [x] FDS (user BIOS, never bundled): core/CLI/app loading, side menu,
      disk-write persistence, 1/2/4/8x disk speed and opt-in auto side swap.
- [ ] Vs. System / PlayChoice-10 (blocked on VS BIOS/game ROMs,
      PlayChoice-10 ROMs, per-title DIP data and the DualSystem/PC10 depth).
- [x] NSF/NSFE parsing and player wiring (unit-tested; corpus validation
      pending).
- [x] Expansion audio levels and per-channel mixing: independent gain and mute for pulse 1/2, triangle, noise, DMC and expansion audio (host preferences, not save state).
- [x] NTSC/composite filter and palette selection: Raw, Composite and CRT
      (composite + scanlines + bloom) post-process filters, plus Canonical
      2C02, Consumer TV, RGB monitor and Monochrome palettes. The filters run
      on the CPU for now; a GPU shader pass remains future work. The canonical
      palette and raw filter are the defaults, so oracle parity is unchanged.
- [x] Debugger: memory viewer and breakpoint UI. Headless pieces exist:
      `unko run --dump-ppu` prints per-frame CPU/PPU/APU state and
      `--break <hex>` stops on a PC. The desktop shell adds a Debugger window
      with CPU registers/disassembly, PPU timing/palette/OAM, APU meters,
      CPU/PPU/OAM hex view + write, and PC breakpoints; F10/F9 step one
      instruction/frame while paused.
- [x] Game Genie / cheat engine.
- [x] ROM Manager: standalone DAT audit, 1G1R preferences, fixdat/CSV and safe
      rebuild (`unko-rom-manager`, Tools → ROM Manager).

## M3 — prove "multi"

- [ ] A second NES core behind the same ABI (pick by measured compatibility or
      performance gap), selected in settings.
- [ ] A second system behind the same ABI. **In progress:** the isolated
      `unko-snes` core (MIT Rust; ares as implementation source; MesenCE,
      Mednafen and RustySNES as black-box oracles) has the CPU, SPC700/DSP,
      PPU, DMA/HDMA and coprocessors implemented, with every reference title
      pixel-identical to ares and all seven reference titles audio-bit-exact
      in one configuration. `unko-snes-host` now adapts the core to
      `unko-core` (load/reset/run-frame, RGBA video, 32040 Hz stereo audio,
      input, save states, SRAM, and user-supplied IPL/CX4/NEC-DSP firmware);
      it is registered for discovery (`unko cores` lists `snes`) and the CLI
      exposes `unko snes` to run a title through the adapter (bit-exact vs
      ares for MMX2 and DKC at 600 frames). `unko-snes-api` adds the SNES
      button/region/metadata extension trait, and the desktop player
      dispatches by image signature through a `NesCore` bridge. The library
      probe now recognises SNES images and persisted `snes_ipl`/`snes_cx4`/
      `snes_dsp1` firmware paths feed the adapter, so a SNES title opens and
      runs through the real `Player::start_with` flow (bit-exact vs ares for
      MMX2 and DKC at 600 frames). The recursive library scan indexes SNES
      cartridge images and Settings exposes the IPL/CX4/NEC-DSP paths. The
      SNES debugger surface is exposed by `unko-snes-api` (CPU/PPU/DSP
      snapshots, WRAM/VRAM/OAM/CGRAM/APU peeks, and a full 65816 opcode table
      with a disassembler), surfaced through `SnesAdapter` and a dedicated
      player window: live listing at PC, a memory editor with writes into
      WRAM/VRAM/OAM/CGRAM/APU RAM/DSP registers, and single-step /
      run-to-breakpoint with 24-bit PC breakpoints. A 65816 assembler remains. Then **N64 from ares** with gopher64 as a GPL structural
      reference. See `docs/systems.md`.
- [ ] Only then consider a C ABI + dynamically loaded core plugins.
- [ ] GameCube/Wii is deferred: Dolphin is GPL-2.0+ and application-shaped, so
      it can only be a separate optional process, never a registry core.

## M4 — distribution

- [x] Desktop packages: Linux tarball and AppImage, Windows zip, macOS
      `Unko.app` zip, generated by `xtask package` / `xtask appimage`
      with deterministic timestamps and `SHA256SUMS`. Flatpak is deferred
      (needs vendored crates for an offline build).
- [ ] Web build (`wasm32`) if the frontend stack allows it.
- [ ] Optional mobile shells.
- [x] Reproducible builds, checksums, license audit in CI: the release
      workflow builds twice in different directories with path remapping and
      compares binaries and the Linux archive byte for byte
      (`docs/platforms.md`).

## Accuracy policy

The NES backend is classed `Accuracy::High`; raising the badge requires the
remaining parity and hardware-capture work, not a second core.
Do not raise the badge without a passing page of the test matrix and a
documented list of known approximations. `docs/accuracy-status.md` and
`docs/accuracy-divergences.md` hold that record.
