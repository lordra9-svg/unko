# Provenance — unko-snes

This crate is MIT. Every module records the public sources and reference
material used to implement it. No GPL emulator source is translated or copied.

## `src/rom.rs`

- SNES cartridge header layout, map-mode values, ROM/SRAM size codes and the
  LoROM/HiROM/ExLoROM/ExHiROM address layouts: public SNES hardware
  documentation and the ISC-licensed ares `ares/sfc/cartridge` and
  `ares/sfc/memory` sources.
- [SNESdev Wiki — Memory map](https://snes.nesdev.org/wiki/Memory_map) and
  [ROM header](https://snes.nesdev.org/wiki/ROM_header).
- [Super Famicom Development Wiki — Non-Power-of-Two ROM
  Size](https://wiki.superfamicom.org/non-power-of-two-rom-size): the
  checksum mirroring rule (repeatedly double the final lowest-set-bit chunk).
- [Super Famicom Development Wiki — Memory
  Mapping](https://wiki.superfamicom.org/memory-mapping).
- ares board manifests (`ares/System/Super Famicom/boards.bml`) were used to
  confirm the LoROM and HiROM bank ranges.
- Header-detection scoring is an original heuristic informed by the public
  behaviour of ares and bsnes: checksum/complement consistency, reset-vector
  range, map-mode match and title plausibility.
- RustySNES (<https://github.com/doublegate/RustySNES>, Apache-2.0) is an
  oracle only. No RustySNES code, tables or comments have been copied.

## `src/cpu.rs`

- WDC 65C816 data sheet and the SNESdev Wiki
  [65C816](https://snes.nesdev.org/wiki/65C816) and
  [CPU vectors](https://snes.nesdev.org/wiki/CPU_vectors) pages.
- [Super Famicom Development Wiki — 65816
  Reference](https://wiki.superfamicom.org/65816-reference), including the
  emulation/native-mode rules, addressing modes and cycle behaviour.
- The ISC-licensed ares `ares/component/processor/wdc65816` implementation was
  used to confirm per-addressing-mode bus-cycle order, decimal arithmetic and
  interrupt/return stack sequences.
- The MIT `w65c816` crate is a **cross-check oracle only** in
  `tests/cpu_oracle.rs`. It is not linked into the core and no code or tables
  were copied from it. Two known 0.1.17 divergences are excluded from the
  oracle comparison: emulation-mode RTI always pulls a program-bank byte, and
  decimal SBC underflow differs from ares; the ares-derived unit tests cover
  those cases.
- [awesome-snesdev](https://github.com/Scottcjn/awesome-snesdev) was used as an
  index to the public documentation above.

## `src/spc700.rs`

- Sony SPC700 / S-SMP public documentation, the SNESdev Wiki
  [S-SMP](https://snes.nesdev.org/wiki/S-SMP) and
  [SPC-700 instruction set](https://snes.nesdev.org/wiki/SPC-700_instruction_set)
  pages.
- [Super Famicom Development Wiki — SPC700
  Reference](https://wiki.superfamicom.org/spc700-reference).
- The ISC-licensed ares `ares/component/processor/spc700` implementation was
  used to confirm per-addressing-mode bus-cycle order and instruction quirks.
- The MIT `spc700` crate is a **cross-check oracle only** in
  `tests/spc700_oracle.rs`; no code or tables were copied. Its `$6A`
  (`AND1 C,/membit`) behaviour is known to differ from ares and is excluded
  from the sweep.

## `src/dsp.rs`

- SNESdev Wiki [S-DSP registers](https://snes.nesdev.org/wiki/S-DSP_registers),
  [DSP envelopes](https://snes.nesdev.org/wiki/DSP_envelopes) and
  [BRR samples](https://snes.nesdev.org/wiki/BRR_samples).
- [Super Famicom Development Wiki — S-DSP](https://wiki.superfamicom.org/).
- The ISC-licensed ares `ares/sfc/dsp` implementation was used to confirm the
  voice scheduling, Gaussian table, BRR filters, envelope rates, noise LFSR
  and echo pipeline. The Gaussian table is an integer transcription of the
  ares formula and is embedded as a constant.
- No external DSP crate is linked. RustySNES remains an oracle/reference only.

## `src/ppu.rs`

- SNESdev Wiki PPU pages: [Backgrounds](https://snes.nesdev.org/wiki/Backgrounds),
  [Tiles](https://snes.nesdev.org/wiki/Tiles),
  [Tilemaps](https://snes.nesdev.org/wiki/Tilemaps),
  [Sprites](https://snes.nesdev.org/wiki/Sprites) and
  [Palettes](https://snes.nesdev.org/wiki/Palettes).
- [Super Famicom Development Wiki — PPU](https://wiki.superfamicom.org/) and
  the SNES Development Manual drawings for the register map.
- The ISC-licensed ares `ares/sfc/ppu` source was used to confirm the register
  map, OAM/VRAM/CGRAM layouts, tiledata bitplane order, sprite size tables,
  Mode 7 affine maths and vblank/NMI timing.
- The renderer is line-based rather than a direct cycle-scheduler port; the
  remaining timing/colour-math gaps are recorded in
  `docs/snes-accuracy-divergences.md`.

## `src/dma.rs`

- SNESdev Wiki [DMA registers](https://snes.nesdev.org/wiki/DMA_registers) and
  [DMA examples](https://snes.nesdev.org/wiki/DMA_examples).
- ISC-licensed ares `ares/sfc/cpu/dma.cpp` for the transfer-mode address
  stepping, A/B-bus validity rules and channel register layout.
- HDMA table scheduling is not implemented yet.

## `src/apu.rs`

- SNESdev Wiki [S-SMP](https://snes.nesdev.org/wiki/S-SMP) and
  [S-DSP registers](https://snes.nesdev.org/wiki/S-DSP_registers).
- [Super Famicom Development Wiki — Memory
  Mapping](https://wiki.superfamicom.org/memory-mapping) and the SPC700
  reference.
- ISC-licensed ares `ares/sfc/smp/io.cpp` and `memory.cpp` for the APU address
  map, IPL window, control bits and port direction.
- The 64-byte IPL ROM is **not** bundled. Users supply `ipl.rom`; local tests
  use the ares copy only as a runtime asset.

## `src/system.rs`

- [SNESdev Wiki — Memory map](https://snes.nesdev.org/wiki/Memory_map) for the
  WRAM mirrors, S-CPU I/O regions and cartridge windows.
- [Super Famicom Development Wiki — Memory
  Mapping](https://wiki.superfamicom.org/memory-mapping) for the LoROM/HiROM
  bank windows.
- ISC-licensed ares `ares/sfc/cpu/memory.cpp` for the banked WRAM and I/O
  ranges.

## Forthcoming modules

The SPC700, S-DSP, PPU and DMA slices will extend this file with their own
citations before merge. MesenCE remains a black-box compatibility oracle and
must not be read while implementing this core.
