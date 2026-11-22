# NES and SNES oracle emulators — feature matrix

A feature-level survey of the emulators Unko uses as oracles or peers,
compared with Unko's current shell. It is input for prioritisation, not a
commitment: the "gaps" section lists candidates, and the project owner picks
what to build.

Method: public documentation and release notes, plus the locally checked-out
reference sources (`refs/ares`, `refs/mesence`, `refs/mednafen-1.32.1`,
`refs/rustynes`) for behaviour that the docs do not state. No GPL code is
copied; only user-visible features and documented behaviour are recorded.

Legend: **Yes** / **Partial** / **No**; "—" means not part of that
emulator's scope.

## At a glance — emulation

| | ares | MesenCE | FCEUX | Nestopia UE | Nintendulator/NRS | puNES | RustyNES | **Unko** |
|---|---|---|---|---|---|---|---|---|
| Accuracy model | cycle-level, per-dot PPU | cycle-level, per-dot | older cycle-ish core, TAS-oriented | cycle-accurate CPU, dot PPU | cycle-accurate, quirk-focused | cycle-accurate (2nd to Mesen on test-ROM rankings) | one master clock, per-half-cycle | dot-locked CPU/PPU, per-cycle bus |
| AccuracyCoin | — (not claimed) | 100% claimed (v2.x) | not claimed | not claimed | test-ROM reference testbed | ranked high | 146/146 | **146/146 non-DRAW (146/151, 5 DRAW)** |
| Mappers | many boards; NES core subset | 290+ | broad | broad + NstDatabase | NRS: all NES 2.0 mappers | broad + UNIF | 208 families | **245 implementations incl. 446** |
| FDS | Yes | Yes | Yes | Yes | Yes | Yes (QD too) | Yes (real BIOS) | **core/CLI/desktop loading with a real BIOS; disk speed and auto-swap queued** |
| VS. System / PlayChoice | VS | VS | VS (fixed list) | VS + PlayChoice-10 | VS | VS Uni + Dual | VS + PlayChoice-10 | **Partial** (mapper 99, input and RGB PPUs; DualSystem sub-CPU/PC10 assets pending) |
| PAL / Dendy | region models | Yes | Yes (NTSC-PAL hybrid too) | Yes | Yes | Yes | Yes | **Yes** |
| Expansion audio | VRC6/7, MMC5, N163, 5B, FDS, EPSM | VRC6/7, MMC5, N163, 5B, FDS, EPSM | VRC6/7, MMC5, N163, 5B, FDS | VRC6/7, MMC5, N163, 5B, FDS | VRC7, MMC5, N163, 5B, FDS (DLLs) | VRC6/7, MMC5, N163, 5B, FDS | VRC6/7, MMC5, N163, 5B, FDS | **VRC6/7, MMC5, N163, 5B, FDS** |
| Devices | gamepad, Zapper, Family BASIC keyboard, data recorder | pad, Zapper, Power Pad, Four Score, paddle, keyboard, microphone, more | pad, Four Score, Zapper, Power Pad, Arkanoid, Space Shadow, Family Keyboard | pad, Zapper, Power Pad, Four Score, exotic peripherals | pad, Zapper, Four Score, Game Genie | pad, Zapper, Power Pad, paddle, Family/Subor keyboard, tape, microphone | Four Score, Zapper, paddle, Power Pad, keyboards, mouse | **pad, Zapper, Paddle, Power Pad + Family Trainer, Four Score, Family BASIC keyboard, Subor keyboard, microphone, tape loopback, Miracle Piano transport with SysEx replies and a 49-key input, Datach scanner + X24C02/X24C01 EEPROMs** |
| Overclocking | No | Yes | No | 2x option | No | PPU overclock | speed control | No |
| Sprite-limit toggle | No | Yes | No (sprite limit disable via CLI) | Yes | No | No | Yes (enhancement) | No |

## At a glance — player and creator features

