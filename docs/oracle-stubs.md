# Oracle and reference stubs

This is a **stub register**, not a support matrix. It inventories the emulators,
reference implementations, hardware descriptions and test-ROM suites kept under
`/var/home/Lordra/ai/refs` so that a future Unko system or feature has a named
place to start. Being listed here means "this exists locally and could inform a
future core"; it does **not** mean Unko supports the system, links the code, or
has verified it.

Each row records what the project is, the system it covers, the upstream project
(and URL where known), the licence stated by the checked-out `LICENSE` /
`COPYING` / `README` (or `unverified` when there is no checkout to read), the
local path, a role, and whether it is wired into Unko today.

Roles:

- **oracle** — behaviour/timing truth, consulted to answer "what does the
  hardware do?"
- **peer** — black-box comparison against another emulator's output.
- **hardware reference** — RTL or HDL describing the hardware itself.
- **test suite** — ROMs or tests that assert behaviour.

Only emulators and reference material are listed. ROMs, BIOS images, firmware,
keys, NAND dumps and game data in the same folder are intentionally excluded and
must not be added to the repository.

## Register

| Project | System | Upstream | Licence | Refs path / file | Role | Status |
|---|---|---|---|---|---|---|
| ares | NES, SNES (and other systems) | https://github.com/ares-emulator/ares | ISC (MAME-derived ROM helpers are BSD-3-Clause) | `ares/` | oracle | wired |
| MesenCE | NES | https://github.com/nesdev-org/MesenCE | GPL-3.0 | `mesence/` | oracle | wired (external process only) |
| Mednafen 1.32.1 | NES, SNES, PS1, and others | mednafen.github.io (tarball) | GPL-2.0 | `mednafen-1.32.1/` | oracle | wired |
| RustyNES | NES | https://github.com/doublegate/RustyNES | GPL-3.0-or-later | `rustynes/` | oracle | wired (differential trace host, external only) |
| RustySNES | SNES | https://github.com/doublegate/RustySNES | MIT OR Apache-2.0 | `RustySNES/` | oracle | wired |
| MiSTer NES | NES (RTL) | https://github.com/MiSTer-devel/NES_MiSTer | GPL-3.0 | `mister-nes/` | hardware reference | wired |
| MiSTer SNES | SNES (RTL) | https://github.com/MiSTer-devel/SNES_MiSTer | GPL-3.0 | `mister-snes/` | hardware reference | wired |
| MiSTer SNES (srg320) | SNES (RTL) | https://github.com/srg320/SNES_MiSTer | GPL-3.0 | `mister-snes-dev/` | hardware reference | wired |
| openfpga-NES | NES (RTL) | https://github.com/agg23/openfpga-NES | GPL-3.0 | `openfpga-nes/` | hardware reference | wired |
| openfpga-SNES | SNES (RTL) | https://github.com/agg23/openfpga-SNES | GPL-3.0 | `openfpga-snes/` | hardware reference | wired |
| TriCNES | NES | https://github.com/100thCoin/TriCNES | MIT | `tricnes/` | peer | available, not wired |
| snes-test-roms | SNES | https://github.com/undisbeliever/snes-test-roms | zlib | `snes-test-roms/` | test suite | reference |
| akatsuki-snes-test-roms | SNES | https://github.com/akatsuki105/snes-test-roms | unverified (mixed per subdirectory) | `akatsuki-snes-test-roms/` | test suite | reference |
| Dolphin | GameCube, Wii | https://github.com/dolphin-emu/dolphin | GPL-2.0-or-later | `dolphin/` | oracle | stub |
| gopher64 | Nintendo 64 | https://github.com/gopher64/gopher64 | GPL-3.0 | `gopher64/` | oracle | stub |
| Cemu 2.6 | Wii U | cemu-project/Cemu | unverified | `Cemu-2.6-x86_64.AppImage` | oracle | stub |
| decaf-emu | Wii U | decaf-emu/decaf-emu | GPL-3.0 | `decaf-emu-release.tar.gz` | oracle | stub |
| DuckStation | PlayStation | stenzek/duckstation | unverified | `DuckStation-x64.AppImage` | oracle | stub |
| PCSX2 | PlayStation 2 | PCSX2/pcsx2 | unverified | `pcsx2-v2.9.183-linux-appimage-x64-Qt.AppImage` | oracle | stub |
| Play! | PlayStation 2 | jpd002/Play- | BSD-2-Clause | `Play--0.77.zip` | peer | stub |
| PPSSPP | PSP | hrydgard/ppsspp | unverified | `PPSSPP-v1.20.4-anylinux-x86_64.AppImage` | oracle | stub |
| RPCS3 | PlayStation 3 | RPCS3/rpcs3 | unverified | `rpcs3-v0.0.43-20640-1808e241_linux64.AppImage` | oracle | stub |
| shadPS4 | PlayStation 4 | shadps4-emu/shadPS4 | unverified | `shadps4-linux-sdl-2026-11-19-443b954.zip` | oracle | stub |
| KytyPS5 | PlayStation 5 | https://github.com/KytyPS5/KytyPS5 | GPL-2.0 | `KytyPS5-KytyPS5-2026-11-19-5ad27cd.zip` | oracle | stub |
| Vita3K | PlayStation Vita | Vita3K/Vita3K | unverified | `Vita3K-x86_64.AppImage` | oracle | stub |
| xemu | Xbox | xemu-project/xemu | unverified | `xemu-0.8.139-x86_64.AppImage` | oracle | stub |
| xenia-canary | Xbox 360 | xenia-canary/xenia-canary | unverified | `xenia_canary_linux.AppImage` | oracle | stub |
| Cxbx-Reloaded | Xbox | Cxbx-Reloaded/Cxbx-Reloaded | GPL-2.0 | `Cxbx-Reloaded-CI-585c49a.zip` | oracle | stub |
| melonDS | Nintendo DS | melonDS-emu/melonDS | unverified | `melonDS-x86_64.AppImage.zip` | oracle | stub |
| mGBA | Game Boy Advance | mgba-emu/mgba | unverified | `mGBA-build-2026-11-18-appimage-x64-9186-9fcaa882f25de40258f9578480854e590db12699.appimage` | oracle | stub |
| SameBoy | Game Boy, Game Boy Color | LIJI32/SameBoy | Expat (MIT) | `SameBoy-1.0.5.zip` | oracle | stub |
| Eden | Nintendo Switch | eden-emu/eden | unverified | `Eden-Linux-d85bad8924-amd64-clang-pgo.AppImage` | oracle | stub |
| eglantine | Nintendo Switch 2 | https://github.com/Eglantine-Community/Eglantine | unverified | `eglantine-linux-x86_64-pgo.AppImage` | oracle | stub |
| RyuSAK | Nintendo Switch | RyuSAK (Ryujinx-derived fork; upstream URL unverified) | unverified | `RyuSAK-linux-x64-1.6.2.zip` | oracle | stub |
| MAME 0.289 | Arcade, multi-system | mamedev/mame | GPL-2.0-or-later | `mame-mame0289.zip` | peer | stub |
| Shocora | Wii U | https://github.com/Thoughts-with-friends/shocora/ | unknown — verify upstream before use | not present locally | oracle (stub) | stub |

