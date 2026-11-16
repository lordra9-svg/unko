# Accuracy divergences — SNES

This ledger records every known place where `unko-snes` does not yet match the
chosen reference (real hardware, then ares, then a public test ROM). It is the
SNES counterpart to `docs/accuracy-divergences.md` and follows the charter in
`docs/accuracy.md`.

Current snapshot: **2026-11-16**, 65C816 + SPC700 + APU + S-DSP.

## Open failures

None known in the implemented 65C816/SPC700 instruction semantics or documented
bus cycle order. The PPU and DMA are not implemented, so there is no frame or
video comparison yet. S-DSP output is not yet bit-compared against ares; the
pipeline is a direct port of the documented behaviour and ares ordering.

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
- **SPC700 `AND1 C,/membit` (`$6A`)** — `spc700` 0.1.0 computes
  `!(bit & C)` instead of `C & !bit`. `unko-snes` follows ares and the unit
  test `spc700_bit_logic` pins the result; the all-opcode oracle sweep skips
  this one opcode.

## Intentional approximations

- The S-CPU bus currently models WRAM, cartridge ROM/SRAM and `$2140-$2143`.
  Other I/O reads return `0` and writes are ignored until the PPU and DMA
  registers are implemented.
- The S-SMP APU has RAM, the IPL ROM window, control/RAM-enable bits and the
  four ports. Timers are register stubs with no tick/catch-up model yet.
- S-DSP synthesis runs one 32-clock sample pass per 32 SPC cycles. The ares
  interleaving granularity is finer; this is the main DSP timing approximation
  to remove once there is an audio oracle.
- S-CPU/APU scheduling uses an exact 3,579,545:1,024,000 cycle credit. Memory
  access wait states are pending.
- Echo writes are active whenever EDL/ESA enable them, but the exact echo
  buffer page wrap and read-only behavior need an ares sample comparison.
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
