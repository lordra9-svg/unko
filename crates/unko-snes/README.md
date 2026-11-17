# unko-snes

Standalone Super Nintendo / Super Famicom core for Unko.

This crate has no dependency on the Unko shell or on `unko-core`. It is the
SNES counterpart to `unko-nes`: a pure-Rust core that can be embedded directly
in another Rust project. A future `unko-snes-host` crate will provide the
`unko_core::Core` adapter.

## Current scope

- 512-byte copier-header (`.smc`) detection and removal
- LoROM, HiROM, ExLoROM and ExHiROM layout detection
- Internal header decoding: title, map mode, cartridge type, ROM/SRAM size,
  country, licensee, version, checksum/complement and reset vector
- SNES checksum calculation, including the non-power-of-two mirroring rule
- SRAM allocation and save-file loading
- 24-bit cartridge bus mapping for ROM and SRAM, with bank mirroring
- Cycle-stepped WDC 65C816 interpreter with emulation/native modes, 8/16-bit
  accumulator and index widths, decimal arithmetic, interrupts, WAI/STP and
  block moves
- Full cycle-stepped SPC700 (S-SMP) with all 256 opcodes
- APU wrapper: 64 KiB RAM, user-supplied IPL ROM, four bidirectional S-CPU
  ports, IPL/RAM control and the S-DSP register file
- S-DSP: eight BRR voices, Gaussian interpolation, ADSR/GAIN envelopes,
  noise/LFSR, pitch modulation, echo and stereo 32 kHz sample output
- Line-based S-PPU: register map, VRAM/CGRAM/OAM, vblank/NMI, modes 0-7,
  tile backgrounds, Mode 7 affine mapping, sprites, subscreen and colour math
- Exact S-CPU memory timing in NTSC master clocks with the ares wait-state
  table, per-access peripheral stepping, an S-SMP micro-op plan that commits
  port writes at their exact clock, and DMA/HDMA stalls
- S-CPU general-purpose DMA, HDMA, controller ports/auto-poll and the
  hardware multiply/divide unit
- Minimal S-CPU memory map: WRAM mirrors, cartridge windows, SRAM, reset,
  `$2100-$213F` PPU registers and `$2140-$2143` APU ports
- Oracle differential tests against the MIT `w65c816` and `spc700` crates

Super Mario World is 99.950% pixel-identical to ares with frames aligned (600) with
bit-identical VRAM/CGRAM/OAM. The next work is dot-level PPU scheduling,
windows/hires/interlace and S-DSP audio parity.

## Accuracy sources

Implementation sources, in decision order:

1. Real hardware captures.
2. ares `sfc/` and `wdc65816` (ISC) — the primary implementation source.
3. [SNESdev Wiki](https://snes.nesdev.org/wiki/SNESdev_Wiki),
   [Super Famicom Development Wiki](https://wiki.superfamicom.org/) and
   [awesome-snesdev](https://github.com/Scottcjn/awesome-snesdev) — public
   hardware documentation and indexes.
4. MesenCE (GPL-3.0) — reference oracle only; no source is read or ported.
5. RustySNES (Apache-2.0) — oracle/reference only until an explicit provenance
   review; see <https://github.com/doublegate/RustySNES>.
6. `w65c816` and `spc700` (MIT) — CPU cross-check oracles only.

See `docs/systems.md`, `docs/accuracy.md` and `PROVENANCE.md`.

## License

MIT. See the repository `LICENSE` and `NOTICE`.
