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
  - **Localised to frame 191.** Per-frame `$13` (ours/RustySNES):
    `190:41/41  191:42/41  192:43/42  193:44/43  194:45/44  195:46/45`, with
    identical vectored NMI counts throughout (106 at 190, 111 at 195). The
    reference performs **one fewer game-loop iteration in frame 191** - it is
    not an extra interrupt. RustySNES' CPU counter agrees it does less work that
    frame (per-frame deltas 45,759 then 44,812) while ours is flat (357,370 then
    357,376 master clocks). Our per-PC histogram shows the S-CPU dominated by
    the `$806B`/`$806D` wait loop (472,529 of 2,691,653 instructions at frame
    190, +4,503 per frame); a matching RustySNES histogram needs the counter in
    `run_frame`'s inner loop, because `step_instruction` is not the path it uses.
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
- blargg **`spc_smp` now reports `PASSED TESTS`** (all 19 sub-tests). The
  path there is worth recording:
  - `Failed 02` (CPU Instructions/Full BRK) was a red herring: the `BRK`
    instruction, the pushed `PC`/`P`, `I=0`, `B=1` and the whole `$072D`
    handler matched RustySNES instruction-for-instruction for 198,411
    instructions. The ROM compares a *cumulative* CRC, so an earlier test's
    output decides a later test's verdict.
  - Diffing the framework's CRC-input stream (`$0613` called with A) against
    RustySNES found the first real divergence at character 510: the check
    table entry for `DECW dp` printed `D0 R1 W0 W1` instead of
    `D0 W0 R1 W1`. ares `instructionDirectModifyWord` loads the low byte,
    **stores** it, then loads and stores the high byte; we loaded both bytes
    before storing either. `INCW`/`DECW` now follow the ares bus order, with
    word flags from the full 16-bit result.
  - The next divergence was test 9 (`smp reg read-write behavior`). ares keeps
    **two independent four-byte latches**: `io.apu0..3` is written by the
    S-CPU and read by the S-SMP at `$F4-$F7`; `io.cpu0..3` is written by the
    S-SMP and read by the S-CPU at `$2140-$2143`. A temporary experiment that
    resolved `$F4-$F7` from the S-CPU write history made an S-SMP read of
    `$F6` return a stale CPU-side value (`0x30`) instead of the real latch
    (`0x00`) - exactly what test 9 checks. Reading the latch directly fixed
    it; `smp_port_reads_see_only_the_cpu_latches` pins both directions.
