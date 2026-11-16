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
- SRAM allocation and save-file loading
- 24-bit cartridge bus mapping for ROM and SRAM, with bank mirroring

The 65816 CPU, SPC700, S-DSP and PPU are the next slices.

## Accuracy sources

Implementation sources, in decision order:

1. Real hardware captures.
2. ares `sfc/` (ISC) — the primary implementation source.
3. MesenCE (GPL-3.0) — reference oracle only; no source is read or ported.
4. RustySNES (Apache-2.0) — oracle/reference only until an explicit provenance
   review; see <https://github.com/doublegate/RustySNES>.

See `docs/systems.md`, `docs/accuracy.md` and `PROVENANCE.md`.

## License

MIT. See the repository `LICENSE` and `NOTICE`.
