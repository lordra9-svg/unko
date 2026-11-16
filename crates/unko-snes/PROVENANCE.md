# Provenance — unko-snes

This crate is MIT. Every module records the public sources and reference
material used to implement it. No GPL emulator source is translated or copied.

## `src/rom.rs`

- SNES cartridge header layout, map-mode values, ROM/SRAM size codes and the
  LoROM/HiROM/ExLoROM/ExHiROM address layouts: public SNES hardware
  documentation (fullsnes/SNESdev notes) and the ISC-licensed ares
  `ares/sfc/cartridge` and `ares/sfc/memory` sources.
- ares board manifests (`ares/System/Super Famicom/boards.bml`) were used to
  confirm the LoROM and HiROM bank ranges.
- Header-detection scoring is an original heuristic informed by the public
  behaviour of ares and bsnes: checksum/complement consistency, reset-vector
  range, map-mode match and title plausibility.
- RustySNES (<https://github.com/doublegate/RustySNES>, Apache-2.0) is an
  oracle only. No RustySNES code, tables or comments have been copied.

## Forthcoming modules

The 65816, SPC700, S-DSP and PPU slices will extend this file with their own
citations before merge. MesenCE remains a black-box compatibility oracle and
must not be read while implementing this core.