| | ares | MesenCE | FCEUX | Nestopia UE | Nintendulator | puNES | RustyNES | **Unko** |
|---|---|---|---|---|---|---|---|---|
| Save states | Yes | Yes | Yes | Yes | Yes | Yes | Yes | **Yes** |
| Rewind | Yes | Yes | Yes | frontend (libretro) | No | Yes (timeline) | Yes | No |
| Fast-forward / speed control | Partial | Yes | Yes | Yes | Yes | Yes (up to 64x) | 25–300% + fast-forward | No (debugger step only) |
| Frame advance | debugger | debugger | Yes | Yes | debugger | Yes | Yes | debugger only |
| Run-ahead | No | No | No | No | No | No | Yes | No |
| Game Genie / cheats | cheats editor | Yes | Yes (GG + RAM) | cheat manager | GG (3 codes) | Yes | GG database + RAM | No |
| Netplay | No | Yes | experimental (2.7) | Yes | No | Yes | rollback, 2–4p | No |
| Movies / TAS | trace | movie recording | rich TAS editor + movies | movie recording | movie recording | No | TAStudio + movie import | No (WAV/PNG/GIF only) |
| Debugger | memory editor, tracer | full debugger suite | full (RAM search, trace, hex, PPU/nametable) | no | full (breakpoints, viewers, palette) | no | conditional bp, watchpoints, hex, callstack | **CPU/PPU/APU + hex + PC breakpoints** |
| Video filters / shaders | slang shaders | NTSC, xBRZ, HD packs | NTSC filters, palettes | Blargg NTSC, HQx, scanlines | palette editor, scanlines | many (NTSC, CRT, xBRZ, hqNx) | NTSC + CRT shader stack | **raw/composite/CRT + GPU scanline/LCD/CRT/NTSC Budget/NTSC Premium presets with brightness/sharpness** |
| Palettes | several | many + custom | many + `.pal` | many + `.pal`, palette editor | palette editor + custom | many + editor + Raw | `.pal` + generated NTSC | **8 presets + `.pal`/`.palette` import** |
| HD packs | No | Yes | No | No | No | No | Yes | No |
| Lua / scripting | No | debugger scripting | Yes (Lua 5.1) | No | No | No | Yes (sandboxed Lua 5.4) | No |
| RetroAchievements | No | Yes (libretro) | No | No | No | No | Yes | No |
| Recording | WAV/streams | video+audio | AVI + WAV, screenshots | AVI/WAV, screenshots | AVI | FFmpeg video/audio | A/V recording | **WAV/PNG/GIF** |
| Archive load | No | Yes | Yes | ZIP/7z | No | Yes | Yes | **ZIP/7Z/RAR** |
| IPS/BPS patching | No | IPS/BPS | IPS | IPS | No | Yes | soft-patching | No |
| Platforms | Win/mac/Linux | Win/mac/Linux, libretro | Win/Linux/mac (Qt) | Win/mac/Linux/BSD | Windows | Win/Linux/BSD/mac | Win/mac/Linux/web/Android/iOS | **Linux/Win/mac/iOS/Android** |
| License | ISC | GPL-3.0 | GPL-2.0 | GPL-2.0 | GPL-2.0 | GPL-2.0 | GPL-3.0 | MIT (+ ISC adaptations) |

## Notes per emulator

### ares (ISC)
Multi-system accuracy project (higan/bsnes lineage). Its Famicom core has a
per-dot PPU, an FDS implementation, VS support, expansion audio including
EPSM, and expansion-port devices: the Family BASIC keyboard, Famicom data
recorder, and EPSM. Controller ports support the standard pad and Zapper.
Desktop features include save states, a rewind buffer, a cheat editor, a
memory editor, a tracer, tape support, and slang shader presets. It has no
netplay, run-ahead, HD packs or achievements. Source: local `refs/ares`
(`ares/fc/expansion`, `desktop-ui/tools`, `desktop-ui/program/rewind.cpp`).

### MesenCE (GPL-3.0)
The reference for feature breadth: 290+ mappers, FDS, VS, Dendy, a
per-dot PPU with the documented quirks, HD packs, NTSC and xBRZ filters,
per-channel audio with stereo/reverb, overclocking, rewind, netplay, cheats,
movie recording, and a deep debugger with memory search, palette/nametable
viewers and scripting. Libretro exposes states/rewind/netplay/cheats/
achievements. Source: libretro docs, emulation wiki, local `refs/mesence`.

### FCEUX (GPL-2.0)
The TAS and ROM-hacking workbench: powerful movie/TAS editor, RAM
search/watch, hex editor, trace logger, code/data logger, Lua 5.1 scripting,
Game Genie encode/decode, palette editor, AVI capture, and a broad device set
(Four Score, Zapper, Power Pad, Arkanoid, Space Shadow, Famicom 4-player,
Family Keyboard). Netplay is basic/experimental in 2.7. Accuracy is not its
selling point; it is useful as a feature benchmark and as a movie/TAS format
source.