- blargg `spc_dsp6` reports `Failed 02` (Echo/esa_changes). It is **parked**:
  five turns of instrumentation excluded every DSP-side cause (see above) and
  showed the `$F0` ramp is written by the test itself, so closing it needs a
  TAS-style trace of the test's own SPC stores rather than more DSP work. It
  used to panic in
  a debug build on a 16-bit BRR/echo address overflow; those sums now wrap like
  ares' `n16` arithmetic. This is the concrete audio item to fix next.
  - Measured state: at frame 1200 only **5 of 128 DSP registers** differ from
    RustySNES and they are the ones the test wrote most recently
    (`$00=0x20`, `$07=0xE0`, `$4C=0x01`, `$4D=0x01`, `$5D=0x0F`), i.e. a
    progress/phase difference, not a register-mapping error. The S-SMP
    instruction count matches (5,078,844 vs 5,148,410 at 1200 frames).
  - The `Echo/esa_changes` readings are close but not equal (e.g. `003F` vs
    `0020`, `0626` vs `05E8`), which points at the *sample* boundary rather
    than the address arithmetic: the DSP main-loop order, echo `_address`
    latching, `_length`/`_offset` wrap and the `$F1`/`FLG` gating all match
    ares line for line.
  - The DSP now advances **one micro-step per S-SMP access**, as ares does
    (`DSP::tick` is `3 * 8` apu clocks = one 2-clock S-SMP access, and the
    thread is synchronized on every access), instead of running the whole
    32-step sample at once. That makes the echo buffer's partial state visible
    at the exact moment an S-SMP read lands. The readings are unchanged, so
    this particular test observes the buffer only at sample boundaries.
  - **Both oracles pass it**, so the target is well defined: the local ares
    build runs the ROM to completion, and RustySNES prints `PASSED TESTS`
    (at 12000 frames; it needs far longer than us only because we abort at
    test 2).
  - Every arithmetic path the echo test depends on was verified bit-exact
    against ares source: `echo.cpp` (history/address/FIR/clamp/feedback and
    the `echoWrite` ordering), `timer` step/synchronise semantics, the 512-entry
    gaussian table generated from `gaussian.cpp` (0/512 entries differ), the
    `counter.cpp` rate/offset tables (exact) and `brr.cpp` decode with all four
    IIR filters (exact).
  - The DSP *tick placement* is not the cause: moving the tick from the
    plan-drain path to the S-SMP access path (ares synchronises the DSP inside
    `SMP::step`) leaves the readings byte-identical.
  - **Measured divergence.** Logging every `$F3` read of `VxENVX`/`VxOUTX`
    (`$08`/`$09`/`$18`/... ) in both emulators gives **34 identical reads**,
    then we read `00` where RustySNES reads `88`. That register is only
    refreshed by the DSP at a sample boundary and the surrounding loop
    (`$0732`..`$0747`) spins until two consecutive reads differ, i.e. the test
    *is* a sample-boundary detector. The burst lengths then differ (ours 14
    reads, RustySNES 19), so the two emulators catch a different boundary.
  - **DSP sample phase is correct.** Sweeping the DSP step machine's initial
    phase over all 32 offsets gives common prefixes of 34 (phase 0), 11, 9,
    13, 13, 3, 3, 2, 3 for the sampled offsets - phase 0 is optimal, so this is
    not a sub-sample offset.
  - **DSP tick placement is correct too.** Ticking the DSP inside the S-SMP
    access (as ares does inside `SMP::step`) rather than at the plan drain makes
    the first divergence *earlier* (read 7 instead of 34), so the drain-time
    model is the closer one and was kept.
  - **Tested and falsified.** The access-granularity hypothesis was implemented
    on branch `spc-resume` (commits `c66eeec`, `4f6e2ae`): the micro-op plan was
    replaced by tick-paced stepping, `Spc700::Pending::DirectRead` defers any
    read of `$00F0-$00FF` to the start of its own access, and the S-SMP is paced
    by a per-instruction access-unit budget so wall time still equals
    instruction cost. The build passes 843 workspace tests, the 8x256 cycle
    oracle, `spc_timer` and `spc_smp` - and the `spc_dsp6` framebuffer is
    **byte-identical to `main`**, as is `spc_smp`'s. So access-exact S-SMP I/O
    reads change nothing for this ROM: the `Echo/esa_changes` failure is on the
    DSP side, not the S-SMP read granularity.
  - **ARAM diff vs RustySNES (same SPC code point).** Snapshotting APU RAM at the
    first execution of a given SPC address gives an exact byte diff:
    `$06CA` (test entry), `$0714` (echo setup) and `$0732` (poll-loop entry) are
    **byte-identical**; by `$0787` (end of the first check block) exactly 13 bytes
    differ - the cumulative CRC `$DC-$DF`, `$01F2`, and nine bytes of the echo
    buffer page (`$F008`, `$F010`, `$F018`, `$F020`, `$F028`, `$F030`, `$F038`).
    The echo words form a descending ramp (0800, 07F8, 07F0, ...) where ares
    holds each value for two samples; ours steps down one sample early
    (`$F008`: ours `07F8`, reference `0800`). So the first divergence is a
    **voice-to-echo value stepped one sample early**, not the CRC, not the
    S-SMP and not the echo addressing.
  - **DSP-register write timing tested and rejected.** The natural counterpart
    to the read work is to commit `$F2`/`$F3` stores at their own access clock
    (ares commits a store at the *end* of its access, `step(wait)` then `write`,
    while a load samples mid-access). Three variants were tried on
    `spc-resume`: (a) deferring every `$00F0-$00FF` store hangs the boot
    entirely; (b) narrowing to `$F2`/`$F3` with the write at the access start
    and (c) with it at the access end both keep `spc_timer` green but regress
    `spc_smp` from `PASSED TESTS` to `Failed`, and `spc_dsp6` then reports
    `Failed 03` (test 1 `Echo/basics`, which passes today). So our current
    commit-at-execution write timing is closer to ares than the deferred one,
    and this is not the one-sample echo offset either.
  - **The envelope/shared-latch family is ruled out by data.** Logging
    `(sample, voice4.envelope, latch.adsr0, latch.output, echo.output[0])` for
    16 samples after every `$47` (V4GAIN) write gives **all zeros** for the
    envelope, `latch.adsr0` and `latch.output` in every window (192 entries).
    The voice therefore contributes nothing while the echo ramp is written, so
    the ramp comes purely from the echo feedback/FIR path
    (`echo.output = sclamp16(echo.input * feedback >> 7) & ~1` with
    `echo.input` = FIR of the echo history). The one-sample-early step is
    therefore in the FIR history phase or in how many samples elapse between
    echo writes - not in the envelope cadence, `misc29`/`misc30` order,
    `clock.counter`, or the shared `latch.adsr0`/`latch.envx` (all of which the
    log shows to be irrelevant here).
  - **The echo feedback path is inactive where the ramp is compared.** Logging
    `(_history_offset, echo.input[0], echo.feedback, echo.history[0][*])` at
    `echo22`, at `echo25` and at every `echoWrite` of channel 0 shows
    `in=0000 fb=00 history=[0;8]` in **every** captured window: the two `$6D`
    (ESA) writes of the whole run (samples 4017 and 12028), the V4GAIN windows,
    and - decisively - all 2000 sampled `echoWrite` calls never observe
    `echo.output[0] != 0`. So the `0800, 0800, 07F8, ...` ramp in the `$F0`
    page is not produced by the echo feedback/FIR arithmetic at all; it is
    written by the test's own SPC stores (or is untouched echo-page data).
    The remaining divergence is therefore in the **test's ARAM write sequence
    or the pages it compares**, not in any DSP arithmetic - every DSP formula
    and cadence has now been excluded by measurement.
  - **`clock.counter` phase is ruled out structurally.** `Dsp::reset` is called
    exactly once (`Apu::reset` from `Snes::reset`), so `clock.counter` starts at
    zero exactly as in the reference (ares does not reset `clock` at all, and it
    is zero-initialised), and `counterTick` runs once per sample in `misc30`.
  - Two intermediate variants (over-counting one access unit per deferred read,
    so the S-SMP ran slower than real time) *did* move the Super Mario World
    content offset from two frames to one (`ours 599 == RS 600` at 100.000%).
    With the delay arithmetic corrected the offset returns to two, so the
    remaining title-screen phase is likewise **not** caused by I/O read
    sampling granularity - the drift was compensating something else.
