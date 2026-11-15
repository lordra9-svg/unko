# Compatibility

This is the current compatibility picture for the NES core and desktop shell.
It is deliberately evidence-based: a check mark means a public test, a synthetic
test, or a smoke audit covers it; a dash means no verdict yet.

## Compatibility levels

| Level | Meaning |
|---|---|
| **Verified** | Public reference suite or checked-in unit test passes |
| **Smoke-tested** | Loads and runs in the licensed-library audit; not a completion claim |
| **Loads only** | The mapper/format loads, but no rendering or behaviour verdict |
| **Blocked** | Missing ROM, BIOS, hardware or credentials |

## Systems and formats

| Target | Status | Notes |
|---|---|---|
| NES / Famicom (NTSC 2C02) | **Verified** | Default core; AccuracyCoin 146/146 non-DRAW |
| NES PAL / Dendy | **Partial** | PPU timing implemented; PAL APU frame periods now pass 10/10 in PAL mode |
| Famicom Disk System | **Verified** | Core, CLI (`--bios`) and desktop loader with a user `disksys.rom`; 1/2/4/8x disk speed, manual side selection and optional auto side-swap; FDS audio is bit-exact against RustyNES for 10.9 s, with a residual at one table upload |
| NSF / NSFE | **Loads only** | Parser and player wiring are unit-tested; corpus validation pending |
| VS. System | **Partial** | Mapper 99 banking, four-screen VRAM, 2 KiB OUT1-gated work RAM, coin/service/DIP input, coin counter and the 2C03/2C04/2C05 RGB palettes with 2C05 security implemented; VS BIOS/game ROMs, per-title DIP defaults and the DualSystem sub-CPU are still missing |
| PlayChoice-10 | **Partial** | 2C03 RGB palette implemented; PC10 BIOS is present; no PC10 game ROMs or coin/service input |
| SNES / N64 | **Not started** | ares-based multi-system work is queued |

## ROM containers and patch formats

| Format | Status |
|---|---|
| iNES / NES 2.0 | **Verified** |
| Raw `.fds`, loose or in ZIP/7z/RAR | **Supported** |
| `.zip`, `.7z`, `.rar` containers | **Supported** (no extraction to disk) |
| IPS / BPS soft patches | **Not implemented** |
| UNIF | **Implemented** (`.unif`/`.unf`, chunks to iNES 1.0/NES 2.0; Nintendo and 60+ documented third-party/licensed boards, `MIRR`/`BATR`/`TVCI`/`VROR`; unknown boards are rejected) |
| Per-game region, port-device and mapper override | **Supported** (`Tools → Game properties…`, stored as `<stem>.game`) |
| NES 2.0 header editor / repair database | **Not implemented** (missing-header warning only) |

The library scanner now recognizes `.fds` files and FDS members inside
archives. The desktop app reads the BIOS path from **Settings → Paths → FDS
BIOS**; it never bundles the BIOS. FDS write-mode changes are persisted to
`<save dir>/<stem>.fds`, and the Emulation menu exposes disk-side selection.

## Mapper coverage

- **7,073 of 7,101** images in the local licensed library load (99.6%).
- **0 unsupported mapper IDs**; the remaining 28 are truncated dumps.
- **285 distinct mapper IDs** appear in the library.
- 20-frame smoke audit: **3,897 rendering / 3,176 blank**.
- 120-frame audit: **6,503 rendering / 570 blank**.
- 240-frame audit: **6,427 rendering / 646 blank**.

“Rendering” means the final sampled frame was not a single uniform colour; it
is not a completion claim. Per-mapper counts and the remaining long-tail work
are in [`mapper-audit.md`](mapper-audit.md).

Public mapper-specific status:

| Area | Status |
|---|---|
| MMC1 / MMC3 / MMC5 / MMC6 | MMC3/MMC6 tests pass; use the alternate-revision selector for revision-A titles |
| VRC2/4/6/7 | Expansion audio verified per-cycle; VRC6 sawtooth follows the wiki/Mesen schedule |
| FME-7, Namco 163, Sunsoft 5B | Expansion audio verified; FME-7 IRQ/audio fixes are unit-tested |
| Unlicensed long tail | Best-effort; save-state round-trip and smoke audit only |

## Peripherals