### Nestopia UE (GPL-2.0)
Cycle-accurate CPU and dot PPU, NstDatabase.xml for region/mapper/4-player
detection, VS and PlayChoice-10, FDS, exotic peripherals (Karaoke Studio,
Barcode Battler, Datach, data recorder, Turbo File), `.pal` palettes and a
palette editor, Blargg NTSC filters, HQx/scanline shaders, cheat manager,
movies, IPS patching, and a 4-player adapter option. Rewind is provided by
libretro, not the standalone app. Source: libretro docs, project readme,
Nestopia UE site.

### Nintendulator / NintendulatorNRS (GPL-2.0)
The quirk-focused test bed. NRS supports every documented NES 2.0 mapper,
famiclone chips (VT02/VT03/VT09/VT32), VS DualSystem and per-device key
configuration. Features: Game Genie (3 codes), 4-player, movies, AVI, a
full debugger (read/write/execute/opcode/interrupt breakpoints, CPU regs,
PPU/sprite/palette memory, scanline breakpoints), palette editor and header
editor. No netplay, shaders, HD packs or rewind. Windows-first.

### puNES (GPL-2.0)
Ranked second to Mesen on public test-ROM rankings. Broad mappers + UNIF, FDS
(with QD), VS Uni/Dual, NSF/NSF2/NSFe, rewind timeline with 1–64x scrubbing,
netplay, GLSL shaders plus NTSC/CRT filters, palette editor + "Raw" palette
for shader decoding, cheats, per-game PPU overclock, FFmpeg video/audio
recording, Family/Subor keyboard, tape and microphone support.

### Mednafen (GPL-2.0)
A multi-system emulator. Its NES/FDS core is useful as an additional
black-box peer for boot, mapper and FDS audio comparisons; it is not the
primary NES oracle (RustyNES/MesenCE are). Its SNES peers are described in the
SNES section below.

### RustyNES (GPL-3.0)
The closest feature peer to Mesen and Unko's primary differential oracle:
208 mapper families, real-BIOS FDS, VS/PlayChoice, rollback netplay,
RetroAchievements, TAStudio, HD packs, CRT shader stack, `.pal` palettes,
save states, rewind, run-ahead, 25–300% speed, Game Genie database, raw RAM
cheats, Lua 5.4, movie formats, and a debugger with conditional breakpoints,
watchpoints, RAM search, hex editor and callstack. Also ships web/mobile/
libretro builds. Source: local `refs/rustynes/README.md`.

## Remaining gaps (2026-11-13)

This is the current answer to "what is still missing from the oracles". The
matrix above is the comparison; this list is the work queue.

**Implemented since the first survey**
- Per-channel audio mixer (5 APU channels + expansion), latency target,
  resampler quality, NES RF/Crisp/Warm/Linear/custom filters.
- `.pal`/`.palette` import and eight display presets (four generated from the
  ares ISC colorimetry).
- Output scaling/aspect/supersampling, window presets and scanline/LCD/CRT
  output presets.
- Microphone, Family BASIC tape loopback, Miracle Piano transport with SysEx
  replies and a 49-key input, and the Datach barcode scanner with its internal X24C02 and
  cartridge X24C01 EEPROMs (bit-banged protocol unit-tested; real Datach ROM
  smoke-tested, save-screen validation pending).
- MiSTer accuracy options: PPU reset behaviour, RAM clear, audio source,
  overscan, edge masking, and the NES 2.0 missing-header warning.
- Per-system library roots (NES scanned), grid/list columns, detachable
  windows, Devices matrix and reset-all.

**Implemented player-facing**
1. **Cheats**: Game Genie (6/8-character with compare) and raw RAM, up to 32
   entries per game, enable/disable and per-game persistence.
2. **Speed, rewind and movies**: 25-300% presets, hold-fast-forward, frame
   advance, exact rewind (30-frame keyframes + input log, 64-256 MiB),
   run-ahead 0-4 frames and `UNKOMOV1` input movies.
3. **GPU shader presets**: scanlines/LCD/CRT/NTSC Budget/NTSC Premium as glow
   fragment shaders with brightness/sharpness.

**Remaining gaps**
1. **Arbitrary shader loading** (GLSL/WGSL/slang) and MiSTer-style
   HQ2x/CRT-25/50/75 scaling. The built-in presets are shader-based, but the
   NTSC Budget/Premium filters are 3/7-tap approximations rather than a full
   composite signal model.
2. **IPS/BPS soft-patching and a NES 2.0 header editor/repair database**
   (missing-header warning only today).