- At 600 frames, 36 of the 128 DSP registers differ from the RustySNES oracle;
  voices 6 and 7 match exactly and voices 0-4 are at different song positions.
- The queued audio is non-silent (RMS about 0.012 vs ares 0.023) and the best
  cross-correlation against ares's dumped 395-sample window is about 0.43 at a
  232-sample lag; the remaining difference is a different point in the song.
- **`$F1` CONTROL port-reset bits.** ares `SMP::writeIO` case 0xf1 clears
  `io.apu0`/`io.apu1` when bit 4 is set and `io.apu2`/`io.apu3` when bit 5 is
  set - the latches the S-SMP reads at `$F4-$F7`. This was missing. It is now
  implemented and pinned by `control_clears_the_port_latches`.
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
- Hires output is **implemented and measured**: `frame_hires` is latched at
  frame start from `SETINI` bit 3 / `BGMODE` 5-6, `frame_width()` returns 512
  for those frames, and `present_line` emits two DAC columns per pixel clock -
  the **Below** screen at the even column and the **Above** screen at the odd
  one, matching ares `Background::run`'s `if(screen == Screen::Above) x++`, each
  with its own colour math against the opposite screen. Verified against
  RustySNES on `PeterLemon-PPU/HiColor/MosaicMode5.sfc` (mode 5, no HDMA colour):
  our 256-wide render matches the hires frame's odd columns at **100.000%**
  (oracle precondition), and the 512-wide output matches at **99.805%** exact
  and, after modelling ares' one-dot pixel pipeline, **100.000%** exact (mean
  channel difference 0.000). The last 224 pixels were output column 0 on every
  row: ares' pipeline is one dot deep, so at the first dot of a scanline no tile
  has been fetched and the Below screen (which owns the even column) shows the
  backdrop. Both reference oracles produce a backdrop column 0, and emitting it
  makes the frame bit-identical.
