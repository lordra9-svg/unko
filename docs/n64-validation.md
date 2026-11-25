# N64 validation status

This document records what the N64 core has been validated against, the
measured evidence for the remaining approximations, and why real-cartridge
validation has not been performed in this environment.

## Real-cartridge gate

`unko-n64-host` contains an ignored gate:

```
UNKO_N64_ROM=/path/to/game.z64 \
UNKO_N64_FRAMES=60 \
UNKO_N64_OUT=/tmp/opencode/n64-gate.txt \
cargo test -p unko-n64-host --lib real_cartridge_validation_from_env -- --ignored --nocapture
```

It parses the image, prints the header (byte order, title, id, country,
version, CRC1/CRC2), boots it, runs the requested frames, and prints FNV-1a
hashes of the RGBA video and f32 audio plus the nonzero-pixel count, samples
per frame and output rate. Any load or frame error fails the gate with the
frame index.

Current environment result:

```
$ cargo test -p unko-n64-host --lib real_cartridge_validation_from_env -- --ignored --nocapture
n64-gate: skipped (UNKO_N64_ROM is not set)

$ UNKO_N64_ROM=.../64DD_IPL_US.n64 cargo test ... -- --ignored --nocapture
n64-gate: ... is not a standard N64 cartridge image: not an N64 image (magic [80, 27, 07, 40])
```

## Why no real cartridge is used here

Measured on this machine (2026-11-25):

- A filesystem search for `*.z64`, `*.n64` and `*.v64` found only 64DD IPL
  firmware images (`64DD_IPL_DEV/JP/US.n64`, `IPL.n64`) and 256-byte synthetic
  test images created by unit tests (`/tmp/unko-n64-start-*/Game.z64`).
- The 64DD IPL images have magic `80 27 07 40` and title `64DD IPL`, not a
  standard cartridge (`80 37 12 40` / `37 80 40 12` / `40 12 37 80`).
- No N64 homebrew toolchain is installed (`mips64-elf-gcc`,
  `mips-linux-gnu-gcc` and `mips64-linux-gnuabi64-gcc` are all absent), so a
  homebrew image cannot be built locally.
- Commercial cartridge images are copyrighted and are not downloaded or
  committed to this repository.

Consequently every N64 result below is from synthetic ROMs and hand-authored
command streams, driven through `Machine::run_frame`. Supplying a cartridge to
the gate above is the only remaining validation step and requires the user to
provide the image.

## Synthetic coverage (measured)

Recent full runs:

```
unko-n64       73 passed
unko-n64-host   6 passed, 1 ignored (the gate above)
unko-app       86 passed, 2 ignored
unko-snes     272 passed
```

N64 coverage highlights:

- CPU: VR4300 integer/64-bit ops, TLB mapping and TLBR/TLBWI/TLBWR/TLBP,
  precise exceptions (TLB refill/general, AdEL/AdES, SYSCALL/BREAK, Ov, FPU),
  delay-slot BD, maskable interrupts, ErrorEPC/ERL entry, Count/Compare
  watchdog and external NMI/cache-error lines, single- and double-precision
  COP1 with `fcr31` flags and FS.
- RSP: scalar + vector subset, DMA round-trip.
- RDP: solid fill, edgewalker with exact 8-bit subpixel coverage, textured
  triangles (RGBA/IA/I/CI+TLUT, clamp/mirror/mask), mip/LOD blending,
  two-cycle combiner with the full selector table (noise, key centre/scale,
  Convert K4/K5, LOD/PrimLODFrac), chroma key from the 17-bit accumulator,
  alpha compare, blender modes/fog/dither, Z-buffered depth with
  interpenetrating coverage and coverage modes, multi-layer AA/depth blending.
- AI: double-buffered DMA read from RDRAM as the DAC clock consumes samples,
  boundary interrupts, live `AI_STATUS`/remaining length, host-rate
  resampling with underrun decay.

## Approximation inventory (with basis)

| Item | Current model | Basis / measured scope |
| --- | --- | --- |
| LOD | RDP 2-cycle LOD: perspective-corrected s/w and t/w derivatives, texel-space scaling by the tile s/t shifts, an explicit level-to-tile mip descriptor table, detail/sharpen tile advancing, and LOD-fraction blending | verified by `perspective_derivatives_use_w_coefficients`, `mip_descriptor_table_maps_levels_explicitly`, `rdp_lod_uses_explicit_mip_table` (level 0->tile 2, level 1->tile 0) and `lod_tile_offsets_and_detail_sharpen`; automatic chains still register level i at base tile+i unless overridden |
| Noise | LCG reseed over `(x, y, primitive)` identical to the reference renderer's `noise.h` | verified against reference seed vectors `(0,0,0)->0x0000`, `(5,9,3)->0xCEC4`, `(1028,16,14)->0xB1BC`; our "primitive" is the triangle counter, not a span-buffer primitive index + base |
| CP0 Count | cycle-cost model (ALU 1, load/store/branch/COP0 2, COP1 4, mul 5, div 37) | verified by `count_advances_by_instruction_cycle_cost`; not a full pipeline/cache timing model |
| MI interrupts | MI_INTERRUPT bits 5:0 are mirrored from AI (bit 2) and can be raised by the system layer; MI_MASK gates them and the OR is delivered to the CPU as Int0 (Cause.IP2), respecting Status.IE/IM. | verified by `machine_mi_interrupt_routes_to_cpu_ip2` through `run_frame`: a masked PI source enters the general vector, the handler clears the source and ERETs; an unmasked source sets no IP2. Bits are write-1-to-clear (scaffold for source-driven clearing). |
| Cache/NMI | cache parity is modelled with 16-byte line metadata; an injected parity fault raises the cache-error exception on the next instruction fetch or data access. NMI is triggered at register level by the modelled MI RCP NMI bit, which asserts `Cpu::signal_nmi`. | verified by `cache_parity_error_triggers_entry_and_eret`, `machine_cache_parity_entry`, `registers_mask_and_nmi_trigger` and `machine_mi_rcp_nmi_entry_and_eret`; there is no real ECC/parity storage, cache replacement policy or PIF/reset-button model |
| Interpenetrating Z | coverage scaled by the 4-bit coefficient of the combined depth delta | matches the reference renderer formula; verified by unit and frame tests |
| CLI/TLUT | palette loaded from the texture image into a 256-entry table | TMEM chunk/address semantics are simplified |

## SNES gates

SNES accuracy remains bit-exact and is re-run after each N64 change (see the
commit log):

```
mmx2: 0 differing / 639656  (100.0000%)
dkc : 0 differing / 639656  (100.0000%)
```