| Device | Status |
|---|---|
| Standard pad, Four Score | **Verified** |
| Zapper | **Verified** (core sensor + mouse aiming) |
| Arkanoid paddle | **Verified** |
| Power Pad / Family Trainer | **Verified** |
| Family BASIC keyboard | **Verified** |
| Subor/Dendy keyboard | **Verified** |
| Famicom microphone | **Implemented** (manual checkbox + default system input via a peak gate; no analog ADC capture) |
| Data recorder tape loopback | **Synthetic/partial** |
| Miracle Piano transport | **Implemented** (transport, documented SysEx 4/11 replies (default firmware 2.03) and an on-screen 49-key note input (mouse or computer keyboard)) |
| Datach scanner + internal X24C02 | **Verified** (synthetic I2C tests + real-ROM smoke) |
| Datach external X24C01 | **Implemented** (128-byte X24C01 on `$8000-$8003` bit 3, shared SDA; NES 2.0 PRG-NVRAM flag selects the 384-byte save) |
| SNES/Subor mouse | **Verified** (32-bit report, buttons, sensitivity and sign-magnitude motion in unit tests) |
| Famicom 3D glasses | **Implemented** (expansion-port `$4016` bit 1 (OUT1) field latch; red/cyan anaglyph output; unit-tested, no hardware capture) |
| Karaoke Studio | **Implemented** (mapper 188 mic A/B + 1-bit ADC at `$6000-$7FFF`; A/B from controller port 1; unit-tested, no local ROM) |

## Player features

| Feature | Status |
|---|---|
| Portable data root (`unko.conf`, ROMs, saves, states, cheats, movies, captures) | **Done** (`--data-dir`, `UNKO_HOME`, `--portable`; relative config paths) |
| Save states, 10 slots, thumbnails | **Done** |
| Exact rewind, 64–256 MiB ring | **Done** |
| Run-ahead 0–4 frames | **Done** |
| Input movies / TAS (`UNKOMOV1`) | **Done** |
| ROM Manager (DAT audit, 1G1R, fixdat/CSV, safe rebuild) | **Done** (standalone `unko-rom-manager`; loose/ZIP/7z/RAR input, deterministic ZIP, TorrentZip-style and 7z output; no CHD or patching) |
| Game Genie + raw RAM cheats | **Done** |
| Speed presets, fast-forward, frame advance | **Done** |
| GPU shader presets (scanlines/LCD/CRT/NTSC Budget/Premium) | **Done** (glow; iOS fallback) |
| Arbitrary GLSL/WGSL/slang shaders | **Not implemented** |
| IPS/BPS patches, header repair DB | **Not implemented** |
| Netplay (rollback, 2-4 players) | **Done** (direct/UDP/hole-punch/relay; LAN-validated with two hosts, Internet path implemented) |
| Lua, RetroAchievements, HD packs | **Not implemented** |

## Evidence

| Check | Current result |
|---|---|
| AccuracyCoin 3.1.0 isolated | 146/146 non-DRAW; 146/151 total, 5 DRAW, 0 failed |
| `nestest` | 8,980/8,980 lines exact |
| blargg CPU / PPU / APU | CPU and PPU suites green; APU 13/13 |
| x0000 APU write-window tests | 10/10 pass |
| NESdev archive | 137 pass / 11 fail / 115 visual/audio/input-only; PAL APU timing leaves 4 failures, adding the alternate MMC3 selector leaves only `read_joy3/count_errors` |
| RustyNES test tree | 159 pass / 11 fail / 171 visual/audio/input-only; 166 / 4 with PAL APU in PAL mode, 1 with the alternate MMC3 selector too |
| Workspace tests | 680 pass, 0 failed |

The actionable accuracy failures are tracked in
[`accuracy-divergences.md`](accuracy-divergences.md); the ROM/BIOS inventory is
in [`test-rom-inventory.md`](test-rom-inventory.md).

## Missing BIOS / firmware

| Asset | Needed for |
|---|---|
| `disksys.rom` | FDS; local copies exist, never bundled |
| VS. System BIOS + game ROMs | Boot validation and per-title DIP defaults (mapper 99 and input are implemented) |
| PlayChoice-10 game ROMs | PC10 compatibility (BIOS already available) |
