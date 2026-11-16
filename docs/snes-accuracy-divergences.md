# Accuracy divergences — SNES

This ledger records every known place where `unko-snes` does not yet match the
chosen reference (real hardware, then ares, then a public test ROM). It is the
SNES counterpart to `docs/accuracy-divergences.md` and follows the charter in
`docs/accuracy.md`.

Current snapshot: **2026-11-16**, 65C816 + SPC700 + APU + S-DSP + PPU + DMA + HDMA.

## Open failures

None known in the implemented 65C816/SPC700 instruction semantics or in the
documented S-CPU wait-state table. The 65C816 differential suite asserts exact
master-clock totals (six per oracle bus cycle) across all 18 suites.

The remaining known video divergence is **dot-level sprite evaluation**:

- *Super Mario World* at 600 frames is **99.950%** pixel-identical to ares
  (mean absolute channel difference 0.084), with 33 differing pixels. At 900
  frames it is 99.927% and at 1500 frames 99.912%; every differing pixel lies
  in the overlap of two equal-priority sprites.
- VRAM (32,768 words), CGRAM (256 entries) and OAM are bit-identical to the
  RustySNES oracle at 600 frames.
- ares evaluates sprites with a 32-entry per-line list, the 34-tile limit and
  the `firstSprite` rotation; `unko-snes` renders all 128 entries per line and
  uses OAM-order tie-breaking. The backlog only shows up when sprites of equal
  priority overlap.

S-DSP output is not yet sample-compared against ares.

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

- The S-CPU bus models WRAM, cartridge ROM/SRAM, `$2100-$213F`, `$2140-$2143`,
  `$4016/$4017`, `$4200-$420D`, `$4210-$421F` and `$4300-$437F`. Unmapped I/O
  reads return `0`; open bus and the joypad auto-poll's exact 128-clock
  sequencing window are not modelled.
- The auto-poll sequence is driven at 128-clock granularity from the line-225
  event rather than ares' exact hcounter window, so `$4218-$421F` can settle a
  few hundred clocks early. Games that read them at the end of NMI are
  unaffected in every measured frame.
- The S-SMP APU timers advance once per retired SPC bus cycle. Hardware derives
  them from the wait-state divider, so their phase can drift under heavy I/O.
- S-DSP synthesis runs one 32-clock sample pass per 32 SPC cycles. The ares
  interleaving granularity is finer; this is the main DSP timing approximation
  to remove once there is an audio oracle.
- S-CPU/APU/PPU scheduling uses an exact master-clock credit (21,477,272 Hz;
  four clocks per PPU dot, 1,024,000 Hz SPC700). Memory wait states follow the
  ares table, but the sub-cycle phase of an individual access is rounded to the
  instruction boundary.
- Echo writes are active whenever EDL/ESA enable them, but the exact echo
  buffer page wrap and read-only behavior need an ares sample comparison.
- `Cpu::reset` uses the documented reset-vector fetch and a short internal
  delay sequence; exact reset pin timing is pending hardware capture.
- Display brightness uses `(brightness + 1) / 16` scaling. ares models the DAC
  luma as `(1 + l) / 16 * (l ? 1 : 0.25)`; the two agree at `INIDISP = $0F`
  (all measured frames) but differ below it.
- PPU windows, hires output, interlace, mosaic and offset-per-tile are not
  implemented. Mode 7 `EXTBG` is partial and the 32-sprite/34-tile limits are
  not enforced.
- `WAI`/`STP` retire two internal cycles after the opcode; the real devices
  idle indefinitely, which the system model represents by not advancing the
  CPU until an interrupt or reset.

## Validation

- 18 differential tests against the MIT `w65c816` crate, now also asserting
  exact master-clock totals.
- All 15 read addressing modes across 7 ALU operations (105 opcode cases),
  24 write modes and 35 modify modes.
- 10 retail images (LoROM, HiROM and ExHiROM): header detection and stored
  checksums match.
- 250-image random No-Intro sample: 237 parsed; all 13 failures are
  headerless/beta/pirate dumps rather than clean retail images.