### Shocora

Shocora is a Wii U emulator kept for a future Unko system or feature. It is
**not wired into any tool**, has no local checkout under
`/var/home/Lordra/ai/refs`, and its licence has not been verified. Treat it as a
placeholder in this register: confirm the upstream licence and obtain a checkout
before it is used for anything, including as a black-box oracle.

## Suspended and unresolvable candidates

- The Switch/Switch 2 forks (Eden, eglantine, RyuSAK) are early and frequently
  relicensed; their licences are recorded as `unverified` above rather than
  assumed from the upstream they derive from.
- `mame-mame0289.zip` ships its own `COPYING`; the archive's aggregate licence is
  GPL-2.0-or-later, but individual drivers and third-party libraries carry other
  terms. Confirm per-file terms before reading any code.

## Licence rules

These restate the repository contract in `AGENTS.md` and `CONTRIBUTING.md`; the
contract there is authoritative if the two ever disagree.

1. **GPL projects are behaviour/timing oracles only.** MesenCE, mednafen,
   Dolphin, gopher64, MiSTer, openfpga, Cxbx-Reloaded, decaf-emu, KytyPS5, MAME
   and RustyNES are GPL (or GPL-compatible only through copyleft). Their source
   must never be read, copied, translated, vendored or linked into Unko. They
   may be run as separate processes and compared black-box.
2. **ares (ISC) may be adapted with attribution.** Any adapted code must be
   recorded in `crates/unko-nes/PROVENANCE.md`, with notices kept in `NOTICE` and
   `licenses/ares-LICENSE.txt`. ISC still requires the copyright and permission
   notice to travel with the adapted code.
3. **Hardware RTL is read for behaviour, never translated.** MiSTer and openfpga
   HDL is a hardware reference: use it to understand timing and bus behaviour,
   then write an independent implementation. Do not port RTL line-for-line, and
   do not paste GPL HDL into the repository.
4. **Permissive sources are still sources.** RustySNES (MIT OR Apache-2.0),
   TriCNES (MIT), SameBoy (Expat) and the test suites may be read, but copying
   code or test data still needs the applicable notice and provenance entry.
5. **`unverified` means unverified.** Do not treat a row marked `unverified` as
   permission to read or copy; check the shipped licence first.
6. **No ROMs, BIOS, firmware, keys or game data.** Their presence under
   `/var/home/Lordra/ai/refs` is not an invitation to include them. Tests that
   need public test ROMs look under `test-roms/` and skip when it is absent.

## Not emulators (deliberately excluded)

The folder also contains ROM sets, BIOS images, firmware, keys, NAND/HDD images,
game data and tooling. These are **not** part of this register: for example
`bios/`, `lib.7z` (PS4/PS5 system modules), `vspC/`, `Minerva_Myrient/`, `ps5/`,
`gta6/`, `refcap/`, `dolphin network files.7z`, `XEMU FILES.zip`, the
`PS*UPDATE*.PUP` files, `ProdKeys*.zip`, `Firmware.*.zip`, `mlc01_*.zip` and the
`Xbox HDD`/`X360 Avatar` archives. `ES-DE` is a frontend, not an emulator.
`nesdev/` holds NESdev Wiki mapper documentation, not an emulator or a test-ROM
suite.

`refcap/`, `ares-trace-host/` and `rustynes-trace-host/` are Unko-side oracle
harnesses, not reference implementations. They are not listed above; note that
`rustynes-trace-host` links `rustynes-core` by path and must stay outside the
repository.
