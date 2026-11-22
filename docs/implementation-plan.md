# Implementation plan — oracle gap closure

Dated 2026-11-13. This is the execution queue for the three oracle tiers,
ordered **easy → hard** and scoped by the project owner's decisions.

Scope decisions from this pass:

- One commit per item, easiest first; later items can be revisited when ROMs
  or hardware captures arrive.
- Cheats: Game Genie + raw RAM, 32 slots, per-game persistence.
- Rewind: configurable ring plus 1-frame run-ahead (0-4 configurable).
- Speed: 25/50/100/150/200/300%, hold-to-fast-forward, frame advance.
- Header repair and IPS/BPS patching: **decision deferred**, not in this queue.
- Video: GPU shader pipeline first, then the NTSC presets on top of it.
- Extra sprites and overclock: **not added** (deliberately not
  accuracy-preserving).
- VS. System / PlayChoice-10: **partial**. Mapper 99 banking, four-screen
  VRAM, OUT1-gated work RAM, coin/service/DIP input, the coin counter and the
  2C03/2C04/2C05 RGB palettes with 2C05 security are implemented; VS BIOS/game
  ROMs, per-title DIP defaults, the DualSystem sub-CPU and PC10 input are still
  missing.
- Peripherals: quick wins, then FDS, then the protocol-ROM items.
- Tier 3 is now in scope following the latest "all three tiers" directive;
  its items are ordered after the remaining Tier 1/2 work, easiest first.

## NES frozen (2026-11-16)

The NES track is frozen as done for now. The unreached or partial items in the
queue and in `docs/accuracy-status.md` — hardware captures, partial mapper
boards (MMC5, OneBus 256, 121, 142, T230), FDS display-only oracles, NSF corpus
validation, real peripheral validation and VS. System/PlayChoice-10 — are
deferred to a later date. Do not start new NES work unless this is reopened.

## SNES track (started 2026-11-16)

`crates/unko-snes` is the isolated MIT Rust core. ares is the primary
implementation source; MesenCE and Mednafen 1.32.1 are black-box oracles
(GPL-3.0 / GPL-2.0), and RustySNES (Apache-2.0) is a second oracle/reference.
Order:

1. ROM container, header detection and cartridge bus mapping. **Done**:
   copier header, LoROM/HiROM/ExLoROM/ExHiROM, header decode, SRAM, checksum
   mirroring and mapping tests.
2. 65816 CPU: reset/vector execution, then cycle-level bus timing. **Done**
   for instruction semantics and documented bus-cycle order: cycle-stepped
   core, emulation/native modes, decimal arithmetic, interrupts, WAI/STP,
   block moves, memory map and a 16-test `w65c816` differential oracle.
   Remaining: hardware-capture edge validation and the S-CPU DMA/IRQ scheduler.
3. SPC700 + S-DSP and the S-SMP communication ports. **Done**: all 256
   SPC700 opcodes, 64 KiB APU RAM, user IPL ROM, four bidirectional ports,
   the DSP register map, eight BRR voices, Gaussian interpolation, ADSR/GAIN,
   noise, pitch modulation and echo. Eight `spc700` differential tests plus
   DSP/APU unit and integration tests cover the slice.
4. PPU dot/scanline timing, background/sprite rendering, then the first
   frame. **Done for the base console**: modes 0-7 with hires and interlace,
   sprites, windows, mosaic, subscreen/colour math, vblank/NMI, DMA/HDMA and
   exact S-CPU wait states. Every reference title is pixel-identical to ares.
5. `unko-snes-host` adapter once the core surface is stable.
6. Coprocessors and enhancement chips. **Done**: SuperFX, SA-1, SDD1,
   SPC7110, CX4, DSP-1/2/3/4, OBC1, MSU1, S-RTC, ST018, Satellaview/BS-X and
   Sufami Turbo. Remaining: retail-audio bit-exactness for SMAS, Chrono
   Trigger and Mega Man X2, which is a per-title boot-handshake phase.

Every SNES item needs unit tests and, where possible, a black-box oracle check
against ares or RustySNES. No GPL Mesen source is read while implementing.

## Queue