3. **VS. System / PlayChoice-10**: mapper 99 banking, four-screen VRAM,
   OUT1-gated work RAM, coin/service/DIP input, the coin counter, NES 2.0
   UniSystem/DualSystem type parsing, the 2C03/2C04-xxxx/2C05 palettes and
   2C05 security are in. Remaining: VS BIOS/ROM boot validation, per-title DIP
   defaults, the DualSystem sub-CPU/24 KiB half selection/watchdog and PC10
   game ROMs/input. PlayChoice-10 BIOS is available.
4. **FDS depth**: loading, BIOS selection, side switching, disk-write
   persistence, 1/2/4/8x disk speed and opt-in auto side swap are in.
5. **Peripherals**: the SNES/Subor mouse, Famicom 3D System (anaglyph
   display model), Karaoke Studio microphone (mapper 188), Miracle Piano
   SysEx replies and the Datach external X24C01 (Battle Rush) are
   implemented.
6. **Optional accuracy knobs**: sprite-per-line limit / extra sprites,
   CPU/PPU overclock. Deliberately not added (not accuracy-preserving).
7. **Creator/community**: TAStudio-style movie editing, netplay (rollback
   2-4 players), Lua scripting, RetroAchievements and HD packs.

ROM management, multi-system work and distribution gaps are tracked in
`docs/roadmap.md` and `docs/compatibility.md`.

## SNES oracle matrix

The SNES core is `unko-snes` (MIT Rust). Its oracle set is smaller than the
NES set because ares is both the primary implementation source and the first
behavioural reference.

| | ares | MesenCE | Mednafen | RustySNES | **Unko** |
|---|---|---|---|---|---|
| Role | implementation source | black-box compatibility oracle | black-box peer (primary `snes` + experimental `snes_faust`) | Apache-2.0 oracle/reference | isolated core |
| Language / license | C++ / ISC | C++ / GPL-3.0 | C++ / GPL-2.0 | Rust / Apache-2.0 | Rust / MIT |
| Accuracy model | cycle-level, dot PPU | cycle-level, dot PPU | bsnes v0.059 fork with scheduler/timing changes; Faust core is approximate | claims Mesen2/ares/higan bar; still early | target: cycle-level |
| 65816 | Yes | Yes | Yes | Yes | planned |
| SPC700 + S-DSP | Yes | Yes | Yes (SPC playback only in `snes_faust`) | Yes | planned |
| PPU | dot/scanline, hires/interlace | dot/scanline, hires/interlace | Yes, hires and interlace | Yes | planned |
| Coprocessors | SuperFX, SA-1, SDD1, SPC7110, CX4, DSP1-4, OBC1, ST018 | broad | CX4, DSP1/2/4, OBC1, SA-1, S-DD1, SPC7110, SuperFX | claims DSP-1/2/4, SuperFX, SA-1, CX4, S-DD1, SPC7110 | later |
| Enhancement | MSU1, Satellaview, Sufami Turbo, BS-X | MSU1, BS-X | BS-X; MSU1 in `snes_faust` | partial | later |
| Save states / rewind | Yes / Yes | Yes / Yes | Yes / Yes (`snes` state caveat) | Yes | planned |
| Netplay | No | Yes | Yes | — | GGRS (shell) |
| Platforms | Win/mac/Linux | Win/mac/Linux, libretro | Win/Linux/mac/BSD | native + WebAssembly | Linux/Win/mac/iOS/Android |
| Local source | `refs/ares` | `refs/mesence` | `refs/mednafen-1.32.1` | not checked out | — |

### Notes

- **ares** is the implementation source because it is ISC and the closest
  silicon model. Its `sfc/` tree and board manifests can be read and adapted
  with attribution.
- **MesenCE** is the broad compatibility oracle. It is GPL-3.0: run it
  black-box, do not read or translate its source.
- **Mednafen** (GPL-2.0) is a multi-system peer. Its primary SNES module
  (`snes`) is a fork of bsnes v0.059 with a Cx4 Op10 fix, scheduler/timing
  changes and S-SMP/SuperFX optimizations; it implements the Cx4, DSP-1/2/4,
  OBC1, SA-1, S-DD1, SPC7110, SuperFX and BS-X. The experimental `snes_faust`
  module is approximate but adds MSU1, CX4, DSP-1/2 (HLE), S-DD1, SA-1,
  SuperFX and SPC/SNSF playback. Save states, rewind and netplay exist, though
  the `snes` module warns states can break some games (e.g. Tales of
  Phantasia). Use it as a black-box differential/feature peer only; it is GPL,
  so do not read or translate its source into this MIT project. Its NES/FDS
  core likewise makes it an additional NES/FDS peer. Source: local
  `refs/mednafen-1.32.1` (`Documentation/snes.html`,
  `Documentation/snes_faust.html`, `src/snes`, `src/snes_faust`).