- Interlace is not implemented. The framebuffer geometry is
  now prepared for hires: `SCREEN_WIDTH` (256) stays the *compositing* width,
  `MAX_SCREEN_WIDTH` (512) is the DAC output width, the backing array is always
  hi-res capacity so a mode change cannot reallocate, and `frame_width()`/
  `frame_hires()`/`frame_buffer()` mirror the reference PPU model
  (`frame_buffer()` returns a resolution-sized slice). Hi-res *fetching*
  (`Background::fetchCharacter(index, half)`, two output columns per pixel
  clock) and the 448-line interlace field output are still to be written.
  A first attempt implemented only the **DAC doubling** (output column 2x =
  Above screen, 2x+1 = Below screen, each with its own colour math) and latched
  `frame_hires` from `SETINI` bit 3 / modes 5/6. Measured against RustySNES on
  `PeterLemon-PPU/HiColor/PseudoHiRes.sfc` at 512 wide it matched **5.767%** of
  pixels (mean channel difference 51.98), i.e. wrong, and was reverted. The
  conclusion is that the DAC-only model is insufficient: hi-res needs ares'
  `Background::fetchCharacter(index, half)` fetch path (two output columns per
  pixel clock, `half` selecting between the two halves of the fetched bitplane
  pair, `pixelCounter` running at 512 per line) before the output stage can be
  validated. The ROM and its SHA-256 are recorded in
  `test-roms/snes/PeterLemon-PPU/README.md`, and the RustySNES harness now
  reports `FBGEOM <frame_hires> <visible_width>` (`true 512` for this ROM) so a
  hi-res dump can be strided on the next attempt.
  A follow-up measurement shows *why* the DAC-only model failed and what the
  oracle actually contains: our **current 256-wide non-hires** render of the
  same ROM matches RustySNES' hires frame at only **5.882%** (even columns) and
  **6.570%** (odd columns), i.e. the demo stores genuine 512-wide detail, so a
  256-wide render of it is aliased no matter how the output stage is written.
  The visual comparison confirms it: the image is present in both but ours is
  decimated. This makes the ROM a valid hires oracle but *only* at 512 wide, and
  it shows that the earlier 5.767% was the aliasing, not the DAC interleave.
  The next attempt must therefore render the Above and Below screens as the two
  interleaved halves (Above at even output columns, Below at odd) **and** verify
  that `sub` (Below) actually carries the alternate detail for a pseudo-hires
  ROM, i.e. check `TS`/sub-layer rendering at 256 wide first.
  That check is now done. `Ppu::debug_line_layers` shows the ROM is **mode 1
  with `SETINI = $08`** (pseudo-hires), `TM = $01`, `TS = $02`: the main screen
  draws BG1 and the sub screen draws BG2, both fully opaque (256/256 pixels),
  and the two screens **differ on 129-149 of 256 pixels** per line. So the
  Below screen does carry its own content and the Above/Below interleave is the
  right model.
  Re-applying the interleave on that basis still measures **5.767%** against
  RustySNES - and our *non-hires 256-wide* render of the same frame measures
  **5.882%** against RustySNES' even columns (6.570% against odd). The two
  numbers being equal is the important result: **the mismatch is in the base
  rendering of this ROM, not in the hi-res path**. This ROM cannot validate
  hires until our 256-wide render of it matches, which points at its HDMA
  colour/palette effect rather than at pixel plumbing.
- Offset per tile **is** implemented for modes 2, 4 and 6, transcribed from
  ares `Background::fetchNameTable`/`fetchOffset`: the per-column horizontal and
  vertical words are read from BG3's tilemap (horizontal at `voffset + 0`,
  vertical at `voffset + 8`, horizontal position snapped to the tile column)
  and applied when bit `13 + layer` of the word is set; mode 4 packs both
  directions into one word, selected by bit 15. Verified by
  `offset_per_tile_shifts_a_bg_column_from_bg3`, which renders the same line
  with and without the lookup word and asserts the column moves from tile 1 to
  tile 2. No public prebuilt offset-per-tile test ROM exists in the collections
  already used here, so this is verified against the ares rule rather than
  against an oracle frame dump.
- Mode 7 is complete for the base console: the matrix/centre registers share
  ares' `latch.mode7` byte (first write low, second high, shared with
  `$210D/$210E`), `M7SEL` bits 6-7 follow ares `io.repeatMode7` (0/1 repeat,
  2 transparent, 3 character 0), BG2/EXTBG is rendered with
  `priority[palette.bit(7)]` and bit 7 cleared before the transparency test,
  and BG horizontal/vertical mosaic is applied to the pre-flip coordinate
  (BG2's vertical mosaic gated by BG1's enable). Verified against RustySNES on
  Peter Lemon `Mode7/RotZoom`: **100.000%** of pixels identical at frames 100
  and 200. The matrix-latch bug alone had collapsed every Mode 7 plane to a
  single pixel.
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