| # | Item | Tier | Difficulty | Depends on | Status |
|---|---|---|---|---|---|
| 1 | 25-300% speed presets, hold-FF (Tab), frame advance | 1 | Easy | – | **done** |
| 2 | Cheats: Game Genie + raw RAM, 32 slots, per-game persistence, Cheats window | 1 | Easy-medium | – | **done** |
| 3 | Quick peripherals: Family Trainer, Subor keyboard, Famicom 3D glasses, mouse | 2 | Easy-medium | protocol notes | **done** (Family Trainer, Subor, SNES/Subor mouse, Famicom 3D glasses and the mapper-188 Karaoke microphone; the glasses latch `$4016` bit 1 (OUT1) at the field boundary and present an anaglyph) |
| 4 | Rewind ring (configurable 64-256 MB) + 1-4 frame run-ahead | 1 | Medium-hard | deterministic states | **done** |
| 5 | GPU shader pipeline (glow/wgpu) and built-in presets | 1 | Hard | renderer abstraction | **done (glow; iOS keeps the overlay fallback)** |
| 6 | NTSC Budget/Premium + brightness/sharpness as shader presets | 1 | Hard | #5 | **done** |
| 7 | FDS disk speed and automatic side swap | 2 | Hard | local `disksys.rom` | **done** (loading, BIOS, side menu, disk persistence, 1/2/4/8x speed and opt-in auto-swap) |
| 8 | Miracle Piano MIDI replies and Datach EEPROM (X24C02) | 2 | Hard | real test ROMs | **done** (X24C02 + external X24C01 persisted; documented SysEx 4/11 replies, no firmware ROM) |
| 9 | Input movies/TAS recording and playback | 3 | Medium-hard | rewind input log | **done** |
| 10 | Lua scripting (sandboxed 5.4) | 3 | Hard | scripting host API | **deferred** (needs an embedded interpreter dependency, a sandbox policy and API surface; out of this pass) |
| 11 | RetroAchievements | 3 | Hard | network + memory hashes | **blocked** (requires RA credentials/network and achievement hashing; no offline validation) |
| 12 | HD packs | 3 | Very hard | graphics pack loader | **deferred** (Mesen-style pack format and tile replacement pipeline) |
| 13 | Generalise `unko-core` (NES extension trait split) | 2 | Medium-hard | a second system, or an explicit cleanup pass | **done**: NES methods, devices, palettes, filters, cheats, button constants and debug snapshots live in `unko-nes-api`; `unko-core` keeps the generic lifecycle |
| 14 | Netplay (rollback, 2-4 players) | 3 | Very hard | deterministic states + transport | **partial**: direct UDP, hole punching and the `unko-relay` room all run the **GGRS** rollback library (`ggrs::P2PSession`) through `NonBlockingSocket` adapters, with repeat-last prediction, input delay and checksum desync detection; `unko-netplay` maps its save/load/advance requests onto `Core` and resets the core before the handshake. Relay frames are directed by player-slot member ids. The legacy transport/session remains as a library fallback and for `--engine legacy`. UPnP-IGD, RFC 5389 STUN, a host/join window with player/slot selection and a manual public endpoint are included; LAN paths pass with the two-host harness, the GGRS harness passes on loopback against the real NES core, and a true Internet run needs two networks |

The remaining blocked assets are the VS. System BIOS/game ROMs and the
PlayChoice-10 game images; the transports and scanners are already
synthetic-tested (see `docs/accuracy-status.md`).

## Deferred / out of scope

- Lua scripting, RetroAchievements, HD packs and netplay: tracked with their
  blocked/deferred status in the queue above; each needs a dependency that is
  out of this pass (interpreter sandbox, credentials/network, pack format,
  rollback transport).
- Extra sprites / sprite-limit toggle and CPU/PPU overclock.
- VS. System / PlayChoice-10: **paused by the owner**. Mapper 99, cabinet
  input, RGB PPU palettes, 2C05 security and 13 staged local images are done;
  a 2C05 title, a known-good DualSystem dump, per-title DIP defaults, PC10
  iNES images and the PC10 second-CPU/input layer remain.
- `unko-core` generalisation: complete. NES methods and vocabulary live in
  `unko-nes-api`; `unko-core` is the generic contract.
- NES 2.0 header editor, repair database and IPS/BPS soft-patching (owner
  deferred the decision). The warning for missing NES 2.0 headers is in.

## Gates per item

Each item must keep `cargo test --workspace`, clippy `-D warnings`, fmt and
the AccuracyCoin isolated battery green, and add focused unit/integration
tests for the new behavior. Display-only changes must leave the canonical
palette + Raw filter trace identical.
