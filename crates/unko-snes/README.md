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
  noise/LFSR, pitch modulation, echo and stereo 32 kHz sample output; the
  pre-resampler PCM is bit-exact against ares on the blargg oracle
- Line-based S-PPU: register map, VRAM/CGRAM/OAM, vblank/NMI, modes 0-7,
  tile backgrounds, Mode 7 affine mapping, sprites, subscreen and colour math
- Exact S-CPU memory timing in NTSC master clocks with the ares wait-state
  table, per-access peripheral stepping, an S-SMP micro-op plan that commits
  port writes at their exact clock, and DMA/HDMA stalls
- S-CPU general-purpose DMA, HDMA, controller ports/auto-poll and the
  hardware multiply/divide unit
- Coprocessors: SuperFX (GSU), SA-1, S-DD1, SPC7110 (+EXSPC7110 expansion),
  CX4, NEC DSP-1/2/3/4, OBC1, MSU1 (data ROM seek/read plus 44.1 kHz
  streaming audio), the Sharp S-RTC and the two-slot Sufami Turbo base
  cartridge
- Versioned save states (S-CPU, S-SMP/S-DSP, PPU, DMA/HDMA, WRAM, SRAM and
  every coprocessor) with a round-trip continuation test
- Minimal S-CPU memory map: WRAM mirrors, cartridge windows, SRAM, reset,
  `$2100-$213F` PPU registers and `$2140-$2143` APU ports
- Oracle differential tests against the MIT `w65c816` and `spc700` crates

Every reference title is pixel-identical to the calibrated ares oracle:
Super Mario World 600/600 presents, Zelda / Super Mario All-Stars / F-Zero /
Donkey Kong Country / Chrono Trigger 1200/1200, Mega Man X3 and Star Fox
10000/10000, and Mega Man X2 9996/10000 with only three proved torn presents
excluded. The 212-entry test-ROM baseline and the whole crate test suite pass.

Open items: retail-music S-DSP audio is not yet sample-exact (the residual is a
register-write micro-step placement difference; see
`docs/snes-accuracy-divergences.md`), and ST018, Satellaview/BS-X, Sufami Turbo
and the Sharp RTC are not implemented.

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