- **RustySNES** (<https://github.com/doublegate/RustySNES>) is a pure-Rust
  Apache-2.0 emulator targeting the Mesen2/ares/higan accuracy bar, including
  coprocessor LLE cores and WebAssembly support. It joins the oracle set for
  differential runs and feature coverage. It is not copied wholesale; any
  future reuse needs a per-module provenance review.
- **w65c816** (MIT) is a 65C816 cross-check oracle only. It drives the
  differential CPU tests; it is not linked into `unko-snes`.

Public SNES documentation used by the implementation:
[SNESdev Wiki](https://snes.nesdev.org/wiki/SNESdev_Wiki),
[Super Famicom Development Wiki](https://wiki.superfamicom.org/) and
[awesome-snesdev](https://github.com/Scottcjn/awesome-snesdev).

## Local oracle revisions (checked 2026-11-22)

| Oracle | Revision / version | Local build |
|---|---|---|
| ares | `5c989a5` (oracle capture hooks rebased) | `refs/ares/build_headless/desktop-ui/ares` |
| MesenCE | `a8e5d02` | `refs/mesence/bin/linux-x64/Release/linux-x64/publish/Mesen` |
| Mednafen | 1.32.1 | `refs/mednafen-1.32.1/src/mednafen` |
| RustySNES | `9f7a1d7` | `refs/RustySNES/target/release` (libretro/harness) |
| RustyNES | `b150717` | `refs/rustynes/target/release/librustynes.so` |
| gopher64 | `5c6aacc` | `refs/gopher64/target/release/gopher64` |
| TriCNES | `c4f13fc` | C#/.NET Framework; source-only here |
| Dolphin | `66df2b4e18` | `refs/dolphin/build_headless/Binaries/dolphin-emu` (not an NES/SNES oracle) |

The ares update from `78bc8ed` to `5c989a5` contains PS1 fixes only; the SNES
behavioural reference is unchanged. MesenCE, RustySNES, RustyNES and TriCNES
were already at their upstream heads.

## Licensing and provenance

ares is ISC; FCEUX, Nestopia UE, Nintendulator, puNES and Mednafen are
GPL-2.0; MesenCE and RustyNES are GPL-3.0; RustySNES is Apache-2.0. None of their code, assets
or databases may be ported into this MIT project without a provenance review.
The observations above are limited to documented, user-visible features and
behaviour. Palette tables are factual colour data, but any imported set will
be recorded with its source and kept in a data file rather than copied from a
GPL implementation unless the license permits it.

## Sources

- ares: <https://ares-emu.net>, local `refs/ares` (`ares/fc`, `desktop-ui`).
- Mednafen: <https://mednafen.github.io/>, local `refs/mednafen-1.32.1`
  (`Documentation/snes.html`, `Documentation/snes_faust.html`).
- MesenCE: <https://docs.libretro.com/library/mesen2/>,
  <https://emulation.gametechwiki.com/index.php/Mesen>,
  local `refs/mesence`.
- FCEUX: <https://fceux.com/web/fceux-sdl-docs.html>,
  <https://fceux.com/web/help/LuaScripting.html>,
  <https://fceux.com/web/home.html>.
- Nestopia UE: <https://docs.libretro.com/library/nestopia/>,
  <https://github.com/0ldsk00l/nestopia/blob/master/readme.html>,
  <https://nestopiaue.com/>.
- Nintendulator: <https://emulation.gametechwiki.com/index.php/Nintendulator>,
  <https://forums.nesdev.org/viewtopic.php?t=18250>.
- puNES: <https://emulation.gametechwiki.com/index.php/PuNES>,
  <https://github.com/punesemu/puNES>.
- RustyNES: local `refs/rustynes/README.md`.
- RustySNES: <https://github.com/doublegate/RustySNES> (Apache-2.0 oracle).
- SNESdev Wiki: <https://snes.nesdev.org/wiki/SNESdev_Wiki>.
- Super Famicom Development Wiki: <https://wiki.superfamicom.org/>.
- awesome-snesdev: <https://github.com/Scottcjn/awesome-snesdev>.
- w65c816: <https://crates.io/crates/w65c816> (MIT CPU cross-check oracle).
