# Accuracy divergences — SNES

This ledger records every known place where `unko-snes` does not yet match the
chosen reference (real hardware, then ares, then a public test ROM). It is the
SNES counterpart to `docs/accuracy-divergences.md` and follows the charter in
`docs/accuracy.md`.

Current snapshot: **2026-11-16**, first 65C816 slice.

## Open failures

None known in the implemented 65C816 instruction semantics or documented bus
cycle order. The PPU, SPC700/S-DSP and DMA are not implemented, so there is no
frame or audio comparison yet.

## Oracle disagreements (resolved)

These are places where the MIT `w65c816` 0.1.17 cross-check oracle disagrees
with ares. The implementation follows ares, which is the primary source.

- **Emulation-mode RTI** — `w65c816` 0.1.17 always pulls a program-bank byte in
  RTI, even in emulation mode. The 65C816 in emulation mode pulls only P, PCL
  and PCH. `unko-snes` follows ares and the unit test `rti_restores_state`.
- **Decimal SBC underflow** — `w65c816` 0.1.17 returns `$98` for
  `$00 - $01` with carry set in decimal mode; ares and `unko-snes` return
  `$99` with borrow. The unit test `sbc_decimal_8_underflow_wraps` pins the
  ares behaviour.

## Intentional approximations

- The S-CPU bus currently models WRAM, cartridge ROM and SRAM only. I/O reads
  return `0` and I/O writes are ignored until the PPU, APU ports and DMA
  registers are implemented.
- `Cpu::reset` uses the documented reset-vector fetch and a short internal
  delay sequence; exact reset pin timing is pending hardware capture.
- `WAI`/`STP` retire two internal cycles after the opcode; the real devices
  idle indefinitely, which the system model represents by not advancing the
  CPU until an interrupt or reset.

## Validation

- 18 differential tests against the MIT `w65c816` crate.
- All 15 read addressing modes across 7 ALU operations (105 opcode cases),
  24 write modes and 35 modify modes.
- 10 retail images (LoROM, HiROM and ExHiROM): header detection and stored
  checksums match.
- 250-image random No-Intro sample: 237 parsed; all 13 failures are
  headerless/beta/pirate dumps rather than clean retail images.
