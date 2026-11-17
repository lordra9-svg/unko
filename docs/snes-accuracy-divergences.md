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

The remaining known video divergence is a **two-frame content phase**, not a
rendering difference:

- With frames aligned, *Super Mario World* is **100.000%** pixel-identical to
  the RustySNES oracle: `unko-snes` frame 598 vs RustySNES 600 (mean absolute
  channel difference 0.000), 898 vs 900, and 1500 vs 1500. This includes the
  previous 33-58 pixel sprite-overlap residual, which was the sprite
  evaluation model (see below).
- **Open: title-screen content offset.** Both emulators turn the display on at
  frame 86 and match exactly at 1500, but our title screen reaches the
  reference content two frames earlier, so same-index comparisons at 600/900
  show 92.4%/91.5%.
  - Measured and rejected as causes: port-write placement (committing S-SMP
    latches at their exact clocks left the frames **bit-identical**), port
    *read* timing (timestamping every S-CPU `$2140-$2143` write and resolving
    S-SMP `$F4-$F7` reads at the read access also left the frames
    bit-identical), the APU
    clock (using ares' 24,606,720 Hz changed the S-SMP instruction count but
    not the frames), timer rate (ticks match RustySNES 4,077 vs 4,078 by frame
    600), and the ares `$4210`/`$4200` NMI hold/enable model (it moved the
    *request* count to 441 with identical frames, and was reverted).
  - The earlier "one extra NMI" was an accounting artefact: `nmi_count` counted
    NMI *requests* while RustySNES counts *vectored* interrupts. Matching the
    accounting (`cpu.nmi_services`) gives identical counts at frames 190-200
    (106..116) and differences of +/-1 at other frames that move with the
    capture point. The game's WRAM `$13` is likewise within one increment.
  - What remains is a genuine phase difference in the boot handshake, and the
    only exact model is ares' bidirectional cooperative threads: the S-CPU and
    S-SMP can each suspend mid-instruction at a port access and run the other
    forward to the same clock. Our single-threaded `Spc700::step` executes a
    whole instruction at once, so an S-SMP poll of `$F4-$F7` observes the CPU
    latch as of the instruction start. Two ways to remove it:
    1. **Resumable SPC700** — rewrite the interpreter as an explicit
       state machine (or a generator) that yields at every bus access; exact,
       but it touches every instruction and addressing-mode helper.
    2. **Coroutine S-CPU/S-SMP** — run the S-SMP interpreter on a second OS
       thread and hand-shake at port accesses; exact and localized, but adds
       threading, synchronization cost and unsafe/shared state.
    This is an architecture decision, not a bug fix; the current model keeps
    the port *writes* exact and the port *reads* one instruction early.
- VRAM (32,768 words), CGRAM (256 entries) and OAM are bit-identical to the
  RustySNES oracle at 600 frames.
- **Frame-index caveat:** the local ares build's `--frames N --dump-frame`
  dumps the content of frame `N - 2` (its internal frame counter starts two
  vblanks later than RustySNES' and ours). Compare ares dumps with `ours N-2`
  or, preferably, use RustySNES' `vram_dump` at the same frame.
- Sprite evaluation now follows ares `Object::evaluate`/`Object::run`: scan
  OAM from `firstSprite`, keep the first 32 on-line sprites, draw the line's
  list backwards with a 34-tile cap, and overwrite object pixels
  unconditionally so the lowest OAM index wins every overlap regardless of the
  per-sprite priority bits (those only order the layer). This removed the last
  differing pixels.

S-DSP output is now instrumented but not sample-identical:

- blargg `spc_smp` reports `Failed 02` (CPU Instructions/Full BRK) and
  `spc_dsp6` reports `Failed 02` (Echo/esa_changes). For `spc_smp` the
  suppressed readings CRC is `40E4B4BE` where the ROM expects `654FF73E`.
  The test's BRK matches ares instruction-for-instruction (stack `02,01,P`,
  `I=0`, `B=1`), so the failure is later.
  - The framework's CRC-input stream (`$0613` called with A) was logged in
    both emulators. The first divergence is at character 510: for one
    stack-fingerprint entry we emit `D0 R1 W0 W1` and RustySNES emits
    `D0 W0 R1 W1`. Same checks, different order, so a data-dependent branch
    in the test's check routine takes the other path; the CRC difference is
    the consequence, not a broken CRC.
  - Next step: decode that check routine's `D`/`W`/`R` marker branches (the
    markers are not `lda #imm` constants, so they come from a table) to see
    which stack/DP value drives the branch, or single-step the `$072D`
    handler in both emulators and diff `SP`/`P`/`PCL`/`PCH` after each stack
    operation. Both failures predate the
  current scheduling work (the framebuffers are byte-identical to the previous
  build) and are the concrete audio/SPC items to fix next. `spc_dsp6` used to
  panic in a debug build on a 16-bit BRR/echo address overflow; those sums now
  wrap like ares' `n16` arithmetic.

