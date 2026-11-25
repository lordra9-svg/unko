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
unko-n64       62 passed
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
| LOD | per-pixel `log2` of max texture derivative, adjacent mip tiles blended by the fraction | derivative coefficients come from the triangle texture words; there is no per-tile mip metadata, detail/sharpen LOD, or perspective-corrected derivative |
| Noise | LCG reseed over `(x, y, primitive)` identical to the reference renderer's `noise.h` | verified against reference seed vectors `(0,0,0)->0x0000`, `(5,9,3)->0xCEC4`, `(1028,16,14)->0xB1BC`; our "primitive" is the triangle counter, not a span-buffer primitive index + base |
| CP0 Count | +1 per retired instruction | VR4300 increments every other CPU clock; per-instruction timing would need a cycle-accurate pipeline model |
| Cache/NMI | external pending lines consumed at `step()` | there is no cache parity/ECC model; conditions are signalled by the system layer |
| Interpenetrating Z | coverage scaled by the 4-bit coefficient of the combined depth delta | matches the reference renderer formula; verified by unit and frame tests |
| CLI/TLUT | palette loaded from the texture image into a 256-entry table | TMEM chunk/address semantics are simplified |

## SNES gates

SNES accuracy remains bit-exact and is re-run after each N64 change (see the
commit log):

```
mmx2: 0 differing / 639656  (100.0000%)
dkc : 0 differing / 639656  (100.0000%)
```
