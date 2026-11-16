# System and core entries

Every system the shell intends to support, the candidate cores, and the
licensing decision. Only cores marked **linkable** can ship inside the MIT
binary; the rest are references, oracles, or separate processes.

Status: `shipped` · `planned` · `deferred` · `reference-only`

## NES / Famicom — shipped

| | |
|---|---|
| Core | `unko-nes` (standalone Rust crate, MIT + documented ISC ares adaptations) |
| Adapter | `crates/unko-nes-host` (`Core` + `NesCore` implementation) |
| Accuracy bar | reference parity; see `docs/accuracy.md` |
| Scope | licensed library, iNES/NES 2.0/UNIF/FDS loading, PAL/Dendy, expansion audio, Zapper, Famicom 3D System, Karaoke Studio microphone, Miracle Piano replies, Datach X24C02/X24C01, VS. (mapper 99, cabinet input, RGB PPUs 2C03/2C04/2C05), NSF, 405 enhanced video |
| Status | default and only NES core; 146/146 non-DRAW AccuracyCoin rows; 7,073/7,101 images load (99.6%), 0 unsupported mapper IDs, 28 truncated dumps; 6,490 render by 120 frames |

The former TetaNES compatibility core was retired after the accuracy core
gained save states and became the default. A future NES core can be registered
beside it; the host chooses by `CoreInfo.id`.

## SNES / Super Famicom — in progress

| | |
|---|---|
| Core | `unko-snes` (standalone Rust crate, MIT) |
| Adapter | `crates/unko-snes-host` (`Core` + `SnesCore`) — planned |
| Implementation source | **ares `sfc/`** (C++, ISC) — primary behavioural source |
| Oracles | **MesenCE `Core/SNES`** (C++, GPL-3.0) — reference only · **RustySNES** (Rust, Apache-2.0) — oracle/reference only |
| Size | ares 19.9k LOC; Mesen 35.7k LOC |
| Coverage target | 65816, SPC700, S-DSP, dot/scanline PPU, SuperFX, SA-1, SDD1, SPC7110, CX4, DSP1-4, OBC1, ST018, MSU1, Satellaview, Sufami Turbo, BS-X |
| Status | isolated core started 2026-11-16: ROM parsing/checksums/mapping, SRAM, cycle-exact 65C816 timing, full SPC700, APU IPL/ports/timers, S-DSP, PPU modes 0-7 + sprites + subscreen/colour math, DMA/HDMA and the memory map are implemented. Super Mario World is 99.5% pixel-identical to ares at 600 frames; dot-level PPU scheduling and audio parity are next. |

Why an isolated Rust core: the SNES crate has no `unko-core` or shell
dependency, exactly like `unko-nes`. ares remains the primary implementation
source because it is ISC and closest to the silicon; MesenCE remains the
compatibility oracle and must not be read while implementing the core. The
adapter crate is added only after the core has a stable CPU/bus/frame surface,
mirroring the `unko-nes` / `unko-nes-host` split.

RustySNES (<https://github.com/doublegate/RustySNES>, Apache-2.0) joins the
oracle set as a second pure-Rust implementation. It is useful for black-box
differential runs and for checking feature coverage, but it is not copied,
translated file by file, or used to generate tables without an explicit
provenance review. The MIT `w65c816` crate is a CPU cross-check oracle only.

Reference documentation: [SNESdev Wiki](https://snes.nesdev.org/wiki/SNESdev_Wiki),
[Super Famicom Development Wiki](https://wiki.superfamicom.org/) and
[awesome-snesdev](https://github.com/Scottcjn/awesome-snesdev).

Integration: implement the core in `unko-snes`, then add `unko-snes-host` with
the generic `Core` plus a SNES extension trait. Do not port Mesen code.

## Nintendo 64 — planned

| | |
|---|---|
| Primary | **ares `n64/`** (C++, ISC) — linkable via a C ABI |
| Reference | **gopher64** (Rust, GPL-3.0) — structural reference only |
| Size | ares 31.9k LOC excluding vendored Vulkan; gopher64 23.9k Rust LOC |
| Renderer | parallel-rdp (MIT) over Vulkan, used by both |
| Coverage | VR4300 CPU + recompiler, RSP + recompiler, TLB/FPU/COP2, VI/AI/PI/SI/RI/MI, RDRAM, PIF/CIC, 64DD, Aleck 64 |
| Status | planned after SNES |

Why ares: ISC, genuinely low-level, recompilers, 64DD/Aleck 64, and the same
framework as the other systems. gopher64 proves the Rust shape but is GPL, so
it is read-only. N64 requires Vulkan; the shell's renderer plan must account
for that before this milestone starts.

## GameCube / Wii — deferred

| | |
|---|---|
| Candidate | **Dolphin** (C++, GPL-2.0-or-later) |
| Size | 476k LOC core; 47k PowerPC/JIT; 95k video backends; 17k DSP |
| Status | deferred; not a registry entry |
| Reason | GPL-2.0+ cannot link into an MIT binary, and Dolphin is an application with no embedding ABI |

If GC/Wii is pursued, the only defensible shape is a **separate GPL process**
launched by the shell, with its own distribution and source offer — a legal and
product decision, not a core adapter. Do not plan it as a `unko-core`
implementation.

## Registry policy

1. A system may have more than one core; the host chooses by `CoreInfo.id`.
2. A core enters the registry only if its license permits distribution with
   this project (MIT/ISC/Apache-2.0/0BSD/Zlib/public domain).
3. Reference implementations are never linked, vendored, translated file by
   file, or used to generate tables.
4. Third-party notices and license texts live in `NOTICE` and `licenses/`.

## Order

1. NES — shipped and frozen as done on 2026-11-16; outstanding fixes are
   deferred to a later date.
2. SNES — isolated Rust core (`unko-snes`) with ares as the implementation
   source and a `unko-snes-host` adapter later.
3. N64 — ares backend + Vulkan renderer.
4. GC/Wii — optional external Dolphin process, only if the product needs it.

## NES backends

| Core id | Crate | Role |
|---|---|---|
| `nes_accuracy_core` | `unko-nes-host` | reference-parity NES backend; links `unko-nes` |

The stable registry id remains `nes_accuracy_core` so existing save-state
filenames and trace headers keep working. The reusable core crate is
`unko-nes`; `unko-nes-host` is only the `unko_core` adapter. Select with
`--core nes_accuracy_core` in the CLI; it is also the default.