- At 600 frames, 36 of the 128 DSP registers differ from the RustySNES oracle;
  voices 6 and 7 match exactly and voices 0-4 are at different song positions.
- The queued audio is non-silent (RMS about 0.012 vs ares 0.023) and the best
  cross-correlation against ares's dumped 395-sample window is about 0.43 at a
  232-sample lag; the remaining difference is a different point in the song.
- **Blargg `spc_timer` passes.** It previously reported `Failed 02` ("timer
  read vs write") because `RET`/`RETI` were one access short and `SLEEP`/`STP`
  five accesses short, so the delay loops around each timer reading cost fewer
  SPC cycles than ares; the timer stage counters were also stepped by the
  glitchy CPU wait table (2/4/10/20) instead of ares' `timerWaitStates`
  (2/4/8/16). Both are fixed and pinned by the new `spc700_cycles` oracle and
  the timer wait-state unit test.
- ares' DRAM refresh (5 x 6+2 clocks per scanline) is not modelled. Adding it
  shifts the SWM boot by three frames; the exact interaction with the SPC
  handshake still needs a timer/refresh-specific test.

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
  `$4016/$4017`, `$4200-$420D`, `$4210-$421F`, `$2180-$2183` and
  `$4300-$437F`. Unmapped I/O reads return `0`; open bus and the joypad
  auto-poll's exact 128-clock sequencing window are not modelled.
- The auto-poll sequence is driven at 128-clock granularity from the line-225
  event rather than ares' exact hcounter window, so `$4218-$421F` can settle a
  few hundred clocks early. Games that read them at the end of NMI are
  unaffected in every measured frame.
- The S-SMP timers advance by ares' `timerWaitStates` (2/4/8/16 clocks) per
  access, including the two half-steps around `$F4-$F7` reads. CPU time is
  accounted in access units (one unit = two clocks) at the documented
  1.024 MHz; the credit is exact, but an individual access is executed at an
  S-CPU instruction boundary rather than at its exact master clock.
- S-SMP port writes are committed at the exact base clock of the access (a
  recorded micro-op plan), but the instruction's **reads** still happen when
  the plan is formed, so an S-SMP poll of `$F4-$F7` can observe a fresh CPU
  latch slightly early. ares' cooperative threads synchronize both directions
  at the exact cycle.
- The S-CPU now advances the peripherals per access (ares `CPU::read/write`
  ordering), but the PPU renders one scanline at a time when the line event
  fires, so mid-scanline register writes are not yet dot-exact.
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
- Display brightness follows ares' DAC luma `(1 + l) / 16 * (l ? 1 : 0.25)`
  with integer rounding. Measured frames match ares at `INIDISP = $0F` and at
  `$0B`; sub-1/255 rounding differences are possible at other levels.
- Hires output, interlace and offset-per-tile are not implemented.
- Mode 7 `EXTBG` is partial.
- `WAI`/`STP` retire two internal cycles after the opcode; the real devices
  idle indefinitely, which the system model represents by not advancing the
  CPU until an interrupt or reset.

## Validation

- 18 differential tests against the MIT `w65c816` crate, now also asserting
  exact master-clock totals.
- `spc700_cycles`: all 256 SPC700 opcodes in four register states, asserting
  exact base-clock totals against the RustySNES (ares-derived) oracle.
- All 15 read addressing modes across 7 ALU operations (105 opcode cases),
  24 write modes and 35 modify modes.
- 10 retail images (LoROM, HiROM and ExHiROM): header detection and stored
  checksums match.
- 250-image random No-Intro sample: 237 parsed; all 13 failures are
  headerless/beta/pirate dumps rather than clean retail images.
