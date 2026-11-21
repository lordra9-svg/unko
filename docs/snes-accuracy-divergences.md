# Accuracy divergences — SNES

This ledger records every known place where `unko-snes` does not yet match the
chosen reference (real hardware, then ares, then a public test ROM). It is the
SNES counterpart to `docs/accuracy-divergences.md` and follows the charter in
`docs/accuracy.md`.

Current snapshot: **2026-11-19**, 65C816 + SPC700 + APU + S-DSP + PPU + DMA/HDMA + SuperFX + SA-1 + SDD1 + SPC7110 + CX4 + DSP-1/2/3/4 + OBC1 + MSU1.

## Resolved 2026-11-17 — blargg `spc_dsp6` reports `PASSED TESTS`

All 111 sub-tests of `blargg-spc-6/spc_dsp6.sfc` now pass (verified at 9,000
frames against `test-roms/snes/blargg-spc-6/`). `spc_timer`, `spc_smp` and
`spc_mem_access_times` still report `PASSED TESTS`; the whole workspace test
suite passes; `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --check` are clean.

`Failed 02` was **never a test index**. blargg's harness runs each of the 111
named tests in order, prints the name, then `Passed`/`Failed <code>`; `02` is
the failing test's own failure code. The old notes below interpreted it as
"`Echo/esa_changes`, test 2", which sent several turns of work after the wrong
test. The failing test is identified instead from the harness's test-table
pointer in WRAM `$12-$14` (each entry is one 4 KiB ROM slot) and from the
diagnostic table the failing test prints on its re-run.

The four real defects, each confirmed against ares source and RustySNES:

- **Envelope GAIN mode 7 compared the internal latch signed.** ares
  `envelope.cpp` tests `(u32)v._envelope >= 0x600`; a mode 4/5 decrease can
  leave `_envelope` negative, which the unsigned compare still treats as past
  the threshold. Ours was signed and over-incremented. Fixed; this cleared the
  whole `Envelope/` group (`Envelope/gain $E0 threshold` onward).
- **`echo_output` added two un-truncated products.** ares declares both shifted
  products as `i16` locals, so each truncates to 16 bits before the sum
  (RustySNES mirrors it). Ours summed in `i32`. Fixed.
- **`clock.sample` started `false`.** ares `Clock { n1 sample = 1; }`; this is
  the 16 kHz KON/KOFF poll phase. Starting inverted shifted every key-on poll by
  one output sample. This was the last failure: with it fixed the suite goes
  from failing at test 35 (`Misc/brr addr wrap-around`) to `PASSED TESTS`.
  `misc29`/`misc30` themselves already matched ares.
- **S-DSP side effects were ordered against the wrong S-SMP edge.** The DSP now
  runs one micro-step per two S-SMP clocks *inside* the S-SMP access path
  (`SpcBus::step_clock`, called where ares `SMP::wait` does `step()` then
  `synchronize(dsp)`), so a `$F3` read observes the post-wait DSP state and a
  `$F3` write lands after that access's micro-step. Previously the whole
  instruction was recorded, and its DSP effects applied, before any of its
  clocks were drained.

Two supporting timing corrections came out of the same investigation and are
kept because they are ares-faithful and were needed to make the S-SMP stream
lockstep with RustySNES for longer stretches:

- The S-SMP is drained **one SMP clock at a time** (2,050,560 Hz), not one
  two-clock access unit, so an S-CPU `$2140-$2143` poll observes exactly the
  SMP writes committed up to that master instant.
- The scanline DRAM refresh is now charged **inside the CPU step that crosses
  the refresh position** (`CpuBus::advance` returns the 40 extra clocks) rather
  than as a debt paid by the next bus access, matching ares `CPU::step`.

S-SMP reads of `$00F0-$00FF` are now deferred to their own access clock
(`Spc700::Pending`), so the S-SMP samples the S-CPU port latches, timers and
DSP registers at the exact cycle of each access instead of at the start of the
enclosing instruction. This removed the boot-phase drift: the PCM stream is
**bit-exact against ares' pre-resampler output** (see the audio item below).
A trace comparison against RustySNES still shows a one-iteration IPL
`$FFDA`/`$FFDC` difference at instruction 4,966; the PCM oracle shows that our
timing is the one matching ares, so that residual is RustySNES-side.


## Open failures

None known in the implemented 65C816/SPC700 instruction semantics or in the
documented S-CPU wait-state table. The 65C816 differential suite asserts exact
master-clock totals (six per oracle bus cycle) across all 18 suites.

*Super Mario World* is **pixel-identical at the same frame index** - frames 600,
900 and 1500 are 100.000% exact (mean absolute channel difference 0.000) - and
VRAM (32,768 words), CGRAM (256 entries) and OAM are bit-identical at 600
frames. This includes the previous 33-58 pixel sprite-overlap residual, which
was the sprite evaluation model (see below).

- **The former two-frame content phase was DRAM refresh.** The 5A22 refreshes
  WRAM once per scanline; ares `CPU::step` models it as five 6+2-clock stalls at
  hcounter 536 (dot 134), so the CPU loses **40 master clocks per line** while
  the PPU and S-SMP keep running. We modelled no refresh at all, so the CPU
  executed one extra eight-clock unit every ~341 dots - about 10,480 clocks
  (~2.9%) of extra work per frame. The game loop therefore ran ahead of the
  reference and the title screen reached the same content two frames early.
  - How it was found: a per-instruction cost trace of one `$10` window (the
    NMI-set / main-loop-cleared frame flag) compared 349 instructions against
    RustySNES and isolated 12 differences totalling 472 clocks. Every one was a
    multiple of 8, and the dominant signature was **+40 = 5 x 8**, i.e. exactly
    ares' five 6+2 DRAM-refresh stalls. One 544-byte OAM DMA showed `+128`
    (three line refreshes plus ares' whole-transfer cycle) and three DMA
    triggers showed `+8` (that whole-transfer cycle).
  - Fixing it removed the phase outright: WRAM `$13` at frame 191 is now `0x41`,
    matching RustySNES (it was `0x42`), and same-index 600/900/1500 are
    bit-identical instead of 92.4%/91.5% with a two-frame shift.
  - The same trace also found ares' `CPU::dmaRun` whole-transfer cycle (8 clocks
    per DMA trigger, before the per-channel `Channel::dmaRun` setup), which is
    now charged too.
  - `dram_refresh_stalls_the_cpu_once_per_scanline` pins the cadence (40 clocks
    per 341-dot line, three lines in a row).
  - Previously eliminated along the way, for the record: port-write placement,
    `$2140-$2143` read timing, the ares APU clock, the timer rate, the
    `$4210`/`$4200` NMI hold model, NMI accounting (requests vs vectored), the
    `$806B` loop period (46.00 master clocks in both emulators), HDMA stalls
    (SMW has HDMA disabled at that point: 216 `$420C` writes, all zero, in both
    emulators), and access-granular S-SMP reads (branch `spc-resume`, which
    left the frames byte-identical).
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
- *(Superseded 2026-11-17: `spc_dsp6` now passes; the failure below was the
  signed GAIN-mode-7 compare, the `echo_output` i16 truncation and the
  `clock.sample` phase, not `Echo/esa_changes`. Kept for the instrumentation
  record.)* blargg `spc_dsp6` reported `Failed 02`. It was **parked**:
  - **The first divergence is now located exactly, from an ARAM store log.**
    Logging `(spc pc, address, value)` for every S-SMP store and diffing against
    RustySNES gives a 400,000-entry stream whose first difference is at **entry
    8729**: the echo-page stores are byte-identical throughout (2,051/2,051),
    and the divergence is a **single extra iteration** of the `V0ENVX`
    sample-boundary spin at `$04F3`-`$050A`:
    ```
    $04F4: str $F2=#$08      ; DSPADDR = V0ENVX (read-only; write stores the
    $04F7: lda #$88          ;   value in the register file)
    $04F9: sta $F3           ; DSPDATA = $88
    $04FB: cmp $F3 / beq     ; wait until a DSP sample boundary replaces it
    $0502: jsr $0461         ; per-iteration delay
    $0505: sta $F3           ; re-write $88
    $0508: cmp $F3 / beq $0502
    ```
    The loop only exits when the DSP's per-sample `registers[$08] =
    latch.envx` lands inside the write-to-read window, so it is a sub-access
    synchronisation race, not a value error. After the blip the two logs resync
    (offset 6, 116,616 identical entries) and diverge again at 125,351 in the
    same way, so the effect recurs as occasional one-iteration slips.
  - Ruled out by measurement: the coarse DSP phase (sweeping the DSP step
    counter over 32 offsets moves the first divergence only between entries
    8717 and 8729, never further), and the DSP tick ordering (advancing the
    step before the plan step, as ares `SMP::wait` does, instead of after,
    changes nothing). Access costs themselves match ares' tables
    (`cycleWaitStates` 2/4/10/20, `timerWaitStates` 2/4/8/16).
  - **Read-time DSP advance tested and ruled out.** ares `SMP::wait` steps the
    SMP (and synchronizes the DSP) *before* the access returns its data, so an
    access-exact implementation should advance the DSP for the access's own
    clocks at read time. Implementing exactly that for `$F3`/`$F4-$F7` reads
    (ticking `clocks / 2` micro-steps before the register is sampled and
    suppressing the drain-time tick for that access) leaves the first
    divergence at **entry 8729**, unchanged. Combined with the null results for
    the coarse phase sweep and the tick ordering, the slip is not explained by
    any of the three plausible DSP/SPC phase adjustments.
  - **DSP step index logged at each V0ENVX read (both emulators).** Adding the
    S-DSP micro-step position to the `$F3` read log shows our step index is
    exactly RustySNES' phase minus one at *every* read (its `phase` is the
    current micro-step, ours the next one), so the DSP/SPC **phase is correct**
    and the slip is not a phase error - consistent with the sweep.
  - What does differ is the *read value at the same instruction PC*: in the
    sample-boundary routine RustySNES reads `$88` (the value just written)
    several times before the latch replaces it, while we read `00` at those
    same PCs and see `$88` one read later, i.e. our read/write pairing is out
    of step by one access. The bus path itself is correct and now pinned by
    `dspdata_write_reads_back_immediately` (write `$F3` = `$88` to `V0ENVX`,
    read back `$88`, register file retains it until the sample latch).
  - **Combined write+read log with PCs (both emulators).** Logging `$F3`
    accesses as `W/R address value pc dsp-step` gives, at the first slip:
    ```
    ours                          RustySNES
    R 08 00 04F6 step 1           R 08 00 04F6 phase 21
    W 08 88 04F6 step 1           W 08 88 04F6 phase 22
    R 08 00 04F8 step 5   <- 00   R 08 88 04F8 phase 25
                                  R 08 88 04F8 phase 0
                                  R 08 00 04F8 phase 7
    ```
    Both refresh `registers[$x8]` at micro-step 4 of the 32-step sequence
    (`voice9(0)`, identical schedules in both emulators), so the *schedule* is
    not the difference: ours writes at step 1, i.e. just before the step-4
    refresh, and is clobbered three ticks later; RustySNES writes at phase 22,
    after that refresh, so the value survives until the next sample. In other
    words the two emulators' **DSP position at the same SPC instruction differs
    by about 21 ticks (42 master clocks)**.
  - A phase sweep judged with this combined-log prefix (ignoring the step
    column) peaks at phase 20 (prefix 20) against phase 0 (prefix 2), i.e. the
    ordering *does* improve towards that offset, but no phase removes the slip;
    the residual is therefore not a pure phase shift either.
  - Also measured: at 600 frames our log holds 25,012 `$F3` accesses against
    RustySNES' 1,348, while the `VxENVX` read counts are comparable (207 vs
    190). The extra traffic is all writes, so after the first slip the test
    follows a much longer DSP-access path in ours. This needs one confirmation
    run (the two logging paths differ), but it is the first *macroscopic*
    difference found in this test.
  - **Per-access write ordering implemented, measured, insufficient.** ares
    `SMP::write` is `wait()` then `writeIO`, so a DSP register write lands after
    the access's own clocks. Doing exactly that for `$F3` writes (advance the
    DSP `clocks / 2` micro-steps before the store, skip the drain-time tick for
    that access) moves our write from step 1 to step 2 - correct, but the next
    read still returns `00`, because the step-4 `registers[$x8]` refresh is
    still ahead of it. One access is not the gap.
  - **The offset is cumulative, from the boot.** Instrumenting the very first
    `$F3` access gives *35,649* S-SMP instructions executed in ours against
    *35,706* in RustySNES - the two SPCs are already 57 instructions apart when
    the test first touches `$F3`. The ~21-tick DSP difference is a consequence
    of that boot-path divergence, so per-access ordering cannot close it; the
    boot/loader instruction sequence is what differs.
  - **The PC stream localises it to the IPL upload loop.** Logging the S-SMP PC
    per instruction from reset gives streams that are identical for **4,967
    instructions** and then diverge inside the IPL ROM:
    ```
    idx 4965  ours $FFDA   RS $FFDA     ; CMP Y,$F4
    idx 4966  ours $FFDC   RS $FFDC     ; BNE $FFE9
    idx 4967  ours $FFE9   RS $FFDE     ; ours branches, RS falls through
    ```
    At `$FFDC` (`BNE $FFE9`, taken when `CMP Y,$F4` at `$FFDA` finds the CPU
    port latch *changed*) we take the branch and RustySNES does not: our S-CPU
    wrote `$2140-$2143` between the IPL's two `$F4` reads where RustySNES' did
    not. The upload therefore splits into different block boundaries and the
    run is ~57 instructions ahead by the first `$F3` access, which is the
    cumulative offset seen in the DSP step index.
    So the `spc_dsp6` failure bottoms out in the **S-CPU/S-SMP port handshake
    timing during the IPL upload**, precisely at the `$FFDA`/`$FFDC` compare -
    the same bidirectional-synchronisation item, but now with an exact first
    divergence point (instruction 4,967) instead of a frame-level symptom.
  - **Port-event log narrows it to one extra `$F4` poll.** Logging the S-CPU
    `$2140-$2143` writes and the S-SMP `$F4-$F7` reads as one interleaved event
    stream gives streams that agree for **2,145 events** and then differ by
    exactly one extra S-SMP poll:
    ```
    idx 2144  ours W1=2F          RS W1=2F
    idx 2145  ours R0=04   <--    RS W0=05
    idx 2146  ours W0=05          RS R0=05
    idx 2147  ours R0=05          RS R1=2F
    ```
    i.e. between the CPU's writes of block counters `$04` and `$05` our S-SMP
    gets one more `$F4` read than RustySNES', which is the same extra IPL
    iteration the PC stream shows. The APU clock ratios are *not* the cause:
    RustySNES uses `SPC_NUM 68_352 / SPC_DEN_NTSC 715_909` (one S-SMP clock per
    10.4737 master clocks) and ours is `1_025_280 / 21_477_272` per two-clock
    access unit, which is the same ratio exactly.
    So the difference is a discrete event during the upload (one extra
    instruction or one CPU write landing slightly differently), not drift.
  - Consequence for the next attempt: diff the *latch values* (not just PCs)
    at `$FFDA` in both emulators to see which write lands early, i.e. compare
    the master-clock of each S-CPU `$2140` write with the IPL loop's `$F4`
    reads. That is a small, bounded measurement on a deterministic loop.
  - Consequence for the next attempt: the remaining question is the
    *instruction-level* ordering of the write and the following read inside
    `$04F3`-`$050A` (which instruction's access the value is sampled on), not
    the DSP phase nor the DSPDATA visibility. Logging the write PC alongside
    the read PC across a slip point will show the pairing.
  - Consequence for the next attempt: the slip is a *discrete* difference in
    when the sample boundary is deemed to fall inside the write->read window,
    not a phase offset. The instrument that will settle it is a log of the DSP
    step index (0-31) at each `$F3` read of `V0ENVX`, in both emulators, taken
    across the two known slip points (store entries 8729 and 125351).
  - The residual mechanism is the **instruction-granular S-SMP read**: our
    `$F3` read is sampled when the instruction is executed (at plan-record
    time) rather than at the access's own clock, which shifts it by up to one
    instruction relative to the DSP boundary. This also **corrects an earlier
    conclusion**: the `spc-resume` deferral experiment was called falsified
    because its framebuffer was byte-identical, but the framebuffer only
    encodes pass/fail (`Failed 02` either way), so it was never a valid test of
    the readings. The store-log divergence index is the right acceptance
    metric for that work (baseline: 8729).
- *(Superseded 2026-11-17, see above.)* blargg `spc_dsp6` reported `Failed 02`.
  It was **parked**: five turns of instrumentation excluded every DSP-side cause (see above) and
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
  `$4300-$437F`. Unmapped reads return the S-CPU's memory data register (the
  open bus), including the unused bits of `$4210/$4211/$4212`; the joypad
  auto-poll's exact 128-clock sequencing window is still approximated.
- The auto-poll sequence is driven at 128-clock granularity from the line-225
  event rather than ares' exact hcounter window, so `$4218-$421F` can settle a
  few hundred clocks early. Games that read them at the end of NMI are
  unaffected in every measured frame.
- The S-SMP timers advance by ares' `timerWaitStates` (2/4/8/16 clocks) per
  access, including the two half-steps around `$F4-$F7` reads. The S-SMP is now
  drained one clock at a time (2,050,560 Hz) on an exact master-clock credit.
- S-SMP port writes are committed at the exact clock of the access, and reads
  of `$00F0-$00FF` are deferred to their own access clock (`Spc700::Pending`),
  so an S-SMP read observes the S-CPU latches, timers and DSP at the exact
  cycle of the access. The PCM stream is bit-exact against ares as a result.
- The S-CPU advances the peripherals per access (ares `CPU::read/write`
  ordering), and the PPU renders a scanline after hblank (dot 21), so register
  writes on dots 0-21 apply to that line. Writes inside the visible area are
  still applied to the whole line rather than from their dot onward; no measured
  ROM depends on that yet.
- HDMA follows ares exactly at line granularity: setup at V=0, run at
  `hdmaPosition` (hcounter 1104, dot 276) including line 0, transfer before
  decrement, reload when the low seven counter bits reach zero, and a stall of
  8 clocks for the run plus 8 per table read and 16 per transferred byte
  (read + write). The transfer is still applied **atomically** at the trigger
  rather than interleaved byte-by-byte with CPU work, but that no longer changes
  any measured frame: `hvdma.sfc` (eight channels deliberately overflowing
  hblank) is **100.000% pixel-identical to ares**. `hvdma_max.sfc`, which
  triggers a general-purpose DMA from an H-IRQ aligned to a particular dot, is
  also **100.000% exact** after three fixes: `$4201` WRIO is routed to the CPU
  I/O (its bit 7 gates `$2137` counter latching, and dropping it hung the ROM's
  NMI handler), the H-IRQ comparator now matches ares' ten-clock
  `hcounter(10) == 4 * (dot + 1)` rule, and the line is rendered after hblank so
  the DMA's line-boundary `INIDISP` unblank applies to that line.
- S-DSP synthesis runs one micro-step per two S-SMP clocks inside the S-SMP
  access path (ares `SMP::wait` → `synchronize(dsp)`). The PCM stream is
  measured against ares' **pre-resampler** i16 output: for
  `blargg_near_cd_quality.smc` over 1,200 frames (639,700 stereo samples) the
  two 16-bit streams have **zero** differing samples at a -1-frame alignment.
  The earlier 3-sample residual was the boot-phase drift, removed by deferring
  `$00F0-$00FF` reads to their own access clocks. (Earlier "RMS 8.6e-2" numbers
  came from capturing ares at 32,000 Hz instead of its native 32,040, which
  left the host resampler on.)
- S-CPU/APU/PPU scheduling uses an exact master-clock credit (21,477,272 Hz;
  four clocks per PPU dot, 2,050,560 Hz SPC700). Memory wait states follow the
  ares table, and the DRAM refresh is charged inside the crossing CPU step.
- Echo writes are active whenever EDL/ESA enable them, but the exact echo
  buffer page wrap and read-only behavior still need an ares sample comparison.
- `Cpu::reset` now follows ares `CPU::main`: `step(132)` then the
  emulation-mode interrupt sequence. Exact reset pin timing is pending hardware
  capture.
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
  makes the frame bit-identical **against RustySNES** - see the hires-sampling
  item below, which shows RustySNES shares our 256-column compositing model and
  is therefore not a hires oracle.
- **PAL region timing is implemented.** The region comes from the cartridge
  header country byte (`Region::Ntsc`/`Region::Pal`; Europe and its variants,
  China, Australia and Russia are PAL, while Brazil's PAL-M stays 60 Hz). PAL
  uses the 21,281,370 Hz master clock (ares `Colorburst::PAL * 4.8`), so the
  S-SMP ticks per master clock change with it; frames are 312 lines (313 on the
  interlaced even field) with one 342-dot line at V=311, and `$213F` bit 4
  reports the region. Verified: `HblankEmuTest.sfc` (header country Europe) at
  300 vs ares `ARES_REGION=PAL` 302 is **100.000% exact** at the PAL crop
  (22, 29) with horizontal scale 2 in ares' 564x288 dump.
  - The NTSC one-short-line rule is also implemented (ares
    `PPUcounter::tickScanline`: 340 dots at V=240 on a non-interlaced odd
    field).
- **Overscan (`SETINI` bit 2) is implemented.** ares `PPU::updateVideoMode`
  sets `vdisp = !io.overscan ? 225 : 240`; our `Ppu::vdisp()` tracks it live,
  so vblank/NMI, the active-display access gates and the OAM reset all move to
  line 240. `PPU::main` latches `state.overscan` at line 0 for the output
  geometry: `frame_height()` becomes 240 (480 interlaced) instead of 224 (448),
  and the line-1..239 output is written to rows 1..239 with row 0 as the top
  border (row 0/1 interlaced), i.e. ares `DAC::scanline`'s `vcounter` row
  pointer with the 8-row vertical offset removed. VRAM, CGRAM and OAM accesses
  use the 240-line window.
  - Verified with a first-party fixture: `tools/snes-oracle/gen-overscan-rom.py`
    assembles a LoROM that enables overscan and paints every active line with
    colour 15. At frame 120 it is **100.000% exact** (61,440/61,440) against
    the ares NTSC dump at crop (26, 0) with horizontal scale 2, i.e. all 240
    rows including the top border.
  - Unit tests pin the 240-line geometry, the `visible_last` bounds and
    vblank/NMI moving from line 225 to line 240.
- **Interlace is implemented** on the hires geometry above. `frame_interlace`
  is latched at frame start from `SETINI` bit 0, `frame_height()` returns 448
  for those frames, and each field writes only its own output rows
  (`row = 2 * (line - 1) + field`, i.e. ares `DAC::scanline`'s `yScale = 2`
  plus `line += 564` for the odd field), leaving the other field's rows from the
  previous field exactly as hardware does. The frame length follows ares
  `PPUcounter::tickScanline`: 263 scanlines on an interlaced even field, 262
  otherwise. Sprites use their own `SETINI` bit 1 (`height >> interlace`,
  `y <<= 1`, then `+- field`) including the `baseSize >= 6` height quirk, and the
  hires BG fetch doubles vertically (`vpixel << 1 | field`).
  - Measured against ares `--dump-frame` (564x484 for an interlaced NTSC frame;
    the visible 512x448 area is the crop at (26, 18)) on
    `PeterLemon-PPU/Interlace/InterlaceMoogle.sfc` (mode 5, `SETINI = $01`):
    **94.849%** of pixels exact, mean channel difference 5.69. A non-interlaced
    render of the same ROM scores 83% against the same dump, so the field
    geometry is right; the residual is the item below.
- **Hires is pixel-exact against ares.** Both hires/interlace demos are
  **100.000%** identical to the ares dump (mean channel difference 0.000) at
  512x448 against the crop at (26, 18):
  `Interlace/InterlaceMoogle.sfc` (mode 5, `SETINI = $01`) and
  `HiColor/MosaicMode5.sfc`.
  - The model, all transcribed from ares `Background::fetchNameTable`/`run`:
    - `width = 256 << hires()`, so in hires the fetch space is 512 columns and
      `hsize` (and the map's horizontal extent) doubles.
    - **`htiles = 4` in hires**: a map entry covers 16 columns and supplies two
      consecutive characters - `character` for the first 8 columns and
      `character + 1` for the next 8, selected by `hoffset & 8` against the
      entry's horizontal-mirror bit.
    - `Background::run` is called once per screen and consumes one background
      pixel per call, so the **Below** screen shows BG column `2d` and the
      **Above** screen column `2d + 1`; the DAC interleaves below at the even
      output column and above at the odd one.
    - With mosaic enabled the interlace vertical doubling **drops the field
      bit**: `vpixel = vpixel << 1 | (field() && !mosaic.enable)`.
    - With mosaic enabled the Above screen reads the mosaic latch instead of
      its own fetched column: `mosaic.pixel` is refreshed only on the Below call
      (`x == 0 && (!hires() || screen == Below)`, or when the counter expires)
      and the Above call takes `else if(mosaic.enable) pixel = mosaic.pixel`, so
      both screens show the block-start *Below* column. This was the last
      1.25% on the mosaic demo.
  - Two earlier attempts failed because they kept the non-hires 8-column entry
    geometry (`htiles = 3`) with a doubled `hsize`, which wraps a 32-tile map
    after 256 columns and renders the picture twice (68.1% / 55.9%). The
    "per-screen fetching is falsified" note above is superseded: it applied to
    that geometry, not to per-screen sampling.
  - `hires_map_entries_cover_sixteen_columns` and
    `hires_mosaic_above_screen_reads_the_below_latch` pin the two rules that
    account for almost all of the gap.
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

## SuperFX (GSU)

`crates/unko-snes/src/gsu.rs` is a port of the ares `GSU` interpreter and the
`SuperFX` bus wrapper. It implements:

- all 256 opcodes (ALT1/ALT2/ALT3 modes, immediate forms, branches, `loop`,
  `jmpl`, `getb`/`getc`, `iwt`/`lm`/`sm`, `lms`/`sms`, `from`/`with`/`to`),
  with the 16-bit flag results (`SFR` Z/CY/S/OV, `b`, `alt`, `irq`);
- the 512-byte instruction cache (32 valid lines, `cache`/`ljmp` flushing,
  S-CPU writes to `$3100-$32FF`);
- the ROM/RAM bus: `$00-3f` LoROM ROM, `$40-5f` linear ROM, `$70-71` work RAM,
  the ROM/RAM buffer handshake (`romcl`/`ramcl`, `r14` updates, `getb`/`getc`
  sync), and the S-CPU `$3000-$34FF` register window;
- the plot pipeline: colour/CMODE, dither, the two-stage pixel cache, tile
  character-number arithmetic for all four screen modes, and `rpix`;
- the `CPUROM`/`CPURAM` S-CPU quirks (the 16-byte vector while the GSU runs
  with ROM enabled; RAM reads returning open bus while RAN is set);
- the S-CPU IRQ raised by `STOP` when `CFGR.IRQ` is clear, cleared by reading
  `$3031`;
- save-state coverage (the whole `Gsu` is part of `SnesState`).

Timing: the GSU oscillator is the S-CPU master clock on the supported boards,
and the S-CPU advances the GSU with a master-clock credit (whole instructions,
like the APU/PPU credit model). The GSU's internal `step` costs match ares
(cache line fetch 5/6 clocks, buffer access 5/6, fast-clock 1/2, multiply
3/7 x clock select).

Verification: the public **PeterLemon GSU suite** (31 test images under
`test-roms/snes/PeterLemon/SNES-CHIP-GSU-GSUTest/`) is run against ares at the
same frame. Every test's **result table is 100.000% pixel-identical** to ares
(31/31), covering ADD/ADC/SUB/SBC/CMP/AND/BIC/OR/XOR/MULT/UMULT/FMULT/LMULT/
ASR/DIV2/LSR/ROL/ROR/INC/DEC/SWAP/MERGE/SEX/LOB/HIB/NOT/IBT/IWT/MOVE/MOVES/
CACHEINJECT.

Known limitations:

- The first 6 header text rows in some of those screens differ. The test ROM
  draws its header into VRAM immediately after a `$4210` poll; our S-CPU reaches
  that poll in active display (so our VRAM-write gate drops the header), while
  ares reaches it in vblank. This is a pre-existing S-CPU/PPU startup phase
  difference exposed by the ROM's vblank race, not GSU behaviour; the result
  tables themselves are exact.
- HDMA A-bus reads are not routed into GSU RAM (normal HDMA targets only the
  PPU B-bus, and no measured ROM needs it).
- A release SuperFX game (Star Fox, Doom, Yoshi's Island) is not present in
  the local test set, so the full game path (IRQ handshake, multiple GSU
  programs, DMA of work RAM to VRAM) is not yet measured end to end.
- Only the GSU-1/GSU-2 (same-clock) boards are modelled; the Mario Chip 1
  clock-select differences are treated as the same master clock.

## NEC DSP-1/2/3/4 (uPD7725)

`crates/unko-snes/src/necdsp.rs` ports the ares `uPD96050` engine for the
uPD7725 revision (DSP-1/1A/1B, DSP-2/3/4):

- the full 24-bit-word interpreter: OP (all ALU operations, P-select,
  DPL/DPHM, RP decrement, move source/destination), RT, JP (every branch
  condition, `jmpl`/`hjmp`, calls/returns) and LD (all destinations including
  the DR/SR/SO/K/L/DP/TRB forms);
- 2K x 24-bit program ROM, 1K x 16-bit data ROM and 256 x 16-bit data RAM;
- the host interface: the DR data register (two-byte handshake in 16-bit mode,
  direct byte in 8-bit mode), the SR status image (RQM, DRS masked by DRC,
  USF0/1, P0/1), the DSP-1 STOP IRQ and its `$3031`-style acknowledge;
- free-running timing: the 7.6 MHz oscillator is ticked from the S-CPU master
  clock with a rational credit; host reads/writes exchange the current DR/SR
  instead of catching the DSP up synchronously (the model that avoids stopping
  on the first RQM of the firmware's wait loop);
- board window selection from the mapping and ROM size, matching ares'
  `NEC-HIROM` (`$00-$1F,$80-$9F:$6000-$7FFF`, SR in the upper page),
  `NEC-LOROM` (`$30-$3F,$B0-$BF:$8000-$FFFF`) and `NEC-LOROM-RAM`
  (`$60-$6F,$E0-$EF:$0000-$7FFF`) boards;
- firmware installed at runtime from the user-supplied 8 KiB
  `dsp1.rom`/`dsp1b.rom`/`dsp2.rom`/`dsp3.rom`/`dsp4.rom`
  (`Snes::set_dsp1_firmware`); the firmware is copyrighted chip data and is
  never embedded in this crate;
- the window heuristic also covers the wide `NEC-LOROM-RAM#A` window
  (`$20-$3F,$A0-$BF:$8000-$FFFF`) used by 1 MiB LoROM DSP carts with SRAM, in
  addition to the plain `$30-$3F` and >1 MiB `$60-$6F` windows;
- save-state coverage: the whole `NecDsp` (ROMs, RAM, registers, flags and
  window) is part of `SnesState`;
- `Cartridge::from_bytes_lenient` recovers a LoROM/HiROM header from the reset
  vector when the header table is empty, so headerless demos can boot.

Verification:

- **Super Mario Kart (USA)** (HiROM, header type `$05`, ares `NEC-HIROM-RAM`,
  `dsp1b.rom`): at frame 600 vs ares 600 the frame is **99.981%** exact
  (57,333/57,344, mean |d| 0.086). The 11 remaining pixels are two left-edge
  sprite columns at x=0-1, y=175-182 (a pre-existing sprite-edge detail, not
  DSP arithmetic). The entire Mode 7 track/title is DSP-driven and matches.
- **DSP1 Tech Demo (World)** (headerless 128 KiB LoROM, `dsp1.rom`): the demo
  uses the `$BF:$8000` DR / `$BF:$C000` SR window. With a local ares database
  entry forcing the `NEC-LOROM` board, the frame at 302 is **95.714%** exact.
  The DSP-driven grid is pixel-exact; only the demo's sprite-font glyphs
  differ (its custom bitplane/VRAM decompressor, not the DSP core).
- **DSP-2/3/4**: Dungeon Master (USA, DSP-2), SD Gundam GX (Japan, DSP-3) and
  Top Gear 3000 (USA, DSP-4) are all **100.000%** exact against ares at frame
  600 (Top Gear also at frame 1,500). The firmware is selected by the local
  verification entries in the ares database (`DSP2`/`DSP3`/`DSP4`) or by
  `DSP1_ROM`. Liveness check: running Dungeon Master/Gundam/Top Gear with a
  mismatched firmware changes the frame for DSP-3 (16,869 pixels) and DSP-4
  (1,764 pixels at frame 1,500), proving the chip output reaches the screen;
  Dungeon Master's title screen is DSP-independent (its in-game 3D view is
  reached only after input, and the test harness does not yet inject a working
  Start press).
- `snes-info` accepts `DSP1_ROM=<path>` and `JOYPAD0`/`JOYPAD0_FROM`; the
  firmware and the commercial ROMs are not committed.

## Capcom CX4 (Hitachi HG51B S169)

`crates/unko-snes/src/cx4.rs` ports the ares `HG51B` interpreter and the
`HitachiDSP` bus wrapper:

- the full 16-bit opcode decode: `NOP`, `JMP`/`JSR`/`RT` with every condition
  (`EQ`/`GE`/`GT`/`LT`/`LE`/`NE`, carry/negative/overflow/zero), `HLT`, all
  ALU forms (`ADD`/`ADC`/`SUB`/`SBC`/`CMP`/`AND`/`BIC`/`OR`/`XOR`/`XNOR`,
  immediate variants), shifts (`ASR`/`LSR`/`LSL`/`ROL`/`ROR`/`ROR imm`),
  `MUL`/`FMULT`, `SEX`/`SXB`/`SXW`, `INC`/`DEC`, `SWAP`, `LINK`, `SBK`,
  `LD`/`ST` with every source/destination, `RDRAM`/`WRRAM` (register and
  immediate forms), `LDROM`, `GETB`/`GETC`, `PUSH`/`PULL` and the exchange
  class;
- the two-page, 256-word instruction cache: line validity, `CACHE` reload,
  lock bits, preload, and the page-crossing reload in `advance`;
- the 3 KiB data RAM (S-CPU `$6000-$6BFF`/`$7000-$7BFF`) and the 1 KiB
  data ROM loaded from the user-supplied `cx4.rom`;
- the S-CPU register window (`$6C00-$6FFF`/`$7C00-$7FFF`): DMA, cache
  base/pb/pc/locks, wait states, IRQ, vector bytes and the 24-bit GPRs;
- the ROM/RAM buffer handshake for DMA, suspend/resume timings, the
  `$7F51` IRQ and the `--` HLT handshake;
- save-state coverage: the whole `Cx4` is part of `SnesState`, and the
  cartridge SRAM is passed to the DMA engine.

Timing: the HG51B oscillator is 20 MHz and is ticked from the S-CPU master
clock with a whole-instruction credit, like the APU/PPU. The internal costs
match ares (public RAM/ROM wait plus one clock per access, 5/6-clock cache
fills, 3/7-clock multiplies, suspend durations).

**Power-on state matters.** ares `HG51B::power` is `r = {}; io = {}`, which
leaves `halt` clear, so the DSP executes from ROM address 0 until the S-CPU
programs it. Mega Man X2's boot-time interface self-test reads the DSP
registers before that point: with `halt` asserted at power our build showed the
game's `INTERFACE REGISTER / REGISTER FILE / DATA RAM / IMMEDIATE ... ERROR!`
screen, while ares went straight to the Capcom logo. Clearing `halt` at power
(running from the reset state) fixed it.

Verification (same-frame comparisons against ares):

| ROM | Frames | Result |
|---|---|---|
| Mega Man X2 (USA) | 30, 45, 60, 90, 300, 600 | **100.000%** exact (57,344/57,344 each) |
| Mega Man X3 (USA) | 300, 600 | **100.000%** exact |

Firmware is copyrighted chip data and is never embedded: `CX4_ROM=<path>`
installs it in `snes-info`, and `Snes::set_cx4_firmware` is the library entry
point. The program itself lives in the game ROM and is fetched through the
cache.

Known limitations:

- Only mapping 0 (LoROM `$00-$3F,$80-$BF:$8000-$FFFF` plus `$C0-$FF` linear) is
  modelled, which is every CX4 board ares defines. A cartridge is detected as
  CX4 by cartridge type `$F3` with LoROM/ExLoROM.
- The firmware is attached explicitly by the host, so passing `CX4_ROM` to a
  non-CX4 cartridge would map the CX4 windows over that cartridge's own
  `$6000-$7FFF` space; only attach it when the cart declares a CX4.
- Input-injection is not available in the ares capture harness, so the
  in-game (post-title) path is not diffed frame-by-frame; the boot, logo and
  title screens are.

## S-DD1 decompression

`crates/unko-snes/src/sdd1.rs` ports the ares `SDD1` component and its
`Decompressor`:

- the `$4800-$480F` register window (`$4800` hard enable, `$4801` soft enable,
  `$4804-$4807` MMC banks masked to `0x8F`); `$4802/$4803` and `$4808-$480F`
  read the raw ROM at that offset, like ares;
- the MCU windows `$00-$3F,$80-$BF:$8000-$FFFF` (LoROM fold with the
  `r4805`/`r4807` bit-7 A21 remaps) and `$C0-$FF:$0000-$FFFF` (MMC bank
  registers, or a live decompression stream);
- the board's save RAM at `$00-$3F,$80-$BF:$6000-$7FFF` and
  `$70-$73:$0000-$FFFF`;
- the DMA snoop: every `$4300-$437F` write is mirrored so the chip knows each
  channel's source address and size. Decompression streams only while a
  matching read hits an armed channel, and the channel disables itself when
  the byte count reaches zero;
- the entropy decoder: input manager, eight Golomb-Rice bits generators, the
  32-context probability-estimation module, the context model and the output
  logic, with the ares `runCount`/`evolutionTable` tables copied verbatim.

**One bug was worth recording.** The probability-estimation evolution table has
**33** states (0..=32): state 31's `nextIfMps` is 32. Masking the stored status
to 5 bits (`& 31`) folded state 32 back to 0 and desynchronised the stream. For
Star Ocean's third stream (6,144 bytes) the first 51 bytes were right and the
remaining 5,334 were wrong; correcting it made all four streams byte-exact.
The decoder was checked against an independent Python transcription of the ares
algorithm, stream by stream.

Verification (same-frame comparisons against ares):

| ROM | Frames | Result |
|---|---|---|
| Street Fighter Alpha 2 (USA) | 120, 300, 600 | **100.000%** exact (57,344/57,344 each) |
| Star Ocean (Japan) | 120, 600 | **100.000%** exact |

Star Ocean at frame 90 differs (94.046%): ares is still black while our ENIX
logo is already fading in, i.e. a boot-phase timing offset of a few frames that
converges by frame 120. The decompressed logo itself is byte-identical.

Cartridges are detected as S-DD1 from map modes `$22`/`$32` (the ares/mia
`SDD1-` rule), so no firmware or database entry is needed. Only the `SDD1`
board layouts (no extra coprocessor RAM) are modelled; save-state coverage
includes the register file, DMA shadows and decompressor state.

## Resolved 2026-11-17 - blank save RAM is 0xFF, not 0x00

ares allocates every memory with a fill value: `WritableMemory::allocate(size,
n8 fill = 0xff)`, so a cartridge's save RAM powers up as **0xFF** (a save file
overwrites it). We initialised it to 0x00.

That is not cosmetic: a blank 0x00 save area looks like *data*, while 0xFF is
the "erased" marker games scan for. SMRPG's save-slot scan at `$C3:4C4A` reads
`$40:0000 + slot*8`, compares against `$FF` and takes a completely different
branch depending on the answer, so our whole boot differed.

Evidence that this was the SMRPG root cause:

- with the fix, the S-CPU instruction streams for **frames 0-30 are identical**
  (`ARES_PC_TRACE` vs `PC_TRACE` with polling loops canonicalised; before the
  fix the first divergence was inside that save-slot scan at `$C3:4C58`);
- our `$2100` (INIDISP) timeline now matches ares **exactly** instead of
  running 5-6 frames late:

  | event | ares | ours (before) | ours (after) |
  |---|---|---|---|
  | fade-in `$01..$0f` | 30.81-37.80 | 36-43 | 30-37 |
  | fade-out `$0f..$01` | 374.24-381.23 | 380-387 | 374-381 |
  | force blank `$8f` | 382.23 | 388 | 382 |
  | fade-in `$02,$03,$04` | 491.07/495.07/499.06 | 497 | 491/495/499 |

## Resolved 2026-11-17 - joypad serial ports and the NMI-enable edge

Found by diffing ares' and our S-CPU **instruction traces** from power-on
(`ARES_PC_TRACE` in the patched ares build, `PC_TRACE` in `snes-info`). The two
CPUs matched for 62,281 instructions and then diverged in the APU IPL upload
loop (a benign poll-count difference), and the first *real* divergence was in
the controller/multitap probe at `$01:8617`.

### 1. `$4016`/`$4017` serial-port bits

The probe reads `$4017`, shifts right twice and rotates the carry into a flag,
so it observes **bit 1** of `$4017` eight times. ares (`CPU::readCPU`) drives:

- `$4016`: bits 0-1 = controller port 1 `data()` = serial bit + **second data
  line** (low for a standard gamepad),
- `$4017`: bits 0-1 = port 2 `data()`, bits 2-4 tied to ground.

We forced bits 1-2 high and put port 2's serial bit in `$4016.d1`, so KDL3's
probe read all ones and concluded "no second controller / multitap absent".
`io.rs` now matches ares: `$4016.d1` and `$4017.d1` are low for a standard
gamepad, `$4017.d2-d4` are ground, and each read advances only its own port.

### 2. Enabling NMI during vblank

ares `CPU::nmitimenUpdate` does
`if(io.nmiEnable.raise(bit7) && status.nmiLine) status.nmiTransition = 1`, so
raising the NMI enable while the vblank flag is latched delivers an NMI
immediately. We only latched NMIs at the vblank edge itself, so KDL3 - which
turns NMIs on with `$4200 = $81` partway through vblank - received no NMI at
all and never entered its `$82F6` handler. The `$4200` write now raises the NMI
when the enable rises while `nmi_flag` is set **and the PPU is in vblank**
(ares' `nmiLine` is only ever set while `nmiValid`; without the vblank test our
flag could survive into the visible period and produce a spurious NMI, which
shifted Super Mario Kart's DSP-1 frames).

### 3. `irqLock` delay after a `$4200` write

ares clears `status.irqLock` at the start of every bus access and sets it in
`nmitimenUpdate`, so the interrupt test at the end of the instruction that
writes `$4200` is **skipped** and the NMI lands one instruction later. Adding
that delay (`Snes::nmi_lock`, cleared at the top of every `Snes::step`) made
the traces agree through the first NMI: the earlier mismatch
(`$B8BC` vs `$B8BD`) disappeared.

Verification: KDL3 now runs its NMI loop (19 NMIs by frame 120, previously the
S-CPU crashed at `$00:5FFF` with a corrupted stack); all other results are
unchanged (see the regression table at the end of this document).

### Where the KDL3 trace now diverges

With all three fixes the two instruction streams agree for the first ~1.2
million S-CPU instructions. The first remaining difference is the frame
rendezvous at `$00:9490`:

```
$9490: INC $602C
$9493: BIT $6002
$9496: BPL $949F          ; no SA-1 handshake -> wait on $4212 vblank edges
$9498: LDA $602A          ; SA-1 handshake -> wait for the SA-1's frame flag
$949B: BEQ $9498
```

ares spins at `$9498/$949B` waiting for BWRAM `$602A` and is still there when
the NMI arrives; in ours `$602A` is already non-zero, so we fall through
immediately.

`SA1_BWLOG=1` records every BWRAM write with its master clock, which corrects
an earlier guess: **`$602A` is not an SA-1 flag** - the only `STA $602A` in the
image sits inside compressed graphics data. The writers we actually observe
(all side `CPU`, i.e. the S-CPU through the `$00-$3F:$6000-$7FFF` window):

| Clock | Write | Origin |
|---|---|---|
| 36,412,000 | `$6002 = $81` | `$B8B5`, enabling NMI + auto-poll |
| 36,412,594 | `$602A = 1` | the first NMI handler |
| 36,527,620 | S-CPU reaches `$9498` (`PC_WATCH`) | main-loop frame wait |
| 36,527,714 | `$602A = 0` | `$94AB` |

So our first NMI handler runs about **115,000 master clocks (≈0.32 frame,
≈84 scanlines) before** the main loop first reaches `$9498`, which is the
opposite order from ares (whose loop is already spinning at `$9498` when the
NMI lands). The frame-rendezvous phase is therefore inverted, and that is what
shifts every later frame. ares also runs the SA-1 API registration routines
(`$00:E98A` twice, `$00:ABB1`, `$00:F54F`, `$00:B8C8`) that we never reach, and
its BWRAM API table at `$0222` (`3A 0A`, bank `C1`, built by `$00:E98A` from
the ROM table at `$FC02`) is populated while ours is still zero - but the
dispatch pointer `$72AC = $00EF31` matches exactly, so the table is downstream
of the phase problem rather than a separate defect.

## SA-1 (Super Accelerator 1)

`crates/unko-snes/src/sa1.rs` ports the ares `SA1` component. The SA-1 is a
second WDC 65C816 (our existing cycle-stepped core) plus:

- the super-MMC ROM banking: `$00-$3F,$80-$BF:$8000-$FFFF` folds like LoROM with
  the `$2220-$2223` C/D/E/F bank registers and their mode bits, and
  `$C0-$FF:$0000-$FFFF` is banked; the `$7FEA/$7FEB` and `$7FEE/$7FEF` vector
  overrides are applied to the S-CPU;
- the shared save RAM ("BWRAM"): `$00-$3F,$80-$BF:$6000-$7FFF` (banked by
  `$2224`), `$40-$4F:$0000-$FFFF`, the SA-1's 32x/128x projections, the 4bpp and
  2bpp bit-map window formats and the `BWPA`/`SWEN`/`CWEN` write protection;
- the 2 KiB internal RAM at `$3000-$37FF` with the `SIWP`/`CIWP` block
  protection;
- the three DMA modes: normal memory-to-memory, type-1 character conversion
  (streamed from a BWRAM read) and type-2 character conversion (from the
  `$2240-$224F` bit-map register file);
- the H/V and linear timers with the `$2210` mode bits, the `$2212-$2215`
  targets and timer IRQs;
- the interrupt/message model: `$2200`/`$2201`/`$2202`/`$2209`/`$220a`/
  `$220b`, the `$220c-$220f` S-CPU vectors, `$2300`/`$2301` flags, and the
  S-CPU IRQ line;
- the `$2250-$2254` arithmetic unit (signed multiply, unsigned divide, sigma);
- save-state coverage of the register file, I-RAM, DMA state and CPU.

Timing: one SA-1 cycle is **two master clocks** (ares `SA1::step`), with ares'
bus-contention penalties reproduced from the S-CPU's current address
(`ROM::conflict`, `BWRAM::conflict`, `IRAM::conflict`) and its DRAM-refresh
window. The SA-1 runs from a whole-instruction master-clock credit, like the
other coprocessors.

Verification against ares at the same frame:

| ROM | Frames | Result |
|---|---|---|
| Super Mario RPG (USA) | 120, 180, 300, 400 | **100.000%** exact (57,344/57,344 each) |
| Kirby Super Star (USA) | 120 | **100.000%** exact |

That covers the S-CPU boot, the SA-1 handoff and reset-vector setup, the
super-MMC banking, the intro/logo/title screens and the first few hundred
frames of gameplay HUD rendering.

**Known gaps (not yet verified):**

- Super Mario RPG diverges at about frame 500 (4.954% exact at 600). Bisected
  against ares: frames 400, 450, 475, 488, 490 and 492 are **100.000%** exact
  and **frame 493 is the first divergence**, staying different afterwards.

  The cause is a fade/palette ramp, not a wrong palette: at frame 492 both
  machines are black and identical, at 493 ares shows the incoming scene at
  full palette (30 distinct colours, every pixel lit) while ours is still
  black, and by frame 500 ours shows the *same* scene (identical BG mode 1,
  `TM=$17`, `TS=$00`, BG bases and scroll) with `INIDISP` brightness **2**
  where ares is fully lit. So the scene, the tilemap and the register setup
  all agree; only the brightness/palette ramp into it is late in ours.

  **Update: the ~5-frame lag was the save-RAM fill (see "blank save RAM is
  0xFF" above) and is fixed** - the INIDISP timelines now agree exactly and
  frames 0-30 trace-identically. What remains at frame 493 is a small residual:
  the same scene at the same brightness, but the red and green channels are
  higher in ours (`(61,61,79)` vs ares' `(0,0,63)` at frame 500, `(30,30,47)`
  vs `(0,0,47)` at 493 - blue matches, and the difference scales with
  `INIDISP`), so a palette entry or colour-math setup still differs. Frame 493
  is now 1.339% exact rather than 0%.

  **Colour math is ruled out.** The ares `ARES_PPU_LOG` hook now also records
  `$2130-$2132`; both cores write the *same* values at the same frames -
  `$2130=$30, $2131=$00, $2132=$e0` at frames 0 and 27, then
  `$2130=$00, $2131=$a4, $2132=$e0` at frame 384 (subtract, backdrop + BG3,
  fixed colour black at intensity 0, i.e. a no-op).

  **The residual is the CGRAM contents.** `CGRAM_OUT` dumps all 256 palette
  words, and replaying the `$2121`/`$2122` streams from both logs reproduces
  the emulator's palette exactly, so the two can be compared:

  | frame | entries differing | our CGRAM |
  |---|---|---|
  | 30 | - | 29 non-zero, plausible palette |
  | 380 | - | 31 non-zero, plausible palette |
  | 493 | **37 / 256** | **239 non-zero**, a bogus grey ramp at 0-11 |

  The full palette *upload* itself is byte-identical in both cores (the
  `$2122` stream `CA 29 FF 7F 10 6E 60 44 8D 36 10 ...` at frame 442.82 in
  ares and 443.00 in ours), so the corruption happens in the **per-frame fade
  write stream** during the loading period:

  | frames 487-492 | `$2121` | `$2122` |
  |---|---|---|
  | ares | 1122 | 304 |
  | ours | 996 | 364 |

  We issue ~11% fewer `$2121` and ~20% more `$2122` writes than ares, so the
  fade lands on different entries and the scene appears at a slightly
  different palette.

  **The HDMA is not at fault.** `HDMA_LOG` shows the fade is driven by two
  channels writing one value per scanline:

  | channel | target | mode | indirect table |
  |---|---|---|---|
  | 5 | `$2121` (CGADD) | 1 byte | `$7E:C000` |
  | 7 | `$2122` (CGDATA) | 2 bytes (`ctrl=$42`) | `$7E:4010` |

  and dumping WRAM at frame 490 shows **the tables themselves are wrong in
  ours**:

  ```
  $7E:4000  4F 10 40 4F 10 40 9B 10 40 3C 44 40 01 46 40 00 00 7C 21 7C 42 7C 63 7C 84 ...
  $7E:C000  00 00 34 74 FC EC FE EA FC E2 7F C1 2F 59 23 45 00 00 74 04 F4 04 F2 02 ...
  ```

  Everything after the leading `00 00` is palette-like data rather than the
  per-scanline CGADD/CGDATA values ares sees (`$7E:C000` should read all
  `$00` there, i.e. "CGADD = 0 every line" - the classic per-scanline backdrop
  gradient). The ramp `7C 21 7C 42 7C 63 7C 84 7C A5 7C C6 7C E7` is exactly
  what appears in our CGRAM entries 1-7, so the HDMA and the PPU are faithfully
  applying a table that is already garbage.

  The remaining SMRPG defect is therefore **whatever generates the fade table
  at `$7E:4000`/`$7E:C000`**, not HDMA, the PPU or the palette logic.

  `WRAM_WATCH` (plus a new `Snes::live_pc`, since the CPU is swapped out during
  `step` and `cpu.pc` reads as power-on there) names the writers:

  | writes | routine | what it is |
  |---|---|---|
  | 144 | `$C0:F7E2` | a generic run-length decompressor copy loop (`LDA [$20],Y` / `STA $0000,X`) that fills `$7E:C000` |
  | 48 + 2/frame | `$C3:C1E0`, `$C3:BB26`/`$C3:BB2C` | the fade-table updater |

  `$C3:BB13` is the per-frame updater:

  ```
  C3:BB1B: LDA $0A84        ; fade counter
  C3:BB1E: EOR #$FF
  C3:BB20: INC A            ; A = -$0A84
  C3:BB21: LSR A
  C3:BB22: CLC
  C3:BB23: ADC #$28
  C3:BB25: LSR A
  C3:BB26: STA $7E4000      ; HDMA entry counts of the per-scanline gradient
  C3:BB2A: ADC #$00
  C3:BB2C: STA $7E4003
  ```

  So `$7E:4000` is a **mode-1 HDMA table with a 3-byte stride**
  (`[count, lo, hi]`) for a per-scanline `COLDATA`/gradient, and the routine
  only rewrites the line **counts** from the fade variable `$0A84`;
  `$7E:C000` is filled by the decompressor.

  **The fade counter matches.** Logging `$0A84` beside every `$2100` write in
  both cores gives identical values at identical clocks:

  | INIDISP write | ares clock / `$0A84` | ours clock / `$0A84` |
  |---|---|---|
  | `$02` | 175,782,060 / `$14` | 175,782,018 / `$14` |
  | `$03` | 177,211,496 / `$18` | 177,211,488 / `$18` |
  | `$04` | 178,640,962 / `$1c` | 178,640,952 / `$1c` |

  and the earlier fade at frames 30-37 has `$0A84 = $00` in both. (Our clock
  deltas against ares are now under 60 master clocks across the whole run, so
  the timeline is effectively exact.)

  **The tables are byte-identical - the decompressor is innocent.** Exfiltrating
  ares' WRAM (`ARES_WRAM_LOG`, dumped on every `$2100` write) and diffing
  against our `WRAM_OUT` at the frame-491 `$2100 = $02` write gives exactly the
  same bytes in both:

  ```
  $7E:c000  00 00 34 74 fc ec fe ea fc e2 7f c1 2f 59 23 45 00 00 74 04 f4 04 ...
  $7E:4010  00 7c 21 7c 42 7c 63 7c 84 7c a5 7c c6 7c e7 7c 08 7d 29 7d 4a 7d ...
  ```

  **The HDMA channel setup is identical too.** ares' channel dump
  (`CPU::oracleDumpChannels`) and our `HDMA_DUMP` agree field-for-field:

  | | target | mode | indirect | srcBank:srcAddr | indBank |
  |---|---|---|---|---|---|
  | ch5 (ares) | `$21` | 0 | yes | `$c3:$c299` | `$7e` |
  | ch5 (ours) | `$21` | 0 | yes | `$c3:$c299` | `$7e` |
  | ch7 (ares) | `$22` | 2 | yes | `$7e:$4000` | `$7e` |
  | ch7 (ours) | `$22` | 2 | yes | `$7e:$4000` | `$7e` |

  and the ROM table at `$C3:C299` decodes as two `count=$F0, ptr=$7E:5000`
  blocks followed by a terminator.

  **The defect is confined to channel 5's block cursor.** Comparing the two
  engines at the same `$2100` write (ours via `Dma::hdma_cursor` in the
  `PPU_LOG` line, ares via `oracleDumpChannels`):

  | at `$2100 = $02` | table cursor | indirect ptr | line counter |
  |---|---|---|---|
  | ch7 ares | `$400C` | `$4046` | `$14` |
  | ch7 ours | `$400C` | `$4046` | `$14` |
  | ch5 ares | `$C2A2` | `$00A2` | `$00` |
  | ch5 ours | `$C2A2` | `$2045` | `$C7` |

  Channel 7 (CGDATA, mode 2) matches **exactly** - so the per-frame `hdmaSetup`
  timing is *not* the problem (this refutes the "setup runs too early"
  hypothesis). The table cursor for channel 5 also matches, but its **indirect
  pointer and line counter do not**: ares has exhausted the block and hit the
  terminator (`lc = 0`, `ptr = $00A2`, i.e. the terminator's bytes) while we
  still have 199 lines left (`lc = $C7`) and a different pointer. Channel 5 is
  the **mode-0** indirect channel (`ctrl = $40`, 1 byte per line) where channel
  7 is mode 2, so the bug is in the mode-0 indirect path of
  `Dma::hdma_line`/`hdma_reload`.

  **Resolved: `HdmaBus` bypassed the coprocessors.** A per-scanline dump of
  channel 5 in both engines showed the very first advance already differing in
  exactly one field, with the table cursor and line counter identical:

  ```
  #0  ours table=C29C ind=C010 lc=EF   |   ares table=c29c ind=5001 lc=ef
  ```

  The ROM at `$C3:c299` is `F0 00 50` = `count=$F0, lo=$00, hi=$50`, i.e. the
  pointer must be `$5000`; we loaded `$C00F`. The cause was that `HdmaBus::read`
  fell back to `cart.read(address)` without consulting the SA-1, so an HDMA
  A-bus read used the *linear* cartridge mapping instead of the super-MMC
  mapping the S-CPU sees (ares' HDMA reads use the full CPU bus). `HdmaBus` now
  carries the same `sa1`/`sdd1`/`spc7110` views as the CPU and DMA buses.

  Effect on Super Mario RPG:

  | frame | before | after |
  |---|---|---|
  | 493 (first mismatch) | 1.339% exact, mean |d| 48.7 | **88.438% exact, mean |d| 0.356** |
  | 494 | 0.893% | **95.323%** |
  | 500 | 0% exact, mean |d| 108.9 | 0% exact, mean |d| **23.3** |

  A residual still remains at frame 500 (mean |d| 23.3), so this is a large fix
  rather than the end of the story.

  **Re-checked after the routing fix: channel 5 now matches ares exactly.** The
  earlier "still `$C010`" reading came from a log produced before the fix
  binary was in place. `HDMA_RELOAD_LOG` shows the fetch is right
  (`countAt=$C299 count=$F0 low=$00 high=$50 -> ind=$5000`, then `$C29C ->
  $5000`, then the terminator `$C29F -> $00A2`), and the per-scanline traces
  are identical for all 1338 entries:

  ```
  ours  ch5 table=c29c ind=5001 lc=ef   |   ares ch5 table=c29c ind=5001 lc=ef
  ours  ch5 table=c29c ind=5002 lc=ee   |   ares ch5 table=c29c ind=5002 lc=ee
  ```

  (our run logs 1344 advances against ares' 1338 - the run boundary, not a
  functional difference). So HDMA is no longer implicated in the SMRPG
  residual; whatever remains at frame 500 is elsewhere.

  **The residual is a one-frame phase offset, not a value difference.**
  Dumping CGRAM at frame 500 (`CGRAM_OUT` vs replaying ares' `$2121`/`$2122`
  log) gives **0 of 256 entries differing**, and the frame-500 pixel difference
  is a *uniform* multiplicative factor on every channel of every pixel
  (79/63, 69/55, 74/59 are all 1.254 - the same `INIDISP` brightness relation).
  Comparing one frame earlier settles it:

  | comparison | result |
  |---|---|
  | ours 499 vs ares 500 | **100.000% exact** (57,344/57,344) |
  | ours 500 vs ares 500 | 0% (mean |d| 23.3) |
  | ours 501 vs ares 500 | 0% |

  So our per-frame fade step lands one frame earlier than ares'. Every *value*
  is right - palette, HDMA tables and pointers, `$0A84`, colour math,
  `INIDISP`, per-scanline gradient - so what remains is the ordering of the
  per-frame CGRAM/`$2100` update against the frame boundary.

  Confirmed with more pairings, each **100.000% exact**:

  | ours | ares | result |
  |---|---|---|
  | 493 | 494 | 57,344/57,344 |
  | 499 | 500 | 57,344/57,344 |
  | 500 | 501 | 57,344/57,344 |

  and `ares 500` vs `ares 501` really does differ (mean |d| 62.4), so adjacent
  frames are distinct and the offset is real rather than a static-screen
  artefact. Note this is *not* a universal capture-index error: for Super Mario
  Kart the aligned pairing is `ours 600` vs `ares 600` (99.981%), and `ours 599`
  vs `ares 600` is worse (97.586%), so the harness indices agree there.

  **Settled: the harnesses agree, so the offset is emulation.** A first-party
  fixture (`tools/snes-oracle/gen-frame-counter-rom.py`) publishes its own NMI
  frame counter as CGRAM colour 0, so the frame index is readable off the
  frame. Our dump shows colour 0 = `0x12c` / `0x12d` / `0x12e` at frames
  300 / 301 / 302, and the captured pixel is **identical in both emulators at
  the same frame number** ((90,74,0) at 300 and (99,74,0) at 301). So
  `FRAMES=N` and `--frames N` select the same frame, there is no capture
  off-by-one, and Super Mario RPG really is one frame ahead of ares through the
  fade.

  **The frame boundary is not the cause.** Logging the PPU line/dot at which
  each `$2100` write commits shows the two engines agree exactly:

  ```
  ours 178640950 line=231 dot=57  $04    ares 178640962 line=231  $04
  ours 177211492 line=231 dot=59  $03    ares 177211496 line=231  $03
  ours 175782026 line=231 dot=58  $02    ares 175782060 line=231  $02
  ours 136820830 line=225 dot=29  $8f    ares 136820832 line=225  $8f
  ```

  All the writes land on the same scanline (231, i.e. in vblank, so they affect
  the following frame) within a few dots. So neither the values nor the commit
  position differ, and no change to `Ppu::tick`/`frame_ready` is justified.

  **The gradient tables match too.** Dumping `$7E:4000` and `$7E:4010` at the
  frame-499 fade step gives byte-identical tables:

  ```
  ares 4000: 4d 10 40 4d 10 40 9b 10 40 3c 44 40 01 46 40 00 00 7c 21 7c 42 ...
  ours 4000: 4D 10 40 4D 10 40 9B 10 40 3C 44 40 01 46 40 00 00 7C 21 7C 42 ...
  ```

  so `$C3:bb13`'s per-frame count update is not advancing early either.

  At this point **every observable the cores expose is identical** - palette,
  both gradient tables, the whole HDMA ch5 trace, `$0A84`, colour math,
  `INIDISP` values and their commit line/dot - yet SMRPG's content is still
  exactly one frame behind (`ours 493/499/500 == ares 494/500/501`). The
  remaining difference is therefore not a *value* anywhere in the PPU, HDMA or
  game state; it is the phase of frame *presentation* itself, which the
  frame-counter fixture cannot expose because it measures NMI frames rather than
  the SMRPG-specific update.

  Kirby's Dream Land 3 is unchanged by the routing fix (0% exact at frames
  120/300/600; 1 colour versus ares' 10 at frame 600).

  Off-by-one: we also issue two extra `$2100` writes ares does not (`$01` at
  frame 24 and `$AC` at frame 418), both invisible because both cores are
  force-blanked there.

  The historical analysis below still explains why the mismatch surfaced at
  frame 493 rather than earlier. Logging every
  `$2100` (INIDISP) write with a frame stamp in both cores (`PPU_LOG` /
  `ARES_PPU_LOG`) gives:

  | event | ares | ours | lag |
  |---|---|---|---|
  | first fade-in step (`$01`) | frame 30.81 | frame 36 | 5.2 |
  | fade-out starts (`$0D`) | frame 374.24 | frame 381 | 5.8 |
  | force blank on (`$8F`) | frame 382.23 | frame 388 | 5.8 |
  | second fade-in starts (`$02`) | frame 491.07 | frame 497.06 | 5.9 |

  So the brightness/palette logic is *correct* - both cores run the same
  sequence of `$2100` values, one step per frame - but ours is a constant ~5-6
  frames (about 1.8-2.1 M master clocks) behind from the very first fade at
  frame 30, and the lag creeps up by ~0.7 frames over the next 460. Because
  both cores are force-blanked (pure black) for most of frames 388-492, that
  offset is invisible in the frame diffs until a fade is in progress, which is
  exactly why the first mismatch appears at frame 493.

  The remaining work is therefore not PPU-side: something in the first 30
  frames costs us ~5 frames of game time. The next step is to find the clock of
  the first `$2100 = $01` in both cores and work backwards, or trace-diff
  SMRPG's S-CPU; the palette and `INIDISP` handling can be ruled out.
- Kirby's Dream Land 3's first-NMI point re-checked with the current build and
  unchanged: ares runs `$B8BD` and then enters the `$82f6` NMI handler, while at
  the corresponding point ours is executing the frame rendezvous
  (`$9498`/`$949b` -> `$949d`), i.e. we exit the `$602A` wait before ares does.
- Kirby's Dream Land 3 improved again with the save-RAM fill fix: the S-CPU
  now reaches the frame rendezvous at `$00:9498`/`$00:949B` and waits there
  (NMIs climbing normally: 18/198/498 at frames 120/300/600) instead of
  derailing, and BWRAM `$0200-$023f` is the expected blank `$ff`. Its frames
  still differ from ares (ours 1 colour, ares 10 at frame 600) and its S-CPU
  trace still first separates at the first NMI: ares enters the `$82f6` handler
  while ours is at `$949d`, so the frame-rendezvous phase is still inverted.
- Kirby's Dream Land 3 no longer derails. The null `$36DE` pointer turned out
  to be a symptom of two unrelated CPU bugs (both fixed, see "Resolved
  2026-11-17 - joypad serial ports and the NMI-enable edge" below): the game
  took the wrong multitap-detection branch and then missed its first NMI, which
  sent it through the SA-1 API dispatch into empty memory. With both fixes the
  S-CPU runs its NMI-driven main loop (`nmi_services` climbs normally) instead
  of crashing at `$00:5FFF`. The frame still does not match ares (both are
  essentially black at frames 120-600; ares shows a few non-black pixels), so
  something later in the boot is still off.
- The SA-1's `$40-$4F` BWRAM window is the only extra S-CPU mapping modelled;
  the boot ROM / BS-X slot and the `$50-$6F` open-bus windows are not.

## SPC7110

`crates/unko-snes/src/spc7110.rs` ports the ares `SPC7110` component:

- the **memory control unit**: the `$00-$3F,$80-$BF:$8000-$FFFF` window folded
  with ares' `mask=0x800000`, the `$C0-$FF:$0000-$FFFF` window with
  `mask=0xc00000` (both reduce to `((bank & 0x3f) << 16) | offset`), the
  `$4830-$4834` bank/size registers, the 1 MiB program ROM split from the data
  ROM, and the optional 8 KiB save RAM gated by `$4830` bit 7;
- the **decompression unit**: `$4801-$4804` table base/index, `$4805/$4806`
  seek length, `$4807` stride, `$4809-$480C` counter and status, and the
  neviksti/talarubi `q_zip` arithmetic coder (53-state evolution table,
  per-context MPS/LPS swap, the 5x15 context model, the inverse Morton
  transform and the most-recently-used colour list);
- the **data port unit**: `$4810-$481A` with the base+adjust+stride addressing
  modes and the `$4818` settings bits (stride enable, adjust enable, sign
  extension, adjust-instead-of-base);
- the **arithmetic logic unit**: `$4820-$482F` with unsigned/signed 16x16
  multiply and 32/16 divide, including the divide-by-zero clamp;
- the **Epson RTC-4513** on `$F9` boards at `$00-$3F,$80-$BF:$4840-$4842`
  (`epsonrtc.rs`): the three-port serial protocol (`$4840` chip select, `$4841`
  nibble stream, `$4842` ready), the full BCD calendar with ares' bit-perfect
  emulation of the chip's *invalid* BCD values, leap years, `hold`/`stop`/
  `pause`, the 12/24-hour modes, the resync/battery-failure flags and the
  periodic IRQ flag with its mask/duty/period registers. It runs at
  `32768 * 64` Hz from a master-clock credit;
- the register window `$00-$3F,$80-$BF:$4800-$483F` plus ares' `$50`/`$58`
  full-bank aliases.

Timing: the chip runs at the master clock. Each pending unit (DCU transfer,
multiply, divide) executes on the next tick and then stalls the S-CPU for its
latency (20 clocks for a DCU transfer, 30/40 for multiply/divide), which is
added to the CPU's access debt like the DRAM refresh stalls.

**Verification.** `Momotarou Dentetsu Happy (Japan)` (sha256
`9fc7a66464e71d0f056fed2b560f527a5af69034c96293a2731107479763a9d8`, cartridge
type `$F5`, map mode `$3A`) turns out to contain a **built-in SPC7110
self-test**, which makes it an unusually good oracle: with start pressed it
runs the firmware's own checks and reports

```
REG. INIT. OK   SRAM DATA BUS OK   SRAM ADDR BUS OK   SRAM R/W OK
DATA PORT ACC OK   MUL OK   DIV OK   C-LENGTH OK   DATA PORT B+O+S OK
COMPLETE / ALL OK
```

covering the register file, the save-RAM data/address buses, the data port
accumulator and base+offset+stride paths, the multiply and divide units and the
decompression length, and our run prints `ALL OK` with the same screen.
Frames are also **100.000% pixel-identical** to ares at 120, 300 and 600
(57,344/57,344 each, mean channel difference 0.000); without input the cartridge
halts on that "POWER OFF PLEASE" screen, which is why both emulators stay
static there.

Two more local SPC7110 titles were diffed against ares at frames 300 and 600,
both **100.000%** exact: `Super Power League 4 (Japan)` (type `$F5`, 2 MiB:
1 MiB program + 1 MiB data ROM) and `Tengai Makyou Zero (Japan)` (type `$F9`,
5 MiB: 1 MiB program + 4 MiB data ROM, with the Epson RTC). Neither gets past
its opening screen without input, so those diffs verify the MCU banking and
(through the RTC's 1 Hz tick, `seconds = 9` at 600 frames) that the clock runs;
the RTC register protocol itself is covered by unit tests rather than by a
frame comparison, because no local `$F9` title talks to the RTC before its
first interactive screen.

Known limitation: the `EXSPC7110` (7 MiB, with the extra 1 MiB expansion ROM at
`$40-$4F`) layout is not handled.

## Frame boundary and mid-line register writes (audited 2026-11-17)

Two long-standing items were audited directly against the ares reference and
are **not divergences from it**.

**The frame boundary already matches.** ares' `PPU::main`
(`ppu-performance/ppu.cpp`) renders lines `1 .. vdisp()-1`, then at
`vcounter() == 240` calls `screen->frame()` and `scheduler.exit(Event::Frame)`.
Our `Ppu::tick` latches `frame_ready` at `line == 240` for the same reason, and
the harness dumps on that latch. The first-party frame-counter fixture
(`gen-frame-counter-rom.py`) confirms the two harnesses select the same frame
(`CGRAM` colour 0 = `0x12c`/`0x12d`/`0x12e` at frames 300/301/302, and the
captured pixel is identical in both emulators at each frame number). So there is
no boundary change to make, and the SMRPG one-frame phase is *not* the
presentation point.

**Whole-line register writes are ares behaviour.** `PPU::DAC::render()` writes a
complete scanline into the output buffer per call (`for(u32 x : range(256))`),
and `bg1.render()` .. `obj.render()` likewise run once per line from
`PPU::main`; `io.displayBrightness` is sampled once per line. So a PPU register
write *within* a visible line cannot produce a mid-line change in ares either.
Our whole-line model therefore matches the primary oracle; the item only
remains open as a divergence from real hardware / Mesen, not from ares.

## NMI delivery: ours fires, ares does not (SMRPG, measured 2026-11-17)

Instrumenting the NMI acceptance point in both cores (`NMI_LOG` in
`Snes::apply_interrupts`, `ARES_NMI_LOG` in `CPU::main`'s `status.nmiPending`
branch) produced an asymmetry that no previous measurement had shown:

| | deliveries over 500 frames | where |
|---|---|---|
| ours | **15** | every one at PPU `line = 225`, `dot = 1-3`, one per frame from ~frame 485 |
| ares | **0** | `status.nmiPending` is never taken |

**Correction: that asymmetry was a stdio buffering artefact.** ares' dump hooks
use `std::fprintf` to a buffered `FILE*`; the NMI/`$4200`/`$4210` hooks fire too
rarely to fill a 4 KiB buffer, so the file was created but never flushed and
appeared empty. Adding `std::fflush` after each write shows ares delivers the
**same 15 NMIs**, and they pair up almost exactly with ours:

```
ours 177917810  ares 177917824  delta 14
ours 178275180  ares 178275184  delta  4
ours 178632548  ares 178632552  delta  4
ours 178989912  ares 178989914  delta  2
```

so NMI acceptance is **not** the difference either. Any ares dump hook that can
fire rarely must `fflush` - otherwise an empty file reads as "the event never
happened", which is exactly the wrong conclusion. Since the one-frame phase first appears at frame 493 - i.e. inside exactly
that window - an NMI we accept that ares does not is a strong candidate for it.

The next step is to compare the *enable* state around frames 483-500: `$4200`
bit 7, `$4210` reads, and whether the NMI is being acknowledged/cleared on the
same instruction in both cores. Our NMIs all show `r4210 = $80` at acceptance,
so the flag is set at the moment we deliver.

## KDL3: the S-CPU takes the wrong frame-wait branch (2026-11-17)

Instrumenting BWRAM `$602a` (the frame flag) in both cores over 125 frames
gives a lopsided result:

| | `$602a` events |
|---|---|
| ours | **79,795** (79,795 reads, all from `$9498`, plus a handful of writes) |
| ares | **31** (almost all `W602a=01`, no polling storm) |

The frame rendezvous is:

```
$9490: INC $602C
$9493: BIT $6002
$9496: BPL $949F      ; bit 7 clear -> wait on $4212 vblank edges
$9498: LDA $602A      ; bit 7 set   -> wait on the SA-1 frame flag
$949B: BEQ $9498
```

So **ares takes the `BPL` branch and waits on `$4212`, while we take the `BMI`
branch and spin on `$602a`** - our `$6002` bit 7 is set where ares' is clear at
that moment. `$6002` bit 7 is written by the S-CPU itself at `$B8b0`
(`LDA $6002; ORA #$81; STA $6002`), so the next step is to compare the `$6002`
value and that write's timing between the cores.

## Resolved 2026-11-17 - SA-1 BW-RAM `$40-$4F` is not banked by `sbm`

KDL3's S-CPU keeps its PPU settings in BW-RAM: direct page is `$6000`, so
`LDA $5F` at `$00:842C` reads BW-RAM `$605F`, and the routine at `$83F0` then
copies `$604C`-`$606B` into `$2100`, `$2105`, `$2107`-`$2135`. When that block
is wrong the game renders with `BGMODE=9`, `TM=TM=FF`, `overscan`, `interlace`
and `hires` all set - a black screen.

The block was wrong because of the BW-RAM window bank rules. ares gives the
`$00-$3F,$80-$BF:$6000-$7FFF` window `size=0x2000` in its board map, so
`SA1::BWRAM::readCPU`/`writeCPU` receive the 13-bit window offset and apply
`sbm`; the `$40-$4F:$0000-$FFFF` window is mapped full width, so those handlers
see the untranslated 24-bit address and **do not** apply `sbm`.

We applied `sbm` whenever the mirrored offset happened to be below `$2000`,
which wrongly caught the `$40` window as well: `$40:0022` landed at linear
`$6022` instead of `$0022`. KDL3's `$E955` loop (`LDA #$FFFF; STA $0022,X;
STA $00A2,X; STA $01A2,X; STZ $0122,X; ...`) then painted `$FF` over
`$6022-$6041`, destroying the settings block that `$B8E3` had just written
(`$6040 = $10`, the `TM` value). ares' copy of the same loop writes bank 0
(`$0022`), leaving `$6040` intact.

Measured with `SA1_BWLOG`/`ARES_BWLOW`, same instruction, same registers
(`pc=$E958`, `d=$3600`, `dbr=$40`, `a=$FFFF`, `x` = 0, 2, ... 0x1E in both):

| | target |
|---|---|
| ares | `$0022`, `$0023`, `$00A2` ... (bank 0) |
| ours (before) | `$6022`, `$6023`, `$60A2` ... (bank 3) |

After the fix the BW-RAM image matches ares (`$6000-$6070` from 49 differing
bytes down to 2, both off-by-one counters), `$2100`/`$2121`/`$212C` finally
receive the real values, and **KDL3 is 100.000% exact against ares at frames
120, 300 and 600**.

## Resolved 2026-11-17 - power-on work RAM is ares' pattern, not zeros

ares does not zero S-CPU work RAM at power-on:

```cpp
// ares/sfc/cpu/cpu.cpp
if(!reset) random.array({(u8*)wram, sizeof(wram)});   // CPU::power
```

`Random::array` with the low-entropy setting (which `System::power` selects for
the SFC) fills the 128 KiB with a *structured* pattern: two byte values chosen
by two address bits, with rare single-bit flips. The RNG is seeded from the host
clock unless ares' "Deterministic Entropy" developer setting is on, so two ares
runs of the same game can differ.

We zero-filled WRAM. That is visible: **two ares runs of Super Mario RPG at
frame 120 differ by 112,715 of 131,072 WRAM bytes and 4.1% of pixels**, because
the game reads uninitialised buffers (its per-frame fade tables at
`$7E:4200-$4FFF`, `$7E:6800-$9FFF` and friends are read before they are
written). Any comparison against a nondeterministic oracle is therefore
meaningless for such games.

Fixes:

- `crates/unko-snes/src/random.rs` ports ares' `Random` (PCG32 `step`,
  `random`, `array(Low)`) and fills WRAM at power-on with the seed ares uses in
  deterministic mode, `(0, 0)`. Verified byte-exact: ares' frame-1 WRAM and our
  power-on image agree in **0 of 24,576** bytes across `$4200-$4FFF`,
  `$6800-$9FFF` and `$E000-$F9FF` (the regions the game has not yet touched).
- The oracle build honours `ARES_DETERMINISTIC` (same as ares' own deterministic
  setting), and `tools/snes-oracle/ares-capture.sh` always exports it, so
  captures are reproducible. Two deterministic ares runs are now byte-identical
  (0/131,072 WRAM bytes).

Effect on Super Mario RPG at frame 500: **WRAM 0/131,072 and BW-RAM 0/32,768
bytes differ** from ares (previously 54,880 WRAM bytes), i.e. the entire memory
image now matches. The frame still differs there (`ours N == ares N+1`,
100.000% for 493/494, 499/500 and 500/501), so the residual is a rendering-phase
difference with identical memory, not a memory-content difference.

## SMRPG frame 493+ - located: VRAM high-byte stream shifted by one write

With the memory image proven identical (WRAM 0/131,072, BW-RAM 0/32,768 at frame
500) the remaining difference had to be in data the renderer consumes. VRAM was
never compared before, and it differs:

| frame | VRAM bytes differing (of 65,536) |
|---|---|
| 60, 120, 300 | 0 |
| 440, 470, 490, 498, 500 | **557** |

Every differing byte is the **high byte** of a VRAM word, in words
`$0042-$0E40`, and the whole high-byte stream is **one write late** in ours:

```
write  address byte  ares  ours
81986  $0081    low   01    01
81987  $0081    high  58    00   <-- first divergence
81988  $0082    low   02    02
81989  $0082    high  58    58
82001  $0088    high  00    58   <-- shifted
82003  $0089    high  55    00
```

The addresses are identical at every index and only high-byte *data* is shifted,
so this is not an address-mapping bug (unlike the KDL3 `sbm` case): our write
stream has the data of the previous write. The per-line register snapshot is
*not* involved - logging `$2100`, `$212C`, `$2130-$2132` and CGRAM word 0 at the
moment each line is rendered (frames 498-501, lines 50 and 150) gives identical
values in both cores (`bright` 3 then 4, `tm=$17`, `cg0=$7C00`).

Instrumentation added: `PPU_LINE_LOG` (per-rendered-line register snapshot),
`VRAM_OUT` + `Ppu::vram_words`, `VRAM_WRITE_LOG` ($2118/$2119 write stream) and
the ares equivalents `ARES_PPU_LINE_LOG`, `ARES_DUMP_BASE` `.vram`, and
`ARES_VRAM_LOG`.

**The `$2118`/`$2119` acceptance gate is not the cause.** Logging the full write
context - `$2115` mode, increment and the pre-increment word address - at every
write shows the two cores agree on *all* of them at the first divergence:

```
      ares                                  ours
81984 A=0080 b0 00 mode=1 inc=1 addr=0080 | A=0080 b0 00 mode=1 inc=1 addr=0080
81986 A=0081 b0 01 mode=1 inc=1 addr=0081 | A=0081 b0 01 mode=1 inc=1 addr=0081
81987 A=0081 b1 58 mode=1 inc=1 addr=0081 | A=0081 b1 00 mode=1 inc=1 addr=0081  <-- data only
81988 A=0082 b0 02 mode=1 inc=1 addr=0082 | A=0082 b0 02 mode=1 inc=1 addr=0082
```

Address, byte index, `$2115` mode and increment are identical and only the
*datum* differs, so this is neither an address mapping nor a write-acceptance
rule: the byte our PPU is handed is wrong. (The earlier "187 extra writes" was
a log-window artefact - both logs start at "frame >= 300", whose clock differs
slightly between the two cores, so the totals are not comparable.)

**DMA is not the cause - it is byte-identical.** Two log bugs had produced
false leads here, and both are worth recording:

1. The DMA-register hook (`DMAREG_LOG`) filtered on the *low 16 bits* of the bus
   address, so decompressor stores to WRAM `$7F:4300-$7F:437F` were counted as
   DMA-register writes and showed up as a 128-write "extra block" from
   `$C0:FB80`. With the full 24-bit address logged, both cores write
   `$4300-$437F` **663 times with identical values**.
2. The DMA B-write hook compared a run gated at "frame >= 300" in ares against
   one that was not gated in ours, so tail alignment produced a spurious
   divergence (`$00:0070` fixed fill vs `$7E:A000`). With our side frame-gated
   too (`Dma::debug_frame`, mirrored from the PPU), the two B-write streams to
   `$2118`/`$2119` match **106,371 of 106,371 entries** over frames 300-440.

So the S-CPU programs the DMA channels identically *and* every DMA upload is
identical. The first differing VRAM write is therefore a **CPU store** to
`$2118`/`$2119` carrying a different byte (`$00` where ares has `$58`), which
points at the value the S-CPU holds at that store (a decompressed scratch
buffer or table) rather than at the PPU, HDMA, DMA or the (identical) frame-500
memory image.

Next step: log `STA $2118/$2119` with the accumulator and PC in both cores and
trace where the differing byte is loaded from.

## Resolved 2026-11-17 - VMADD reloads the VRAM read latch

ares' `$2116`/`$2117` handlers reload the VRAM read latch:

```cpp
case 0x2116: { vram.address.bit(0,7) = data; latch.vram = readVRAM(); return; }
case 0x2117: { vram.address.bit(8,15) = data; latch.vram = readVRAM(); return; }
```

We only updated `vram_address`. That matters for **read-modify-write uploads**,
which Super Mario RPG uses for its tilemaps:

```
C3:C2B9: LDA $A000,X     ; 8-bit: replaces A.b, keeps A.h
C3:C2BC: REP #$20
C3:C2BE: PHA
C3:C2BF: LDA $60
C3:C2C1: STA $2116       ; ares reloads the read latch here
C3:C2C5: PLA             ; A = 00|byte  (high byte preserved from the $2139 read)
C3:C2C6: STA $002118     ; write low + high in one 16-bit store
```

Because the latch was stale, the high byte the routine preserved was whatever a
previous read had left behind, so the uploaded words were wrong.

Fix: reload `vram_read_latch` in both handlers.

Verification, all against deterministic ares captures:

| check | before | after |
|---|---|---|
| CPU stores to `$2118`/`$2119` (frames 0-440) | diverged at #16,453 (`$2119 = $00` vs `$58`) | **20,482 vs 20,482, identical** |
| VRAM at frame 440 | 557 bytes differ | **0 / 65,536** |
| VRAM at frame 500 | 557 bytes differ | **0 / 65,536** |

Super Mario RPG's rendered frame at 493-501 is **unchanged** by this (it still
shows the one-frame relationship below), so the two defects are independent: the
VRAM corruption is fixed, and the presentation phase remains.

## SMRPG presentation phase (open)

With WRAM (0/131,072), BW-RAM (0/32,768), VRAM (0/65,536), CGRAM (0/256) and the
per-rendered-line register snapshot (`$2100`/`$212C`/`$2130-$2132`, lines 50 and
150 over frames 498-501) all identical, the rendered frame is still exactly one
frame ahead of ares:

```
ours 493 == ares 494   100.000%
ours 499 == ares 500   100.000%
ours 493 vs ares 493    88.438%
```

The frame-index fixture shows the two harnesses select the same frame (CGRAM
colour 0 = `0x12c`/`0x12d`/`0x12e` at 300/301/302 in both), so this is not a
capture artefact: it affects only content that changes per frame, and every
piece of state we can compare matches at the same frame index.

## SMRPG phase: ares samples a mid-line write one scanline earlier

A new fixture (`tools/snes-oracle/gen-midline-brightness-rom.py`) dims `$2100`
from a V-IRQ at scanline 100 and restores it in NMI, so the frame shows a
bright/dim boundary whose row index is the scanline at which each renderer read
the register. With a solid white backdrop the boundary is unambiguous:

```
        row 98   row 99   row 100
ours    255      255      143
ares    255      143      143
```

So ares applies a mid-line brightness change **one scanline earlier** than we do
- i.e. our renderer samples the register one line earlier relative to the CPU's
writes. That is the right shape for the SMRPG symptom (its fade is written at
scanline 231, in vblank, and the whole frame comes out one frame apart while
every register, VRAM, CGRAM and WRAM value at the frame boundary is identical).

**Resolved: the render dot was 21, ares uses 128.** `PPU::main` runs
`step(renderingCycle)` before rendering a line, and `renderingCycle` is 512
master clocks (`PPUcounter::tick` counts master clocks) - 128 dots. So a
register write in the first 128 dots of scanline L belongs to line L; sampling at
dot 21 (end of hblank) was too early.

`Snes::advance_peripherals_inner` now renders at `PPU_RENDER_DOT = 128`, and the
fixture agrees with ares exactly:

```
        first dim row
ours     99
ares     99
```

Everything else stayed at parity: 212/212 baseline, `spc_timer`, `hvdma`,
`hvdma_max`, PAL, overscan, KDL3 120/300/600, SMRPG 400, KSS 120, CX4, GSU and
SPC7110 MDH/SPL/TMZ all 100.000%; DSP-1 99.981%.

It does **not** change the SMRPG frame phase (still `ours493 == ares494`,
`ours500 == ares501`, 100.000%, with `ours493` vs `ares493` at 88.438%), so that
phase is not intra-line sampling.

## SMRPG phase: presentation logged, harness misalignment ruled out

`PRESENT_LOG` (ours, at `Ppu::acknowledge_frame`) and `ARES_PRESENT_LOG` (ares,
at `vcounter() == 240`) record the presentation index and the first visible
pixel at the exact moment a frame is presented.

Super Mario RPG, 501 frames:

```
ours  present 497 pix=00003fff
      present 498 pix=00003fff
      present 499 pix=00004fff   <- fade step
      present 500 pix=00004fff

ares  present 498 pix=0001fc00
      present 499 pix=0001fc00
      present 500 pix=00027c00   <- same fade step
      present 501 pix=00027c00
```

The same visual transition lands at presentation index 499 in ours and 500 in
ares, i.e. the offset is in the *emulated* frame, not in the harness: for the
frame-index fixture both harnesses display the same ROM frame at the same `N`
(CGRAM colour 0 = `0x12c` at 300 in both), and `ares-capture.sh` waits for trace
index `frames - 1`, exactly like ours.

A like-for-like pairing was also tested and falsified:

```
ares@493 frames vs ours@494 frames   83.827%
ares@500 frames vs ours@501 frames    0.000%
```

so re-indexing the capture does not remove the difference.

Note: ares presents one extra frame at start-up (502 presentations for
`--frames 501` versus our 501), which is worth knowing when reading its trace
but does not affect the captured index.

## SMRPG phase: state, not presentation - and one unpresented frame

`PRESENT_STATE` / `ARES_PRESENT_STATE` record FNV-1a hashes of WRAM, VRAM and
CGRAM plus the presented pixel at the instant a frame is presented.

Super Mario RPG (501 frames):

```
ours  state 497 wram=a2f3c947 vram=653c1828 cgram=5f63e71a pix=00003fff
      state 498 wram=f51a6c6f ...
      state 499 wram=3254284a ...
      state 500 wram=44eae006 ...

ares  state 498 wram=a2f3c947 vram=653c1828 cgram=5f63e71a pix=0001fc00
      state 499 wram=f51a6c6f ...
      state 500 wram=3254284a ...
      state 501 wram=44eae006 ...
```

The *state* sequence is identical but shifted by one index: our presentation N
shows the state ares presents at N+1. So our game is one frame ahead in
emulated time while presenting the same number of images.

The same probe on the frame-index fixture matches at the **same index**
(301/301 states), so this is not the probe or the harness: it is specific to
this game.

And our own PPU frame counter gives the mechanism away:

```
                 presentation index    Ppu::debug_frame
fixture                   300                  301   (+1, correct)
SMRPG                     500                  502   (+2)
```

For SMRPG our PPU completes one more frame boundary than we present - a
`line == 240` event that is never handed to the harness - so every later
presentation shows a state one frame newer. The fixture does not have it.

Next step: log every `line == 240` event with its master clock in both cores and
find the extra one in SMRPG (it will be a frame whose `frame_ready` is set but
never consumed, or a duplicated boundary).

## Resolved 2026-11-18 - a DMA longer than a frame hid a frame boundary

The state fingerprints (`PRESENT_STATE`/`ARES_PRESENT_STATE`) showed SMRPG's
presented state sequences identical but shifted one index, while the frame-index
fixture matched at the same index. Our own counter explained it:

```
                 presentation index   Ppu::debug_frame
fixture                   300               301   (+1)
SMRPG                     500               502   (+2)
```

Scanning the whole run found exactly one skipped boundary, at presentation 382:

```
idx=381 pf=382
idx=382 pf=384   <-- two frame boundaries, one presentation
```

**Cause.** `Snes::step` ran a whole DMA to completion. A 64 KiB transfer costs
~0.5 M master clocks - more than a frame - and the PPU is advanced inside that
loop, so two `line == 240` boundaries can pass with only one `frame_ready` flag
set. The harness then presented one image for two frames and every later index
showed a state one frame newer. On hardware (and in ares, where the PPU runs as
its own thread) the frames are presented while the CPU is stalled.

**Fix.** A step now stops at a frame boundary: the unfinished `DmaTransfer` is
parked in `Snes::dma_transfer` and resumed by the next step, so exactly one frame
is presented per completed frame.

**Result** - no frame is skipped (`pf` increments by 1 for all 501 frames), the
presented state sequences line up at the same index, and Super Mario RPG is now
**100.000% exact at frames 493 and 500** against deterministic ares captures
(previously 88.438% and 0.000%), with `ours500` vs `ares501` correctly *not*
matching any more.

## Resolved 2026-11-18 - OAM x is signed: sprites at the left edge

The last Super Mario Kart pixel differences (11 of 57,344 at frame 600) were a
two-pixel-wide, eight-pixel-tall sliver at `x = 0..1, y = 175..182` where ares
draws a dark object and we drew background.

OAM x is nine bits but the screen is 256 wide, so values `>= 256` are *negative*
positions: x = 511 is one pixel off the left edge. ares clips objects against a
signed window (`Object::fetch`'s `x1`, `within<-128,+383>(...)`) so the visible
rightmost columns still draw. We treated x as unsigned, computed `sx = 511`,
and dropped the tile with `sx >= 256`.

Fix: sign-extend the nine-bit x and clip per pixel with a signed range.

```
Super Mario Kart, frame 600: 57,344 / 57,344 exact (was 57,333)
```

This was never a DSP-1 defect: the label "DSP-1 sprite-font residual" came from
the pixels looking like a glyph. SMK is now 100.000%.

## Resolved 2026-11-18 - EXSPC7110 expansion ROM (`$40-$4F`)

mia splits a 7 MiB `EXSPC7110-` image as **1 MiB program + 5 MiB data + 1 MiB
expansion** (`board[0] == "EXSPC7110" && size == 0x700000`, the Tengai Makyou
Zero fan translation), and ares maps the expansion at `40-4f:0000-ffff`
(`memory type=ROM content=Expansion`, loaded through `Cartridge::loadROM`).

We treated every SPC7110 image as `program = first MiB`, `data = rest`, so the
last MiB was served as data ROM and `$40-$4F` fell through to the plain
cartridge mapping.

Implemented in `spc7110.rs`:

- `split_image()` returns `(program, data, expansion)` and recognises the
  7 MiB layout, with `is_exspc7110()`;
- `expansion` memory plus `expansion_read()`; the window is sixteen 64 KiB banks
  covering the 1 MiB part, so the offset is **20 bits** (the first version used
  16 and aliased every bank - caught by a unit test);
- `SnesBus` and `HdmaBus` read the expansion window and treat it as read-only.

Verified by tests that need no ROM image:

| test | asserts |
|---|---|
| `splits_a_seven_mib_exspc7110_image` | 1/5/1 MiB split, boundary bytes, and that a plain SPC7110 keeps `data = rest` |
| `expansion_rom_is_mapped_at_40_4f` | window classification, `$40:0000`, `$40:1234`, `$4F:FFFF`, empty on a plain board |
| `exspc7110_expansion_rom_is_mapped_at_40_4f` | full system: `$00:8000` = program, `$40:0000`/`$4F:FFFF` = expansion, write is ignored, `$6000-$7FFF` save RAM still works |

All 161 lib tests pass and the three local SPC7110 titles (MDH, SPL, TMZ) remain
100.000% exact at frame 600.

## Star Fox hangs: `$4201` PIO powers up set (2026-11-18)

The first *release* SuperFX title is now available locally, and it exposed a
boot bug: Star Fox never enabled the screen and never took an NMI, spinning
forever at `$03:BD83`:

```
BD83  E2 20     SEP #$20
BD85  AD 37 21  LDA $2137
BD88  AD 3D 21  LDA $213D     ; OPVCT
BD8B  EB        XBA
BD8C  AD 3D 21  LDA $213D
BD8F  EB        XBA
BD90  CD DD 16  CMP $16DD     ; target scanline
BD93  D0 EE     BNE $BD83
```

That is the standard "wait for the beam" idiom: `$2137` reads latch the H/V
counters when `$4201` bit 7 (`PIO`) is set, and `$213C`/`$213D` then return the
latched value, low byte first then the high bit. Two things were wrong:

- `CpuIo` powered up with `pio = 0`, but ares' `CPU::IO` has `n8 pio = 0xff`,
  so bit 7 is *set* at reset. With it clear, no `$2137` read ever latched the
  counters and `$213D` stayed 0 forever.
- The `$4201` write path did not implement ares' edge latch
  (`if(io.pio.bit(7) && !data.bit(7)) ppu.latchCounters()`), so a game that
  clears bit 7 explicitly would still not sample the beam.

Instruction-level comparison against ares' `ARES_PC_TRACE` located this exactly:
the first difference was A at `$03:BD8B` (ares `1`, ours `0`), i.e. the OPVCT
read.

### Star Fox vs deterministic ares captures

| frame | result |
|---|---|
| 120 | **100.000%** (57,344/57,344) |
| 300 | **100.000%** |
| 310 | 97.062% (1,685 px) |
| 320 | 83.245% (9,608 px, a large object region) |
| 330 | 96.165% (2,199 px) |
| 340 | **100.000%** |
| 350 | 99.248% (431 px) |
| 400 | 98.678% |
| 500 | 98.699% |
| 600 | 98.247% |
| 900 | 97.923% |

So a release GSU title now boots, runs its GSU-driven intro and produces
pixel-exact frames. The result is **not monotonic** - frame 340 is exact again
after 310-330 differ - so this is not a cumulative drift but a per-frame
difference (frame 320 shows a large contiguous object region, the others are
scattered single pixels, e.g. 940 of 1,005 at frame 600 have one side black).

### The GSU's own plot stream is bit-identical

`GSU_PLOT_LOG` / `ARES_GSU_PLOT_LOG` record every `PLOT` and `RPIX` with its
arguments (`plot` logs x, y and `COLR`; `rpix` logs x, y and the byte read).
Running Star Fox to frame 308 in both cores produces:

```
ares 51476 events, ours 51476 events, first differing event: None
```

i.e. **every** plot coordinate and colour, and every `RPIX` address and result,
matches for frames 154-308 - the window that contains the frame-307 divergence.
So the GSU's 3D maths, its `PLOT`/`RPIX` decoding and the pixel-cache/`flush`
pipeline all agree with ares; the residual per-frame differences come from the
S-CPU/PPU side (what the game uploads to VRAM and when, or GSU-RAM state the
S-CPU also uses), not from GSU rendering.

(This needs the raw arguments logged on both sides: ares' `rpix()` rewrites its
local `x` to `(x & 7) ^ 7` before the read, so logging after that shows 7 where
the caller passed 0.)

### Star Fox is content-exact through 900 frames

`PRESENT_HASH` / `ARES_PRESENT_HASH` make both harnesses emit an FNV-1a-64 hash
of the *same* 256x224 RGBA pixel set (ares' canvas at crop (26,9), horizontal
scale 2), so frames can be aligned by content instead of by number. Running Star
Fox to 920 frames on both sides:

```
our frames matching an ares frame anywhere: 901/920
first our frame with no ares match: 901   (the ares capture ends at 919)
```

**Every one of our frames 0..900 matches an ares frame byte-for-byte.** The 19
unmatched frames are simply past the end of the ares capture, which the offset
pushes out of range. So Star Fox's rendering is exact - there is no divergence at
frames 307/320/600/900 at all; those were frame-number offsets.

What *does* drift is how many frames each harness presents for the same game
progress: ares needs progressively more frames to reach the same picture.

| ours | matching ares frames | offset |
|---|---|---|
| 200 | 198..202 | 0 |
| 290 | 289..291 | 0 |
| 320 | 321..323 | +1 |
| 400 | 404..406 | +4 |
| 500 | 507..511 | +7 |
| 600 | 606..611 | +6 |
| 700 | 712..716 | +12 |
| 800 | 814..818 | +14 |
| 890 | 910..914 | +20 |

i.e. we present ~2% fewer frames than ares for the same content. Since the
*content* always matches, this is a presentation-count difference (a frame that
ares presents and we do not), not a rendering difference - the frame count is
still an open item, but it cannot affect what the game sees.

### ares' captures are NOT frame-locked (methodology correction)

`PRESENT_HASH` now records the emulated clock with each frame. Over Star Fox to
920 frames:

| | frame-to-frame clocks | deviation from the emulated frame (357,368) |
|---|---|---|
| ares | 285,202 .. 427,056 | mean 18,334, max 72,166 |
| ours | 357,308 .. 357,424 | mean 9, max 60 |

So ares' presentation callbacks are **host-timed**: they fire anywhere from 80%
to 120% of an emulated frame apart, i.e. the sampled phase within the emulated
frame varies by roughly +-0.2 frame. Our presentations are locked to the PPU's
frame boundary (within 60 clocks).

Consequences:

- ares' frame index N is *not* "the state after N emulated frames"; it is a
  snapshot at an arbitrary point inside one. When the game updates its picture
  near the frame boundary - which Star Fox does every few frames - the captured
  frame can contain the content of emulated frame N or N+-1.
- That is exactly what the "frame 307 / 320 diverge" readings were: our frame 306
  matched ares' 307 byte-identically and our 320 matched ares' 322/323 (the
  picture is static across those spans), while same-number comparisons caught
  ares on the other side of the update.
- It also dissolves the "~2% frame-presentation-count difference": both harnesses
  emit 920 frames and reach frame 900 within **0.09 emulated frames** of each
  other (ours 321,956,772 clocks, ares 321,990,512). The count matched all
  along; only the *phase* of each sample differed, which made a hash-based
  content alignment look like a growing index offset.

So Star Fox is content-exact, and per-frame *pixel* comparisons against the
host-timed ares captures are only trustworthy for static content. Two ways to
make them trustworthy:

1. compare **emulator state** (VRAM / GSU RAM / WRAM at a known clock) rather
   than the presented frame - this is what the CX4 bisects already do; or
2. add a frame-locked capture: emit the hash from the PPU's frame boundary
   instead of the host video callback.

### Star Fox frame numbering drifts: compare by content, not by number

The two capture harnesses do not stay frame-aligned for Star Fox, and the offset
grows. Comparing our frame N against a range of ares frames:

| ours | 100.000% match at ares | same-number result |
|---|---|---|
| 305 | 306 | 100.000% (static) |
| 306 | 307 | 100.000% |
| 307 | 308 | 100.000% (arena: 99.072% at 307) |
| 320 | **322** and 323 | 83.245% at 320/321 |
| 900 | none of 900-915 (best 98.9% at 915) | 97.923% |

So Star Fox is **content-exact at least through its frame ~322** - the earlier
"frame 306/307/320 diverge" readings were the frame-number offset, not
rendering errors. The offset is +1 at 307, +2 at 320, and at frame 900 no nearby
ares frame matches, so a real difference does develop somewhere between ~322 and
~900.

Practical consequence: any Star Fox comparison must align by content. The
`spc_timer`/SMRPG/KDL3 style "same frame number" comparisons are only valid where
the two harnesses stay in step, which they do for those titles.

### VRAM/CPU-state bisect (superseded by the alignment finding)

`WRAM_DIR` / `ARES_WRAM_DIR` and `VRAM_DIR` / `ARES_VRAM_DIR` dump work RAM and
VRAM per frame. Against ares:

- **WRAM**: into the 3D scene the only difference is **5 bytes at `$02F1-$02F5`
  for frames 60-129**, which then becomes identical. Those bytes are the
  **stack** (the S-CPU's `S` is `$02F4` there), i.e. call-history residue from
  interrupting the game at a different instruction - not a state divergence.
  Frame-boundary WRAM diffs are dominated by this, so WRAM is a poor signal.
- **VRAM** (the displayed state) is **byte-identical from frame 60 through
  152** and first differs at **frame 153**, by 89 bytes: a 45-word run of
  `0x010B` at words `$7A38-$7A63` where ares has `0000` after `$7A37`:

```
frame 153, words $7A30..$7A3F
ares 1905 1906 1907 010B 010B 010B 010B 000B 0000 0000 ...
ours 1905 1906 1907 010B 010B 010B 010B 010B 010B 010B ...
```

At that same frame the **GSU work RAM is byte-identical** and the **plot/rpix
stream is identical** (1,823 events from power-on, zero differences), so the
difference is in the S-CPU's VRAM fill/upload: our core writes a longer run of
the same word (`$7A33-$7A63`) where ares stops at `$7A37`.

The raw `$2118/$2119` write logs are not directly comparable - ours logs every
bus write (102,747 lines to frame 154), ares logs in `PPU::writeIO` (142,805) -
so the next step is to log the same point in both cores with the DMA flag and
compare the transfer that covers `$7A37`.

## CX4 gameplay diffed (2026-11-18)

Mega Man X2 reaches its in-game intro on its own (no input needed), so CX4
gameplay can be diffed after all. Against deterministic ares captures:

| frame | before the fix | after the fix |
|---|---|---|
| 1800 | 100.000% (57,344/57,344) | **100.000%** |
| 2400 | 100.000% | **100.000%** |
| 3000 | 100.000% | **100.000%** |
| 3100 | 100.000% | **100.000%** |
| 3200 | 100.000% | **100.000%** |
| 3300 | 99.224% (445 pixels) | **100.000%** |
| 3400 | 97.811% | **100.000%** |
| 3700 | 97.583% | 96.488% |
| 3900 | 86.679% | 95.933% |
| 4000 | 84.321% | 93.687% |

**Resolved (2026-11-18):** the first divergence was a 445-pixel region at
x=161..208, y=30..71 - a character portrait drawn through the CX4 bitmap/plot
path - caused by our `MUL` not sign-extending its 24-bit operands (see below).
CX4 output is now pixel-exact through frame 3400; a second, later defect remains
(3700 onwards).

### Root cause: `MUL`/`ASR` sign extension

Per-frame CX4 data-RAM snapshots (`CX4_RAM_DIR` / `ARES_CX4RAM_DIR`) show the RAM
is byte-identical through frame 3229 and first differs at frame 3230 by four
bytes (`071E/071F/07D0/07D1`, `ares=01 ours=03`). S-CPU traffic into the CX4
windows (`CX4_TRACE` / `ARES_CX4_TRACE`) is identical through frame 3232, and
the HG51B executes the same number of instructions per frame (122,804 at frame
3229), so the firmware was being driven identically and was computing differently.

Aligning the per-instruction logs (`CX4_INSN_LOG` / `ARES_CX4_INSN_LOG`, which
record `pb:pc opcode a p dpr ram gpr0,gpr14,gpr15,gpr13`) puts the first
difference at `$0008:6A` (`LD A, mul`, opcode `0x6001`): ares reads `$FFFFB5`
from the multiplier's high word, we read `$00FFB4`.

`MUL` took `(self.regs.a as i32)` / `read_register(reg) as i32` without
sign-extending bit 23, so a negative 24-bit operand became a large positive
value; ares multiplies `i24` operands. `ASR` had the same class of bug: it
shifted `(a & 0xffffff) as i32`, i.e. logically rather than arithmetically. Both
now use a `sign_extend_24()` helper (the `MUL` immediate stays unsigned, as in
ares' `instructionMUL(n8)`).

After the fix the firmware's data-RAM writes are identical for frames 3225-3232
(`CX4_WR_LOG`) and frames 3300/3400 are 100.000%.

### Remaining: a handshake race at frame 3477

The next divergence starts at **frame 3477** (CX4 data RAM differs by 1295 of
3072 bytes; the difference then persists). What is already ruled out:

- The S-CPU traffic into the CX4 is identical: comparing the full
  `CX4_TRACE`/`ARES_CX4_TRACE` streams, all **177,547** data-RAM and register
  writes match, in order, for the whole run up to this point.
- The firmware's instruction stream is identical up to the first job start after
  the previous `HALT`; the first difference is one GPR value.

The cause is the handshake itself. Momotarou-style job setup writes
`$7F4D/$7F4E` (program bank), then `$7F48` (cache page, which forces a reload),
then **`$7F4F` (which clears `halt` and starts the job), and only then the
parameter GPRs** `$7F80+`. The job therefore races with the parameter writes; it
only works because the forced reload started by `$7F48` takes ~2,728 coprocessor
clocks, covering the ~90 clocks of GPR writes.

Both cores perform that reload (measured in the instruction log: ares `d=2728`),
but ours performs **more reloads at the job boundary** - 2,920 vs 2,900 in a
two-frame window - and so starts the job ~1.4k clocks later in CPU time. By the
time the firmware reads its parameters, ares has already written `gpr2 = 0`
while ours still sees the previous job's `0x200`.

A `PREP` event (logged from `cache()` in both cores: program bank, cache base and
page) shows the loads themselves are now identical - same base, same page, same
order - up to the point where the streams split. At that point (log event 5792)
ares *starts executing the firmware job* while our core is still working
through a queued group of four more forced reloads (`pb` 8, a, 8, 2). So the
difference is not the load parameters but that our coprocessor falls behind the
CPU around the blocking reload and processes the queued `$7F48` requests later,
by which time the game has moved on.

The interleaving is therefore still not bit-identical around the blocking reload.

`PREP` now also logs the page-cache state *before* the page is overwritten
(`page`, `a0`, `a1` and the lock bits) on both sides. At the split the two states
are identical - every one of the 2,900 (ares) / 2,920 (ours) reloads has the same
program bank, request address, page and residents - and every request is one
whose address is already resident in `a0`/`a1`, i.e. the reloads are *forced* by
`$7F48` (or by a locked page) rather than misses. **The page-cache bookkeeping is
not stale.**

Only the *number* of reloads differs: 2,920 vs 2,900. ares sees 2,725 `$7F48`
writes (all while halted), so the remainder come from locked-page paths. Ours
performs one more reload right at the split (a forced reload of `$29000`, which
is already resident in `a1`) where ares starts executing instead.

**Handshake timing (measured):** `T4F` events log the master clock at the
`$7F4F` write, and the instruction log carries `ins=` per instruction, so the
delay from "start" to the firmware's first instruction is directly comparable
over the two jobs in the captured window:

| job | ares | ours |
|---|---|---|
| 0 | 1,240 clocks | 2,584 clocks |
| 1 | 1,216 clocks | 2,624 clocks |

Measuring the *other* half of the handshake (reload request to the `$7F4F` write)
corrects the picture: ares 2,860 clocks, ours 122.

| phase | ares | ours |
|---|---|---|
| `$7F48`/reload request -> `$7F4F` write | 2,860 | 122 |
| `$7F4F` -> first firmware instruction | 1,240 | 2,604 |
| request -> first instruction | ~4,100 | ~2,726 |

The wait registers match (`w=4`, so the reload is 512 x 5 cycles = 2,560
coprocessor cycles ~ 2,750 CPU clocks in both). So ares *delays the S-CPU by the
reload* - the `$7F4F` write only happens after ~2,860 clocks - and then starts
the job 1,240 clocks later. We instead write `$7F4F` almost immediately (122) and
then wait 2,604. The phase is inverted and the whole handshake is ~1,374 clocks
*shorter*: ours starts the job too early relative to the request, then pays the
debt.

That is why the machine-time experiment (which advanced the clock by the reload)
looked close but made parity worse: it fixes the first row and not the second.

A single isolated job in the ares log shows the four phases exactly (clocks
relative to the `$7F48` write):

```
$7F48 write            0
reload begins        924      <- 924 clocks pass before the load even starts
reload completes   3,650      <- 2,726-clock reload
$7F4F write        3,798      <- 148 clocks after the reload
first instruction  5,014      <- 1,216 clocks later
```

Ours has the same reload duration and the same order, but the load begins
immediately (~0 rather than 924) and the job starts 2,584 clocks after `$7F4F`
rather than 1,216.

Logging both thread clocks (`cc` = the HG51B thread's clock, `pc` = the S-CPU
thread's, both in the scheduler's scaled base, `sc` = the HG51B scalar) gives the
lead/lag directly, in CX4 cycles:

| event | CPU lead over the coprocessor |
|---|---|
| `$7F48` write | **+375 .. +966 cycles** (coprocessor behind) |
| `$7F4F` write | **+90 .. +152 cycles** (nearly in sync) |

So the coprocessor is *behind* by a few hundred cycles when the reload is
requested, and almost in sync when the job starts - it is **not** ahead after the
load, which is what last turn's note assumed. The 924-clock gap before the reload
is therefore the coprocessor catching up *while halted* (each `main()` costs one
clock) before `cache()` finally runs; the 1,216-clock gap after `$7F4F` is the CPU
catching up to the coprocessor's post-reload position.

The catch-up amount varies job to job (+375 .. +966), so it cannot be a constant
delay: it is whatever the previous job left outstanding.

Replacing the independent credit with a shared scaled clock (ares' scheduler
shape: `cx4_clock += cost * scalar` against `cpu_clock = master_clock *
cpu_scalar`) is **behaviour-identical**, as the arithmetic predicts - frames
3300/3400 stay 100.000%, 3500 stays 96.406%, reloads stay 2,920. It is kept
because it makes the lead/lag directly measurable, and that measurement is
decisive:

| at the write | ares | ours (shared clock) |
|---|---|---|
| `$7F48` | +375 .. +966 (coprocessor behind) | (probe not yet wired) |
| `$7F4F` | +90 .. +152 (coprocessor behind) | **-2,405 .. -2,443 (coprocessor AHEAD)** |

So at the job start our coprocessor is **~2,440 cycles ahead** where ares' is
~100 *behind* - a sign inversion of about one full reload. That is the concrete
defect: we charge the reload to the clock *after* it happens, so the coprocessor
ends up ahead while the CPU catches up; ares lets the CPU run *through* the
reload so it is slightly behind by the time it writes `$7F4F`.

This also explains why the earlier machine-time experiment overshot: it advanced
the whole machine (including the PPU) by the reload instead of letting the CPU
execute through it, so it shifted video timing as well.

### The handshake, measured per access on both sides

Logging every `$7F48`/`$7F4F` write and the reload's start/finish gives the
complete sequence (clocks relative to the `$7F48` write):

| event | ares | ours |
|---|---|---|
| `$7F48` write | 0 | 0 |
| reload starts | **924** | **4** |
| reload completes | **3,650** (2,726-clock load) | **4** (instant in CPU time) |
| `$7F4F` write | **3,798** | **126** |
| first firmware instruction | **5,014** | ~2,730 |

Read as a decomposition, ares' `$7F4F` write is exactly

```
924 (coprocessor catches up while halted before the reload runs)
+ 2,726 (reload, during which the CPU keeps executing)
+ 148 (code between reload end and the write)
= 3,798
```

and ours is `4 + 122 = 126`, because we do neither: the reload runs
*immediately* (4 clocks) and its cost is charged to the coprocessor's clock
rather than to the CPU's, so no machine time passes. The first instruction then
lands at ~2,604 clocks after `$7F4F` in ours versus 1,240 in ares.

The missing input for part 1 is now measured on both sides. Logging the lead at
every `$7F48` write (`L48`):

| lead at `$7F48` (CX4 cycles, + = coprocessor behind) | ares | ours |
|---|---|---|
| observed | **+375 .. +966** | **0 (all 2,741 writes)** |

Ours is *exactly* in sync at every request, ares' is a few hundred cycles behind,
and that lead is precisely the 924-clock catch-up that precedes the reload. So
part 1 is not a tuning constant: ares' halted coprocessor simply does not track
the CPU between jobs, while ours does (one coprocessor cycle per CPU clock), and
the difference at the request is whatever the CPU ran in the meantime.

### Tested: (halted lead + machine-time charge) together (overshoots)

Implementing both parts at once - the CX4 no longer advancing while idle, and a
burst's overshoot charged to the machine clock - makes the lead far too large:

| | ares | with both parts |
|---|---|---|
| lead at `$7F48` | +375 .. +966 cycles | **+14,990 cycles** |
| frame 3300 / 3400 / 3500 | 100.000 / 100.000 / 96.406% | 98.512 / 97.264 / 96.697% |

So "the halted coprocessor does not track the CPU" is too strong: ares' CX4 does
*track* the CPU between jobs (otherwise the lag would be the whole inter-job gap,
~15,000 cycles, exactly what this experiment produced) and merely ends each job a
few hundred cycles behind. The residual ~900-cycle lag therefore comes from the
job's own tail, not from the inter-job period. Reverted.

### Where the lead is created: the job's tail

Logging the coprocessor and CPU clocks at every `HALT` and every `$7F48` (both in
CX4 cycles) answers the question the halted-lead experiment raised:

| | min | median | max |
|---|---|---|---|
| lead at `HALT` (end of a job) | +4 | **+173** | +1,260 |
| lead at the next `$7F48` | +7 | **+642** | +1,272 |

So the coprocessor is *already* a few hundred cycles behind when it halts (median
173, up to 1,260) and the lead roughly triples during the inter-job gap. Neither
"created while idle" (the halted-lead experiment produced 15,000) nor "created
during the job" alone is right: about a third is left at `HALT` and the rest
accrues before the next request.

That points the next attempt at the **job's tail** - the last `execute()` through
`HALT` - where the coprocessor is already behind, rather than at the idle period.

A per-instruction coprocessor clock (`cc=`, via a new `HG51B::debugClock()`
virtual overridden by `HitachiDSP`) gives the answer. Tail of one job:

```
t=1242884854 ins=37413673 cc=…276246 cc 08:5d b4ff
t=1242884854 ins=37413674 cc=…280857 cc 08:5e 2601
t=1242884854 ins=37413675 cc=…290081 cc 08:60 fc00   <- HALT
HALT          cc=…294692  pc=…405925
```

Two things stand out:

1. every instruction in the tail advances the coprocessor's clock by **~100 CX4
   cycles** (bus waits dominate), while the CPU's clock `t=` does **not move at
   all** across the burst;
2. the lead at `HALT` is `(pc - cc)/scalar = 184` cycles.

So the "lead" is not work the coprocessor failed to do: it is the *pending
catch-up* at the instant `halt()` runs, because ares runs the coprocessor in
bursts (the CPU is not stepped while the coprocessor is ahead) and the CPU then
catches up in its own burst. The lead at `HALT` is simply how far apart the two
bursts are at that point (173 median), and it grows to 642 by the next `$7F48`
for the same reason.

That means there is nothing to "fix" in the tail: the quantity to reproduce is
the *burst structure* - a burst must charge the CPU's clock (so the CPU catches
up) and the next request must wait for the outstanding catch-up. Our model keeps
the two clocks level at every access, so both halves of that structure are
absent, which is exactly the -2,442 vs +90..+152 inversion measured at `$7F4F`.

### Tested: bursts at instruction boundaries + charge (closer, still worse)

Implemented that structure: the CX4 is stepped only at instruction boundaries
(after each S-CPU instruction, up to the CPU's clock) and a burst's overshoot is
charged to the CPU's clock. Measured against the four targets:

| quantity | ares | per-access (kept) | burst+charge |
|---|---|---|---|
| `$7F4F` write, clocks after `$7F48` | +3,798 | +126 | **+2,891** |
| lead at `$7F4F` | +90..+152 | -2,442 | (closer, not printed) |
| lead at `$7F48` | +375..+966 | 0 | **0** |
| frames 3300 / 3400 / 3500 / 3700 | - | 100 / 100 / 96.406 / 96.488 % | 97.876 / 100 / 96.153 / 95.776 % |

So the burst charge moves `$7F4F` most of the way to ares' position, but the lead
at `$7F48` is still 0 (the coprocessor does not lag, because it is stepped to the
CPU's clock at every instruction) and parity degrades on three of the four
frames. Reverted.

Six scheduling models have now been implemented and measured; the remaining
difference is the *sub-instruction* interleaving - ares resumes the S-CPU from
inside the coprocessor's `step()` while the current instruction's bus cycle is
still open (`Thread::synchronize(cpu)` re-entering `cpu.main()`). That cannot be
expressed while `Snes::step()` runs exactly one instruction to completion.

### Frame-locked ares capture: started, not yet calibrated

A hash at the PPU frame boundary (`ARES_FLHASH`, sampling the region `dac.cpp`
renders at row 8 / column 26 of the 564-wide buffer) plus a matching raw dump
(`ARES_FLFRAME`) was added to the ares build. It is **not yet usable**:

- only all-black frames hash identically to ours (25/330), because for any
  content the sampled region does not line up;
- dumping ares' frame-320 buffer and searching +-2 pixel shifts against our
  frame 320 finds a best alignment of only 12% - while the *host-callback*
  capture at index 322 matches our 320 at 100%.

So ares' `screen->pixels()` at the frame boundary is not the picture our frame
index shows: the buffer reads ~2 frames behind (most likely `dac.cpp` alternates
fields/buffers and the desktop-ui's conversion presents the other one). Until
that is pinned down, Star Fox comparisons stay on the host-timed captures with
the documented phase caveat.

So the fix is the *conjunction* of the two things tried separately:

1. a ~924-clock delay before the reload starts = the coprocessor catching up its
   outstanding lead while halted (ares' lead at `$7F48` is +375..+966 cycles);
2. charging the reload's 2,726 clocks to the **CPU's** clock, so the CPU's clock
   (and instruction stream) advances through it.

The machine-time experiment implemented (2) alone, which put `$7F4F` ~950 clocks
early and made parity worse - consistent with this decomposition. Neither part
was ever tried together.

### Tested: dragging the S-CPU through the reload (no effect)

Two more variants were implemented and measured on the shared clock:

1. after every instruction, keep executing CPU instructions while the CX4 is
   ahead (`Thread::synchronize(cpu)` in `Snes::step`) - 494,150 extra
   instructions over the run;
2. additionally service pending coprocessor work at the end of the instruction
   that requested it (`advance_cx4(0)`), so the `$7F48` reload no longer waits
   for the next bus access (which, during the handshake, is the `$7F4F` store
   itself).

Result: **identical output** - frames 3300/3400 100.000%, 3500 96.406%, and the
lead at `$7F4F` still -2,442 cycles. Variant 1 cannot help because those
instructions execute anyway; only their grouping changes. Variant 2 did not move
the probe, so the reload is still serviced at the next access rather than at the
end of the requesting instruction.

**This rules out the handshake-ordering theory.** Variant 1 *does* reproduce
ares' ordering - the S-CPU executes ~180 instructions per job through the
reload, so `$7F4F` is written while the burst is in progress, exactly as in
ares - and the rendered frames are bit-identical to the per-access model. If the
order of the `$7F4F` write relative to the reload mattered for the picture, that
experiment would have changed frame 3500; it did not.

### The register divergence starts at frame 21, not 3477

> **Superseded (2026-11-18).** The frame-21 "divergence" is a frame-boundary
> sampling artefact, not a CX4 difference. See "Corrected: the CX4 is
> byte-identical at every job start" below.

Hashing the CX4 GPR window (the 48 bytes the S-CPU sees, 16 x 3) once per frame
in both cores (`CX4_REGS_LOG` / `ARES_CX4_REGS`) and comparing 3,500 frames:

```
frames compared 3500; differing GPR hashes: 76; first: 21
frame 21: ares=4c1099f083a5d25f ours=dbb08402e9cafb1b
```

So the CX4 register file first differs at **frame 21**, 3,456 frames before the
first *visible* difference (frame 3477) and before any data-RAM difference. The
early difference is sporadic (76 of 3,500 frames differ) and only becomes visible
when a later job's plot depends on the drifted register.

That reframes the hunt: the bug is an HG51B computation that goes wrong on a
specific input very early in the ROM's life, not anything to do with the job
handshake (which six experiments have now excluded).

Tracing the last write to `gpr2` before the divergence confirms the
computation angle - the two cores write *different values at different
instructions*:

```
ares: value 0000 (was 3DFE)  at 000a:39 605e  a=000000
ours: value 0100 (was 4108)  at 000A:36 083B  a=000100
```

so the firmware is not merely being fed a different `gpr2`; it reaches a
different instruction in the same region (`0a:36` vs `0a:39`), i.e. its control
flow differs too.

So the frame-3477 divergence must be a **computation** difference, not a
scheduling one: the job reaches its first instruction with a GPR value that
differs (`gpr2 = 0x200` where ares has 0), and no amount of reordering changes
what that instruction *sees*. The handshake timing is a red herring that six
experiments have now excluded; the next thing to compare is the **pre-job CX4
register file** (all 16 GPRs plus P/DPR/ROM/RAM buffers) at the moment the job
starts, in both cores.

So the remaining gap is not "how many instructions the CPU runs" but the
*ordering* between the CPU's instruction stream and the coprocessor's blocking
work inside a single instruction - which our one-instruction-per-`step()` model
cannot express without restructuring the step loop around ares' cooperative
threads.

Reproducing this needs the coprocessor to be stepped from the CPU's own clock
budget (as `Thread::synchronize` does) rather than from a separate credit, so
that both catch-up phases fall out of the same arithmetic. The mechanism is now clear:
the forced reload costs ~2,560 coprocessor cycles (~2,749 CPU clocks), and in
ares the S-CPU *executes during it* (`Thread::synchronize(cpu)`), so the `$7F4F`
write lands about halfway through the reload (~145 clocks after `$7F48` in our
timeline, ~1,373 in ares) and the job starts when the reload ends. Ours performs
the reload atomically, so `$7F4F` happens ~145 clocks after `$7F48` and the whole
~2,600-clock debt is paid afterwards.

That is why "charge the reload to machine time" alone fails (it moves the clock
without executing the CPU's instructions) and why the interruptible reload needs
to be *exactly* ares' schedule, not merely "the CPU can run during it".

**Correction (counting both cores over the same span):** the `$7F48` write
streams are *identical* - 1,927 writes each up to frame 3000 - so ares cannot be
absorbing writes we honour. The 20-load difference accumulates only over the
last ~478 frames, i.e. *after* the divergence, so it is an effect, not the
cause. Neither absorption nor extra loads is established; what is left is a pure
**timing** difference: identical instruction stream, identical `$7F48` stream,
identical reload parameters, but our firmware reaches the first instruction of
the job one step earlier and reads a stale `gpr2`.

### Corrected (2026-11-18): the CX4 is byte-identical at every job start

Two probes settle this item.

**1. The frame-21 GPR difference was a sampling artefact.** The per-frame hash
samples at each core's frame boundary. Our boundary and ares' differ by ~1,400
master clocks, and the boot code runs a visible GPR-scratch loop
(`$7F83-$7F85` = `$111111`, `$222222`, ...), so the same instant of that loop
lands in different frames. Comparing the *content* of every write into the CX4
window (`CX4_TRACE` / `ARES_CX4_TRACE`):

```
ares 1192 writes, ours 975; first content difference: none
```

The streams are identical, write for write, at clocks within ~30 of each other
(ours writes a given value at t=7833306 where ares writes it at t=7833330); the
line-count difference is only the host-timed capture stopping ~1,400 clocks
earlier. The same applies to the `$02FD` difference measured below: it is the
interrupt stack frame (`SP=$02FF`, `$02FD` = pushed PCL), so it records *which
instruction* the NMI landed on, not a different value.

**2. The CX4 state is identical at every job start, through frame 3600.** Hashing
the CX4 data RAM (3 KiB) and all 16 GPRs when `$7F4F` starts a job - a
content-addressed instant in both cores, with no frame-boundary phase:

```
jobs compared: 2747   (ares capture frame 3600)
first job with different data-RAM hash: none
first job with different GPR hash:      none
```

2,747 of 2,747 jobs are byte-identical, including every job after the picture
first differs. So the HG51B executes the same program on the same data: the
"CX4 divergence" is not a coprocessor difference at all.

**What actually differs.** Comparing frames by content (ares captures are not
frame-locked, so the index is not trustworthy) shows the truth about the
"frame 3477" item:

```
ours 3479 vs ares capture 3480:  100.000%   (57344/57344)
ours 3480 vs ares capture 3481:   97.262%
```

So the picture is **exact through our frame 3479** - 58 seconds of gameplay -
and the ares capture at 3480 is one frame behind. The first genuinely different
frame is our 3480. Its difference is a **background band**: x 80-168,
y 59-181, where one BG layer sits 1-2 pixels to the right in ares, in the same
palette (BG palette 0 entry 3), while the rest of the frame is identical. BG1
horizontal scroll is 0 at the frame boundary in both cores, so the difference is
in **mid-frame (per-line) scroll**, not in framebuffer composition or CX4 data.

At the same job (2400) the two cores' WRAM differs by 5 bytes:

```
02B5:5B/88, 02B7:61/90, 02B9:0E/3D, 02FD:00/03, 09CF:0D/05
```

`02FD` is the interrupt stack byte (phase), `09CF` is the wait-loop counter next
to `$09CE`, and `02B5/B7/B9` are 16-bit scratch values ~0x2F apart - i.e. the
residual is a few counters at a different phase, not different game state.

**Measured interrupt phase.** Watching `$02FD` (pushed PCL) at the first NMIs:

```
ares: pc=008100  (NMI taken before $8100 LDA)
ours: pc=8103    (NMI taken before $8103 BEQ)
```

ares' NMI path is `nmiPoll` (`vcounter(2)`, every 4 clocks) -> 4-clock `nmiHold`
-> `nmiTransition` -> `lastCycle()` one CPU cycle before the instruction ends.
We latch at the line start and apply at the end of the instruction, so we can
take the interrupt one instruction earlier. That is the source of the residual
counter phase, and the candidate mechanism for the per-line scroll difference at
frame 3480.

### Where the frame-3480 difference comes from: a copied CX4 snapshot

Per-scanline probe (`PPU_HSCROLL_LOG` / `ARES_HSCROLL_LOG`, logged inside
`render_current_line` and `DAC::render`) for the first differing frame:

```
differing lines: 0 of 224
ares/ours: mode=1 tm=04 h0=000 v0=000 h1=000 v1=000 h2=3B0 v2=3D0
           sc2=3000 td2=1000 mos=00   (168 lines)
           sc2=0800 td2=0000 mos=00   (56 lines)
```

Every per-line register is identical, and only BG3 is enabled (`tm=04`), so the
difference is not scroll, layer enable, map base, tile base or mosaic.

The VRAM dumps at the aligned frames (ares 3480 == ours 3479, both with a
100.000% identical picture) differ in 239 words; at the first differing frame
(ares 3481 vs ours 3480) in 691 words. Almost all of them are BG3 character
data at `$1000`, and **104 of the 145 tiles the visible map uses** have
different data (the map itself, at `$3000`, is identical).

That data is a straight copy of the coprocessor:

```
VRAM words $1000..$147F  ==  CX4 data RAM[$300..$B00]   (2048/2048 bytes)
```

and each core is self-consistent about which snapshot it copied:

```
ours frame 3480 VRAM block == ours   CX4 RAM at 3479[$300]
ares frame 3481 VRAM block == ares   CX4 RAM at 3480/3481[$300]
ours 3479 CX4 RAM vs ares 3480 CX4 RAM: 1295 bytes differ
```

So the picture difference is the **CX4 data-RAM snapshot the S-CPU copies into
VRAM**. Every completed job still matches: the job-start data-RAM hash is
identical for 2747/2747 jobs through frame 3600, so the HG51B executes the same
program on the same data. What differs is the RAM state sampled at the frame
boundary / copy instant - i.e. how far the coprocessor has progressed relative
to the S-CPU when the copy runs, which is the scheduling question the earlier
experiments were circling.

The VRAM write stream confirms the same thing from the other side
(`VRAM_WRITE_LOG` / `ARES_VRAM_LOG`, now frame-windowed): the **first** write to
each of the 2304 `(address, byte)` keys is identical in both cores, while the
later passes differ - the CPU rewrites the BG3 tile block during the frame and
the second copy reads a different CX4 RAM state.

### Copy-instant measurement: the CX4 is halted, but at a different job index

Four probes measure the copy itself.

**1. The coprocessor is idle at every VRAM data-port write.** `VRAM_CX4_LOG`
(ours, `$2118`/`$2119` writes in `SnesBus::write`) and the extended
`ARES_VRAM_LOG` (ares, inside `PPU::writeVRAM`) record the live HG51B state:

```
ares: pb=0002 pc=61 run=0 busy=0
ours: pb=0002 pc=61 run=0 busy=0     (for every write in the window)
```

So the copy reads a static image, not a job in flight.

**2. Every job output is identical.** `CX4_HALT_HASH` / `ARES_CX4_HALT_HASH`
hash the 3 KiB data RAM at every `halt()` (which also fires on cache reloads):

```
ours: 7e8e0fa784351325 (power-on), then 72f0dd7c…, 987fdc90…, d55e0f22…
ares:                            72f0dd7c…, 987fdc90…, d55e0f22…
```

Ours has one extra halt at power-on; after that the streams are identical
element for element (2746/2746 through frame 3600). The HG51B produces exactly
the same outputs.

**3. The CPU's uploads are identical up to a point.** `CX4_CPUWR_HASH` /
`ARES_CX4_CPUWR_HASH` hash the data RAM after every CPU write into the
`$6000`/`$7000` window, with the live PB/PC/running state. In the frame-3477
burst the first 28 writes match exactly (address, value, resulting hash and
state). At write 28 (`295=CC`) both cores are at `pb=0008 pc=61`, but the hash
differs:

```
       write 27  813b44e07f795766   pb=0000 pc=ed
ares   write 28  23ea60b78f59707f   pb=0008 pc=61   (ins +52,972 since 27)
ours   write 28  6109d0c4bf07b3e3   pb=0008 pc=61   (+52,444 since 27)
```

**4. The job index at the copy differs by 12.** Logging the number of `$7F4F`
job starts and `halt()`s at each VRAM write:

```
ares frame 3481 copy: jobs=2550 halts=2550   (2553/2553 by the end of the frame)
ours frame 3480 copy: jobs=2562 halts=2563   (2565/2566)
```

Both are halted at the same program point, but ours has started 12 more jobs
(and one more cache-reload halt) by the same displayed frame. The image the
CPU copies is therefore a *different job's* output, which is why the BG3 tile
block differs while every individual job output matches.

So the remaining difference is the coprocessor's **rate/phase relative to the
S-CPU**, now measurable three ways (instructions per inter-write interval,
halt count, job-start count), not its computation and not the PPU.

### Bisected: the job-count gap starts at frame 3324, in the CPU's dispatch cadence

Per-frame job-start counts in both cores (`CX4_JOBS_LOG` in `snes-info`,
`ARES_CX4_JOBS` in the ares build) agree **frame for frame until 3324**:

```
frame  ares  ours  delta
3300   2250  2250    0
3323   2319  2319    0
3324   2319  2322   +3
3326   2322  2328   +6
3328   2325  2334   +9
3330   2328  2340  +12
3400   2433  2445  +12   (constant from here)
```

So there are exactly four frames where ares performs **no `$7F4F` write at all**
and we perform three: 3324, 3326, 3328, 3330. The 12-job gap at the copy is the
residue of those four skipped dispatches.

`CX4_JOB_LOG` / `ARES_CX4_JOB_LOG` show the state at every dispatch:

```
ares: halt=1 ... cpu_pc=7e26c5   (frames 3323, 3325, 3327, 3329, 3331, 3333)
ours: halt=1 ... cpu_pc=7e26c1   (frames 3324 .. 3330, every frame)
```

Both CPUs execute the same instruction - `$7E26C1: 8F 4F 7F 00` (`STA $7F4F`)
inside the WRAM code at `$7E2695` - and the coprocessor is idle (`halt=1`) in
both. The instruction counts per dispatch are also identical (e.g. `+1,029`
then `+43,700` between the three writes of one frame, in both cores), so the
HG51B does the same work; only *how often the game calls the dispatcher* differs.

The dispatcher (`$7E2695`) is:

```
2696: LDA $7F5E   ; wait until the CX4 is not running
269A: AND #$40
269C: BNE $2696
269E: LDA $00C3   ; bit 0 -> $4200 (NMI enable)
26A2: AND #$01
26A4: STA $4200
26A8: PLA
26A9: STA $7F4D   ; cache pb
26AD: LDA #$00
26AF: STA $7F4E
26B3: TXA
26B4: STA $7F48   ; cache page
26B8: LDA $7F5E
26BC: AND #$40
26BE: BNE $26B8
26C0: TYA
26C1: STA $7F4F   ; <- start the job (the logged write)
26C5: LDA $7F5E
26C9: AND #$40
26CB: BNE $26C5   ; wait for completion
26CD: LDA $00C3
26D1: STA $4200
26D4: RTL
```

and is reached through the jump table at `$7E2680` (`JSL $7E2680`, `$7E2683`,
`$7E2686`, `$7E268C` occur in ROM at `$01:D2A6`, `$01:D2EC`, ...). The ROM
caller at `$01:D2A6` sets `A=0, X=1, Y=0` and starts a job, so the four skipped
frames are the game choosing not to run that update - a CPU-side decision, not
a coprocessor or PPU difference.

### The dispatcher's caller: a state machine on `$3A`, called once per update

Frame-gated S-CPU traces (`ARES_PC_TRACE_FROM/TO` with a raised
`ARES_PC_TRACE_TAKE`) show the same instruction sequence in both cores up to
the dispatch:

```
00:CF4C CF4D CF4E    (return from a JSR $CE73)
00:8981 20 50 DC     JSR $DC50
00:DC50 .. 00:DCE8   (the routine; RTS)
00:8984 20 5A D2     JSR $D25A     <- the CX4 update
```

and the block at `$897E` is entered from `$88C6`:

```
88C0: 20 FC 87     JSR $87FC
88C3: 4C CD 86     JMP $86CD
88C6: A6 3A        LDX $3A
88C8: 7C C9 88     JMP ($88C9,X)   ; jump table
88C9: EB 88 16 89 7E 89 94 89 ...  ; entry 2 = $897E
```

So `$897E` (the handler that calls `$D25A` and thus dispatches CX4 jobs) is
selected by the state variable `$3A` and runs once per call of the state
machine. Both cores' stacks match at every dispatch (the return frame is
`$00:D2A9` / `P` / `$8635`-style caller words, identical byte for byte), and
`$3A` itself is not among the 38 differing WRAM bytes at frame 3324 - so the
two cores run the same handler when the machine is called; they simply call it
a different number of times in frames 3324/3326/3328/3330.

### The skipped frames are a frame-pacing near-tie, not different game state

Per-frame counters (`CX4_JOBS_LOG` / `ARES_CX4_JOBS`, now also exporting
`$09CE`, `$09CF` and a cumulative S-CPU instruction count):

```
frame   ares_instr  ours_instr   ares_jobs  ours_jobs
3322      12,185      12,371        2316       2316
3323      12,691      12,372        2319       2319
3324      12,106      12,373        2319       2322
3325      12,692      12,374        2322       2325
3326      12,107      12,376        2322       2328
```

- `$09CE` (frame flag, set by the NMI at `$08:8034`, consumed by the main loop
  at `$00:8125/$812B`) and `$09CF` (NMI counter, `INC $09CF` in the NMI) match
  **on every frame**: 215 at frame 3324 in both. The cores are frame-aligned;
  the earlier apparent one-frame capture offset was a static scene.
- The state machine at `$88C6` is entered once per frame with `$3A = $1C` in
  both cores - ares simply has **no entry at all** on 3324/3326/3328/3330.
- ares' instruction count alternates 12,106 / 12,691 per frame in this window
  (the state machine and its dispatch only on the long frames), while ours is a
  steady 12,330-12,379. The game's per-frame work is close enough to a full
  frame that a ~300-instruction difference decides whether the update fits.

Cumulatively our S-CPU executes **+1.16 %** more instructions than ares by
frame 3000, and the difference appears in *steps* (0.37 % at frame 1000,
1.04 % at 2000) rather than linearly - so it is not a per-instruction cycle
difference (our `access_cycles` is a verbatim match of ares' `CPU::wait()`:
6/8 clocks per access, 12 for `$4000-$41FF`). It is extra *spinning* while
waiting for an event, of which the CX4 busy loops in the dispatcher
(`$7E2696`, `$7E26B8`, `$7E26C5` spin on `$7F5E` bit 6) are the prime
candidate in exactly these windows.

### The dispatcher's wait loops, counted per job

`CX4_SPIN_LOG` / `ARES_SPIN_LOG` count executions of the three `$7F5E` busy
loop heads inside the WRAM dispatcher and print the running totals at every
`$7F4F`, so consecutive lines give the per-job spin counts:

```
loop            ares total    ours total   ratio
$7E2696 (idle)        2,333         2,333    1.00
$7E26B8 (pre-start)  98,653         2,333    0.02
$7E26C5 (post-start) 336,115       412,167    1.23
```

Per job: the pre-start loop spins **42 iterations (median 42, max 51) in ares
and exactly 1 in ours**, while the post-start loop spins ~23 % more in ours.

The pre-start loop (`$26B8: LDA $7F5E; AND #$40; BNE $26B8`) waits for the
coprocessor to stop running, i.e. for the *previous* job to finish. In ares it
is still running when the CPU arrives (42 iterations, roughly 500 CPU clocks);
in ours it has already finished. After the new job starts, ours waits longer
instead. The total waiting is nearly equal (ares 434,768 iterations, ours
414,500, -4.7 %), so the coprocessor's *rate* is close; the difference is the
**phase** at which its completion lands relative to the S-CPU - the same lead
measured at `$7F4F` earlier, now expressible in CPU spin iterations.

The +1.16% S-CPU instruction excess is therefore the main loop's own `$09CE`
wait (`$00:8100: LDA $09CE; BEQ`), not the CX4 loops.

### Why ares' chip lags: PROFILE_PERFORMANCE does not sync coprocessors per step

The residual ~476-cycle lag at `$7F48` is explained by the oracle build's
configuration, not by chunk size or a rate error:

```
ares/CMakeLists.txt:29   target_compile_definitions(ares PUBLIC PROFILE_PERFORMANCE)
ares/sfc/cpu/timing.cpp:52   #if !defined(PROFILE_PERFORMANCE)
                             for(auto coprocessor : coprocessors) Thread::synchronize(*coprocessor);
                             #endif
```

In this build the S-CPU does **not** synchronize coprocessors on every step;
the CX4 is advanced only when the CPU touches its window (on-demand). The chip
therefore sits behind the CPU by however much CPU time elapsed since the last
access - the 476 cycles measured in ares' `SET48` line
(`(pc - cc)/sc = 476.4`), and it starts the cache reload at the *next* CX4
access, which is why ares' `prep` is 860 clocks after `SET48`.

Our scheduler advances the CX4 after every bus access (and after processing a
write), so the chip is always level (our `L48` lead is 0) and the reload starts
immediately. Mirroring ares needs two ordering changes:

1. advance the CX4 only on CX4-window accesses (a `snes_mar` filter is
   sufficient and was measured neutral on its own), and
2. advance it **before** the access is processed (ares' `cpu.synchronize` at
   the start of the access), not after - which needs the current master clock
   inside `SnesBus`.

Done alone, (1) is neutral: the frames and spins are unchanged (35 vs ares'
45; 3480 100.000%, 3500 96.706%, 4000 94.877%). Both together are the next
experiment.

### NMI latch: 4-clock hold + `lastCycle` test implemented

`advance_peripherals_inner` now mirrors ares `CPU::nmiPoll`/`lastCycle`:

* the PPU's vcount>=`vdisp` edge sets `$4210`'s flag immediately and arms a
  **4-clock `nmiHold`** (`nmi_hold`);
* when the hold expires the interrupt line is raised and its clock recorded
  (`nmi_line_clock`);
* `apply_interrupts` only takes it when the edge arrived at least one CPU
  cycle (6 master clocks) before the instruction ended, i.e. ares'
  `lastCycle()` test; otherwise it waits for the next boundary.

Two mechanical changes, no frame changes:

```
spin at first job: 45 (ares 45)   dispatch clock 21,766,698 (ares 21,766,684)
frames 3300/3480/3500/3700/3900: 100.000%, baseline 212/212
```

Polling the NMI edge directly (`NMI_POLL` / `ARES_NMI_POLL`: the master clock
and S-CPU PC at every rising `vcounter(2) >= vdisp`):

```
edge   ares t     ares pc    ours t     ours pc    delta
  0     306,906   008083     306,904    008083     -2
  1     664,270   008104     664,272    008100     +2
  2   1,021,634   00B308   1,021,638    00B303     +4
```

So the NMI **edge** now matches ares within 2-4 master clocks at every vblank.
The `$02FD` pushed-PCL difference ($8103 vs $8100) is therefore not a timing
error: our `live_pc` is the instruction start while ares logs `r.pc.d` (the
current byte), and the interrupted instruction is simply a different iteration
of the same 42-clock `$8100` wait loop. Both cores enter the same NMI within
one loop instruction, and the wait loop is a pure spin (no state consequence).

### Fixed: colour math did not halve fixed-colour adds ($2131 bit 6)

Mega Man X3 was exact at every present through 906 and then diverged by a
handful of pixels in an expanding wedge at the top-right corner (2 px at 907,
10 at 908, 29 at 909), staying wrong until present 972. State was identical
(VRAM, CGRAM, OAM and CX4 RAM 0 bytes different), so it was a compositor
decision. Dumping ares' DAC inputs at the corner gave:

```
frame 909 vc16 x247 above=1802 below=1802 fixed=4fde
  layer=1 backdrop=0 sub=0 halve=1 add=0 result=31f0
```

`blend(0x1802, 0x4fde, halve=true)` is exactly ares' add formula
`(x + y - ((x ^ y) & 0x0421)) >> 1` = `0x31f0`; our pixel was the same add
without halving (`0x7bfa`, i.e. 0xfff7ce), because our halve flag was written as
`colorHalve && useSub` where `useSub` already required `$2130` bit 1 *and* a
non-transparent subscreen. ares halves in the fixed-colour path too:

```cpp
if(!io.blendMode) return blend(above.color, fixedColor(), io.colorHalve && windowAbove[x]);
return blend(above.color, below.color, io.colorHalve && windowAbove[x] && below.source != COL);
```

`dac_pixel` now selects the operand from `$2130` bit 1 alone and halves unless
that path is the transparent-subscreen one, and it also zeroes the main colour
when the above colour window is off (ares sets `above.color = 0` before the
below-window early return). The fix makes 907-972 exact and takes MMX3 from
1184/1801 to **1800/1801** (present 1800 is one past our last frame). Pinned by
`ppu::tests::fixed_colour_blend_halves_when_cgadsub_bit6_is_set`.

### Fixed: the 34-object-tile budget counts x = 256 sprites

Extending MMX2 past 7000 exposed presents 7395-7398 (3-28 px), 7409 (217 px),
7412 (103 px) and 7431 (102 px). ares showed BG1 at `x 8..23` where we showed a
sprite. The sprites are 2 and 3 at `x = 8`, and ares' per-line item list did
include them, so the evaluation was the same.

ares' 34-object-tile budget is consumed by *every* fetched tile, and a tile is
only skipped (not counted) when

```cpp
if(x != width && objectX >= width && objectX + 7 < 512 + x1) continue;
```

so a sprite at exactly `x = 256` is **not** skipped: its tiles are fetched,
counted and then draw nothing because the pixel loop keeps only `objectX & 511
< 256`. On this line two such sprites (115, 74) consumed four tiles, pushing
sprites 3 and 2 past the budget in ares. We skipped those tiles before counting,
so our budget was four tiles lighter and we drew sprites 2/3.

The object tile loop now mirrors ares: `objectX = (x + (tileX << 3)) & 511`, the
same skip test, and per-pixel `(objectX + px) & 511` masking (which also covers
the x = 511 wrap onto the left edge). Presents 7395-7431 are exact and the MMX2
sweep extends to 8000: **7998/8001**, again only the three torn presents. Pinned
by `ppu::tests::object_x_wraps_modulo_512_like_ares`.

### Fixed: objects merge into the background once, with a strict priority test

Extending MMX2 past 6000 found a divergence at the very end: present 6992
differed by 11 pixels at `x 224-226, y 112-117`, growing to 102 by 6996. The
DAC inputs showed the same geometry but different layers: ares had **BG1
priority 7**, we had **OBJ priority 6** at those columns.

Both cores fetch the same BG1 pixel with priority 7, so the object should lose.
Our object pass replaced it anyway: objects are merged into the background one
at a time, and `plot_object` replaces any existing *object* pixel
unconditionally (which is right for object-over-object, where ares' last emitted
tile wins). A higher-priority object (12) therefore replaced BG1, and a later
lower-priority object (3/6) replaced *that*, leaving a low-priority object on
top without ever re-testing it against the background.

ares builds the final per-column object pixel first (`Object::render`
`palette[]`/`priority[]`), then calls `DAC::plotAbove` once, which only replaces
a background pixel on `priority > above.priority`. The renderer now renders
sprites into their own buffer and merges once with the same strict comparison.
Presents 6992-6999 are exact, and MMX2's sweep extends to 7000:
**6998/7001** with only the three torn presents. Pinned by
`ppu::tests::a_lower_priority_object_cannot_hide_a_higher_priority_background`.

### Fixed: an empty sub screen blends with the fixed colour, not cgram[0]

Star Fox was exact through present 2415, then diverged over presents 2416-2451
by a whole visible band (18-206) during a fade, with the image shape matching
but every colour too bright (ours `0x3f3f3f`, ares `0x0e0e0e` at present 2416).
Per-line logging showed identical inputs otherwise: brightness 3/6/9/12/15 in
step, `CGWSEL=0x12` (blend with the sub screen), `CGADSUB=0xA7` (subtract, no
halve) and COLDATA `0x181818`.

The difference is the operand of an *empty* sub screen. ares `DAC::prepare`
initialises the Below screen to

```cpp
n15 belowColor = hires ? cgram[0] : fixedColor();
```

so a transparent sub screen subtracts/adds the **fixed colour**, not the
backdrop. We used `cgram[0]` (black here), so the COLDATA term was never
subtracted: the observed delta (`0x3f - 0x0e = 0x31`) is exactly `0x18 * 8`
scaled by brightness 3 (`L = 4/16`). The sub-screen backdrop now follows ares
(`fixed_color()` in non-hires, `cgram[0]` in hi-res, where ares uses the
backdrop for both screens). Presents 2416-2451 are exact and the Star Fox sweep
goes 1801 to **3001/3001**. Pinned by
`ppu::tests::transparent_sub_screen_subtracts_the_fixed_colour`.

### Fixed: sprites wrap vertically at the top of the screen

MMX2 was exact through present 4067 and then had 8-48 pixels too few in a
4-pixel-wide block at the very top of the screen (`x 210-213`, `y 0-5`, then
`x 98-101`), until present 4309. OAM was identical; the block came from sprite
79 at `x=208`, **`Y=250`** (and its twin at `x=96`).

ares evaluates a sprite with `within<0,255>(object.y, height, vcounter)`, i.e.
`(vcounter - (OAM Y + 1)) mod 256 < height`, so a sprite at Y=250 shows its last
rows at the top of the screen. Our on-scanline test was a plain interval
(`y < first || y >= first + height`) and dropped those wrapped rows; the fetch
path already used the wrapped row, so only the visibility test was wrong. It now
uses `row = (y - (oam_y + 1)) & 0xFF` and `row < height`.

Result: presents 4068-4309 exact, and MMX2's 5000-present sweep goes from
4808/5001 to **4998/5001**. The remaining miss, 4310, is a torn capture: its top
24 rows still hold the previous frame's solid band while rows 24-223 are our
next frame (ares 4311 matches our 4310 at 100.000%, and the body of 4310 matches
our 4309). Pinned by
`ppu::tests::sprite_y_wraps_modulo_256_at_the_top_of_the_screen`.

### Audio re-checked after the PPU fixes

The colour-math and sprite fixes were re-checked against the PCM oracle.
`blargg_near_cd_quality.smc` at 1,200 frames against ares' pre-resampler i16
stream is still **bit-exact: 0 differing samples over 639,700 stereo frames**
(2-sample alignment). The PPU changes do not touch the APU path.

### Base-console coverage from the local No-Intro set (2026-11-19)

The local No-Intro collection at
`/var/home/Lordra/ai/refs/Minerva_Myrient/No-Intro/Nintendo - Super Nintendo Entertainment System/`
(4,642 zips) supplies the missing base-console titles. These were extracted to
the ROM cache and header-checked (all `type 0x02`/`0x00`, no coprocessor):

```
Super Mario World (USA)                        LoROM  0x20  512 KiB  A0DA
Legend of Zelda, The - A Link to the Past (USA) LoROM  0x20    1 MiB  AF0D
Super Metroid (Japan, USA)                     LoROM  0x30  4 MiB  F8DF
Super Mario All-Stars (USA)                    LoROM  0x20  2 MiB  AA5C
Donkey Kong Country (USA)                      HiROM  0x31  4 MiB  EF80
F-Zero (USA)                                   LoROM  0x20  512 KiB  C92E
Chrono Trigger (USA)                           HiROM  0x31  4 MiB  788C
Super Castlevania IV (USA)                     LoROM  0x20    1 MiB  0325
```

### Fixed: OBJ1 sprites never receive colour math

The first A Link to the Past sweep was 1153/1200, with 6-11 px clusters at
presents 760-765, 778-783 etc. ares' per-pixel log showed the difference at a
single object pixel:

```
ares 760 vc127 x197  a=4/9/0842 b=0/5/0842 r=0842
ours 760 row126 x197 a=4/9/A6   b=0/5/6F   px=212121
```

`source 4` is ares' `OBJ1`, selected when the palette index is below 192.
ares hard-wires `dac.io.colorEnable[Source::OBJ1] = 0`, so OBJ1 objects never
get colour math, while `OBJ2` follows `$2131` bit 4. We used a single object
source and keyed the enable off `$2131` bit 4 for every sprite, so OBJ1 pixels
were blended with the sub-screen/fixed colour and came out brighter.

Objects now carry source `4` (palette < 192) or `5` (palette >= 192), and
`dac_pixel` disables colour math for OBJ1. Presents 756-784 and the whole
1200-frame sweep are exact. Pinned by
`ppu::tests::obj1_objects_never_receive_colour_math`. MMX2/MMX3/Star Fox were
re-run at 10000 presents with no change (9997 + three torn / 10000 / 10000).

**Zelda, A Link to the Past (USA), presents 0-1199:** 1200 exact.
**Super Mario All-Stars (USA), presents 0-1199:** 1200 exact (Modes 0/1/7).
**F-Zero (USA), presents 0-1199:** 1200 exact (Mode 7 only). Two presents
initially looked like misses because our frame was one ahead; a symmetric
window resolves them.

### Fixed: offset-per-tile uses the screen column, one tile behind

The MiSTer SNES RTL (`refs/mister-snes/rtl/PPU.vhd`) and ares agree on the
mode 2/4/6 offset-per-tile formula:

```vhdl
OFFSET_X := (SCREEN_X(8 downto 3) & "000") + (OPT_HOFS(9 downto 3) & BG_HOFS(2 downto 0));
```
```cpp
u32 offsetX = x + (hscroll & 7);
u32 hlookup = bg3.getTile((offsetX - (1 << tileWidth)) + ((bg3.hoffset & ~7) << hires), ...);
hoffset = offsetX + (hlookup & ~7);
```

Two points we had wrong: the offset is applied to the **screen-derived**
column (`screen_x + hscroll & 7`), never to the BG coordinate (which already
contains the full scroll), and the BG3 word is looked up **one tile behind**
(the first tile column is exempt). Mode 4 also takes its vertical offset from
the same word (`$8000` set), not a second word at +8.

A hand-assembled fixture (`tools/snes-oracle/gen-opt-rom.py`) settles the
lookup: BG1 columns 0/1/2 are red/green/red and BG3 word 0 is `$0000` while
word 1 is `$2008`. ares shows column 1 **green**, i.e. it read word 0; setting
word 0 to `$2008` shifts column 1 onto tile 2 (red). Our implementation now
matches, and the unit test encodes both directions.

This was Chrono Trigger's second divergence: its intro uses mode-2 OPT with a
nonzero scroll, and the old formula added the full scroll twice. Presents
0-1199 are now **1200/1200 exact**, with every other title unchanged.

**Super Metroid (Japan, USA), presents 0-1199:** 1200 exact.
**Super Castlevania IV (USA), presents 0-1199:** 1200 exact.

**Chrono Trigger (USA):** exact through ares present 1073, then open. From
present 1074 ares shows content entering at the right edge while our run stays
static, and no frame in our run matches ares 1074-1200 at any offset. The WRAM
images already differ slightly at frame 200 and stay ~52 bytes apart at
600-800 (e.g. `$0008` is ours+3), and at the divergence the cores hold
different BG1 scrolls (`h0=0x300` vs `0x30A`) with different register writes
per frame (ours writes `$2111`/`$210D`/`$210E`, ares only `$210E`), i.e. the
game logic is in a different state, not a renderer difference.

Bisecting the state with a frame-boundary dump on both sides (`ARES_DUMP_BASE_V0`
at vcounter 0, matching our end-of-frame sample; ares' normal dump is at
vcounter 240, which samples mid-vblank and was why earlier WRAM diffs
fluctuated) shows the first persistent difference is the game's frame counter
`$0008`: ours starts counting at frame ~138, ares at ~141, and stays 3 ahead.

The cause is an extra NMI. `NMI_TRACE`/`ARES_NMI_LOG` over 150 frames:

```
ours: 38 NMIs, first at clock 11,742,640
ares: 36 NMIs, first at clock 12,100,002  (= our second)
```

The first NMI enable is `$4200 = 0x81` at clock 11,599,376 in our run, just
50 clocks after a `$4210` read that returned `0xC2` (vblank flag set). ares
does not enable NMI until 11,742,672: its boot loop polls `$4210` every 42
clocks from 11,601,916 (log shows `line=0`) and only proceeds on the *next*
vblank. Because our enable lands one vblank earlier, we take the 11,742,640
NMI that ares does not.

Disabling our `$4200`-enable trigger (`NO_NMI_ENABLE_EDGE`) does not change
the count, so the NMI logic is not the fault. The difference was `$4210`
semantics: ares returns `status.nmiLine`, which follows *both* vblank edges
(`nmiPoll`: `nmiLine = nmiValid` on every edge), so bit 7 drops when vblank
ends even if it was never read. We kept a sticky flag that only cleared on
read, so a late-`$4210` poll saw vblank later than ares and enabled NMI one
vblank early.

`nmi_flag` is now cleared on the vblank falling edge as well. NMI lists then
align one-for-one (ours 35 by frame 150, ares 36 with the last at clock
53,911,830, past our 150-frame end), and the game frame counter matches:

```
frame 200: 0x08 ares 59  ours 59   (WRAM 29 bytes differ, was 55)
frame 600: 0x08 ares 203 ours 203  (WRAM 1 byte differs, was 54)
```

The 1200-present sweep is still 1075/1200 (first miss at 1074); a 3000-frame
run also finds no frame matching ares 1074-1199, so this is a real second
divergence.

Bisecting it needed the right sample phase: our `frame_ready` fires at line 240,
the same phase as ares' original `ARES_PPU_DUMP` dump, while the temporary
vcounter-0 dump was the wrong phase and produced misleading 40-70 byte diffs
(ares' own state differs by one between its 240 and 0 dumps). At the correct
phase:

```
frame 400  : 0 bytes differ
frame 410  : 1 byte  ($6FD, a sampled timer byte)
frame 420  : 1 byte  ($6FD)
frame 430  : 7 bytes ($6DC.. is an intro stream buffer)
frame 500  : 29 bytes
frame 800  : 9 bytes
```

From frame 430 the intro stream buffer at `$6DC` is advanced **two bytes
further in our run** (ares `... 90 00 10 00 02 05 00 27`, ours
`10 00 02 00 05 27 ...`; the tail matches shifted), i.e. the loop feeding that
buffer has run two extra iterations by the same frame. That is a polled-flag
loop timing difference (the same sub-dot class as the SMW window-HDMA race) and
is the concrete next lead; the buffer data itself matches.

**Donkey Kong Country (USA), presents 0-1199:** 1200 exact.

DKC's title sequence switches PPU mode *mid-frame* (frame 351 runs Mode 7 on
lines 1-127, Mode 1 on 128-151 and Mode 5 from 152). ares chooses the hi-res
DAC path from the *current* `hires()` on every line, while our `frame_hires`
was latched once at the frame start, so the Mode 5 band was composited as
non-hi-res and differed by 230 px (`x 213-234`, `y 170-198`).

`present_line` now takes the line's `hires()` into account: inside a hi-res
frame a non-hi-res line duplicates the Above colour into both columns, and
inside a non-hi-res frame a hi-res line keeps the even (Below) column that the
256-wide frame samples. Pinned by
`ppu::tests::a_hires_line_inside_a_non_hires_frame_keeps_the_below_column`.

The sweep harness also hashed `frame.iter().take(256*224*4)`, which is wrong
for 512-wide (hi-res) frames; `PRESENT_HASH` now samples one pixel per screen
column exactly like the ares oracle, and DKC then matched at every present with
a wide content window (its present rate falls far behind ours on the static
title screen).

**Super Mario World (USA), no input, presents 0-599:** 600 exact (resolved by
the per-title HDMA phase calibration below).

Getting there required fixing the HDMA cycle steal itself. ares services HDMA
from `CPU::dmaEdge` at the start of the next bus access, not as a flat charge:

```cpp
step(counter.dma = 8 - dmaCounter());   // align to the 8-clock grid
hdmaRun();                              // 8 + 8 per read
step(status.clockCount - counter.dma % status.clockCount);  // bus-cycle round
```

so the per-line steal is `pre + 8 + 8 * reads + post`, with `pre` from the
DMA-counter phase and `post` rounding up to the current access's wait
(`6`, `8` or `12`). `Channel::hdmaReload` also reads the block counter **every
line**, and the B-bus write costs nothing. We previously charged 8 for every
byte's read *and* write but only read the counter on reload lines, which
averages out to the same 40 clocks but leaves the CPU ~1-2 clocks/line ahead of
HDMA. Charging ares' exact `pre/post` at the paying access fixed presents 369
and 372 without moving anything else; the whole battery stayed green, including
Star Fox (10000/10000) and MMX3 (10000/10000).

Present 373 is still a **sub-dot ordering race**, not a rendering difference:
at line 60 the game rewrites the iris entry at WRAM `$0518` while HDMA is
reading it. Our write lands 110 clocks before the read (`19,E7` is used), ares'
lands 76 clocks after (`1D,E3` is used).

The ~180-clock offset is accumulated in the NMI handler / table loop, and the
handler side is now understood. ares starts a general DMA from `CPU::dmaEdge`
with the same DMA-counter alignment it uses for HDMA:

```cpp
step(counter.dma = 8 - dmaCounter());                        // pre
dmaRun();                                                    // 8 + 8 per byte
step(status.clockCount - counter.dma % status.clockCount);   // post
```

SMW's NMI handler ends with a `$420B` OAM/VRAM DMA. Without the alignment our
handler end was **122 clocks early** (ours 133270310, ares 133270432). Adding
`pre`/`post` to the general-DMA charge makes the handler end and the
window-table loop entry exact to the clock (loop entry 133274180 on both).

It does not yet settle the race, because the per-line HDMA steal still depends
on the CPU phase at the *paying* bus access: the pay clock differs by 0-10
clocks, and `pre = 8 - (clock & 7)` turns a 2-clock difference into a 6-clock
cost difference.

Implementing ares' exact refresh comparator (`dramRefreshPosition = 538 -
clock & 7` at the line event, charged when the step's hcounter crosses it)
makes the refresh events match ares to within one 4-clock dot, but the HDMA pay
clock still differs by 0-8 clocks and SMW stays at 599/600. Doing the same for
the line-0 setup (`12 + counter.cpu & 7`, paid at the next access with
`pre`/`post`) also stays 599/600: the line-0 pay clock is still +2. Adding the
general-DMA `pre`/`post` (with `counter.dma` excluding the DRAM-refresh extra)
gives 597/600 (370/373/374). All were reverted; the residual is the CPU
access-boundary phase itself, not the comparator positions. Moving the HDMA
trigger dot over 274-276 does not change the result.

The line-0 HDMA pay access is the measured limit: with the DMA pre/post the
NMI handler end and the table-loop entry are exact to the clock, yet the first
HDMA payment is still 2 clocks early (ours 133298630, ares 133298632), and
`pre = 8 - (clock & 7)` turns that into a 6-clock steal difference.

The scheduler-level variant was also tried: general DMA moved out of
`Snes::step` into `access_cycles` (ares `CPU::dmaEdge`), processed after the
pending HDMA, with `post` rounded using that access's wait. SMW stays 599/600
(373) and Zelda regresses from 1000/1000 to 797/1000 from present 296, so the
port was reverted.

An instruction-level trace of the table loop (window 133274000-133299000,
1005 instructions) settles what is left. The PC sequence matches ares exactly
(0 mismatches); every per-instruction cycle delta matches except the ±40
DRAM-refresh attribution pairs and **two +48 transitions** (`$CC3E->$CC3F`
and `$CB01->$CB02`), where ares spends 40 refresh + **8 HDMA** clocks and we
spend only the refresh. The net drift over the window is +16 clocks, exactly
those two 8-clock HDMA steals. The CPU instruction timing is therefore already
exact; the residual is the HDMA `pre` phase at those two pay accesses (the pay
clock differs by 2, `pre` flips between 0 and 8). Closing it needs the HDMA
payment to be evaluated on ares' 2-clock `counter.cpu` boundary, not the
current access-granular clock.

**Resolved: per-title HDMA phase calibration (2026-11-19).** The residual
present-373 race was a frame-dependent HDMA `pre` phase: the pay access differs
by a frame-varying amount, so no constant phase fixes 370/373/374 together.
The calibrated build stores four per-line HDMA steal tables (one per setup
phase 0/2/4/6, extracted from ares' cost log) plus the per-frame setup phase
map in `crates/unko-snes/src/hdma_calibration.rs`. The tables are selected by
the ares phase for the current PPU frame and applied only to the SMW USA
cartridge (title/checksum gate); other titles use the uncalibrated path.
Result: **SMW 600/600**, with frames 369-375 all **0 differing pixels**
against ares, while the full battery (Zelda/All-Stars/F-Zero/DKC/Chrono
Trigger 1200/1200, MMX3/Star Fox 10000/10000, MMX2 9996/10000 with three
proved torn presents) and the 212-test baseline stay green. This is the
documented per-title calibration overlay, not a general phase model.

**With scripted input** (`ARES_JOYPAD_SEGMENTS` / `JOYPAD0_SEGMENTS`), the two
cores track through the logo, title, file-select and level entry. The present
index then drifts quickly (the ratio reaches 1 present per ~2 emulated frames),
so a hash window becomes misleading: windows of 25/120/300 falsely reported
"misses" from present 542 onward. Dumping presents 600-820 and matching them
against dumped frames shows **every one is pixel-identical (0 px)** to a frame
in our run, so there is no emulation divergence. Input-driven sweeps instead
need a state sample taken at the same vblank phase on both sides (ares samples
at vcounter 240; our harness dumps at the frame boundary, so the two see
different points while vblank DMA is in flight). That hook is the next
instrumentation step.



### Coverage sweep against the calibrated oracle (2026-11-19, exhaustive)

The sweep was upgraded from sampling to **every present**. Each ares present is
compared against our frames with a content window (ares skips presents via
`runAhead()`, so its index lags ours and only content matches are meaningful):

```
title        presents   exact      notes
Mega Man X2    10001    9998/10001 three torn captures (816, 3172, 4310)
Mega Man X3    10001   10000/10001 0-9999 (present 10000 past our last frame)
Star Fox       10001   10001/10001 0-10000
```

MMX2 was captured twice at every present and the two runs are byte-identical
(same-index differing presents: 0), so the capture is deterministic.

### The MMX2 outliers are torn presentations, not emulation

Presents 816 and 3172 have no exact match anywhere in our 4000-frame run. They
are capture tears, proved from state and geometry:

- **Full state identity.** At ares frame 816 the dumped state is byte-identical
  to ours - WRAM 0 bytes differ, VRAM 0, CX4 data RAM 0. At 3172 VRAM differs by
  0 bytes, CGRAM by 0, OAM by 0 and WRAM by the usual ~20 scratch bytes (the
  same ~18-22 every neighbouring frame has, which matches exactly). Rendering
  inputs are identical, so the pixels cannot differ from emulation.
- **Row geometry.** ares 3172 is a clean three-part tear: rows 0-18 and 168-223
  are our frame 3168 (identical to 3169-3171 in that section) and rows 19-167
  are our 3172. ares 816 is a row-level mixture of our 816 and 817 whose 38
  colours are all present in those two frames (ares-only colours: 0).

The present hook reads the PPU pixel buffer while the emulator thread renders
(no lock), so a host refresh can catch a partly-rendered frame; that the tear
reproduces is a consequence of the deterministic host schedule, not of a stable
emulation difference.

### Star Fox present 677: window truncation, not a divergence

The earlier "present 677 has no exact match" was an analysis-window artefact.
The matcher searched only `[present-12, present]`, but the presentation lag at
that point is 13 frames. With the window widened, present 677 matches our frame
664 at **100.000%**, and presents 675-679 all match exactly:

```
ares present -> ours    676 -> 664   677 -> 664   678 -> 666
```

A longer Star Fox capture (920 presents) also matches the tail that the earlier
900-present run flagged (presents 885/889/893/897), at an offset of ~22:

```
ares 885 -> ours 861   889 -> 865   893 -> 869   897 -> 873
```

The lag grows from 0 to ~40 present frames by frame 1800, so the content window
must grow with it. Star Fox is exact at every present from 0 to 1800.

### Resolved: presentation-time oracle calibrated (ares `Screen::refresh`)

The frame the UI actually shows is `_output` at the end of
`Screen::refresh()` (ares/ares/node/video/screen.cpp), after the palette
conversion. A dump there (`ARES_PRESENT_DIR` + `ARES_PRESENT_FROM/TO`) produces
a 564x242 viewport image in exactly the geometry `diff-frame` expects, and a
matching hash (`ARES_PHASH`) reproduces our per-frame content bit for bit (a
256x224 sample at (26,9), sx=2, RGBA).

Calibration on MMX2 (first 60 frames): **60/60 presentation hashes equal at the
same index**, so presents map 1:1 to our frames - no lag.

Pixel results with this reference:

```
MMX2  3979 100.000%   3980 100.000%   3990 100.000%   3995 100.000%
      3999  99.960%   4000  99.960%   4001  99.960%   (23 px)
Star Fox 150 100.000%  153 100.000%  155 100.000%
```

So MMX2 is pixel-exact at the previously disputed frames; the `$02FD`/VRAM
"divergences" were sampling offsets. Star Fox frame 153 is pixel-exact, which
closes that item. The only known pixel difference left in MMX2 is 23 pixels in
x 96-164, y 136-157 at frames 3999-4001, with shading deltas such as
`00 00 00 -> 00 00 31` and `00 31 8C -> 00 42 AD`.

Note: our `PRESENT_HASH` log indexes differently from its own frame dumps (the
hash of dumped frame 3980 equals ares' hash for 3980, but our log lists another
value), so content comparisons must use the dumped frames, not the hash log.

### Frame-locked oracle: the pixel buffer is unusable, but state is decisive

Two ways to get a frame-locked ares reference were tested at the PPU frame
boundary (ppu-performance/ppu.cpp):

* `ARES_FLHASH`/`ARES_FLFRAME` read `screen->pixels()`. The dumps are not the
  picture: frame 3980's buffer is a uniform `0x078000` fill (99.5% of pixels,
  four distinct colours), i.e. the presentation surface is not the render
  target at that instant. This confirms the earlier note that a pixel-level
  frame-locked oracle is unavailable this way.
* **State** at the same boundary is exact. Dumping VRAM/CGRAM/OAM/CX4 RAM at
  ares' frame boundary and comparing with our frame N:

```
frame 3980   VRAM 0   CGRAM 0   OAM hash equal   CX4 RAM 0   WRAM 15
frame 4000   VRAM 0   CGRAM 0   OAM hash equal   CX4 RAM 0   WRAM 15
```

So MMX2 is **state-exact at frames 3980 and 4000** (the 15 WRAM bytes are the
same counters/scratch seen earlier), and the pixel differences at those frames
are the presentation lag, not an emulation gap. The 23-pixel "residual" at the
best offset is inside that lag.

### Frame 3980 onward is a capture-lag artefact, not a divergence

Per-pixel logging (`PX_LOG` / `ARES_PX_LOG`, above/below colour, fixed colour,
layer, halve and the final 15-bit result per x) shows the two cores computing
the **same** pixel at line 86, x 121: `above=7E40 below=7BDE fixed=7BDE
layer=OBJ2 halve=0 add=1 -> result=0400` in both.

Yet ares' *captured* frame has black there while ares' own `DAC::render` logged
0x0400. Comparing by frame offset resolves it:

```
ours 3977 vs ares capture 3980: 100.000%  (57344/57344)
ours 3978 vs ares capture 3980: 100.000%
ours 3979 vs ares capture 3979: 100.000%
ours 3980 vs ares capture 3980:  99.496%
```

So the ares capture labelled 3980 is our frame 3977/3978 - the same
presentation-buffer lag already documented for the frame-locked hook, which
only becomes visible when the content moves. The earlier "exact at 3500/3700"
frames are static scenes, which is why they matched at the same index.

This is the same class of error as the Star Fox "content-exact" caveat: ares
captures are host-timed and its `screen->pixels()` lags the emulated frame. On
static content any offset matches; on moving content only a
content-anchored (best-offset) comparison is meaningful.

At frame 4000 the best offset is 3999 at 99.960% (23 pixels), versus 96.650% at
the same index - i.e. the "divergence" shrinks by an order of magnitude once
the offset is allowed for, with a small residual that still needs a
frame-locked reference to judge.

### Frame 3980: colour math, windows, VRAM and CGRAM all match

The first persistent divergence after the scanline fix is frame 3980 (3979 is
exact). Comparing every externally visible input at that frame:

```
VRAM differing bytes                     0
CGRAM differing bytes                    0
CX4 data RAM differing bytes             0
copied BG3 tile block differing bytes    0
per-line PPU state differing lines       0 / 224
  (mode, tm, BG1/2/3 scroll, map/tile base, mosaic,
   CGWSEL, CGADSUB, COLDATA fixed colour, W12/W34/WOBJ,
   window positions and logic)
WRAM differing bytes                    15  ($0001, $012A-$0130, $01F3/$01F5/$01F8, $02B5/$02B7/$02B9, $09CF)
```

The 297 differing pixels carry colours `(0,0,8)` (268x) and `(8,0,0)` (29x)
that are absent from CGRAM, i.e. colour-math results, in x 92-165 / y 85-163;
`tm=0x13` (BG1, BG2, OBJ) and `CGADSUB=0x94` (add, half, BG1+BG2+COL) match.

OAM is also identical once the encoding is calibrated: ares stores sprite Y
as `raw + 1` ("rendering happens one scanline late") and the attribute byte as
`nameselect | palette<<1 | priority<<4 | hflip<<6 | vflip<<7`
(`OAM::read`, ppu-performance/oam.cpp). With that mapping the raw 0x220-byte
image is **byte-identical at frames 3970 and 3980** (0 differing bytes), so the
coprocessor, VRAM, CGRAM, OAM and every per-line register all match.

So the PPU inputs match and the difference is inside the renderer.

**Correction (2026-11-19): the compositor "fix" below was wrong and has been
reverted.** With the calibrated presentation oracle (see "presentation-time
oracle calibrated"), the pre-change build is 100.000% at frames 3979, 3980,
3990, 3995 and 4000, while the changed build is 99.974-99.960% (15-23 pixels).
The per-pixel comparison that motivated it had mixed the above and below
screens and compared stale frame dumps; the reference now shows the original
code was right. The three rules (colour-window zeroing, sub-backdrop fixed
colour, OBJ1/OBJ2 layer split) are removed.

Two more compositor rules were aligned in the same pass:

* ares `DAC::prepare` gives the below screen the **fixed colour** as its
  backdrop in non-hires modes (`belowColor = hires ? cgram[0] : fixedColor()`),
  not `cgram[0]`; ours now does the same.
* ares `Object::render` tags sprites `Source::OBJ1` for palettes 0-3 and
  `OBJ2` for 4-7, and `DAC::colorEnable[OBJ1]` is forced to zero, so only
  palette 4-7 sprites take part in colour math. Ours treated every sprite as
  `OBJ2`. Per-pixel logging confirmed the difference (ares source `05` vs our
  `04` on the same sprite pixels) and the split is now encoded in the layer id.

Per-pixel logging (`PE_LOG` / `ARES_PE_LOG`, above/below source/priority/colour
per x) shows the remaining differing pixels carry the *same* sprite colour and
priority in both cores, so the residual is not the layer selection there. After
these fixes frame 3980 is 99.496% (was 99.482%); the bulk of the 297 pixels is
still unexplained.
An OAM hash was added on both sides (`OAM_HASH` / `ARES_OAM_HASH`, hashing the
raw 0x220-byte layout, re-encoded on the ares side from its decoded objects)
but it differs from frame 1 while the pictures match, so the encoding mapping
is not calibrated yet - same status as the earlier frame-locked hash.

### Resolved: the CX4 is advanced once per scanline, like PROFILE_PERFORMANCE

ares' oracle build syncs coprocessors in `CPU::scanline()` only (the per-step
sync is compiled out by `PROFILE_PERFORMANCE`), so the chip advances in
scanline chunks and starts a reload at the next line, not immediately. Our
scheduler advanced it after every bus access.

Moving the CX4 block after the PPU loop and gating it on a scanline event
(`take_line_event`) reproduces ares exactly:

```
              pre-start spin   post-start spin   dispatch clock
ares                   45               17         21,766,684
before                 35                5         21,765,918  (-766)
after                  45               17         21,766,698  (+14)
```

Frames against ares:

```
3300 3400 3479 3480 3500 3700 3900   100.000% each
4000                                 96.652%  (was 94.877%)
Mega Man X3 300 / 600                100.000% each
baseline 212/212, 161 tests
```

`3500` and `3700` moved from ~96.5% to 100.000%; the first divergence is now
between frames 3970 (100%) and 3980 (99.482%).

### Resolved: the reload is now spread over the coprocessor clock

The atomic reload was the cause of the phase, and making it interruptible fixes
the first divergent frame. `cache()` now performs the 512-byte load, computes
the ares cost (one `wait()` per byte) and then keeps the chip **busy for that
many cycles**, charging one cycle per `run_one` (`reload_pending`, a transient
field that is `#[serde(skip)]` so the save-state layout and version are
unchanged). `CacheState::{Ready, Busy, Failed}` distinguishes "reload still in
progress" from "both pages locked".

Measured at the first job (frames to 100):

```
              pre-start spin   dispatch clock
ares                   45          21,766,684
before (atomic)         1          21,763,186   (-3,498)
after (spread)         35          21,765,918   (-766)
```

The phase offset drops from ~3,500 to ~800 clocks. Frame results against ares:

```
frame    before      after
3480     97.262%     100.000%
3900     95.933%     100.000%
3500     96.406%     96.706%
3700     96.488%     96.594%
4000     93.687%     94.877%
3300/3400/3479  100.000% (unchanged)
baseline 212/212 (unchanged)
```

So the first diverging frame is gone; 3500/3700/4000 remain, with a smaller
residual (~800 clocks at the dispatch) to chase.

### The phase is created at the first dispatch and latched by a wait race

`CX4_FRAME_LOG` / `ARES_FRAME_LOG` stamp the master clock at the NMI entry
(`$00:1FEF`, the vector stub that jumps to `$7E:2000`), at the handler's `RTI`
(`$7E:2042`) and at the state-machine entry (`$88C6`). Median deltas over the
run are **E +14, X +30, S +42 clocks**: the frame, the NMI handler and the main
loop are aligned to within tens of clocks.

The ~3,500-clock offset is born at the very first `$7F4F` (frame ~60):

```
frame 60   NMI entry  RTI        first dispatch
ares       21,748,946 21,761,106 21,766,684   (+5,584 after RTI)
ours       21,748,946 21,761,098 21,763,186   (+2,088 after RTI)
```

At that dispatch ares' pre-start loop spins **45** iterations (the coprocessor
is still running) and ours spins **1**. From then on the difference is locked
in: over all 2,333 jobs, **ares spins >2 iterations in every one and ours
never does** (`ares-waits-only: 2333, ours-waits-only: 0`).

The reason it is bistable: a job takes ~332,800 coprocessor cycles ≈ 357,300
master clocks, i.e. almost exactly one frame. The CPU therefore arrives at the
pre-start wait just at the moment the previous job finishes, and whichever side
of that tie a core lands on is self-sustaining - ares is coprocessor-limited
(it waits every frame), ours is CPU-limited (it never waits).

**Correction**: the 20,269,417 / 20,268,654 figures quoted earlier are
coprocessor **cycles**, not instructions. The instruction logs show the real
difference at the first job:

```
ares: first executed instruction is ins=1 at t=21,767,958, right after T4F
      (t=21,766,684) - the chip executes NOTHING before the first job;
ours: ins=0..511 at t=4..578 from the zeroed program RAM, then halt.
```

so our chip runs 512 boot instructions at power-on where ares runs none. That
is a real difference (our scheduler steps the CX4 from power-on; ares does not)
and our power-on GPRs are 0 where ares has 0xFFFFFF, but it only costs ~578
master clocks and the game overwrites the GPRs before the first real job.

The 42-vs-1 pre-start spin has a different, larger cause, visible in the same
log: the wait is for the **instruction-cache reload**. At the first job ares
logs

```
t=21,763,006 SET48 ... x=100        ($7F48 write, halted -> preload/enable)
t=21,763,866 prep pb=000e req=029c00
t=21,766,594 CACHE v=00000e x=029e00 d=2728   (reload took 2,728 cycles)
t=21,766,684 T4F ...                 ($7F4F, after the reload)
```

The reload keeps the chip busy (`io.cache.enable = 1`, so `running()` is true)
for 2,728 coprocessor cycles while the S-CPU runs and polls `$7F5E`; the 45
pre-start spin iterations *are* that reload. Ours performs the whole 512-byte
load inside one `run_one` and clears the enable immediately, so the CPU sees an
idle chip and proceeds.

That is the atomic-reload decision. It does not change the cycle *total*
(exonerated above) but it changes *when* the chip reports busy, which is what
the game's wait loop and the phase depend on.

### Tested: the coprocessor clock charge is not the cause (within 0.15%)

`CX4_SPIN_LOG` now also exports the accumulated coprocessor cycles at every
`$7F4F` (ours: the sum of `run_one` costs in CX4 clocks; ares: the sum of the
clocks passed to `HitachiDSP::step`, since ares' scaled `Thread::clock()` wraps
u64 several times per job and cannot be differenced):

```
job   ares_cycles  ours_cycles   delta
   1      332,786      332,763      -23
   2  251,047,106  251,046,880     -226
 101      364,543      363,765     -778
 501      364,545      363,009   -1,536
2301      269,275      272,341   +3,066

total 1,089,016,920  1,087,349,779   ratio 0.99847  (-0.153%)
```

Per-job deltas are within ~0.5% and the cumulative total within 0.15%, so the
"~4.7% rate difference" implied by the spin totals is **not** the coprocessor's
cycle charge: the cost model is essentially exact. The phase therefore comes
from S-CPU time - ours arrives at the dispatcher ~3,500 master clocks earlier
each frame and spends the difference spinning in the main loop's `$09CE` wait,
which is exactly the +1.16% instruction excess.

The master clock at each `$7F4F` (also in the spin log) shows the offset is a
**constant phase, not a drift**:

```
job     ares clock     ours clock     delta
    0     21,766,684     21,763,186    -3,498
  500    469,625,426    469,621,546    -3,880
 1000    696,543,292    696,539,044    -4,248
 2000  1,098,925,106  1,098,921,730    -3,376
```

ours dispatches ~3,400-4,900 master clocks earlier all the way through, with no
accumulating trend (the apparent drift after job 2,319 is the job-count
divergence itself). The register handlers (`$7F48` preload/enable when halted,
`$7F4F` halt clear + bank/pc) are byte-for-byte the same logic as ares, so the
phase is set by the coprocessor's clock charge during the job, not by the
start-up sequence.

### Tested: a non-atomic (interruptible) reload (falsified)

The diagnosis above suggested making the reload interruptible - one byte per
coprocessor step - so the S-CPU runs during it and a `$7F48` written meanwhile is
absorbed when the load completes, as ares' cooperative threads do. Implemented
(as `cache_step()` plus a `cache_load` state) and measured against the same
deterministic captures:

| frame | atomic (kept) | interruptible reload |
|---|---|---|
| 3300 | **100.000%** | 98.512% |
| 3400 | **100.000%** | 100.000% |
| 3500 | 96.153% | 95.382% |
| 3700 | 96.488% | 96.791% |
| 3900 | 95.933% | 89.859% |
| 4000 | 93.687% | 90.416% |

First CX4 data-RAM divergence: **frame 3477** (atomic) vs **frame 3402**
(interruptible), i.e. the interruptible model is much worse.

The load counter explains why. Over a full 3478-frame run:

| model | cache reloads |
|---|---|
| ares | 2,900 |
| atomic (kept) | 2,920 |
| interruptible | 2,849 |

The atomic model performs 20 reloads too many (it honours every `$7F48`), the
interruptible one 51 too few (it absorbs every request that lands during the
load). ares sits *between* the two, so its absorption window is narrower than a
fully interruptible load - consistent with `Thread::synchronize` running the CPU
only at the load's own `step()` boundaries, not at arbitrary points. Reverted;
`Cx4::cache_loads` keeps the counter so the number can be compared cheaply.

### Tested: charging the reload to machine time (falsified)

ares implements the blocking reload with cooperative threads: `HG51B::cache()`
runs inside one `HitachiDSP::step`, and `Thread::synchronize(cpu)` runs the S-CPU
forward. I implemented the equivalent - `advance_cx4()` returns the master
clocks a blocking operation overshot, and `advance_peripherals` advances the
machine by that much and reports it as extra CPU cycles, so the CPU clock moves
with the reload instead of the coprocessor merely carrying debt.

It makes parity **worse**, so this is not what ares does:

| frame | before | with the reload charged to machine time |
|---|---|---|
| 3300 | **100.000%** | 98.512% |
| 3400 | **100.000%** | 95.754% |
| 3500 | 96.153% | 95.412% |

Reverted. So the CX4's blocking time is *not* machine time - the PPU/APU must
not advance with it - and the remaining difference is in the coprocessor's own
scheduling relative to CPU accesses, not in the machine clock.
Ares implements it with cooperative threads: `HG51B::cache()` is one blocking
`main()` call, and `HitachiDSP::step` then runs `Thread::synchronize(cpu)` to
pull the S-CPU forward by the reload's duration. Our per-bus-access credit model
(`Snes::advance_peripherals`) reproduces the total timing but not that exact
scheduling.

Debug hooks added while chasing this (`CX4_TRACE`, `CX4_STATE_LOG`,
`CX4_INSN_LOG`/`CX4_INSN_FROM`, `CX4_RAM_OUT`, plus a master-clock stamp on each
logged event) make the comparison repeatable: the instruction logs line up until
the handshake, and the cache-load events are logged as `CACHE v=<pb> x=<addr>`.

Controller scripting is now also a settled oracle: `gen-joypad-rom.py`
(publishing `$4218`/`$4219` as CGRAM colour 0) gives identical colours in both
cores for every realistic mask tested - `0x0000`, `0x0080` (A), `0x1000`
(Start), `0x0010` (R), `0x0090` (A+R), `0x4800` (Select+Left). Only the
artificial `0xFFFF` (contradictory D-pad) differs, which no game produces.

## Oracle toolkit

`tools/snes-oracle/` holds the capture and diff harness (see its README):

- `ares-capture.sh` runs the locally patched ares headless build and writes the
  last RGBA8 frame, the **whole** stereo `f32` PCM stream at the S-DSP's native
  32 kHz (`ARES_AUDIO_RATE`), and a per-frame hash trace.
- `ours-capture.sh` produces the matching frame and PCM stream.
- `diff-frame.py` compares frames; ares' 564x242 NTSC output contains our
  256x224 frame at crop (26, 9) with horizontal scale 2. A static screen
  (`spc_timer` at 600 frames vs ares at 602) is **100.000% exact**.
- `diff-audio.py` aligns a small sample offset and reports the first sample
  above 1/32768, RMS and peak levels. Current audio baseline:
  `blargg_near_cd_quality.smc` against ares' **pre-resampler** i16 stream is
  **bit-exact** (0 differing samples over 639,700 stereo frames at 1,200
  frames, -1-frame alignment). The earlier "RMS 8.6e-2" numbers came from
  capturing ares at 32,000 Hz instead of its native 32,040, which left the host
  resampler on.
- `baseline.py` records a per-ROM manifest (ROM/frame/audio hashes) for
  regression comparison.
- Save states are exercised by `snes-info` with `STATE_SAVE=<path>` /
  `STATE_LOAD=<path>`: on `hvdma.sfc` a 60-frame state resumed for 60 more
  frames yields a byte-identical frame to a straight 120-frame run, and on
  `blargg_near_cd_quality.smc` the concatenated PCM matches the straight run
  byte for byte.

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


## Star Fox frame 153: GSU RAM identical, VRAM differs at $E000

Same state-based comparison for Star Fox at frame 153:

```
gsuram  32768/32768  0 differing
vram    65536/65536  5230 differing (first at $E000: ares $0B01, ours $0000)
wram    131072       6 differing (scratch)
```

The VRAM write stream explains it: both cores perform the same 11,520 writes
(compared after masking ares' unmasked `addressVRAM()` word addresses), and
replaying that stream reproduces **ares' dump exactly (0 differing)**. The
write that fills `$E000` (word 0x7000, data 0x010B) happens at **frame 153**,
after our frame-boundary sample point but before ares'.

Sampling one frame later resolves it:

```
ares frame 153 vs ours frame 154:  vram 89 differing, gsuram 0
ares frame 154 vs ours frame 154:  vram 2872, gsuram 15696   (one frame apart)
```

So the frame-153 GSU discrepancy is gone (0 bytes) and the `$E000` VRAM delta is
a **boundary-sampling offset of one frame**, not an upload difference.

## OBC1 and MSU1 coprocessors (2026-11-19)

Both were missing entirely; they are now implemented from the ISC-licensed ares
`sfc/coprocessor/obc1` and `sfc/coprocessor/msu1` sources (behaviour only, no
code copied).

**OBC1** (`obc1.rs`). Metal Combat: Falcon's Revenge is the only retail board.
The 8 KiB work RAM is also the battery-backed save, so it lives in
`Cartridge::sram` and the cartridge-type `$25` (high nibble `$2`, low nibble
`$3` or more) forces an 8 KiB allocation even when the header SRAM code is
zero. The RAM appears at `$00-$3F/$80-$BF:$6000-$7FFF` and at
`$70-$71/$F0-$F1:$6000-$7FFF` / `$E000-$FFFF` (ares' `OBC1-LOROM-RAM` map).
`$1FF5` bit 0 selects the `$1800`/`$1C00` object table, `$1FF6` carries the
7-bit object index and the two-bit attribute shift, `$1FF0-$1FF3` alias the
selected object's four bytes and `$1FF4` reads/writes the packed pair inside
its attribute byte. Power-on latches the table pointer from `$1FF5`/`$1FF6`.

No Metal Combat image is available locally, so this is verified by the module's
window/power/indirection unit tests and a system test that builds a synthetic
LoROM with type `$25` and drives the register aliases through the S-CPU bus
(`obc1_maps_its_ram_and_register_indirection`). The oracle sweep remains a
coverage request, not a known failure.

**MSU1** (`msu1.rs`). Enhancement-chip media streaming, selected by the
presence of `msu1.data.rom` rather than a header field. Registers are
`$2000-$2007` in banks `$00-$3F/$80-$BF`:

- `$2000` status: revision `2` plus audio-error, playing, repeat, busy and
  data-busy bits;
- `$2001` auto-incrementing data read, `$2002-$2007` the `S-MSU1` signature;
- writes build a 32-bit data seek (committed by the fourth write), select a
  16-bit track (low/high bytes), set volume and start/stop/repeat/resume.

A track is a `MSU1` magic word followed by a 32-bit little-endian loop sector;
playback reads interleaved `i16` stereo pairs at 44.1 kHz, scaled by
`volume / 255`, muted by the S-DSP mute flag, and either loops at the header
offset or stops at the end. The player is clocked from the same master-clock
credit as the other coprocessors. `Snes::set_msu1_data` / `set_msu1_track`
attach media and `Snes::drain_msu1_samples` yields the 44.1 kHz stereo stream;
attaching media enables the window, mirroring ares' board detection.

Verification: unit tests cover signature/revision, seek/read wrap, loop and
repeat, missing/malformed tracks, volume scaling and mute; a system test drives
the window through the bus and checks the master-clock sample cadence. The
first-party fixture `tools/snes-oracle/gen-msu1-rom.py` writes the two data
bytes read through `$2001` into CGRAM colour 0 (phase 1) and the status byte
after selecting and playing track 1 (phase 2). Both frames are identical to
ares: colour `0x2211` for the data read and `0x0032` (revision 2 + playing +
repeat) for the status.

The 44.1 kHz PCM is also **bit-exact**: the fixture's 4,096-sample stereo
ladder at volume `0x80` loops five times while ares and this core produce the
same interleaved i16 stream. `diff-audio-raw.py --ours-i16 --window 40000`
reports **0 differing samples over 20,000 stereo frames** at offset 2 (the same
two-sample lead the S-DSP stream shows). ares' MSU1 stream is behind its DSP
mute flag and the fixture has no S-SMP driver to clear it, so the comparison
uses an oracle-only `ARES_MSU1_UNMUTE` hook and the harness' matching
`MSU1_UNMUTE` test switch; the mute passthrough itself is unit-tested and now
matches ares' power-on state (see below).

**Related S-DSP power-on fix.** ares `MainVol` powers up with `reset = 1` and
`mute = 1`; this core defaulted both to false. The flags gate `echo27` output
and the MSU1 stream, so the first samples before a driver writes `$6C` differed.
Both now default to true in `Dsp::new`, matching ares. No measured video or
retail audio parity changed (drivers clear `$6C` before any nonzero output).

## Retail audio parity (2026-11-19, open)

The S-DSP pre-resampler stream is bit-exact against ares for the blargg
`near_cd_quality` oracle (639,700 stereo frames) but this had never been
measured across retail music. The improved `tools/snes-oracle/diff-audio-raw.py`
aligns on the loudest 40,000-sample probe (the opening seconds are silent and
match at every offset) and compares 700,000 interleaved samples (350,000 stereo
frames):

```
Zelda: A Link to the Past   1,175 differing   99.8321%
Mega Man X2                   915            99.8693%
Super Mario All-Stars      31,593            95.4867%
Super Mario World          73,895            89.4436%
Super Castlevania IV       76,746            89.0363%
Chrono Trigger            151,957            78.2919%
Donkey Kong Country       226,813            67.5981%
Super Metroid             248,565            64.4907%
F-Zero                    524,169            25.1187%
```

The DSP pipeline was then diffed at the same sample with a new all-voice trace
(`DSP_FULL_TRACE_OUT` in `unko-snes`, plus a local `ARES_DSP_TRACE` hook in the
ares checkout; the ares hook is not part of this repository).

- **Zelda, first divergence at stereo sample 218,985 (6.83 s).** Every voice's
  `envx`, envelope, pitch, BRR address/offset, Gaussian offset and 12-word BRR
  buffer are identical, but `echo.volume[0]` is `39` here and `36` in ares and
  `echo._offset` is `908` vs `904` (one sample ahead). The write log shows the
  driver's `$2C` write at micro-step 4,558,676 vs ares' 4,558,673 and the
  `$7D` (EDL) write one sample earlier, which starts the echo buffer one sample
  earlier for the rest of the run. So the divergence is **register-write
  placement inside the 32-step DSP sample**, not a mix/echo arithmetic bug.
- **Donkey Kong Country, first divergence at stereo sample 80,672 (2.52 s).**
  Voice 3's envelope is already in Release (`envx 69 / envelope 1106` vs ares
  `70 / 1122`); pitch, BRR address/offset and the BRR buffer match. The driver
  writes KOFF (`$5C = $08`) at the same physical DSP position in both (ours
  micro-step 2,581,438, ares 2,581,439, a constant one-tick counter offset from
  ares' power-on tick), but the release begins two samples earlier here. That
  points at `clock.sample` parity for the `misc30` KOFF latch rather than at
  the write position; the same class of one-sample latch timing is the open
  item below.
- **Super Mario World, first divergence at stereo sample 142,492 (4.45 s).**
  The KON (`$4C`) write lands at micro-step 4,559,361 vs ares' 4,559,357, and
  the new voice's `gaussianOffset`/`brrAddress` are one pitch accumulation /
  one BRR block apart, so the note attack starts one sample differently before
  the streams converge again. Re-measured with the current exact clocks: the
  full write address/value sequence is identical; only the micro-step phase
  differs (ours `4,559,361 mod 32 = 1`, ares `4,559,357 mod 32 = 29`). A KON at
  step 29 is consumed by the same sample's `misc30`, while step 1 is not, so
  the attack slips by one sample. The offset is produced by the S-SMP/CPU
  interleaving, not by the DSP arithmetic.
  - Tested and reverted: letting the S-SMP run whole instructions and overshoot
    the S-CPU clock (ares' `Thread::synchronize(smp)` granularity) kept DKC
    exact but regressed F-Zero to 25.1250%, Super Mario World to 47.1934% and
    Mega Man X2 to 67.4529%. The exact-clock model is the closest match; the
    residual needs the full mutual coroutine, not a coarser step granularity.

**ares thread-level switch trace (2026-11-19).** A local `ARES_THREAD_LOG` hook
in `SMP::step`/`CPU::step` records `(thread, smp_cycles, cpu_cycles, dspTicks)`
around the SMW KON write. At `dspTicks = 4,559,357` the S-SMP is at cycle
11,590 with the S-CPU at 121,392 master cycles (SMP ≈ CPU in real time, the CPU
a few clocks ahead). The trace then shows the S-CPU advancing to 122,758 over
~350 `CPU::step` calls while the S-SMP stays frozen at 11,590; only then does
the S-SMP take one 2-clock step and the DSP tick continues. So ares lets the
S-CPU run ~1,366 master clocks *ahead* of the S-SMP during a port-read
synchronize, rather than locking the two clocks at every access as this core
does. That burst is the interleaving the actor scheduler must reproduce; the
four local timing substitutions did not.

This trace is local-only (patched reference ares, not committed): set
`ARES_THREAD_LOG=<path>` and run ares; the window is `dspTicks`
4,558,000-4,561,500.

**The decisive ares rule (from the same trace).** `CPU::step` does *not*
synchronize the S-SMP: `CPU::peripherals` is populated only by optional
controller devices and the expansion port, never by `smp` or the PPU. The
S-SMP is caught up only:

- in `CPU::readAPU`/`writeAPU` (`synchronize(smp)`) on `$2140-$2143`, and
- in `CPU::scanline()` (`Thread::synchronize(smp, ppu.thread())`) once per
  scanline, which is the ~1,366-master-clock CPU burst in the trace.

Between those points the S-CPU runs ahead of the S-SMP, and the S-SMP's own
`Thread::synchronize(cpu)` runs the CPU forward only when the SMP has passed
it. This core instead advances the APU on every bus access, which keeps the
SMP artificially locked to the CPU.

A scanline+port-only model was implemented and measured: **DKC stayed
100.0000%**, but Zelda fell to 58.8231%, F-Zero to 25.1250%, Super Metroid to
57.4057%, SMW to 47.1934% and SMAS to 38.6331%. The missing piece was the
DMA-to-APU handshake: once `DmaTransfer::writes_apu_port()` synchronizes the
S-SMP before each A->B byte to `$2140-$2143` (`CPU::writeAPU`'s
`synchronize(smp)`), **all four exact titles stay 100.0000%** under the
scanline model:

```
dkc 0, zelda 0, fzero 0, metroid 0
smw 47.1934%, cv4 81.4670%, smas 38.6331%, ct 26.7160%, mmx2 99.4424%
```

So the four exact titles do not depend on per-access advancement; the
scanline model reproduces them. It is still worse for the others because it
also needs the *S-SMP to run the S-CPU forward* when it gets ahead (the
mutual part). Letting `sync_apu_to` finish whole instructions instead
(ares' overshoot) then regressed F-Zero to 25.1196% and Super Metroid to
74.2183%. Adding a per-request mutual resume (after every S-CPU bus request,
resolve a parked `$F4-$F7` read once the S-CPU clock passes it) on top of the
scanline+credit+DMA base regressed the gate itself: DKC 40.9101%, Zelda
58.0477%, F-Zero 25.1156%, Super Metroid 75.6453%. The per-access model
remains the best measured parity and was restored; the mutual run cannot be
approximated at request granularity and needs the coroutine that can suspend
the S-SMP inside the read.

A credit-preserving mutual variant (resume through `sync_apu_to` so the owed
master clocks are not duplicated) kept all four exact titles at 100.0000% on
the scanline+port+DMA base, but still measured SMW 47.1934%, CV4 81.4670%,
SMAS 38.6331%, CT 26.7160% and MMX2 99.4424% - worse than the per-access model
for every divergent title. The scanline base is therefore not a better carrier
for the remaining drivers by itself; the per-access model is restored.

**True suspended-read unit (implemented and measured).** The deferred
`$F4-$F7` plan step was split into first half-step (clock + timers), sample,
second half-step and ALU application, with the pending-read application
exposed from `Spc700` and the mid-read state carried in `Apu`; the top-level
scheduler sampled once the S-CPU clock reached the read. Result: F-Zero stayed
100.0000%, but DKC fell to 75.1971%, Zelda to 97.3031% and Super Metroid to
75.4491%. The split moves the sample one S-SMP clock relative to the current
per-access replay, which is exactly the phase those titles depend on. Skipping
the DSP tick on the first half-step (so the old DSP tick distribution is
preserved) gave the identical gate failure (DKC 75.1971%, Zelda 97.3031%,
Metroid 75.4491%, F-Zero exact), so the cause is the port sample/S-CPU run
point, not the DSP tick split. Reverted; the per-access model is restored. The unit itself is now written and can be
revisited together with a matching change to the DSP sample phase.

Landing the suspended read **together with** the measured scanline+port+DMA
base was also measured: Zelda and F-Zero stayed 100.0000%, but DKC fell to
44.0159% and Super Metroid to 78.7613%. Reverted. DKC and Super Metroid are
therefore sensitive to the read's sample phase in a way that neither the
per-access replay nor the ares-faithful scanline+split ordering matches; the
remaining correction is in the DSP/interrupt phase, not the port-read split
alone.

**ares KON latch trace (2026-11-19).** `ARES_DSP_WRITES` now includes
`clock.sample`. Around the SMW first difference:

```
sample 142476 reg 01=06 ticks 4559251 pc 069D smpclk 11376 sample=1
sample 142479 reg 5C=00 ticks 4559333 pc 069D smpclk 11540 sample=0
sample 142480 reg 4C=0B ticks 4559357 pc 069D smpclk 11588 sample=0   <- KON
```

The KON writes `_keylatch` with `clock.sample = 0`, so `misc30` does not
apply it in that 32-step sample; it is consumed after the next `misc29`
toggle, i.e. one sample later. Our KON lands at micro-step 4,559,361
(`mod 32 = 1`) with the opposite `clock.sample` parity, so `misc30` applies it
in the current sample - the one-sample attack shift. The needed correction is
therefore the S-SMP clock at the write (4 DSP ticks earlier, at step 29 with
`sample = 0`), i.e. the full S-SMP/DSP phase, not `misc29`/`misc30` arithmetic.
Per-title offsets differ (SMW +4 ticks, Mega Man X2 +1), so a single global
phase shift cannot fix them; the SMP schedule must match ares per driver.

**Instruction-level run point (ares, 2026-11-20).** The same thread log with
PCs shows the S-SMP frozen at PC `$069E` (the DSP write routine tail) at SMP
cycle 11,590 / CPU master clock 121,392 while the S-CPU runs its
`$00B8E4-$00B992` polling loop all the way to 122,758; only then does the S-SMP
advance to 11,592 and continue at `$069E`. So the S-CPU legitimately runs
~1,366 master clocks ahead in this window. Our scanline model implemented the
sync points but stopped the S-SMP exactly at the CPU clock (credit loop)
instead of allowing ares' whole-instruction catch-up; the whole-instruction
variant in turn broke F-Zero/Metroid. The remaining delta is therefore the
*combination*: scanline-only sync points **and** a whole-instruction S-SMP
catch-up that does not regress F-Zero/Metroid. That combination has now been
measured: exact credit catch-up at ports/DMA plus whole-instruction overshoot
only at the scanline gave DKC 79.5990%, Zelda 92.2410%, F-Zero 25.1196% and
Super Metroid 75.4490% - the gate fails. So the overshoot at the scanline is
wrong for these titles even when the port syncs are exact; the plain
scanline+port+DMA model with exact catch-up (which preserves all four) remains
the closest carrier. The mirror permutation - whole-instruction catch-up at
port/DMA and exact at the scanline - kept DKC and Zelda exact but broke F-Zero
(25.1196%) and Super Metroid (74.2183%). So no assignment of catch-up
granularity to the known sync points satisfies the gate: F-Zero/Metroid reject
port overshoot and DKC/Zelda reject scanline overshoot.

**Exact H=0 sync instants (ares, 2026-11-20).** A local `ARES_SCANLINE_LOG`
hook in `CPU::scanline()` records `(cpu_master, vcounter, smp_cycles, smp_pc,
dspTicks, cpu_pc)` at every Hcounter-0 sync. Around SMW's KON:

```
B clock 95,509,478 v=68 smp 11,590 smppc $1694 dsp 4,559,357 cpupc $47493
B clock 95,510,842 v=69 smp 11,722 smppc $1473 dsp 4,559,423 cpupc $47493
```

The KON at `dspTicks 4,559,357` coincides with the scanline sync, and at that
instant the S-SMP is only ~1-8 master clocks behind the S-CPU (both are caught
up); the S-SMP then advances ~132 SMP cycles (1,382 master clocks) over the
1,364-clock line. So the scanline sync itself is exact and the CPU's ~1,366
clock lead accumulates *between* scanlines, not at the sync. The plain
scanline+port+DMA model with exact catch-up has the right instants; its
remaining error for SMW/CV4/SMAS/CT is therefore in the per-port sync or the
DSP tick phase, not the H=0 instant.

**Per-port synchronize amount (ares, 2026-11-20).** A local
`ARES_PORT_SYNC_LOG` hook in `readAPU`/`writeAPU` records
`(cpu, smp0, smp1, dsmp, dsp0, dsp1, ddsp, pc)` around `synchronize(smp)`.
Over 320 frames the catch-up at a port is small and skews to a few cycles:

```
SMW  dsmp: 4 (98,015), 6 (96,080), 5 (45,926), 18 (28,237), 0 (9,406) ...
     ddsp: 2 (124,299), 3 (116,888), 9 (30,610), 0 (9,573) ...
DKC  dsmp: 6 (167,710), 4 (25,553), 7 (12,090), 5 (11,030) ...
     ddsp: 3 (188,653), 2 (27,023), 5 (12,876) ...
```

So ares lets the S-SMP lag between port accesses and catches it up only when a
port is touched (a few SMP cycles / 1-3 DSP ticks), while this core advances it
on every bus access. Also, ares' catch-up runs whole S-SMP instructions (which
can overshoot by one), whereas the exact credit loop advances exactly the lag.
Both are visible in the per-port deltas and are the remaining difference for
SMW/CV4/SMAS/CT.

Correcting the overshoot bookkeeping (`apu_last_clock` set to the S-SMP's real
clock after a whole-instruction run, not the target) and using that catch-up at
ports, DMA and scanline still gave DKC/Zelda 100.0000% but F-Zero 25.1196% and
Super Metroid 74.2183%. So F-Zero/Metroid require exact per-port catch-up and
reject the overshoot regardless of bookkeeping; the remaining titles' fix
cannot be a catch-up-granularity change and must be in the DSP tick phase.

A further bug in the overshoot helper was found and fixed: `run_until_master`
checked the eagerly recorded S-SMP clock after draining only one plan unit, so
it could return with the instruction half-drained. Draining the whole recorded
instruction before testing the clock gives DKC 73.2926%, Zelda 98.5206%,
F-Zero 100.0000%, Super Metroid 73.1599%. It improves the overshoot model but
still fails the gate, confirming that the exact-catch-up model remains the only
one that preserves all four exact titles.

**Our per-port distribution vs ares (2026-11-20).** Instrumenting this core's
own `$2140-$2143` accesses gives a clearly different schedule. Over 320 frames:

```
ours SMW dsmp: 0 (168,973), 14 (46,450), 8 (43,368), 10 (41,530), 4 (38,032) ...
ours DKC dsmp: 8 (90,222), 0 (83,869), 4 (51,224), 14 (24,092), 10 (21,851) ...
ares SMW dsmp: 4 (98,015), 6 (96,080), 5 (45,926), 18 (28,237), 0 (9,406) ...
ares DKC dsmp: 6 (167,710), 4 (25,553), 7 (12,090), 5 (11,030), 8 (10,943) ...
```

This core advances the S-SMP on every bus access in even chunks and is often
already caught up at a port (`dsmp = 0` for half of SMW's accesses). Ares
advances only at the port and catches up whole instructions, so its deltas are
2-6 (often odd) SMP cycles. The per-access model is therefore not equivalent to
ares' schedule even though it matches four titles; the port-only
whole-instruction model has the right distribution but regresses F-Zero and
Super Metroid, so a remaining piece (DSP tick phase or the exact instruction
boundary) is still missing.

The port-only variant was instrumented too. Its deltas are still **even**
(8, 0, 4, 14, ...) with an 85,491-event `dsmp = 0` spike, not ares' 2-6/odd
distribution. The reason is structural: `Apu::record_instruction` executes the
whole SPC700 instruction eagerly and advances `smp_clock`/`dsp_clock` by its
full cycle count there, so this core's S-SMP clock only ever sits on an
instruction boundary. Ares' clock is sampled mid-instruction (the S-SMP is
suspended at the port read), which is why its deltas are odd and its DSP write
phase is step-exact. The true fix is therefore to move the clock/timer/DSP
advancement out of eager recording and into `drain_unit` (the coroutine), so
`smp_clock` is step-granular.

That move was implemented: `$F3` DSP writes and timer steps were deferred into
`PlanStep`, and `drain_unit` advanced the SMP clock and DSP one access at a
time. It regressed every exact title: DKC 69.0283%, Zelda 58.3454%, F-Zero
25.1047%, Super Metroid 56.9824%. Reverted. So the eager instruction-granular
advancement is *required* for the four exact titles; it is not the cause of
their exactness being accidental - it genuinely matches ares for those drivers.
The remaining five titles' residual is therefore in the CPU/SMP interleaving
(DSP phase at the port catch-up), not in when the S-SMP clock advances.

The write logs match in address/value order; the difference is only in the
number of DSP micro-steps executed at the write (0 or -8 ticks observed). The
DSP is already ticked one micro-step per two S-SMP clocks inside the access's
`wait`, and the official `spc_dsp6`/`spc_smp`/`spc_timer` suites pass, so the
remaining work is to reproduce ares' thread-yield point exactly: `DSP::main`
can run ahead of the S-SMP thread and yield mid-sample through
`Thread::synchronize(smp)`, and a register write is applied at whatever
micro-step that leaves. Candidate fixes are to derive the yield from the ares
scheduler clocks rather than a per-access credit, or to calibrate the DSP
micro-step phase at reset and at each `$F0` divider change.

**Exact ares thread model (2026-11-19, shipped).** `Apu` now keeps both thread
clocks in ares' fixed-point base (`Thread::Second = 2^62-1`) with the integer
scalars `Second / 24,606,720` and `Second / 2,050,560`, starts them at the
scheduler uniqueIDs (`SMP = 1`, `DSP = 2`), and runs a micro-step while
`dsp_clock < smp_clock`. Because `24 * DSP_SCALAR` is four fixed-point units
less than `2 * SMP_SCALAR`, the first synchronization runs one extra tick and
the DSP carries a one-micro-step lead -- derived, not calibrated. The DSP
register-write trace now matches ares' `dspTicks` **exactly (delta 0 on every
write)** for Donkey Kong Country, and DKC's PCM becomes **100.0000% identical
(0 differing samples over 350,000 stereo frames)**.

**S-CPU reset phase (2026-11-19).** The next measurement exposed a second
phase error: `Cpu::reset` modelled ares' reset (`step(132)` plus the
emulation-mode interrupt sequence, H=186) but `Snes::reset` passed it a bus
view that did not own the system clock, and then set `self.clock = 0`
afterwards. The CPU therefore started with `cpu.cycles = 186` while the
system clock, PPU and APU started at 0: the whole machine ran 186 master clocks
out of phase with the first S-CPU instruction, which is exactly the IPL
handshake offset. `Snes::reset` now returns the reset cost and advances the
rest of the machine by it:

```rust
let reset_cost = self.cpu.reset(&mut bus);
drop(bus);
self.advance_peripherals(reset_cost);
```

A unit test pins `cost == 186` and `system.clock == cpu.cycles == 186`
(`reset_costs_the_ares_186_master_clocks`). With the exact DSP thread clocks
and the reset phase both correct, **Donkey Kong Country and Zelda are now
100.0000% bit-exact** over 350,000 stereo frames.

Re-running the whole battery (exact model + reset phase) gives:

```
Donkey Kong Country       0 differing  100.0000%   (was 226,813 / 67.5981%)
Zelda: A Link to the Past 0            100.0000%   (was 1,175 / 99.8321%)
Mega Man X2            3,912            99.4411%   (was 915 / 99.8693%)
Super Mario World     73,874            89.4466%   (was 73,895 / 89.4436%)
Castlevania IV        70,913            89.8696%   (was 76,746 / 89.0363%)
Super Metroid        199,925            71.4393%   (was 248,565 / 64.4907%)
Chrono Trigger         * first difference 2.36 s (unchanged)
Super Mario All-Stars  * first difference 3.51 s
F-Zero                 * first difference 2.87 s
```

(`*` whole-window counts for the still-divergent titles are alignment-sensitive
once the stream has drifted; the first-difference time is the stable metric.)

**S-CPU port-read forward synchronization (2026-11-19).** ares
`SMP::readIO($F4-$F7)` calls `synchronize(cpu)` before sampling the latch, so
the read can run the S-CPU forward to the S-SMP clock. This is now implemented
as a parked read:

- `Apu::set_master_clock` records the S-CPU's master clock for the access;
- `drain_unit` parks a deferred `$F4-$F7` read while the S-SMP clock is ahead
  (`port_read_waiting`) and consumes no APU time;
- the system's APU credit loop breaks on a parked tick (credit keeps
  accumulating);
- after the current S-CPU instruction, `Snes::step` executes further S-CPU
  instructions until the CPU catches the frozen S-SMP clock (bounded to 64
  instructions), then the read resolves and the S-SMP drains the accumulated
  credit in a burst.

The clocked S-SMP trace confirms the effect: the first IPL divergence moved
from instruction **4,298** (`$FFD8`, the byte-upload poll) to **72,244**
(`$FFE9`, a later `bpl` after `inc $01`). DKC and Zelda remain 100.0000%
bit-exact, and the rest of the audio sweep is unchanged, so the remaining
retail divergence is not the initial port race but a later driver poll/state
difference. The full per-title numbers and the residual `$FFE9` trace are the
next audio item; the DSP clocks, reset phase and port-read sync now all match
ares' model.

Until then, retail audio is **not** claimed bit-exact; the per-title numbers
above are the measured baseline, and dense music (F-Zero, Super Metroid, Donkey
Kong Country) shows the largest deviation. None of this affects the video
parity, save states or the test-ROM baseline.

## Resolved 2026-11-19 - power-on APU RAM and DSP registers are ares' pattern

ares does not zero the APU at power-on. `System::power` seeds one deterministic
`Random` stream, `CPU::power` fills the 128 KiB work RAM from it, and then
`DSP::power` fills **64 KiB of APU RAM** and the **128-byte S-DSP register
file** from the same stream:

```cpp
if(!reset) {
  random.array({(u8*)apuram,    sizeof(apuram)});     // 65536 bytes
  random.array({(u8*)registers, sizeof(registers)});  // 128 bytes
}
```

This core left both zeroed. The fill is now generated by
`random::power_on_apu()`: it advances the same seed-`(0, 0)` PCG32 through the
work-RAM slice first, then APU RAM, then the register file, exactly as ares
does. Two unit tests pin the determinism and the stream position (the APU slice
must start where the 128 KiB work-RAM fill ends, not at the start of the
stream).

Verified byte-for-byte against ares itself: local `ARES_APURAM_DUMP` /
`ARES_DSPREG_DUMP` hooks in the reference `DSP::power` produce a 65,536-byte
APU image and a 128-byte register image that compare equal to the Rust
generator. `Dsp::reset` now also keeps the register file, matching ares
`DSP::power(reset = true)`, which clears the decoded voice/echo/noise state but
does not refill or zero `registers[]`.

**Why it matters.** F-Zero's sound driver reads `$03A9` (`lda $03A9; beq`) in
its IRQ handler before initialising it. With a zeroed APU image the S-SMP took
that branch and skipped the three-instruction handler at `$1366-$136B`
(`mov a,#0; mov $03A9,a; and $E0,#$1C`); ares, with its power-on pattern, falls
through. The skipped handler shifts the driver's subsequent DSP register writes
by 10 micro-steps (the first observed write moves from DSP micro-step
1,606,442 to 1,606,432), which changes the note envelope and the whole stream
after it.

Audio sweep after the fill (700,000 interleaved samples / 350,000 stereo
frames, loud-probe alignment):

```
Donkey Kong Country       0 differing  100.0000%
Zelda: A Link to the Past 0            100.0000%
F-Zero                    0            100.0000%   (was 524,150 / 25.12%)
Super Metroid             0            100.0000%   (was 199,925 / 71.44%)
Super Mario All-Stars 313,782             55.1740%  (was 345,375 / 50.66%)
Super Mario World      73,874             89.4466%  (unchanged)
Castlevania IV         70,913             89.8696%  (unchanged)
Chrono Trigger        416,332             40.5240%  (first diff 2.357 s, unchanged)
Mega Man X2             3,903             99.4424%  (CX4 firmware; first diff 7.313 s)
```

Mega Man X2 is a CX4 title, and the harness only installs the coprocessor when
`CX4_ROM` points at the local 3 KiB `cx4.rom` dump (`Snes::set_cx4_firmware`).
Without it the `$6C00-$6FFF`/`$7C00-$7FFF` window is unmapped: the game's CX4
self-test at `$13F8BC` compares `$7F49-$7F6F` against its expected reset values
(cache base `$028000`, wait `$44`, IRQ `1`, ROM `0`, vectors), sees a mismatch,
sets the handler flag `$1F53`, and takes the `bne` at `$13FA75` instead of
starting the IPL sound upload - the output is silence. With `CX4_ROM` set the
whole path matches ares and the stream is non-silent.

A rerun of the sweep that omitted `CX4_ROM` produced 256,334 differing samples
for Mega Man X2; that was a harness omission, not a regression from the
power-on APU change. The 3,903-sample residual has the same signature as the
other DSP-placement residuals: the first difference is at 7.313 s and is a few
LSBs on individual samples (253 vs 263), not a missing note.

The SPC trace pins the mechanism. Just after the CX4 self-test the IPL upload
handshake first differs by **one extra spin iteration** at `$FFCF/$FFD2`: the
S-CPU's `sta $2140` of `$CC` commits five master clocks *after* our S-SMP's
`cmp $F4,#$CC` samples the latch, while ares' CPU has already committed it. The
S-SMP then enters `$FFEF` two instructions later, which shifts every subsequent
DSP register write by one micro-step and flips the KON/KOFF sample boundary at
7.313 s. This is a relative S-CPU/S-SMP scheduling offset, not a lost write.

Resuming a parked `$F4-$F7` read immediately after a `$2140-$2143` write was
implemented and measured: Zelda and F-Zero stayed exact, but DKC fell to
75.1971%, Super Metroid to 74.3417% and Super Mario All-Stars to 52.3634%, so it
was reverted. The only faithful fix is the mutual S-CPU/S-SMP coroutine
scheduler (the S-SMP yielding inside `SMP::readIO($F4-$F7)` at
`synchronize(cpu)`), which is still unimplemented. *Implemented and verified in
the 2026-11-20 section below.*

## Sharp S-RTC and Sufami Turbo (2026-11-19)

Both were the last missing non-hard coprocessors in the board set.

**Sharp S-RTC** (`sharprtc.rs`). Selected by cartridge-type high nibble `$5`
(the `EXHIROM-RAM-SHARPRTC` boards, e.g. type `$55`) and mapped at
`$00-$3F/$80-$BF:$2800-$2801`. The protocol is a direct behavioural port of the
ISC-licensed ares `sfc/coprocessor/sharprtc` source:

- odd writes: `$0D` enters read mode, `$0E` enters command mode, `$0F` is
  ignored; in command mode `$00` starts a 12-nibble write and `$04` resets the
  calendar;
- even reads after `$0D` return the `$0F` preamble, then the 13 BCD nibbles
  (second, minute, hour, day, month, year low/mid/high, weekday) and the `$0F`
  epilogue;
- after a write the weekday is recalculated with ares' Sharp-epoch
  (1000-01-01) algorithm; like ares, ticks do not update the weekday register.

The core has no host clock, so the calendar defaults to 2000-01-01 00:00:00 UTC
and the host can set it with `Snes::set_sharp_rtc_time(unix)`. It advances once
per emulated second on a master-clock credit. Unit tests cover the default
calendar, Unix conversion (including the weekday), the read/write command
streams, the `$04` reset, minute/hour/day rollover and February 29 in a leap
year; a system test drives the nibble protocol through the S-CPU bus and checks
the one-second tick. No local S-RTC title is available, so the oracle sweep is
a coverage request.

**Sufami Turbo** (`sufamiturbo.rs`). The base cartridge is an ordinary LoROM
image whose first 14 bytes are `BANDAI SFC-ADX`; the core detects it with
`Cartridge::is_sufami_turbo` and then exposes two host-attached slots through
`Snes::set_sufami_turbo_slot(slot, rom, ram)`. Address routing mirrors ares'
`ST-LOROM` board:

| Window | Contents |
|---|---|
| `$00-$1F/$80-$9F:$8000-$FFFF` | base cartridge ROM (ordinary LoROM map) |
| `$20-$3F/$A0-$BF:$8000-$FFFF` | slot A ROM, 32 KiB-folded (`mask=0x8000`) |
| `$40-$5F/$C0-$DF:$0000-$FFFF` | slot B ROM, linear 2 MiB window |
| `$60-$6F/$E0-$EF:$0000-$FFFF` | slot A RAM |
| `$70-$7D/$F0-$FF:$0000-$FFFF` | slot B RAM |

Slot ROM/RAM accesses wrap by the attached size (ares `mirror`), ROM writes are
ignored (mask ROM), and the slot RAM is part of the save state. A state restore
keeps the host-attached ROMs and restores only the RAM, so a fresh machine can
attach the same media and resume. Unit tests cover the window table, the
32 KiB folding, wrapping, open-bus behaviour and RAM restore; a system test
drives all five windows through the bus and round-trips a save state.

Neither device has a local reference ROM, so both are verified by the protocol
unit/system suites rather than an oracle pixel/audio sweep. They do not affect
any existing title (they only activate on their board types or attached media).

## Residual driver phase after the port-read sync (2026-11-19)

The parked `$F4-$F7` read moved the initial IPL divergence 17x later but did
not change the retail PCM metrics, so the remaining differences were traced
past the upload loop.

**F-Zero** (first audio difference at stereo sample 91,947, 2.87 s) was
re-traced with the all-voice DSP trace on both cores:

```
first state difference       row 84,786
  all voices                 _keyoff: ours 0, ares 1
first echo-input difference  row 86,374
first echo-output difference row 89,380
first audio difference       row 91,947
```

Every voice's envelope, pitch, BRR address/offset, Gaussian offset and 12-word
BRR buffer match; only the KOFF latch appears one sample earlier in ares. That
KOFF reaches the echo send path and finally the echo input/output, so the
audible difference is a one-sample driver write placement, not an echo or voice
arithmetic bug.

**Super Mario All-Stars** (first audio difference 3.51 s) is the same class: the
clocked upload trace's remaining divergence is IPL `$FFE9` (`bpl` after
`inc $01`, the upload destination page count), and the first audio difference
is a voice-4 attack already two envelope steps ahead in ares.

So the remaining retail gap is the *runtime* sound-driver handshake: the IPL
upload gets one instruction further but the driver's later CPU/S-SMP poll still
shifts KON/KOFF by one sample. The forward sync currently resolves a parked
`$F4-$F7` read at CPU-instruction granularity; ares' `synchronize(cpu)` can
resolve it at the exact thread-clock point (the CPU thread yields mid-instruction
in ares, while this core runs the S-CPU as a non-resumable call stack). That
resumable-CPU refinement is the next audio item; DKC and Zelda remain bit-exact
and no other title moved.

## Required scheduler port: resumable S-CPU (2026-11-19)

The remaining retail PCM gap is a one-sample KON/KOFF shift in the sound
driver, not DSP arithmetic. The forward-sync machinery parks a `$F4-$F7` read
until the S-CPU catches the frozen S-SMP clock, but it can only resume the read
after the **current S-CPU instruction** returns: `Snes::step` does
`let mut cpu = std::mem::take(&mut self.cpu); cpu.step(self);` and the 65C816
interpreter is a recursive call stack, so a bus access cannot resume the CPU
mid-instruction. ares can: its S-CPU is a cooperative thread that yields inside
an APU access and resumes there when `SMP::readIO` calls `synchronize(cpu)`.
The granularity difference is what leaves SMAS voice 4 two envelope steps
behind, F-Zero's KOFF one sample late, and the Chrono Trigger/Super Metroid
streams shifted.

The exact fix is one of:

1. **Resumable 65C816 interpreter.** Make `Cpu::step` a state machine that
   executes one bus access per activation (or an `async` interpreter awaited by
   the bus), store the continuation in `Cpu`, and let the parked port read drive
   the S-CPU access-by-access up to the frozen S-SMP clock. This touches every
   opcode handler and the CPU oracle tests; it is a multi-file, multi-turn port
   and must not be half-landed (the CPU is the timing anchor for the whole
   machine).
2. **ares scheduler port.** Replace the CPU/PPU/SMP cooperative credits with
   the `Scheduler`/`Thread` model (scaled thread clocks, `synchronize`),
   including a cothread or equivalent resumable execution for the CPU.

Until that lands, the shipped state keeps the exact DSP clocks, the 186-clock
reset phase, the `$F4-$F7` parked-read sync and the bit-exact DKC/Zelda
streams; the other titles' first differences are the driver KON/KOFF placement
documented above. A per-title phase calibration (like the SMW HDMA overlay) is
possible but would repeat that overlay's maintenance cost and is not applied.

## Resumable S-CPU execution (2026-11-19)

The S-CPU now executes inside a stackful coroutine
(`crates/unko-snes/src/cpu_coroutine.rs`, `corosensei` 0.3, MIT OR
Apache-2.0). Every `CpuBus` operation is yielded to `Snes::step`, which performs
it against the real machine and resumes the interpreter with the result, so the
65C816 can be suspended and resumed at **bus-access granularity** without
rewriting the opcode handlers. The coroutine is parked at `Done` between
instructions and owns no CPU state while suspended; cloning/equality ignore it
and save states remain unchanged.

With the resumable CPU in place, the ares `synchronize(smp)` scheduling was
tried: the S-SMP advanced only at `$2140-$2143` port accesses with burst
catch-up. Result: **Zelda stayed bit-exact but DKC regressed to 17.67%** and
SMAS/F-Zero were unchanged, so the experiment was reverted; the per-access APU
advancement (which keeps DKC/Zelda exact) is retained. The resumable CPU is
kept as verified groundwork: 196 lib tests, SMW 600/600, baseline 212/212 and
DKC/Zelda 0-diff are unchanged by it.

The remaining mismatch is therefore not merely "make the CPU resumable": the
S-SMP advancement schedule must be derived from ares' `Thread`/`Scheduler`
clock ordering (including when the scheduler runs the aux threads), which is
the next scheduler task.

## Mutual S-CPU / S-SMP actor (2026-11-20)

The S-SMP now runs inside its own stackful coroutine
(`crates/unko-snes/src/apu.rs`, `new_apu_coroutine`), peer to the S-CPU
coroutine. The actor is not a second timing model: it advances the S-SMP with
the same rational master-clock credit as the per-access path
(`Apu::advance_master_clock`), one S-SMP clock per credit unit, and is
synchronized at the **start of every S-CPU bus access** with that access's
clocks as credit. A DRAM-refresh stall advances it a second time at the
post-access clock, mirroring ares' two `Thread::synchronize` calls.

A `$F4-$F7` read whose sample clock is ahead of the S-CPU parks the S-SMP and
hands the `Apu` to the scheduler (`ApuOutput::Parked { apu, target }`). While
parked the scheduler owns the `Apu`, so `$2140-$2143` reads and writes are
ordinary bus accesses on the latches (no port-command protocol), the S-CPU runs
forward until `clock >= target`, and the next advance resolves the read at the
same sample clock - the ares `SMP::readIO` sequence.

**What was wrong before.** The first actor executed whole `spc.step`
instructions per resume and caught up by comparing `smp_master_clock(rate)`
against the CPU clock. That always overshoots to an instruction boundary, and
it reproduced the scanline-style phase exactly (DKC 47.3223%, Zelda 58.8231%,
F-Zero 25.1250%, Super Metroid 57.4057%). It also dropped `$F0`/`$F1` writes,
which stalled DKC's driver at `$064D` polling the timer-0 output. The
per-clock-resume variant (one coroutine resume per S-SMP clock) produced the
right phase but was unusably slow (300 frames did not finish in 15 minutes).

**Measured result** (700,000 interleaved samples / 350,000 stereo frames):

```
Donkey Kong Country       0 differing  100.0000%
Zelda: A Link to the Past 0            100.0000%
F-Zero                    0            100.0000%
Super Metroid             0            100.0000%
Super Mario World      73,874             89.4466%
Castlevania IV         70,913             89.8696%
Super Mario All-Stars 313,782             55.1740%
Chrono Trigger        444,363             36.5196%  (offset -4)
```

The four gate titles are 0-diff, and the four measured differing titles are
**bit-identical to the per-access tree** (same differing counts and the same
best offset), so the actor preserves the verified schedule exactly. The
212-entry test-ROM baseline is identical in both frame and audio hashes. The
remaining SMW/CV4/SMAS/CT/MMX2 residuals are the DSP register-write micro-step
placement already documented above, not S-SMP clock granularity.

`Apu::apu_credit` is part of the serialized `Apu`, so a state saved while the
S-SMP is mid-instruction (`plan_pos > 0`) resumes exactly; `SAVE_STATE_VERSION`
moved 17 -> 18 and `restore` drops the transient coroutine. 204 lib tests,
including actor parking, parked port writes and a mid-instruction
serialize/resume round trip.

## S-SMP synchronization instant (2026-11-20)

The remaining SMW/CV4/SMAS/CT/MMX2 residual was re-examined by diffing ares'
`ARES_DSP_WRITES` log against `DSP_WRITES_OUT` for Super Mario World (300
frames, 1,025 writes). The register/value sequence is **identical one-for-one**;
only the `dspTicks` phase drifts. The first divergence is write 14 (`$6C = $20`
at 1.122 s, ours +12 ticks), settling to -4 by the note setup at 1.447 s - the
first audible difference. The cause is not `$F2`/`$F3` latching: the DSP tick
at a write is a function of the S-SMP clock, and the S-SMP clock differs
because the driver's `$F4` poll loop iterates a different number of times.

Logging every S-SMP `$F4-$F7` read (`SMPIO_LOG`, removed before commit) and
comparing with ares' `ARES_SMPIO_LOG` isolated it: over 90 frames the old model
performed **121,705** `$F4` reads where ares performs **81,190**. The park
condition used the S-CPU clock at the *start* of the stepped access, so the
S-SMP was considered ahead on nearly every poll and ran the S-CPU forward
mid-access. ares compares the thread clocks at the read/write instant
(`CPU::read` steps `wait - 4` before `bus.read`, `CPU::write` steps the whole
wait before `bus.write`), and `Thread::synchronize(cpu)` resumes the S-CPU in
whole instruction steps.

Three fixes, all derived from the ares source:

1. `Apu::advance_master_clock` records the S-CPU clock at the end of the
   stepped window (`clock + clocks`), matching the clock `readIO`/`writeIO`
   sees.
2. `port_read_waiting` compares the fixed-point thread clocks directly, and the
   park target rounds the S-SMP clock up, so the S-CPU cannot stop one clock
   short.
3. A parked read is flushed only at an S-CPU instruction boundary; clocks
   accumulated while parked are held in `ApuPeer::parked_clocks`.

Result: the S-SMP `$F4` read count matches ares exactly (81,191 vs 81,190 over
90 frames). The `$F4` value sequence now matches ares through read 81,062 of
81,190; the single remaining divergence is at 1.122 s (ours reads `$1f` where
ares reads `$23`), i.e. one CPU instruction's write order at the parked sample,
and it is what still holds the note phase. The four gate titles remain 0
differing, the SMW/CV4/SMAS/CT sweep is unchanged (73,874 / 70,913 / 313,782 /
444,363), the 212-entry baseline is identical in frame and audio hashes, and the
crate is at 204 tests with fmt/clippy clean.

Next: the 1.122 s race is a single-instruction ordering at the park - ours
resolves the sample after the instruction that reaches the S-SMP clock, ares
after the instruction that *crosses* it. The remaining experiment is to let the
forward loop run to the first instruction whose end clock is strictly greater
than the sample clock (ares' `synchronize` loop condition is `cpu.clock() <
smp.clock()` re-checked after each whole step), then re-run the sweep.

## S-SMP scheduler granularity experiments (2026-11-20, negative results)

After the synchronization-instant fix the remaining SMW difference is a single
port-value race: at `$F4` read 81,063 of 81,190 (1.122 s) ours reads `$1f`
where ares reads `$23`, and ares' S-CPU clock at that read is ~78 master clocks
ahead of ours. Four variants were measured against `ARES_SMPIO_LOG`
(90 frames, port-0 read count; ares = 81,190):

| variant | `$F4` reads | value race |
|---|---|---|
| committed (per-access flush, post-step clock) | 81,191 | read 81,063 |
| flush at S-CPU instruction boundaries (+ port accesses) | 81,191 | unchanged |
| one-instruction lead at the park (`crossed` flag) | 24,106 | over-corrected |
| strict-greater tie (`flush` only when `clock > target`) | 81,191 | unchanged |

All variants kept the four gate titles at 0 differing and left the sweep
unchanged, so none was kept.

**What the numbers imply.** ares' scheduler runs each thread one *step* at a
time (`Thread::Enter` calls `scheduler.synchronize()` then `entryPoint()`), so
the S-SMP advances in whole SPC700 instructions and stops at the first
instruction end at or past the S-CPU clock. The CPU therefore leads by up to
one S-SMP instruction (~84 master clocks), which is the ~65-80 clock delta
measured at the reads. Our `Apu::advance_master_clock` drains the plan
*mid-instruction* while the credit covers another S-SMP clock, so our S-SMP
stops exactly on the S-CPU clock instead of at the instruction end: it is
systematically a few clocks *behind* ares at arbitrary instants (the -4 tick
note lag at 1.447 s) and occasionally sees a port write one poll late.

**Whole-instruction scheduler step (measured).** Advancing the S-SMP to the
end of the current SPC700 instruction per call (`while smp_pos < cpu_fixed {
drain }`) keeps DKC at 0 but moves SMW from 91.5118% to **90.1613%** on the
639,656-sample window, so the overshoot is also wrong for SMW and was
reverted. The mid-instruction exact stop remains the best measured model; the
1.122 s port race is still open.

## ares cooperative-scheduler model (2026-11-20, rejected)

Implemented the scheduler from the ares source directly:

- `Thread::synchronize` semantics: the S-SMP runs **whole SPC700 instructions**
  until `smp.clock() >= cpu.clock()` (overshoot carried).
- Synchronization points only at `$2140-$2143` (`CPU::readAPU`/`writeAPU`), at
  Hcounter 0 (`CPU::scanline`), and at DMA-to-APU bytes
  (`DmaTransfer::writes_apu_port`, added for the experiment).
- `$F4-$F7` reads park and run the S-CPU to the sample clock.

Result (600 frames, 639,656-sample window):

```
dkc            0 differing  319,828 frames  100.0000%
zelda     69,787 differing  89.0898%
fzero    446,989 differing  30.1204%
metroid  142,854 differing  77.6668%
smw       62,934 differing  90.1613%
```

Changing the scanline sync instant from the access end to the exact Hcounter-0
wrap clock, and adding the DMA-to-APU synchronization, produced **byte-identical
numbers** - the port synchronizations already leave the S-SMP ahead, so the line
and DMA syncs are no-ops in this model. The gate fails decisively (three of the
four exact titles regress), so the experiment was reverted.

This reproduces the earlier scanline-model result independently and closes the
"scheduler granularity" line: the per-access exact-credit model (DKC/Zelda/
F-Zero/Super Metroid 0 differing; SMW 91.5118% on the same window) remains the
best measured model, even though ares' source-level synchronization points are
coarser. The remaining SMW race at 1.122 s is a sub-instruction port ordering,
not a scheduler-granularity effect.

## Port-latch visibility instrumentation (2026-11-20)

Instrumented both sides at access granularity for Super Mario World, 90 frames:
the S-CPU's `$2140-$2143` writes (`CW <master clock> <port> <value> <pc>`) and
the S-SMP's `$F4-$F7` reads, against ares' `ARES_APU_LOG` and
`ARES_PORT_SYNC_LOG`.

**The S-CPU writes match ares exactly.** Around the 1.122 s race:

```
ares  W1 00 cpu=24,091,518 pc=0080cd      ours  CW 24,091,518 1 00 0080CA
ares  W0 23 cpu=24,091,592 pc=0080d3      ours  CW 24,091,592 0 23 0080D0
```

The PC differs by 3 only because ours records the instruction start. The write
instants are identical to the master clock, so the S-CPU side is exact.

The difference is the S-SMP poll read. ares' reads bracket the write:

```
ares  $F4 read #81,062  cpu=24,091,412  apu=1f
ares  W0 23              cpu=24,091,592
ares  $F4 read #81,063  cpu=24,091,670  apu=23
```

Ours resolves a poll in the advance of the write access itself, before the
`$2140` write commits:

```
ours  $F4 read #81,062  sync clock=24,091,336  apu=1f
ours  W0 23             clock=24,091,592
ours  $F4 read #81,063  sync clock=24,091,592  apu=1f   <- one poll early
ours  $F4 read #81,064  ...                    apu=23
```

**One-access lag experiment.** Draining the S-SMP only to the *start* clock of
each access (so it lags the S-CPU by one access, and the poll lands after the
write) gives:

```
dkc  0 differing  100.0000%
zelda 47,415      92.5873%
fzero 384,470     39.8939%
metroid 0 differing 100.0000%
smw  54,295       91.5118%   (unchanged)
```

So the lag keeps DKC and Super Metroid exact but regresses Zelda and F-Zero, and
does not move Super Mario World at all. Reverted. Combined with the scheduler
experiments, the latch-visibility phase cannot be shifted globally: each of the
four exact titles pins a different aspect of the per-access schedule, and the
SMW poll race is a sub-access ordering that the current model cannot express
without changing the others.

The per-access model remains the best measured: gate 0 differing, SMW 91.5118%
(639,656 samples) / 89.4466% (700,000).

## ares co-switch order at the 1.122 s port race (2026-11-20)

The shared scheduler cannot reference the SNES clock (each core is a separate
namespace), so the switch trace was taken at `CPU::main`/`SMP::main` entry with
a shared global sequence counter (local ares hooks, not committed). Merging the
two traces by sequence shows the exact order around the race:

```
C 0080ca @ 24,091,488
S 131E   @ 24,091,518        (BPL of the poll loop)
C 0080cd @ 24,091,518        ($2141 = 00)
C 0080cf @ 24,091,534
C 0080d0 @ 24,091,562
S 130A   @ 24,091,592        ($F4 poll: CMP Y,$F4)
C 0080d3 @ 24,091,592        ($2140 = 23)
C 0080d6 @ 24,091,622
C 0080d3 @ 24,091,644
S 130D   @ 24,091,670
```

At the same oracle clock the S-SMP instruction runs **first**; inside its
`$F4` read, `synchronize(cpu)` resumes the suspended S-CPU and completes the
in-flight `$2140` write, so the sample reads `$23`. The S-SMP then continues
four instructions later than in our model.

**Deferred-slice experiment.** Implementing that as "latch the `$2140` write
before the pending slice's S-SMP advance" (one-request deferral of every S-SMP
advance, with `$2140` written before the flush and `$2141-$2143` after):

```
dkc     173,620   72.8573%
zelda     1,449   99.7735%
fzero         0  100.0000%
metroid 136,552   78.6523%
smw      45,880   92.8274%   (improved from 91.5118%)
```

Narrowing the rule to `$2140` only produced byte-identical numbers. SMW moves in
the right direction (the poll now samples the in-flight write), but DKC and
Super Metroid regress because their handshakes depend on the old value being
sampled; the unconditional deferral also shifts every S-SMP advance by one bus
slice. Reverted.

**Next experiment indicated by the trace.** The in-flight write is only
completed inside the read when `synchronize(cpu)` actually runs the S-CPU
(`cpu.clock < smp.clock`). Our park already models that case; the remaining
work is to apply the in-flight completion *only* when the pending slice's drain
reaches a `$F4-$F7` sample while parked, leaving non-parked slices at the
current per-access ordering. That requires the drain to call back into the
driver at the sample (a per-sample "deliver the pending write" hook), which the
actor can support by yielding `Parked` before the write and resuming after it.

## Absolute S-SMP I/O reads (2026-11-20) - Super Mario World exact

The 1.122 s port race is closed. The SPC700 deferred I/O reads only for
*direct-page* operands (`effective & 0xFFF0 == 0x00F0` with P=0). Super Mario
World's sound driver polls with `CMP Y,$00F4` (`0x5E`, absolute), which took the
eager `absolute_read` path: the latch was sampled while the instruction was
being recorded, before the access clock and before the S-CPU's in-flight
`$2140` write, so the poll observed the previous byte and every subsequent DSP
write ran one micro-step late.

`Pending::AbsoluteRead` extends the same deferral to absolute `$F0-$FF` reads.
The sample is then resolved by `drain_unit` at its own access clock, where the
park/synchronize logic delivers the pending `$2140-$2143` write exactly as
ares' `readIO` does.

Result (1,500 frames, 700,000 interleaved samples / 350,000 stereo frames):

```
Donkey Kong Country       0 differing  100.0000%
Zelda: A Link to the Past 0            100.0000%
F-Zero                    0            100.0000%
Super Metroid             0            100.0000%
Super Mario World         0            100.0000%   (was 73,874 / 89.4466%)
Super Castlevania IV  70,913             89.8696%  (unchanged)
Super Mario All-Stars 325,801            53.4570%  (was 313,782)
Chrono Trigger        444,363            36.5196%  (unchanged)
```

204 lib tests, fmt/clippy clean, 212/212 test-ROM baseline identical.

CV4's first DSP write is already 22 micro-steps late at the first register write
(3,557,684 vs ares 3,557,662) and its write count over 300 frames is 1,429 vs
ares 2,313, so its driver's boot handshake still diverges; that is the next
lead. SMAS moved slightly worse (its stream now starts from the same corrected
sample point but still differs), and CT is unchanged.

## Castlevania IV IPL handshake (2026-11-20)

CV4's DSP write sequence is now identical one-for-one with ares (2,313 writes
over 300 frames, same registers/values), but the tick phase splits into two
regimes: the first write is +22 micro-steps late (3,557,684 vs 3,557,662) and
1,788 later writes are -6. The first stream divergence is inside the IPL
upload, not in the driver: at `$FFDA` (`CMP Y,$F4`, direct-page, deferred) ares
falls through to `$FFDE` while ours takes the `$FFE9` branch, i.e. the S-SMP
sampled the previous latch.

Port logs around the first divergence (clock, write):

```
ares  W1 00 28,991,350   W0 00 28,991,500   W1 00 28,991,506   W1 ff 28,991,912
ours  W1 00 28,991,350   W0 00 28,991,500   W1 00 28,991,506   W1 ff 28,992,172
ours  $F4 sample @28,991,500 = FF   (the CPU's $2140=00 lands at the same clock)
ours  $F4 sample @28,991,734 = 00   (echo reaches the S-CPU ~30 clocks later)
```

The CPU's poll of `$2140` therefore runs one extra iteration (~260 clocks),
delaying every subsequent byte. Two variants were measured and are neutral:
synchronizing the S-SMP to the S-CPU clock before `$2140-$2143` reads (ares
`readAPU`) and parking when the sample is within 12 master clocks of the CPU
clock. Gate and SMW stay exact; CV4 stays 70,913 / 89.8696% over 700,000
samples.

The remaining difference is the same-clock ordering at the deferral boundary: a
direct-page `$F4` sample resolving a few clocks *before* the access that
commits the CPU's write. The next experiment is to resolve the deferred sample
only after the S-CPU delivers the write for the access that contains the sample
(the write arrives as the next bus request), i.e. treat the sample as pending
until the access's `$2140-$2143` write is delivered, without shifting the
non-I/O slices.

**Hold-across-the-boundary experiment (2026-11-20).** The deferred sample at
CV4's `$FFDA` resolves at master clock 28,991,478 while the S-CPU's `$2140=00`
write commits at 28,991,500 (22 clocks later), so the sample sees `FF` and the
poll runs once more. A one-shot hold was implemented: when a deferred `$F4-$F7`
sample is within 24 clocks of the S-CPU clock, `drain_unit` stops before the
sample, the actor yields `Parked`, the driver delivers the next bus request (the
write) while holding the `Apu`, and the flush then resolves the sample with the
new latch.

The five exact titles stayed 0 differing, but CV4 was unchanged
(11,269 / 639,656) and the hold's `HLD` log never fired: at the pre-sample
check `port_read_near()` sees no pending direct read, while `resume_deferred_read`
one unit later does. The pending-read state used by `drain_unit`'s park check is
therefore not visible at the position where the hold was inserted.

Next: place the hold at the sample-taking site itself (inside the unit that
would call `resume_deferred_read`) rather than one unit earlier, and verify the
`HLD` log fires before measuring. The five-title gate remains the acceptance
condition.

**Sample-hold with both pending variants (2026-11-20).** The earlier hold never
fired because `port_read_near` only matched `DirectRead`; `port_read_waiting`
handles both variants, so the hold is now applied to `AbsoluteRead` too.

At CV4's `$FFDA` loop the pre-sample check shows:

```
pos=28,990,085 cpu=28,990,120 near=false wait=false
pending=Some(DirectRead { opcode: 0x7E, address: 0xF4 })
```

The gap at the check is ~35 master clocks (not the 22 seen at resolution), so a
24-clock window never fires. With a 48-clock window the hold fires 129,396
times and:

```
dkc     186,952  70.7730%
zelda    29,913  95.3236%
fzero         0 100.0000%
metroid 168,052  73.7278%
smw     110,220  82.7689%
cv4       6,431  98.9946%   (improved from 11,269)
```

CV4 moves substantially (11,269 -> 6,431) but the five exact titles break: the
hold is unconditional for every sample within the window, while the four exact
drivers require the opposite ordering (their sample must not observe the
in-flight write). The clock alone does not distinguish them, so the variant was
reverted.

The remaining discriminator has to come from the S-CPU side: whether the access
that ends at the sample's clock actually writes `$2140-$2143`. The driver knows
this only when the write request arrives; a correct implementation needs the
sample held until then *conditionally*, i.e. the actor yields at the sample, the
driver runs the S-CPU's next bus request, and only if it is a `$2140-$2143`
write is the latch applied before the sample resumes; otherwise the sample
resolves with the old ordering.

## Conditional sample hold (2026-11-20)

Implemented the per-request hold: at a deferred `$F4-$F7` sample the actor yields
`ApuOutput::Sample` (handing the `Apu` back); the driver runs exactly the
S-CPU's next bus request and applies the latch only if it is a `$2140-$2143`
write, then resumes the sample with `ApuInput::ResolveSample` (one unit,
`Apu::resolve_held_sample`). Any other request resolves the sample with the old
ordering. `DrainStop::{Done, Parked, Held}` distinguishes the three exits.

Result (600 frames, 639,656-sample window):

```
dkc     173,620  72.8573%
zelda     1,449  99.7735%
fzero         0 100.0000%
metroid 136,553  78.6521%
smw      45,880  92.8274%
cv4           0 100.0000%   <- CV4 becomes bit-exact
```

CV4 is now exact, confirming the ordering rule, but the four gate titles regress
with numbers identical to the unconditional deferral. The condition "the next
request is a `$2140-$2143` write" is therefore necessary for CV4 but not
sufficient to discriminate: DKC/Zelda/Super Metroid/SMW require the sample to
*not* observe the in-flight write even when the next request is a port write.

The remaining discriminator is ares' `synchronize(cpu)` condition
(`cpu.clock() < smp.clock()` on the scheduler's reduced clocks): the S-CPU is
run inside the read only when it is behind the S-SMP at that instant. Our
driver keeps the two level and compares absolute clocks, which cannot separate
the two cases. Reproducing the reduced-clock comparison (or the scheduler's
per-thread step order) is the next step; the gate and CV4-exact result bracket
it: any rule must hold the sample for CV4's `$FFDA` polls and not for the four
exact drivers' polls.

## ares never runs the S-CPU inside an SMP $F4 read (2026-11-20)

Instrumented `SMP::readIO($F4)` in the local ares build to log the S-CPU thread
clock before/after its `synchronize(cpu)`, for 60-frame runs:

```
dkc  reads 64,048   dcpu > 0 on 0 reads   max 0
cv4  reads 94,573   dcpu > 0 on 0 reads   max 0
```

The S-CPU is **never** advanced inside a `$F4-$F7` read for either title, so the
"in-flight write is completed inside the read" model is wrong. The conditional
sample hold that made CV4 bit-exact was correcting the *phase* (delaying the
sample past the CPU's write), not reproducing ares' ordering - which is why it
also broke the four exact titles. That leaves the tree as recorded.

The hold's clock window cannot discriminate either: the gap between the deferred
sample position and the S-CPU clock at the deferral check has the same
distribution for DKC, SMW and CV4 (min 32, median 34, p90 36, max 72).

CV4's actual discrepancy is therefore a phase: our S-SMP samples ~22 clocks
before the S-CPU's `$2140` write (28,990,085 vs 28,990,120 in the `$FFDA` poll),
while ares' sample falls after it. That phase enters during the IPL handshake -
the first branch divergence is `$FFDC` (`BNE` taken by us, not by ares) - and the
next step is to trace where the 22 clocks are lost in that handshake loop
(S-CPU write instruction vs S-SMP poll phase, cycle by cycle), not to change the
sample ordering.

## CV4 $F4 sample sits one access before ares (2026-11-20)

With the local ares `SMP::readIO($F4)` hook logging the latch value, the same
handshake window reads:

```
ares  28,991,350  apu0=ff  (x5)
ares  28,991,506  apu0=00     <- first read after the $2140=00 write at 28,991,500
ours  28,991,478  val=ff      (sample in the write's own access)
ours  28,991,709  val=00
```

So ares resolves the sample in the access *after* the write (CPU clock
28,991,506 vs the write at 28,991,500), while ours resolves it in the write's
own access, before the write request is delivered. The S-SMP instruction clocks
match one-for-one up to the divergence (the ares clock column only jumps when
the scheduler reduces all thread clocks together, which preserves differences),
so this is a pure one-access ordering offset.

Both CV4 and SMW write port 0 ~40k times each during their IPL uploads, so
filtering the hold by port cannot discriminate either. The clock-window
distribution is also identical across titles (min 32, median 34, max 72).

The remaining option is to resolve *every* deferred `$F4-$F7` sample at the
start of the following access (which is ares' natural position) rather than at
the end of the current one, and re-measure: this differs from the rejected
"hold" experiments in that it applies uniformly and unconditionally, matching
the trace rather than conditionally delivering a write.

## Absolute-indexed I/O reads (2026-11-20)

`absolute_indexed_read` was the last read form that sampled `$F0-$FF` eagerly
while the instruction was recorded. Super Mario All-Stars and Super Mario World
use it for `$F4-$F7` (886 and 792 hits in 120 frames; CV4 and CT do not). It now
defers through `Pending::AbsoluteRead` like the absolute and direct-page forms,
so the sample resolves at its access clock with the same park/synchronize
behaviour.

Measured: five exact titles still 0 differing, CV4 70,913, SMAS 325,801, CT
444,363 (700,000-sample window) - byte-identical to the previous build, so the
change is a completeness/correctness fix with no measurable audio effect. 204
tests, fmt/clippy clean, 212/212 baseline identical.

## Remaining audio gaps are per-note phase offsets (2026-11-20)

DSP write streams for CV4 and CT match ares register-for-register (2,313 and
12,517 writes over 300 frames) but the `dspTicks` offset is piecewise constant:

```
CV4  +22 at the first write, then -6 for 1,788 writes
CT   +20, then +133, then +111 ... (constant within each section)
```

Each offset appears at a driver section boundary and stays constant, i.e. the
same class as SMW's old one-poll drift: our S-SMP reaches the section's writes a
fixed number of clocks early or late, set by the driver's timing loop at that
boundary. It is not a clock-rate drift (no linear growth) and not the sample
ordering (the conditional hold moved CV4 to 0 but broke the exact titles).

The remaining work is therefore per-driver clock accounting in the handshake
loops (IPL upload for CV4, the driver's init sections for CT/SMAS), not another
global sample-order rule.

## MMX2 with the canonical CX4 firmware (2026-11-20)

The No-Intro/batocera CX4 dump (`cx4.data.rom`, 3 KiB, sha256
`ae8d4d19…`) is byte-identical to the libretro coprocessor archive, so the
earlier 99.44% figure is not reproducible with the canonical firmware:
Mega Man X2 measures **63.2951%** (256,934 differing over 700,000, best offset
-221, first audio difference **6.909 s**). A fresh ares capture made with the
same `cx4.rom` installed is byte-identical to the archived reference, so the
capture is sound.

DSP write logs (480 frames, 2,686 ares / 2,672 ours) diverge in sequence at
4.264 s: ours emits `$5C = $FF` (KOFF all voices) where ares continues a note
setup (`$77/$75/$67/$65/$57/$55`), then the registers realign shifted by one
write. The S-CPU sound-command stream (`$2140` command / `$2141` data from
`$83EB-$83F6`) is the same but runs a **constant ~104 master clocks ahead** of
ares (offsets are -104 for most commands, not growing), and ours issues 46 more
`$2140` writes over the run.

The APU access ordering is not the cause: the S-SMP bus already matches ares
(`wait` before read/write, `$F4-$F7` split into halves). A constant ~104-clock
lead is about one SPC700 instruction - the same handshake-phase class measured
for CV4 (+22 micro-steps) and CT (+20/+133/+111 ticks), i.e. the S-SMP's echo
reaches the S-CPU one instruction early, and the driver then takes an extra
command branch.

ST018 is now verifiable end-to-end as well: the firmware (`st018.program.rom`
131,072 B + `st018.data.rom` 32,768 B) concatenates to the No-Intro dump
(`6df209ab…`) and `Hayazashi Nidan Morita Shougi 2 (Japan)` is in the
Minerva_Myrient set; ares has no ST018 core, so that implementation has no
first-party reference.

## MMX2 boot-handshake path (2026-11-20)

The first S-CPU port accesses already differ, before any DSP write:

```
ares 83,229,850 W3 0D  pc=00842c
ares 83,229,936 W2 A0  pc=008438
ares 83,230,160 R0 AA  pc=00844a

ours 83,229,326 R0 AA  pc=0083C5     <- first access is a CMP $2140 wait
ours 83,229,332 R1 BB  pc=0083C5
ours 83,229,838 W3 0D  pc=008429
ours 83,229,924 W2 A0  pc=008435
```

The ROM code (LoROM) is:

```
$83C5: CD 40 21   CMP $2140        <- wait loop entry ours takes first
$83C8: E2 20      SEP #$20
$83CA: D0 EF      BNE $83C5
...
$842C: ...        upload address setup
$843E: 8D 41 21   STA $2141
$8444: 8D 40 21   STA $2140
$8447: CD 40 21   CMP $2140
$844A: D0 FB      BNE $8447        <- echo wait
```

Our CPU enters the `$83C5` wait ~524 master clocks before ares reaches the
upload setup, i.e. it arrives at the handshake from a different point in the
boot. The later command stream is a constant ~104 clocks ahead and diverges at
4.264 s (`$5C=$FF` extra KOFF), but that is a consequence of this earlier
difference, not an APU phase error.

So MMX2's residual starts in the CPU/CX4 boot path (the code before `$83C5`,
which depends on the CX4 self-test state), unlike CV4/CT where the divergence is
inside the IPL upload loop itself. The next MMX2 step is to compare the boot
path and CX4 status reads before `$83C5` between ares and ours.

## Port synchronize (ares readAPU/writeAPU) (2026-11-20)

Added ares' `Thread::synchronize(smp)` to the S-CPU port accesses: run the
S-SMP forward to the S-CPU clock before `$2140-$2143` reads and writes
(`Apu::synchronize_to`, `ApuInput::Synchronize`). The credit-based per-access
advance can stop fractionally short of the clock, so a port write completed in
the access may be missed; this was the MMX2 extra-`$8447`-poll mechanism.

Measured (600 frames, 639,656-sample window):

```
                      credit (current)   both syncs     read-only sync
dkc                   0 (100.0000%)      173,620 72.86%  173,620 72.86%
zelda                 0                 29,913 95.32%    29,913 95.32%
fzero                 0                355,081 44.49%   355,081 44.49%
super metroid         0                 0 100.0000%     161,390 74.77%
super mario world     0                 16,177 97.47%    16,177 97.47%
castlevania iv        11,269            6,206 99.03%     27,273 95.74%
super mario all-stars 308,432           0 100.0000%      0 100.0000%
chrono trigger        384,266           270,213 57.76%   282,406 55.85%
```

With both syncs SMAS becomes **bit-exact** and CV4/CT improve substantially; the
write sync also cancels the read sync's regression on Super Metroid
(74.77 -> 100.0000%). But DKC/Zelda/F-Zero/SMW regress, so the gate forbids
keeping it.

A whole-instruction-overshoot variant (finish the current instruction after
reaching the clock) was also built; it is pathologically slow at every port
access (>30 minutes for the 8-title sweep without finishing) and was abandoned.

The mechanism and instant match ares (`readAPU` at access start + wait - 4,
`writeAPU` at the access end), so the regression must be in the clock values
compared: our `smp_clock_at_position()` versus ares' `smp.clock()` in the
scheduler's fixed-point base. The next step is to log both at the same port
accesses (`ARES_PORT_SYNC_LOG` already prints `smp0`/`smp1` per access) and
align the comparison; the prize is SMAS plus near-exact CV4/CT without losing
the four exact titles.

## Full ares schedule with the read deferrals (2026-11-20)

The ares clock data settles the unit question: `System::information.apuFrequency`
is `32040 * 768 = 24,606,720`, the DSP is created at that rate and the S-SMP at
`apuFrequency / 12 = 2,050,560`. `smp.clock()/smp.scalar()` is periodically
reduced by the scheduler, so absolute alignment from the logs is impossible;
only deltas are meaningful. `ARES_PORT_SYNC_LOG` shows the S-SMP lagging the
S-CPU by up to ~68 SMP cycles at a port read (`dsmp`), i.e. the S-SMP really is
advanced only at the synchronization points.

Implemented that schedule on top of all the read deferrals: no per-access
advance, `synchronize_to` (whole-step forward run) at `$2140-$2143`, and the
same at Hcounter 0.

```
dkc            0 differing  100.0000%
zelda     69,787             89.0898%
fzero    446,989             30.1204%
metroid  142,854             77.6668%
smw       16,177             97.4710%
cv4       14,683             97.7045%
smas     308,432             51.7813%
ct       450,384             29.5875%
```

The lagging model matches DKC only; the four other exact titles require the
no-lag per-access advance. SMAS does *not* become exact in this model - it only
does with the per-access advance **plus** the port synchronize, which breaks the
four exact titles. So the two observed behaviours bracket the true ares
behaviour and the remaining error is in the scheduler clock accounting itself
(how much the S-SMP is behind at each sync), not in the sync points or the I/O
read forms. Reverted; the per-access model remains the best measured.

## Scanline sync rate and fixed-point base (2026-11-20, negative results)

Two suspected errors in the rejected full-schedule model were corrected and
measured:

**Scanline H=0 rate.** ares performs 2,613 synchronizations in 10 frames
(261/frame). Our hook fired only 1,410 (141/frame): the boot DMA spends ~1.6M
master clocks (1,190 lines) with the actor parked, and the scanline sync was
skipped there. Adding a direct `synchronize_to` on the real APU in the DMA path
raises the count to 2,598/10 frames (260/frame, matching ares). The audio of the
full-schedule model is **byte-identical** with and without that fix, so the
missing DMA syncs were not the cause of its gate failure.

**Fixed-point base.** ares `Thread::Second` is `(u64)-1 >> 1 = 2^63-1`; ours was
`2^62-1`. Correcting the constant leaves DKC and SMW bit-exact on the committed
per-access model, and leaves every full-schedule result byte-identical as well:
the base is not the discriminator either.

With both corrections the full ares schedule still gives:

```
dkc 0 · zelda 89.09% · fzero 30.12% · metroid 77.67% · smw 97.47%
```

So the synchronization points, their rate, and the clock base are all now
verified to match ares; the residual difference must be in the per-step clock
advance inside `SMP::step` (how the S-SMP's own clock moves within an
instruction/access), not in when the threads are synchronized. That is the next
bounded experiment: replicate `SMP::step`'s `Thread::step(clocks)` exactly per
access and compare the resulting `smp.clock()` with `ARES_SCANLINE_LOG`'s `smp=`
values at H=0.

## Per-access S-SMP clock vs ares at every H=0 (2026-11-20)

Logged our executed S-SMP clock (`Apu::smp_cycles`, the step-granular position)
at every Hcounter-0 point and aligned it with ares' `ARES_SCANLINE_LOG` `smp=`
column (B lines), which prints the S-SMP clock at each line's synchronization.
Both are 2.050560 MHz cycles; ares' values are periodically reduced by the
scheduler, so only the line-to-line change of the relative phase
(`smp - cpu * 2050560/21477272`) is meaningful.

Super Mario World, 60 frames, 15,678 matched H=0 points; Donkey Kong Country,
10,136 matched (its boot DMA has no actor-side log):

```
                 matched   mean d(phase)   median
smw              15,678    -0.004 cycles    0.0
dkc              10,136    +0.003 cycles    0.2
```

There is **no systematic drift**: the per-access clock advance matches ares to
rounding (residual spread ±2-12 cycles is alignment/rounding noise, mean zero).
The per-access S-SMP trajectory is therefore correct at line granularity, and
the residual audio difference in CV4/SMAS/CT/MMX2 is **not** in the clock
accounting or in the synchronization points, both of which are now verified.

That leaves the sub-line interaction: the exact master clock at which an S-SMP
port access commits relative to an S-CPU port access within the same line, and
the DSP's register-write micro-step. CV4/CT's piecewise-constant per-note tick
offsets and MMX2's boot-path difference are the concrete instances.

## S-SMP PC at H=0: fetch pointer vs instruction start (2026-11-20)

Logged our S-SMP PC at every Hcounter-0 point and compared it with ares'
`ARES_SCANLINE_LOG` `smppc=`. The literal match rate is low (SMW 5,146 of
15,678; DKC 70 of 10,136), but the offset distribution shows why: ours - ares is
0 for 5,146 points, +1 for 4,034, +2 for 2,331, +3 for 637 on SMW. Our
`Spc700::pc` is the fetch pointer, which advances past the opcode and operands
during the instruction; ares `r.pc` is the instruction start. The comparison is
therefore a logging-definition mismatch, not a divergence, and the previous
line-granular clock comparison (mean drift ~0 over 15,678 points) already shows
the instruction stream is timing-identical at every line.

To make the PC comparison meaningful, `record_instruction`'s `entry_pc` must be
exposed (it is currently only stored in the debug trace), or the H=0 point must
be snapped to the next instruction boundary. Neither changes the emulation.

This closes the line-granularity checks: clock, sync points, sync rate and base
all match ares. The remaining difference is the within-line execution instant of
the S-SMP (ares runs it in bursts at synchronization points, our per-access model
runs it continuously). The lagging-burst model was implemented and fails the
gate (DKC only), so the true ares behaviour lies between the two and cannot be
reached with the current scheduler abstraction: it needs the exact
`co_switch`/`Thread::step` instants within a line, which the existing hooks do
not expose.

## ares co_switch order trace, CV4 (2026-11-20)

Added a switch-order hook to the local ares `Thread::synchronize` (logging
caller/target uniqueIDs and both thread clocks before each `co_switch`) and ran
Castlevania IV for 110 frames. Switch counts by pair:

```
DSP -> SMP  1,862,761      CPU -> SMP   253,330      CPU -> PPU  30,133
SMP -> DSP  1,862,760      SMP -> CPU   253,129      PPU -> CPU  30,133
```

So the S-SMP and S-DSP synchronize **mutually at every S-SMP access** (2 per
access, ~34k per frame), while the S-CPU synchronizes the S-SMP ~2,300 times per
frame (scanline plus port accesses where the S-SMP is behind) and the PPU ~274
times per frame. That per-access SMP/DSP mutual synchronization is exactly what
`Apu::step_dsp_clock` models, which explains why the DSP write streams match
register-for-register for every title.

In the CV4 1.35 s window (cumulative CPU clock 28.986-28.996M) the trace contains
only one CPU -> SMP switch per line (the scanline sync) and **none at the
`$2140-$2143` writes**: `synchronize(smp)` short-circuits there because the S-SMP
is already ahead of the S-CPU, so no execution happens at those accesses. Our
per-access model is level and therefore also does nothing there, i.e. the two
models agree at those writes; the earlier apparent "sample before the write"
difference must be re-derived from the exact poll sequence rather than from the
aggregate clocks.

This leaves the remaining CV4/CT/SMAS/MMX2 difference in the CPU -> SMP switch
count per frame (2,300) and where those switches land inside a line - the only
quantity the trace exposes that the current model does not reproduce.

**PROFILE_PERFORMANCE confirmed.** The reference ares build defines
`PROFILE_PERFORMANCE` (`ares/CMakeLists.txt:29`), which disables
`CPU::step`'s per-step coprocessor synchronization; coprocessors are then run
only from `CPU::scanline()` at Hcounter 0. Our core already implements exactly
that (`cx4_scanline` is set at the H=0 dot and the CX4 is advanced in one chunk
per line to the end of the access), so MMX2's residual is not a CX4 stepping
difference. The only other `PROFILE_PERFORMANCE` conditional in the SNES core is
that one, so the S-SMP/S-DSP schedule is unaffected by the build profile.

## Exact-H=0 burst model with whole-instruction overshoot (2026-11-20)

Implemented the switch cadence observed in the co_switch trace as literally as
possible: no per-access S-SMP advance; `synchronize_to` at `$2140-$2143` and at
the **exact Hcounter-0 instant** (`event_clock`, not the access end); the
synchronize runs to the S-CPU clock and then **finishes the current
instruction** (ares switches whole steps).

Measured, 120 frames, offset 0:

```
dkc   0 differing over 127,858 (100.0000%)   <- exact
smw  23,326 differing (81.7563%)             <- worse than per-access
```

The committed per-access model at the same window gives DKC and SMW both 0.

So the literal ares cadence reproduces DKC exactly but not SMW, confirming the
pattern from every burst-model variant: DKC is insensitive to the within-line
SMP execution instant, the other titles are not, and the per-access model is the
only one that satisfies all of them. The co_switch distribution alone is
therefore not enough to reconstruct ares' audio behaviour; the remaining
difference must be in the *DSP-side* application of the register writes at the
switch instants (the S-DSP thread runs one `DSP::main` per synchronize, and our
`step_dsp_clock` advances it while behind the S-SMP, not in the same switch
cadence).

## Correction: our port-log PC is the step-start PC (2026-11-20)

The earlier "MMX2/CV4 boot path differs at $83C5" conclusion is retracted. Our
harness logs `Snes::live_pc`, which is set once per `Snes::step`; when the park's
forward loop runs extra S-CPU instructions in the same step, the logged PC stays
at the first instruction while the port access happens in a later one. ares logs
`r.pc.d` at the `readAPU`/`writeAPU` call itself, i.e. the access instruction.

Disassembly confirms the mismatch: $83C5 is `8D 2D 21` (`STA $212D`, a PPU
register write), not a `$2140` access, so it cannot be the instruction that read
the port in our log.

Reliable evidence therefore remains the port event values and clocks (ares
`ARES_APU_LOG` vs our port log) and the audio/DSP-write comparisons; the PC-based
path comparisons need the access PC carried in the bus request (or tracked in
the CPU coroutine) before they can be used. No emulation change was made.

## Correct access PCs and the CV4 upload entry point (2026-11-20)

`Snes::step` now refreshes `live_pc` before each forward-loop re-entry, so a port
access in an extra instruction is logged with its own instruction address
(emulation unchanged; 3 lines).

With correct PCs, the CV4 first-handshake comparison becomes meaningful:

```
ours  CR 4,165,004 $2140=AA  pc=028596   <- CMP $2140; BNE $8596 (wait loop)
ours  CW 4,165,098 $2141=01  pc=02859F
ours  CW 4,165,144 $2140=CC  pc=0285A4

ares                                W1 01  pc=0285A2
ares                                W0 cc  pc=0285A7
```

The ROM at the LoROM mapping (bank 2, offset 0x1058F) is:

```
$02858F: C2 20      SEP #$20
$028591: A9 AA      LDA #$AA
$028593: BB         TYX
$028594: 64 20      STZ $20
$028596: CD 40 21   CMP $2140
$028599: D0 FB      BNE $8596
$02859B: E2 20      SEP #$20
$02859D: A9 01      LDA #$01
$02859F: 8D 41 21   STA $2141
$0285A2: A9 CC      LDA #$CC
$0285A4: 8D 40 21   STA $2140
```

Ours executes the documented wait loop at `$8596`; ares' first port access is the
`STA $2141` at `$859F`, i.e. it enters the routine **past the wait**. The write
clocks and values otherwise match exactly (4,165,098 / 4,165,144, `01`/`CC`).
So CV4's divergence begins with the CPU entering the upload routine at `$859B`
rather than `$858F`; no `JSR $858F`/`JSR $859B` exists in the ROM, so the entry is
through a computed jump or a different caller branch that still needs to be
located. That is the concrete remaining question for CV4.

## Correction: CV4's CPU path matches in the upload window (2026-11-20)

With the corrected access PCs, the S-CPU instruction traces of ares and our core
in the CV4 upload window (4,163,000-4,166,500, 115 instructions) are **identical
PC for PC**, including `CMP $2140` at `$028596` / `BNE` / `STA $2141` / `STA
$2140`. Both cores therefore enter and execute the documented upload routine the
same way.

The earlier "ares enters past the wait" observation was an artefact of the
`ARES_APU_LOG` file: `writeAPU` opens it with `"a"` and `readAPU` with `"w"`,
so a read truncates whatever was written, and the resulting log is not a
faithful event sequence at boot. Read/sequence conclusions from that log are
unsound; `ARES_PORT_SYNC_LOG` (which logs both directions from one place) is the
trustworthy source.

CV4's real divergence therefore remains the measured DSP write phase: the first
register write is +22 micro-steps late (3,557,684 vs 3,557,662), then 1,788
writes are -6. The audio first difference is at 9.033 s; the IPL upload (3.47 s)
completes on both sides with identical CPU code.

This closes the CPU-path line for CV4: the difference is downstream of the
driver upload, in the S-DSP/S-SMP phase, not in the S-CPU.

## CV4: instruction durations match; the divergence is the $FFDA sample value (2026-11-20)

Compared the S-SMP instruction streams of ares (`ARES_SPC_TRACE`, clock column)
and our core (`SPC_TRACE`) around CV4's first divergence:

- The PC sequences and **per-instruction durations are identical** for the first
  371,351 instructions (checked only where the ares clock delta is a normal
  positive value; reductions excluded). The apparent +676-cycle "lead" earlier
  derived from accumulating positive deltas is an artefact of the scheduler's
  clock reductions, not a real timing difference.
- At instruction 371,351 (`$FFDA: CMP Y,$F4` in the IPL upload loop) ares takes
  the equality path to `$FFDE` while ours loops back to `$FFE9`: the S-SMP read
  a different `$F4` value at that poll.
- The S-CPU `$2140-$2143` write clocks are identical at that point (port log), so
  the difference is the **sub-clock instant at which the S-SMP samples `$F4`
  relative to the CPU's write within the same access**. Every global change to
  that instant (deferral, hold, burst, overshoot) either broke the five exact
  titles or left the failing ones unchanged.

So CV4's residual is confined to a single sub-access ordering question: which of
the S-SMP sample and the S-CPU port write wins when they land in the same
master-clock window. The available logs bound it (both happen within ~20 master
clocks at 1.35 s) but do not expose the ordering inside ares' `readIO`.

## Sub-access ordering at the CV4 upload trigger (2026-11-20)

Added a single monotonic event log to the local ares covering `SMP::portWrite`
(CPU->SMP latch), `readIO($F4-$F7)` and `writeIO($F4-$F7)` (SMP->CPU latch).
Around CV4's upload trigger:

```
R 0 00  (x9)      <- S-SMP polls $F4 = 00
C 1 01            <- S-CPU writes $2141 = 01
C 0 CC            <- S-CPU writes $2140 = CC
R 0 CC            <- S-SMP reads $F4 = CC   (sees the write)
R 2 00 / R 3 00
R 0 CC / R 1 01 / R 0 CC
S 0 CC            <- S-SMP echoes CC
C 0 00            <- S-CPU writes $2140 = 00
...
```

So ares applies the **S-CPU writes first**, then the S-SMP's `$F4` sample reads the
new value. Our deferred model resolves the sample during the access's `Advance`,
i.e. **before** the CPU's port-write requests are delivered - the opposite order.

The earlier conditional-hold attempt did reorder exactly these two, but it also
moved the drain's clock/credit to the instruction boundary (and to `clock_end`),
which is why it broke DKC/Zelda/F-Zero/SMW even though SMAS and CV4 became exact.
The remaining fix must therefore reorder only the latch application and keep the
drain at the same clock: apply the access's `$2140-$2143` write, then resolve the
pending sample at the same master clock, without changing the credit window.

## Timing-preserving sample reorder (2026-11-20)

Implemented the reorder with the clock/credit held fixed: when the drain reaches
a deferred `$F4-$F7` sample within the access (`port_read_near`, 48-clock
window), the actor yields `Sample` without consuming the unit; the driver
delivers the access's `$2140-$2143` write (if the next request is one) and then
resumes with `ResolveSample`, which sets the master clock to the same value,
drains exactly the held unit and subtracts exactly one credit unit. The rest of
the credit drains afterwards as before, so no boundary move occurs.

Measured (600 frames, 639,656-sample window):

```
dkc     173,620  72.8573%     zelda   1,449  99.7735%     fzero 0 100.0000%
metroid 136,553  78.6521%     smw    45,880  92.8274%     cv4   0 100.0000%
smas          0 100.0000%     ct    439,377  31.3104%
```

CV4 and SMAS become bit-exact and F-Zero stays exact, but DKC/Zelda/Super
Metroid/SMW regress with the *same* numbers as every earlier hold variant. So the
regressions are not caused by the boundary clock or credit (both held fixed
here): the four exact drivers must resolve their colliding polls with the
opposite order (sample before the CPU's write), while CV4 and SMAS resolve them
write-first. A single global ordering cannot satisfy both; the discriminator has
to come from something driver-specific that we have not identified.

Reverted; the per-access model is restored and all five exact titles re-verified.

## The write-before-sample order is uniform across titles (2026-11-20)

Captured the ares port-order log for CV4, DKC and SMW (100 frames each) and
checked every S-CPU `$2140` write against the next S-SMP `$F4` read:

```
           C0 writes followed by R0    sees written value    sees another value
cv4                 24,013                  24,013                   0
dkc                 16,237                  16,237                   0
smw                 40,519                  40,519                   0
```

So ares is **uniformly write-first**: in all three titles the S-SMP sample
observes the S-CPU's write, at the IPL upload and everywhere else. The ordering
is not the discriminator between the exact titles and the failing ones.

Therefore the regressions produced by our reorder (DKC 72.86%, Zelda 99.77%,
Metroid 78.65%, SMW 92.83% - identical in the hold variant and in the
whole-advance deferral variant) are an artefact of *how* we defer the advance,
not evidence that those drivers want the opposite order. Both our variants move
the S-SMP's execution point relative to the following bus request (the hold
defers one unit, the deferral variant moves a whole slice), which can change the
park/timeout interactions even when the latch value is identical.

The fix must preserve the advance's execution point exactly and only reorder the
latch application. That is not expressible in the current driver protocol, where
the port write is a later request than the advance that contains the sample: it
needs the write to be applied inside the sample's synchronize, i.e. the CPU
request stream must let the driver complete the in-flight write while the actor
is at the sample.

## Write-before-drain protocol experiment (2026-11-20)

Applied the S-CPU `$2140-$2143` write before its access's APU drain (instead of
after, ares' literal `step`-then-`write` order), so an S-SMP sample in that
access observes the new latch without any hold/deferral machinery:

```
dkc 173,620 72.8573%   zelda 1,449 99.7735%   fzero 0 100.0000%
metroid 136,553 78.6521%   smw 45,880 92.8274%
cv4 0 100.0000%        smas 0 100.0000%       ct 438,456 31.4544%
```

Identical to every previous variant: CV4 and SMAS exact, the four gate titles at
the same percentages. Combined with the port-order result (ares is uniformly
write-first and its latch does not move relative to the sample), this isolates
the cause of the regressions: they react to the **write's clock** (it now commits
at the access start instead of the access end), not to the sample ordering.

So the four exact titles depend on the write committing *after* the access's
clock advance, while CV4/SMAS require the sample to see that write. ares gets
both from its cooperative recursion: the write is stepped at the access end,
`writeAPU` then synchronizes the S-SMP, the S-SMP's `readIO` recursively resumes
the suspended S-CPU and completes `portWrite`, and only then does the sample
read. Reproducing that needs the driver to suspend the S-CPU *inside* the write
and let the actor resume it at the sample - a scheduler/protocol change, not a
request-order tweak. Reverted; the five exact titles are re-verified.

## Faithful writeAPU recursion emulation (2026-11-20)

Implemented the ares recursion directly: `Cpu::write` announces an APU port write
before its access advance (`CpuBus::notice_port_write` -> `BusRequest::Notice`),
the driver forwards the announced value with the access's `Advance`, and
`drain_unit` applies the latch **at the `$F4-$F7` sample inside that access** -
i.e. exactly when ares' `writeAPU` synchronize lets the SMP's `readIO` complete
the in-flight write before sampling.

Measured (600 frames, 639,656 window):

```
dkc 173,620 72.8573%   zelda 1,449 99.7735%   fzero 0 100.0000%
metroid 136,553 78.6521%   smw 45,880 92.8274%
cv4 0 100.0000%        smas 0 100.0000%       ct 439,377 31.3104%
```

This is the *same* split as every simpler variant (hold, deferral,
write-before-drain): CV4/SMAS/F-Zero exact; DKC/Zelda/Super Metroid/SMW at the
identical regressed percentages. The ordering hypothesis is therefore
exhausted: applying ares' write-before-sample rule at the sample breaks the four
exact titles no matter how faithfully it is implemented.

Conclusion: the four titles' exactness in the current model must rest on a
different aspect of the interleaving that every write-ordering change also
perturbs (most likely how the S-SMP's park/credit interacts with the CPU's
request stream), and the analysis of the write/sample ordering cannot be
extended further without restructuring the driver as a true cooperative
scheduler (S-CPU suspendable inside `writeAPU`, S-SMP suspendable inside
`readIO`, both resumable from the other side) rather than the request/response
loop.

## Invariant across every write-order implementation (2026-11-20)

Five independent implementations of ares' write-before-sample rule now exist and
have all been measured:

1. per-request sample hold (unit-granular);
2. whole-slice advance deferral;
3. write-before-drain (CPU side);
4. announced write applied at the sample (notice protocol);
5. the same with the announced write cleared at the end of its access (fixing a
   stale-notice path).

Every one produces exactly:

```
cv4 0 100.0000%   smas 0 100.0000%   fzero 0 100.0000%
dkc 173,620 72.8573%   zelda 1,449 99.7735%
metroid 136,553 78.6521%   smw 45,880 92.8274%   ct 439,377 31.3104%
```

The percentages are identical to the digit across mechanisms, so the effect is a
deterministic consequence of the ordering change itself, not an implementation
artefact. Since ares' port-order log shows write-before-sample uniformly
(80,769 write->read pairs, all write-first), the four exact titles cannot be
explained by the pre-write latch: their exactness in our model must depend on how
the write ordering interacts with the rest of the interleaving (park, credit and
the request stream), which only a true cooperative scheduler reproduces as a
whole.

That scheduler - S-CPU suspendable inside `writeAPU`, S-SMP suspendable inside
`readIO`, each resumed from the other side - is the remaining work; the current
model is the best measured and is preserved.

## Burst schedule plus write-at-sample (2026-11-20)

The one untried combination: ares' synchronization cadence (no per-access
advance; synchronize_to at `$2140-$2143` and at the exact Hcounter-0 instant;
direct sync during DMA) together with the announced port write applied at the
`$F4-$F7` sample inside that synchronize.

```
dkc 173,620 72.8573%   zelda 0 100.0000%   fzero 384,417 39.9025%
metroid 237,622 62.8367%   smw 103,599 83.8040%
cv4 106,939 83.2789%   smas 299,949 53.1078%   ct 483,504 24.4119%
```

Only Zelda is exact; every other title regresses, including CV4 which had been
exact under both ingredients separately. This confirms that the two mechanisms
interact destructively and that the simple composition of the two measured
behaviours is not ares.

At this point every combination of (advance cadence) x (write/sample ordering)
has been implemented and measured: per-access (five exact), burst (DKC only),
burst+write-first (Zelda only), per-access+write-first (CV4/SMAS exact). No
combination reproduces the reference across all titles, so the difference is not
a parameter of this design space. Replicating ares requires its actual
cooperative scheduler semantics (threads resumed from each other inside
`writeAPU`/`readIO`), which the request/response driver cannot emulate by
composing the endpoints. The per-access model remains the best measured and is
restored.

## First cooperative-scheduler port (2026-11-20)

Implemented the authorised scheduler path: no per-access S-SMP advance; ares
`Thread::synchronize(smp)` run in **whole instructions** (overshoot to the
instruction end) at `$2140-$2143` and at every S-CPU instruction boundary, with
the S-SMP still parking at `$F4-$F7`.

Short window (120 frames, 128,000 samples) looked like a breakthrough:

```
dkc 0 · zelda 0 · fzero 0 · metroid 0 · cv4 0 · ct 0
smw 23,326 (81.76%) · smas 19,930 (84.41%)
```

Full window (600 frames, 639,656 samples) shows it diverges after ~2 s:

```
fzero 0 (100.0000%)        zelda 2 (99.9997%)
dkc 186,952 (70.7730%)     metroid 240,231 (62.4437%)
smw 318,205 (50.2537%)     cv4 83,456 (86.9530%)
smas 370,451 (42.0859%)    ct 483,016 (24.4882%)
```

The per-access model over the same window is far better (five exact, cv4 11,269,
smas 313,782, ct 416,143), so the scheduler variant was reverted. The apparent
six-title match at 120 frames was a short-window illusion of exactly the kind the
project has guarded against.

A second variant (credit-exact catch-up at instruction boundaries, tracked via an
`apu_sync_clock`) livelocked around frame 50 in DKC: the accumulated APU credit
and the park target interacted so the forward loop never finished. Reverted with
the rest.

Conclusion: whole-instruction synchronization at the endpoints is not sufficient;
the scheduler port needs the mutual suspension (the S-SMP resuming the S-CPU
*inside* `readIO`) that all previous endpoint compositions failed to emulate.

## S-DSP tick phase at H=0 (2026-11-20)

Logged our S-DSP micro-step counter (`samples_run*32 + step`, ares `dspTicks`)
at every Hcounter-0 point and compared with `ARES_SCANLINE_LOG` for CV4:

```
ares B line  clock 1,364   dsp=0        (logged before CPU::scanline synchronizes the SMP)
ours  H line  clock 1,364   dsp=67
ares B line  clock 2,730   dsp=65
ours  H line  clock 2,730   dsp=131
```

The apparent ~66-tick lead is the logging point: ares' `B` line is emitted
*before* `Thread::synchronize(smp, ppu)` catches the SMP up, while our hook runs
after the access has advanced the APU. The expected post-sync value at clock
1,364 is ~65 ticks (1,364 master clocks = ~130 SMP cycles = ~65 DSP ticks), and
ours reads 67 - i.e. within the one-tick DSP lead the ares scalar rounding
produces. The local ares `A` (post-sync) lines are missing from the capture
(only one line written), so the comparison cannot be completed from that log, but
the values are consistent with no DSP phase difference.

So the DSP tick phase is not the source of the remaining CV4 offset either. The
search returns to the S-SMP sample/park interaction, where the mutual-suspension
implementation is the remaining untried piece.

## Root cause located: the S-SMP/IPL boot handshake, not the scheduler (2026-11-20)

The scheduler question was settled by direct execution comparison. With
`ARES_SPC_TRACE` extended to emit `dspTicks`, our S-SMP trace was diffed
against ares for 240 frames of CV4:

```
instructions compared      1,140,309
PC sequence mismatches     0 (until the first divergence)
per-instruction cycle
  counts mismatched        0 (271,580 instructions over 60 frames)
per-instruction dspTicks   0 mismatches up to instruction 371,351
```

So the port/interleave model already reproduces ares' S-SMP program path and
instruction timing exactly. The divergence is a single event:

```
instruction 371,351 (master clock 28,991,600, 1.35 s)
  ares FFE9 (branch taken)   ours FFDE (branch not taken)
  -> one instruction earlier, `CMP Y,$F4` at FFDA read a different $F4
```

Tracing further back with the corrected `ARES_SMPIO_LOG` (the read hook opened
the log with `"w"` while the write hook used `"a"`, which truncated the boot
sequence - same defect previously found in `ARES_APU_LOG`), the *first*
divergence is at instruction 182,300 (0.68 s), in the IPL ROM download
handshake:

```
182,299  FFEB  CMP Y,$F4      Y=0x22
182,300  FFED  ares z=1 c=1   ($F4 == 0x22)   ours z=0 c=0  ($F4 != 0x22)
```

The S-CPU writes `$2140 = 0x22` at master clock 14,624,280; the S-SMP reads
`$F4` at DSP tick 698,133. ares returns the *new* value, we return the old one.

Cause: a one-unit stop-boundary difference. ares runs
`while(smp.clock() < cpu.clock()) co_switch(smp)`, so a unit whose *start* is at
or beyond the S-CPU clock is not executed; it runs on the next synchronize,
after `portWrite` has updated `io.apu0`. Our per-access advance uses a carried
credit (`apu_credit += clocks*2_050_560; while credit >= rate`) and executes
that unit in the same advance, before `port_write`, so the `$F4-$F7` midpoint
sample reads the stale latch.

Replacing the credit with an absolute fixed-point comparison
(`while smp_clock_at_position() < clock_ticks`) removes this boundary error and
keeps DKC/F-Zero/Super Metroid/SMW/CV4 exact at 120 frames - but it drifts over
long runs:

```
600 frames: dkc 0 (100%)   zelda 231,892 (63.75%)   fzero 446,989 (30.12%)
            metroid 238,681 (62.69%)   smw 16,177 (97.47%)   cv4 69,470 (89.14%)
```

because the two truncated scalars (`SMP_SCALAR = Second/2_050_560` and
`Second/master_clock`) round differently over millions of units, while the
rational credit does not. The credit is therefore required for long-term rate
fidelity and the boundary needs a separate mechanism (e.g. recording each
access's exact start/end rational bounds and stopping before a unit whose start
is >= the target, without changing the accumulated rate).

## DSP phase and trace comparison (2026-11-20)

`ARES_DSP_TRACE` (204 columns per output sample, same layout our
`DSP_FULL_TRACE_OUT` already emits) was diffed for 560 frames of CV4:

```
first differing row 43,537 (1.36 s): column 172 `brr._byte` (ares 200, ours 199)
first differing *audio* row 289,423 (9.03 s): right channel only, ares 1183,
  ours 1268, with every traced DSP state column identical
```

So the first audible difference is a single right-channel sample whose traced
state matches ares exactly: the left channel is `mainvol.output[0]` (scaled by
`MVOL` in `echo26`) while the right is the raw accumulator `mainvol.output[1]`,
and the two only diverge if a voice's contribution was added on a different
side of `echo27` - a within-sample accumulation ordering effect downstream of
the `$F4` handshake divergence above, not an independent DSP bug.

Register-write phase (from `ARES_DSP_WRITES`): our `dspTicks` at each write is
offset by a constant `+6 (mod 32)` micro-steps in 1,788 cases and `+10` in 525,
i.e. a sub-sample constant phase, consistent with (not the cause of) the
boundary behaviour; the DSP rate itself matches (25 micro-steps per 50 S-SMP
clocks at every write in both).

The `YA` word-order scare (our `(Y<<8)|A`) was a mislabeled trace column in the
local ares hook (`r.ya.byte.l` is `A`, not `Y`); the encoding matches ares and
the change was reverted.

## Exact-rational and ares-fixed-point stop boundaries (2026-11-20)

Three stop rules for the per-access S-SMP advance were implemented and measured
on the full 600-frame window.

1. **Exact rational counts** - `units * rate < master_clocks * 2_050_560`, using
   the retired S-SMP clock count (`units`), avoiding both truncated scalars.

```
dkc 0   zelda 16,503 (97.42%)   fzero 355,081 (44.49%)
metroid 0   smw 45,880 (92.83%)   cv4 0 (100%)
smas 313,782 (50.95%)   ct 417,093 (34.79%)
```

2. **ares fixed-point comparison** - `1 + units*SMP_SCALAR <
   master*CPU_SCALAR`, both scalars truncated exactly as `Thread::setFrequency`
   does, with `Second = 2^63-1` and `CPU_SCALAR` taken at ares' rounded
   `Colorburst::NTSC*6 = 21,477,273` Hz (the integer master clock is
   21,477,272, so using the latter for the scalar matters):

```
dkc 0   zelda 1,449 (99.77%)   fzero 355,081 (44.49%)
metroid 1   smw 45,880 (92.83%)   cv4 0 (100%)
smas 313,782 (50.95%)   ct 416,097 (34.95%)
```

3. **Carried credit** (the existing model) - `credit += clocks*2_050_560;
   while credit >= rate { drain; credit -= rate }`:

```
dkc 0   zelda 0   fzero 0   metroid 0   smw 0   cv4 11,269 (98.24%)
smas 313,782 (50.95%)   ct 416,143 (34.94%)
```

So the ares-exact boundary does what it was designed to do - it removes the
CV4 IPL-handshake misread and makes CV4 bit-exact for the whole 600 frames,
and it brings Zelda to 99.77% and Metroid to a single sample - but it breaks
F-Zero (44.49%) and SMW (92.83%), which the carried credit keeps exact. SMAS and
CT are unaffected by the rule. Each rule is exact for a different subset, so
the remaining difference is not in the boundary rule alone.

F-Zero's first divergence under the strict rule is the mirror of CV4's, and it
pins down what is still missing:

```
instruction 78,259 (DSP tick 351,311)
  FFD6 `MOV Y,$F4`
  ares Y=0x00 (new latch)   ours Y=0xCC (old latch)
```

i.e. at an identical DSP tick and identical S-SMP clock, ares' `$F4` read sees
the S-CPU write that ours placed on the other side of the same access. CV4's
case wants the read held back past the write; F-Zero's wants it run before it.
Since both now use ares' exact truncated clocks and both threads' clocks match
at every instruction boundary, the residual is the ordering *inside* the access
(ares applies `writeAPU` after `synchronize(smp)`, and its `synchronize(cpu)`
inside `readIO` can advance the S-CPU mid-read), which the deferred-read model
resolves at a fixed midpoint instead of a re-entrant resume.

The carried-credit model is retained as the best measured overall state
(five titles exact, CV4 98.24%, SMAS/CT unchanged).

## Instruction-granularity overshoot, with and without the read park (2026-11-20)

ares' `Thread::synchronize(smp)` switches threads one **instruction** at a time
(`while(smp.clock() < cpu.clock()) co_switch(smp)`), so the S-SMP overshoots the
S-CPU clock to the end of the instruction it is executing. The S-CPU port write
clocks are already proven identical between the two emulators:

```
ours  7357876 W 2 00     ares W cpu=7357876 ... dsp0=351226 dsp1=351250
ours  7357882 W 3 3C     ares W cpu=7357882 ...
ours  7358028 W 1 01     ares W cpu=7358028 ... dsp0=351250 dsp1=351257
ours  7358102 W 0 CC     ares W cpu=7358102 ... dsp0=351257 dsp1=351261
ours  7359128 W 0 00     ares W cpu=7359128 ... dsp0=351294 dsp1=351310
ours  7359134 W 1 00     ares W cpu=7359134 ...
```

So the only difference at F-Zero's first divergence is which side of a write the
`$F4` read lands on: at DSP tick 351,311 ares' `MOV Y,$F4` returns 0x00 (the
write that completed at master clock 7,359,128) while ours returns 0xCC.

Implementing the instruction-granularity stop with the per-access cadence kept
(the existing `$F4-$F7` read park remains active; a read that overshoots the
S-CPU clock parks and the driver runs the S-CPU to the next instruction
boundary before the sample is taken) gives, at 600 frames:

```
dkc 156,064 (75.60%)   zelda 0   fzero 0   metroid 240,414 (62.42%)
smw 318,197 (50.26%)   cv4 101,591 (84.12%)   smas 370,472 (42.08%)
ct 484,068 (24.32%)
```

The overshoot makes Zelda and F-Zero exact but breaks DKC/Super Metroid/SMW,
exactly the complement of the clock-granularity credit model. The instruction
stream itself is identical to ares in both (PC sequence matches for every
compared instruction), so the difference is purely which latch a `$F4-$F7`
midpoint sample observes.

Summary of every stop/ordering rule measured so far, at 600 frames:

| rule                                        | exact titles                       |
|---------------------------------------------|------------------------------------|
| carried credit, clock granularity (current)  | dkc, zelda, fzero, metroid, smw    |
| exact rational, clock granularity            | dkc, metroid, cv4                  |
| ares fixed-point, clock granularity          | dkc, cv4 (+ zelda 99.77%, metroid 1 sample) |
| instruction-granularity overshoot            | zelda, fzero                       |

No rule is exact for all titles, and each is exact for a different subset with
identical S-SMP clocks and identical S-CPU write instants. The remaining
variable is the mutual suspension itself: in ares the suspended S-CPU runs
forward *from inside* the S-SMP's step (`SMP::step` calls `synchronize(cpu)`),
which a driver that only stops, parks and flushes at access boundaries cannot
express. That is the symmetric-coroutine rework, not another boundary rule.

## Symmetric CPU/SMP driver (`SNES_MUTUAL=1`, 2026-11-20)

The mutual suspension is now implemented behind `SNES_MUTUAL`, so the verified
per-access path remains the default.

What it does, mirroring ares' cooperative threads:

- `Apu::advance_master_clock_mutual` stops the S-SMP at **instruction**
  granularity (`while(smp.clock() < cpu.clock())` checked between whole
  instructions), using ares' rounded `Colorburst::NTSC*6 = 21,477,273` Hz for
  the S-CPU scalar and `Second = 2^63-1`.
- A `$F4-$F7` read whose position is ahead of the S-CPU clock parks the S-SMP.
- The driver keeps running the S-CPU coroutine, accumulating APU clocks
  (`parked_clocks`) and answering port accesses against the live latch, and
  resumes the S-SMP only once `self.clock >= target` - ares'
  `while(cpu.clock() < smp.clock()) co_switch(cpu)` checked between whole
  S-CPU instructions.

The park is exercised: for F-Zero at power-on the S-SMP is ahead by ~19 master
clocks at each event (`PARK cpu=7358934 smp=7358953`, ...).

Measured at 600 frames:

```
default (verified path): dkc 0   zelda 0   fzero 0   metroid 0   smw 0
                         cv4 11,269 (98.24%)   smas 313,782   ct 416,143

SNES_MUTUAL=1:           zelda 0   fzero 0
                         dkc 156,064 (75.60%)   metroid 240,414 (62.42%)
                         smw 318,197 (50.26%)   cv4 101,591 (84.12%)
                         smas 370,472 (42.08%)   ct 484,068 (24.32%)
```

So the full mutual-suspension mechanism, implemented as ares describes it,
reproduces the instruction-granularity-overshoot result exactly: it is exact for
Zelda and F-Zero and worse for DKC/Super Metroid/SMW. The extra S-CPU run
forward at the park does not change any sampled latch for these titles, because
the writes that matter complete before the parked read's clock in both cases.

Conclusion: the driver now supports the symmetric resume, but the residual
difference is not the presence of the mechanism. The two families of results
(clock-granularity credit: dkc/zelda/fzero/metroid/smw; instruction-granularity
mutual: zelda/fzero) are produced by identical S-SMP clocks and identical S-CPU
write instants, so the remaining variable is inside the S-SMP's own
port-read/write interleaving at the same clocks - the `$F4-$F7` latch timing
relative to `SMP::step`'s intra-instruction CPU synchronize, not the driver's
scheduling structure.

## Three-case latch comparison: ares always holds the newer latch (2026-11-20)

Instrumented both emulators' `$F4-$F7` path (`PORT_OUT` now logs the DSP tick of
each S-SMP read and the master clock of each S-CPU port write; `ARES_SMPIO_LOG`
logs the S-SMP read value and PC). First divergence per title, and the latch on
each side at the *same* S-SMP clock:

| title | rule under test | read (S-SMP tick) | ares latch | ours latch | S-CPU write commit |
|---|---|---|---|---|---|
| CV4 | carried credit | 698,133 | 0x00 | 0xFF | master 14,624,280 |
| F-Zero | carried credit | 351,311 | 0x00 | 0xCC | master 7,359,128 |
| DKC | `SNES_MUTUAL` | 77,918 | 0xCC | 0x00 | master 1,632,150 |

In every case ares observes the **newer** latch at the same S-SMP clock, and the
write's commit clock is at or before that clock. So the invariant is not
"write before/after sample" as an ordering flag but: *the read is timed so that
a write completing at or before the read's clock is already visible*.

Two different mechanisms in our driver violate it, in opposite directions:

- carried credit: the read at CV4's clock executes inside the advance that
  precedes the write request, so the latch is still old;
- instruction-granularity (`SNES_MUTUAL`): the S-SMP overshoots past the access
  end and executes the read before the write for *that* access is applied, so
  DKC's handshake poll needs one extra iteration (`FFD2 FFCF FFD2 FFCF` vs ares'
  `FFD2 FFCF FFD2`).

The ares-exact fixed-point rule (`1 + units*SMP_SCALAR < master*CPU_SCALAR`,
with `Second = 2^63-1` and the S-CPU scalar at ares' rounded
`Colorburst::NTSC*6 = 21,477,273`) computes the boundary on the same fixed-point
clocks ares uses and does make CV4 exact, but puts DKC on the wrong side. Since
both rules claim to implement the same comparison and yet disagree per title,
the residual is a sub-clock difference in the S-SMP's own clock between the
port-write commit and the read position - not a scheduling-rule choice.

Concretely, satisfying the invariant needs the S-CPU port write to be ordered
before the S-SMP read that follows it by the *access end* rather than by the
request stream. The driver currently receives `Advance` before `Write` for the
same access, so the latch cannot be updated during that advance without
reordering the coroutine protocol (emit the write with its commit clock ahead
of the advance, and apply it at the access end while reads beyond that end see
it).

That protocol change is the next concrete step; every stop/ordering rule that
does not touch it has now been measured and is exact for a different subset.

## Latch committed before the access advance (2026-11-20)

The invariant above (ares holds the newer latch) was implemented directly: for
`$2140-$2143` writes the latch is committed *before* the access advance, so any
`$F4-$F7` read inside that access sees it. Measured at 600 frames:

```
cv4 0 (100%)   smas 0 (100%)   fzero 0 (100%)
dkc 173,620 (72.86%)   zelda 1,449 (99.77%)
metroid 239,756 (62.52%)   smw 45,880 (92.83%)   ct 440,267 (31.17%)
```

This is the same split as every previous write-before-sample rule (compare the
125,000+ `C0->R0` pairs measured earlier): the exact set simply flips. Together
with the three-case table this proves the two families are mirror images -
some reads in each title require the newer latch and others the older one at
identical S-SMP clocks - so a single global latch-ordering rule cannot be
correct. Reverted.

## S-SMP clock at every `$F4-$F7` read: identical to ares (2026-11-20)

Both sides now log the S-SMP clock at every `$F4` read (`PORTDBG` in our build
writes `units tick pc value`; the ares SMPIO hook was extended with
`smp=clock()/scalar()`). At CV4's first divergence:

```
ours  1396265 698133 FFED FF
ours  1396279 698140 FFDC 00     (delta 14 S-SMP clocks)

ares  14624286 RD0 ... apu0=00 y=22 pc=ffed smp=217
ares  14624432 RD0 ... apu0=00 y=22 pc=ffdc smp=231     (delta 14)
```

The clocks are **identical** (delta 14 on both sides) and the read sequence
aligns one-for-one, so the S-SMP clock accounting is correct. That rules out
the clock-accounting branch of the question. Only the latch value differs at
the same clock: ares has the `$2140` write that committed at master clock
14,624,280 applied, we do not.

Combining the two measures this implies - the exact-rational stop (no carried
remainder) **and** committing `$2140-$2143` before the access advance - gives at
600 frames:

```
dkc 0 (100%)   cv4 0 (100%)   zelda 1,450 (99.77%)
fzero 355,081 (44.49%)   metroid 238,764 (62.67%)
smw 62,934 (90.16%)   smas 313,782 (50.95%)   ct 438,546 (31.44%)
```

i.e. two exact titles, fewer than the carried-credit model's five, so it was
reverted as well.

Conclusion: the S-SMP clock, the S-CPU write instants and the split-wait read
position are all proven identical to ares (`wait_cycles` returns
`{1,2,5,10}` units, the read resumes at `clocks/2 = {1,2,5,10}` S-SMP clocks,
which is exactly `cycleWaitStates >> 1`), yet every latch-ordering rule flips a
different subset of titles.

The observable that has *not* been compared is the sequence of intermediate
`cpu_master_clock` targets handed to the APU: we advance the APU once per
S-CPU bus access, while ares only synchronizes at `$2140-$2143`, at Hcounter 0
and from the S-SMP's own `step`. The totals (S-SMP clocks, write commit clocks)
agree exactly, but the intermediate visibility windows differ, and a `$F4-$F7`
read's latch depends on which window it is resolved in. Reproducing ares'
synchronization *points* (not its totals) is therefore the remaining variable,
which is what the `SNES_MUTUAL` cadence was meant to do but currently combines
with the wrong stop rule.

## ares synchronization-point cadence, carried-credit stop (2026-11-20)

The last untested degree of freedom was the *cadence*: advance the APU only at
ares' synchronization points (a `$2140-$2143` access, Hcounter 0, and from the
S-SMP's own step) while keeping the carried-credit stop. Implemented behind
`SNES_MUTUAL` (deferred clock accumulation in `ApuPeer`, synced at port
accesses and at the `wrapped` Hcounter-0 event).

Two problems:

1. It is pathologically slow. A 600-frame run produced only 75,692 audio samples
   (~141 frames) before the timeout, for every title, because each deferred
   chunk drains a long S-SMP stretch and the park/flush then re-runs the S-CPU,
   making the cost superlinear.
2. On the frames it did produce, it is not correct: `metroid 0` and `cv4 0` but
   `dkc 5,942/75,692`, `zelda 10,266`, `fzero 2,236`, `smw 82,875`,
   `smas 13,328`, `ct 19`.

Also measured with the instruction-granularity stop still active (the earlier
`SNES_MUTUAL` semantics) for the full 600 frames:

```
fzero 0   zelda 2   dkc 186,952   metroid 240,033   smw 318,205
cv4 83,384   smas 370,451   ct 483,153
```

Both variants are reverted.

Status of the search: the S-SMP clock, the S-CPU port-write commit clocks and
the split-wait read position are all proven identical to ares; the carried
credit is the only stop rule exact for five titles; every cadence and ordering
variant measured (twelve now) is exact for a different subset. The residual is
not reachable by choosing a cadence, a stop rule, or a latch-ordering rule in
the current deferred-read architecture.

## Concurrent resumption with the carried-credit stop (2026-11-20)

`SNES_MUTUAL` now selects only the concurrent resumption: a parked `$F4-$F7`
sample is resumed as soon as the S-CPU has reached its clock, at S-CPU *access*
granularity, while the stop rule stays the carried credit. The instruction-
granularity dispatch was removed from `advance_master_clock` for this test.

600 frames, `SNES_MUTUAL=1`:

```
dkc 0   zelda 0   fzero 0   metroid 0   smw 0
cv4 11,269 (98.24%)   smas 313,782   ct 416,143
```

Byte-for-byte identical to the default path. So the park/resume mechanism
(suspending the S-SMP at a `$F4-$F7` sample and resuming it after the S-CPU
catches up) produces no output change at all once the stop rule is the carried
credit: the parked reads either do not occur in these titles or their latch is
already correct by the time they resolve.

This completes the concurrency axis. Every way of expressing "both threads
suspended and resumed mid-instruction" that the driver can support has now been
implemented:

| concurrency variant | result |
|---|---|
| per-instruction overshoot, flush at instruction boundary | zelda + fzero exact |
| per-instruction overshoot, flush at access boundary | zelda + fzero exact |
| carried credit, flush at access boundary | five exact (identical to default) |
| ares sync-point cadence (ports + H=0) | slow, diverges |

Combined with the earlier stop-rule and latch-ordering tables, the conclusion is
that the S-SMP/S-CPU concurrency structure is not the remaining variable: the
carried-credit per-access driver already reproduces the reference exactly for
five titles and differs only in single-sample latch visibility for the others.

## S-CPU clock at the read: exactly one access late (2026-11-20)

The one quantity not yet compared is the S-CPU clock at the moment of a
`$F4-$F7` read. ares' SMPIO line was extended with `cpu=clock()/scalar()` and
ours with `cpu_master_clock`, at CV4's first divergence:

```
ours  1396264 698133 FFED FF   cpu=14624280
ares  ... now=14624286 smp=217 cpu=2286   apu0=00  pc=ffed
```

`oracleNow()` on the ares side is the S-CPU master clock, so ares performs the
read at master clock **14,624,286** while ours performs it at **14,624,280** -
exactly **one S-CPU access (6 clocks) earlier**. The S-CPU port write commits at
14,624,280 in both (already proven identical), so ares resolves the read on the
following access (new latch) and we resolve it on the write's own access (old
latch).

This is the whole of the CV4/SMAS/CT difference, and it is why the
exact-rational stop - which pushes the last unit past the access boundary -
made CV4 and SMAS bit-exact. The same one-access shift moves some reads in
DKC/Zelda/F-Zero/SMW across a different write, which is why every rule that
fixes one family breaks the other: the shift is a property of ares' *access
window* accounting, and applying it uniformly changes reads that are correctly
placed today.

So the remaining question is not a stop rule or a latch-order flag but the exact
access-window boundary used when the S-SMP catches up: ares synchronizes the
S-SMP at the S-CPU's *next* port-access clock, and our per-access advance stops
at the current access end. A rule that moves only reads *at* the boundary
(and not the general case) is the target.

## Boundary-rule variants measured: no change (2026-11-20)

The hypothesis "park a read that lands on the access boundary so it resolves on
the next access" was implemented two ways on top of the carried-credit stop:

1. `port_read_waiting` compares the *sample's* clock (position + one unit) with
   `>=` against the access end;
2. the credit drain requires `credit > rate` instead of `>=` (one unit earlier).

Both leave the 600-frame result bit-identical:

```
dkc 0   zelda 0   fzero 0   metroid 0   smw 0   cv4 11,269 (98.24%)
```

So neither formulation reaches the CV4 read. The instrumentation explains why:
at the divergence the sample position is ~31 master clocks *before* the access
end (position 14,624,249 vs end 14,624,280), not on it, and ares resolves it
with the CPU at 14,624,286. That means ares' S-SMP had **not yet reached** the
read at the 14,624,280 synchronize while ours had - i.e. the two S-SMP clocks
differ by roughly two S-SMP clocks at that instant, even though the
instruction-boundary deltas match. Chasing the remaining difference therefore
requires locating where the S-SMP clock diverges by ~2 clocks, which the
per-instruction delta comparison (equal over 271k instructions) does not show;
it is either a sub-instruction effect at a port access or a difference in the
S-CPU clock used for a specific synchronize.

Both variants reverted.

## The S-SMP divergence starts at the first port write (2026-11-20)

Both emulators were logged at **every** S-CPU port access (`ARES_PORT_SYNC_LOG`
gives `smp0/smp1/dsmp`; ours logs the retired S-SMP clock at each `$2140-$2143`
access). The two logs have exactly 142,413 entries for 70 frames of CV4 and the
S-CPU access clocks match one-for-one, so they align index by index.

The very first divergence is at the **third port access**, the first write after
boot:

```
idx  cpu clock   ours (retired)   ares smp1   ares dsmp
  0  4165004     397657           25204       70
  1  4165010     397657           25204        0
  2  4165098     397666   (+9)    25214       10      <- one clock short
  3  4165144     397670   (+4)    25218        4
  4  4165216     397677   (+7)    25224        6
  5  4165268     397682   (+5)    25229        5
```

`ares` advanced the S-SMP by 10 clocks across that access window; ours by 9.
From there the two advance in step per access but with the one-clock offset
already present - which is exactly the "~2 S-SMP clocks" seen later at CV4's
`$F4` read (it accumulates through subsequent parked/flush boundaries).

So the root difference is in the *first* catch-up advance after power-on, not in
the later port-read boundary. 4,165,004 master clocks is 397,650 S-SMP clocks,
and our counter reads 397,657 there, so the offset is in how the very first
synchronization seeds and drains the credit, not in the steady-state rate (the
per-access deltas agree from idx 3 onward).

Next step: reproduce ares' first `Thread::synchronize(smp)` exactly for the
power-on window, when the S-SMP clock starts at its uniqueID (1) and the S-CPU
clock at 0, and confirm the third access advances 10 clocks instead of 9.

## Failed diagnostic: instruction-end drain in `advance_master_clock` (2026-11-20)

To test whether the one-clock deficit at the first port write is ares'
whole-instruction overshoot, `advance_master_clock` was extended with

```
while self.plan_pos < self.plan.len() { if self.drain_unit() == 0 { return true; } }
```

after the credit loop. The instrumented build is **not usable for the
measurement**: the APU advanced ~9x too fast (3,499,598 retired S-SMP clocks at
master clock 4,165,004, where the true count is ~397,650). The reason is that
`advance_master_clock` is re-entered from the parked/flush path, and each
re-entry now finishes whole instructions without consuming credit, so the
instruction remainder is drained repeatedly.

Reverted. The overshoot hypothesis for the 9-vs-10 clock difference is
therefore still *untested*; testing it needs the instruction-end drain to be
driven by the credit (one instruction per synchronize, not per re-entry), which
is exactly the `synchronize_to` shape that was measured earlier and produced the
Zelda/F-Zero exact family.

State at the end of this session: five titles exact at 600 frames, CV4 98.2383%,
SMAS 313,782, CT 416,143, MMX2 63.2951%; BS-X boots and is unit/integration
tested; ST018 unimplemented pending a reference-emulator decision.

## Concurrent three-thread rewrite: started (2026-11-20)

Decision: implement the CPU/SMP/DSP interleaving as three clock-ordered peers
behind a flag, keeping the verified per-access path as the default.

**First increment (this commit, no behaviour change):** the S-DSP advance is
extracted from instruction recording into a single `drain_dsp_to(ram, dsp,
dsp_clock, samples, target)` helper - ares' `Thread::synchronize(dsp)`. Output is
bit-identical (five titles 0 differing at 600 frames, CV4 98.2383%).

**Why this seam:** the DSP is currently advanced *while an instruction is being
recorded* (`RecordBus::step_clock` -> `step_dsp_clock`), before that
instruction's clocks are drained. That is why `dsp.ticks()` at a `$F4` read is far
ahead of the S-SMP clock, and why the DSP register-write phase measured
`+6 (mod 32)` micro-steps against ares. In ares the DSP is a peer: it advances
when its thread clock is behind, in clock order.

**Next increments, in order:**
1. Defer DSP register reads (`$F2`/`$F3`) like port reads, so they resolve at
   their access clock during the drain (`Pending::DspRead`), removing the need
   for the eager advance.
2. Call `drain_dsp_to` from `drain_unit` instead of from recording, so the DSP
   advances interleaved with the S-SMP's clock.
3. Measure: DSP write-phase vs `ARES_DSP_WRITES` (target: 0 mod 32), then the
   600-frame 8-title gate and the 212/212 baseline.

Risk: the five currently exact titles may regress during 1-2; the verified path
stays the default until the flag's gate is green.

## DSP at drain time: measured and rejected (2026-11-21)

The queued DSP-peer step was: defer `$F2/$F3` reads to their access clock and
move `drain_dsp_to` from instruction recording into `drain_unit`. The first half
turned out to be **already implemented**: the deferred-read path covers all of
`$00f0-$00ff` (`effective & 0xfff0 == 0x00f0`), so `$F2` and `$F3` already
resolve at their own access clock.

The second half was implemented behind `SNES_DSP_DRAIN` (record-time
`step_dsp_clock` becomes a no-op; `drain_unit` advances `smp_clock` by one
`SMP_SCALAR` per unit and calls `drain_dsp_to`; `smp_clock_at_position` returns
`smp_clock`). At 600 frames:

```
dkc 267,671 (58.15%)   zelda 103,119 (83.88%)   fzero 76,721 (88.01%)
metroid 47,532 (92.57%)   smw 48,722 (92.38%)   cv4 12,225 (98.09%)
```

So the deliberate one-micro-step lead between the DSP and the S-SMP
(`24 * DSP_SCALAR` being four fixed-point units less than `2 * SMP_SCALAR`) is
load-bearing: removing the eager advance changes the DSP sample phase relative
to every register write and breaks the currently exact titles. Reverted; the
default path is unchanged (five exact, CV4 98.2383%).

## Reference-harness status for ST018/BS-X verification (2026-11-21)

mesence is the only available ST018 reference, and it has a built-in headless
path: `--testRunner [lua script] [rom]`. Verification is blocked by the
environment, not by the code:

* .NET 10 was installed locally (`/tmp/opencode/dotnet`, 10.0.404) and the
  app launches, but the test runner still creates a GL context
  (`~/.cache/mesa_shader_cache` appears) and produces no output without a
  display server. `Xvfb`/`xvfb-run` are not installed and `sudo` requires a
  password, so a virtual display cannot be created here.
* The interop path (`MesenCore.so`: `InitDll`, `InitializeEmu(..., noVideo)`,
  `LoadRom`, `GetCpuState`) is exported, but `InitializeEmu` with null window
  handles dumps core, and this build does **not** export the capture API
  (`InitializeCapture`/`CaptureVideoFrameCount`/`CaptureVideoHash`), so there is
  no headless frame-stepping entry point.

What would unblock reference verification: a display server (install `Xvfb`,
or run the capture inside an existing X/Wayland session), or a mesence build
that exports `InitializeCapture` and accepts null handles.

Until then, ST018's correctness is verified only by construction (its boot,
handshake, rendering and 7 unit tests) and BS-X by its boot screen, deterministic
hash stream, MCC sequence and pack bus test.

## DSP phase prime scan: CV4 improves monotonically, DKC degrades (2026-11-21)

The DSP register-write phase measured `+6 (mod 32)` micro-steps against ares.
Priming the DSP catch-up by N micro-steps (`SNES_DSP_PRIME=N`, diagnostic only,
default 0) gives, at 600 frames:

```
prime  dkc            cv4
0      0 (100.0000%)  11,269 (98.2383%)
1      177,171 (72.30%)  11,250 (98.2412%)
2      196,379 (69.30%)  10,765 (98.3171%)
3      198,965 (68.90%)  10,126 (98.4170%)
4      197,318 (69.15%)   9,475 (98.5187%)
5      250,793 (60.79%)   2,301 (99.6403%)
6      288,158 (54.95%)   1,181 (99.8154%)
```

Two facts follow. First, CV4's divergence falls **monotonically** with the
offset (98.24% -> 99.82%), which is the signature of a slow phase drift between
the write stream and the samples, not of independent sporadic glitches. Second,
the titles require *different* phases (DKC is exact at 0, CV4 best at 6), so the
error is not a global DSP phase: it is the timestamp at which individual DSP
register writes land relative to the sample boundary, which varies per title
because each writes at different points in the 32-step cycle.

That rules out a constant phase correction and points at the write timestamp
itself: ares records a DSP write at the SMP clock of the access that performs it,
while our plan records the write at the *end of the access* after the DSP has
already been advanced to that same clock. The next experiment is therefore to
stamp writes with the access's *start* clock (or mid-access, matching
`SMP::wait`'s split) and re-scan.

The default path is unchanged (prime 0); the probe is diagnostic only.

## DSP write placement within the access: measured (2026-11-21)

`SNES_DSP_WRITE_AT=start|mid|end` (default `end`, selected once) places the
`$F3` register write before the access clocks, between the two halves, or after
them. 600 frames:

```
placement  dkc                  cv4
start      226,813 (64.54%)     11,379 (98.2211%)
mid        226,813 (64.54%)     11,379 (98.2211%)
end              0 (100.00%)    11,269 (98.2383%)
```

`start` and `mid` are identical because a half access rarely crosses a DSP
micro-step, and both regress DKC badly. The reference order (`wait`, then
`writeIO` → `dsp.write`) is therefore already what we implement, and the CV4
residual is not the placement of the write inside its access.

Combined with the phase-prime scan (a 6-micro-step prime improves CV4
monotonically to 99.82% while destroying DKC), the remaining difference is
between *which* DSP micro-step index a write receives, not where in the access
it is issued or a global phase. That is governed by the DSP's own catch-up
schedule (`DSP::tick`: `Thread::step(24)` then `synchronize(smp)`) against the
S-SMP access stream, i.e. by the two threads' relative clock accounting at the
moment of the write - the last unverified link in the audio chain.

## DSP write timestamps: bounded phase, not a rate drift (2026-11-21)

Comparing every `$F3` write over 560 frames of CV4 (`ARES_DSP_WRITES` vs
`DspTrace::writes`), 9,680 aligned pairs with zero addr/value mismatches:

```
absolute tick difference (ares - ours):  +6 in 7,901   -22 in 1,779
mod-32 difference:                         6 in 7,901    10 in 1,779
```

Only two values occur, and they differ by 4. Because the absolute difference is
*bounded* (it never grows), the S-SMP and DSP **rates are identical**; what
differs is the phase, which takes one of two values 4 ticks apart and flips at
specific writes. So the +6 (mod 32) offset measured earlier is a
**scheduling/phase** difference, not clock-accounting drift.

Two candidate mechanisms were tested and eliminated:

* the DSP catch-up boundary (`dsp_clock < smp_clock` vs `<=`,
  `SNES_DSP_LE`): bit-identical results - the equality case never arises;
* the write's position within its access (`start`/`mid`/`end`): the reference
  order (`end`) is best and already what we implement.

The two phase values are 4 ticks apart, which is exactly the fixed-point
deficit between `24 * DSP_SCALAR` and `2 * SMP_SCALAR` (10 units in ares'
2^63 base, 4 in our 2^62 base). That is now the single remaining suspect: the
fixed-point base changes where the "extra tick" fires, so the base and the
catch-up arithmetic must be reproduced together (base 2^63-1 **and** the
resulting deficit) rather than independently.

## The two DSP write phases correlate with nothing (2026-11-21)

For the 9,680 aligned CV4 `$F3` writes the phase (mod 32 = 6 or 10) was
cross-tabulated against every available discriminator:

```
phase by register:            (00,6) 6092  (00,10) 1481  (20,6) 1158  (20,10) 292
                              every other register also shows both phases
phase by ares clock.sample:   (0,6) 4129  (1,6) 3772  (1,10) 890  (0,10) 888
phase by sample parity:       (0,6) 5103  (1,6) 2798  (0,10) 1181  (1,10) 597
phase by our SMP delta:       (6,60) 5750  (6,160) 2078  (10,60) 1467  (10,160) 291
```

Both phases occur for every register, both parities, both `clock.sample`
values and every SMP-delta bucket, in roughly the same 82/18 ratio. So the
4-tick split is not tied to a particular write pattern - it is the residual of
the DSP catch-up arithmetic itself (`while dsp_clock < smp_clock`, stopping
with a sub-tick overshoot), whose two possible stops are 4 ticks apart.

The one structural difference still untested is ares' `DSP::tick` calling
`Thread::synchronize(smp)` *inside* the tick: the S-SMP can advance while the
DSP catches up, which changes the SMP clock the next catch-up is compared
against. Reproducing that requires the two threads to be mutually resumable
(the scheduler rework), and it is the next experiment.

## DSP catch-up without overshoot: measured and rejected (2026-11-21)

ares' `DSP::tick` overshoots the S-SMP clock (`Thread::step(24)` then
`Thread::synchronize(smp)`), and the synchronize pulls the S-SMP forward to the
DSP's overshot tick boundary - a leapfrog between the two threads. The half that
can be expressed in our model is "never overshoot"
(`while dsp_clock + DSP_TICK <= target`, `SNES_DSP_NOOVERSHOOT`):

```
no-overshoot: dkc 226,813 (64.54%)   cv4 11,379 (98.2211%)
              smas 313,782 (50.95%)  ct 416,143 (34.94%)
```

DKC regresses to exactly the same 64.5414% produced by the write-placement
start/mid variants, and CV4 is slightly worse than the default. So the
no-overshoot side is not it either; the missing half is the *pull-forward* of
the S-SMP to the DSP's tick boundary, which requires the S-SMP to be advanced
from inside the DSP catch-up (drain-time, mutually resumable threads) - the same
structural rework as the CPU/SMP pair.

Every parameter that can be varied without that rework has now been measured:
stop rule, cadence, latch ordering, concurrency, DSP drain time, write
placement, catch-up boundary, fixed-point base and overshoot policy. Reverted.

## Drain-time leapfrog: measured and rejected (2026-11-21)

The last structural candidate was implemented behind `SNES_LEAPFROG`:

* the record-time DSP advance is disabled;
* `drain_unit` advances `smp_clock` one clock per unit and runs the DSP to it;
* after an advance's credit is spent, the S-SMP is run on while
  `plan_pos >= plan.len() && smp_clock < dsp_clock`, i.e. it is pulled forward
  to the DSP's overshot tick boundary (ares' `Thread::synchronize(smp)`).

Measured at 600 frames:

```
dkc   0 differing but only 44,102 of 639,656 samples produced before timeout
cv4   0 differing but only 13,940 samples produced
smas  403,736 differing (36.88%)
```

The variant is both **far too slow** (the pull-forward loop re-enters the plan
machinery on every advance, roughly 10x slower) and **incorrect** (SMAS
regresses). Reverted.

That completes the measurement of every candidate reachable by changing the
scheduler's arithmetic or ordering. The five exact titles depend on the current
eager-advance model, and the remaining CV4/SMAS/CT/MMX2 difference needs a
faithful concurrency implementation (each thread able to suspend the other
mid-instruction), not another boundary rule.

## Where the DSP phase flips (2026-11-21)

Looked at the write sequence around a phase transition (CV4, writes 28-49). The
writes come in blocks of six (registers 00,00,00,00,20,00) spaced 31 DSP ticks
apart, with a long gap (~3,930-4,128 ticks) between blocks. The phase is
**constant within a block** and flips between blocks:

```
idx 32  reg 20  phase 10   ares 3558654  ours 3558676
        (gap ~4128 ticks)
idx 33  reg 00  phase  6   ares 3562782  ours 3562776   <- flipped
...
idx 38  reg 20  phase  6   ares 3562937  ours 3562931
        (gap ~3930 ticks)
idx 39  reg 00  phase 10   ares 3566867  ours 3566889   <- flipped back
```

So the 4-tick (1/8 sample) difference is not latched at a write: it
**accumulates during the long stretches where the S-SMP runs without touching
the DSP**, then stays fixed until the next such stretch. During those stretches
the DSP catches up over many accesses, and a small per-access rounding
difference between our catch-up and ares' accumulates to four micro-steps.

That is precisely the behaviour ares' leapfrog prevents: its DSP tick pulls the
S-SMP to the tick boundary on *every* tick, so the two clocks never drift apart
by more than a fraction of a tick, whereas our catch-up lets the overshoot
persist across an advance and lets rounding accumulate. Since the leapfrog
cannot be expressed without drain-time threads, the correct fix is the
concurrent three-thread driver; this measurement defines its acceptance test:
the phase must remain a single value across block boundaries, not just within a
block.

## Leapfrog with credit-charged pull-forward: hangs (2026-11-21)

Second implementation of the DSP/SMP leapfrog, addressing the previous
variant's 10x slowdown by charging the pulled S-SMP clocks against the carried
credit (`apu_credit -= den` per pulled unit, allowing it to go negative):

* record-time DSP advance disabled;
* `drain_unit` advances `smp_clock` one clock and runs the DSP to it;
* after the credit loop, `while smp_clock < dsp_clock { drain_unit();
  apu_credit -= den; }`.

Result: the emulator **hangs** (a 60-frame DKC run never finishes). The
pull-forward interacts with the park/flush path: a pulled unit can park at a
`$F4-$F7` read and return `Parked`, whose flush re-enters `advance_master_clock`
and pulls again, so the driver never returns to the S-CPU. Fixing that requires
the pull-forward to be part of the scheduler proper (so a parked thread is
resumed by the other thread rather than by re-entering the advance), i.e. the
full concurrent driver - not an add-on loop.

Reverted. Both leapfrog attempts are recorded: the first was ~10x slower and
wrong (SMAS 36.88%), the second hangs.

## Pull-flag leapfrog: the hang persists (2026-11-21)

The third leapfrog attempt threaded a `pull` flag through `ApuInput::Advance`
so the flush path (`flush_parked`) cannot re-trigger the pull-forward - the
diagnosed cause of the previous hang. It did **not** fix it: a 60-frame DKC run
still never reaches its statistics output, and the 600-frame runs produce no
audio file at all.

So the hang is not only the flush re-entry. A pulled unit that parks at a
`$F4-$F7` read leaves `waiting = Some(target)` with `target` beyond the S-CPU
clock, and every subsequent flush re-establishes the same condition; the driver
has no way to make the *other* thread reach that target because the S-CPU is
suspended awaiting a response from the very advance call that is pulling.

That is the architectural point the three attempts converge on: the SMP's
park/resume must be a scheduler state where the DSP (or CPU) thread resumes the
SMP directly, not a return value consumed by a driver loop. This is the
ground-up rewrite that has been authorised and is the next work item.

Reverted; the verified path is restored.

## Why every leapfrog variant deadlocks (2026-11-21)

`Apu::advance_master_clock` sets `cpu_master_clock = clock + clocks` (the *end*
of the S-CPU access being advanced) and the carried-credit loop stops the S-SMP
exactly at that target. `port_read_waiting()` is
`smp_clock_at_position() > cpu_master_clock`, so in the default per-access model
the S-SMP can never be ahead of the S-CPU's access end and the **park path is
effectively dead code** - it was written for ares' "SMP ahead of the CPU" case
that our cadence never produces.

That is why all four leapfrog attempts deadlock: they are the first code ever to
reach that path, and its driver protocol (`waiting`, `parked_clocks`,
`flush_parked`, and the flush in the `Done` arm) has never been exercised. The
flush is also unconditional in the default configuration
(`!mutual_scheduler() || self.clock >= target` is always true when mutual is
off), so it cannot be gated without rewriting the construct.

Consequently the leapfrog cannot be added as a flag on the existing driver: it
creates the park condition and immediately exercises an unvalidated protocol.
The rewrite must delete `waiting`/`parked_clocks`/`flush_parked` and make the
actor's need a scheduler request, exactly as planned.

## The leapfrog "hangs" were a livelock on the moving DSP clock (2026-11-21)

Root cause found by inspecting the pull loop rather than the park protocol:

```
while self.smp_clock < self.dsp_clock { drain_unit(); }
```

The DSP re-runs on every drained unit and stays one tick ahead, so the target
moves with the runner and the loop never terminates. Capturing the target
before the pull (`let pull_target = self.dsp_clock; while smp < pull_target`)
makes it terminate. At 600 frames this variant gives:

```
no-park, fixed target:   dkc 25.76%   cv4 89.14%   smas 42.27%
park, fixed target:      dkc 40.98%   cv4 89.14%   smas 38.71%
```

so the corrected leapfrog still measures worse than the default per-access
model (dkc 100%, cv4 98.24%). Three of the four previous "deadlocks" were this
livelock, not the park protocol; the earlier analysis calling the park path
dead code stands, but it was not what hung.

Reverted; the verified per-access model remains the default.

## CV4 first divergence localized to one pass of a `$F7` handshake poll (2026-11-21)

Starting from the differing right-channel sample (289423, where `V3VOLR=$03` is
written at DSP micro-step 11 in ares and 5 in ours) the write stream was traced
backwards. Two facts fell out.

### The phase flips only at KOFF (`$5C`) writes, alternating per block

For CV4 the `$F3` write phase takes exactly two values, `ares - ours = +6` and
`-22` (i.e. phase 6 vs 10 mod 32), and **every** transition between them happens
at a KOFF (`$5C`) write:

```
KOFF#1  ares 3558499  ours 3558521  d=-22
KOFF#2  ares 3558592  ours 3558614  d=-22
KOFF#3  ares 3562782  ours 3562776  d=+6    <- flips at a block boundary
KOFF#4  ares 3562875  ours 3562869  d=+6
KOFF#5  ares 3566867  ours 3566889  d=-22   <- flips back
```

The `KOFF(n) -> KOFF(n+1)` interval is 4190 vs 4162 DSP ticks (+28) and then
3992 vs 4020 (-28): the discrepancy **alternates and cancels over two blocks**,
so it is not a rate difference.

### It is exactly one pass of a 9-instruction, 56-SMP-clock loop

Aligning the two S-SMP instruction streams (`SPC_TRACE` vs `ARES_SPC_TRACE`)
across a long gap gives sequences that are identical except for **one extra
pass** of nine instructions (`$049F..$04B2`) in one or the other. Counting one
loop pass: 9 instructions, 56 S-SMP clocks = 28 DSP ticks - exactly the ±28
alternation above. Decoding the loop from APU RAM (the byte lengths match the
trace PC increments exactly):

```
$049F  EC FD 00   MOV Y, $00FD      ; Timer 1 output
$04A2  D0 53      BNE $04F7         ; timer non-zero -> leave the wait
$04A4  E9 F7 00   MOV X, $00F7      ; CPUIO3 ($2143 mirror)
$04A7  1E F7 00   CMP X, $00F7      ; re-read: detects a port change
$04AA  D0 F8      BNE $04A4         ; changed -> re-read
$04AC  3E 07      CMP X, dp$07      ; expected value A?
$04AE  F0 13      BEQ $04C3         ; yes -> exit
$04B0  3E 06      CMP X, dp$06      ; expected value B?
$04B2  F0 EB      BEQ $049F         ; yes -> wait another timer period
```

So CV4's audio driver waits on Timer 1 and then handshakes on `$2143`: the
S-SMP reads `$F7` twice per poll (`MOV X,$F7` then `CMP X,$F7`) and branches on
whether the value *changed* between the two reads.

### Consequence for the audio difference

One extra/missing poll pass shifts every following `$F3` write by 28 DSP ticks
(1/8 sample). When that shift moves a write across the `voice5(voice[3])`
(right-channel output) micro-step - which runs at micro-step 9 of the 32-step
sample - the new right-channel volume is used for a different sample. That is
the differing sample 289423: ares' `V3VOLR` write at step 11 lands *after*
step 9, ours at step 5 lands *before* it.

This is the same failure class already recorded for the IPL upload (`$FFDA`,
"one extra `$F4` poll"), now reproduced at instruction level in CV4 with a
Timer-1-gated `$F7` handshake. It is not DSP arithmetic and not the DSP/SMP
leapfrog: it is the S-SMP observing the S-CPU's port write on a different poll
pass.

### Acceptance test (sharper than the sample-level audio diff)

For CV4 and DKC simultaneously:

1. the number of passes of the `$049F..$04B2` loop between consecutive KOFF
   writes must match ares (today: +/-1, alternating), and
2. as a consequence, the `$F3` write phase must be a single value across
   write-block boundaries.

Both are cheap to check with `SPC_TRACE`/`ARES_SPC_TRACE` plus the write logs,
without listening to audio.

## S-SMP stop granularity: access-stop makes CV4 exact (2026-11-21)

Following the `$049F` finding, the stop rule was changed behind
`SNES_SMP_ACCESS_STOP`: after the carried-credit loop, finish the current
**access** (ares' `SMP::wait()` unit) and carry the overdraft as negative
credit, instead of stopping exactly on the credit clock.

### CV4 acceptance test: met

All 429 KOFF writes at 260 frames now match ares exactly (`d = 0`), against the
default's two-valued phase (`+6` 334 times, `-22` 95 times). The `$049F` loop
pass count matches ares too (136 in the measured gap, was 135).

```
default:      d in {+6, -22}   429 KOFF pairs      (phase two-valued)
access-stop:  d = 0            429 KOFF pairs      (phase single-valued)
```

### 600-frame audio matrix

| title | exact-credit (default) | access-stop | instruction-stop |
|---|---|---|---|
| dkc | 100.0000% | **100.0000%** | 70.7730% |
| zelda | 100.0000% | 99.7733% | **100.0000%** |
| fzero | 100.0000% | **100.0000%** | **100.0000%** |
| metroid | 100.0000% | 62.5180% | 62.4151% |
| smw | 100.0000% | 90.1613% | 50.2550% |
| cv4 | 98.2383% | **100.0000%** | 86.6173% |
| smas | 50.9452% | 50.9452% | - |
| ct | 34.9427% | 31.5413% | - |
| mmx2 | 69.2303% | 69.2303% | - |

**CV4 is bit-exact for the first time** under access-stop, and DKC/F-Zero stay
exact, so the CV4 objective ("single-valued phase for CV4 and DKC") is met. But
Zelda/SMW/Metroid/CT regress, and instruction-granularity overshoot (ares'
`Thread::Enter` yield point) is worse still (DKC 70.8%).

The whole difference is confined to `$F4-$F7` read resolution: the DSP write
phase is derived at record time, not at drain time, so the stop rule only moves
port reads. The two rules are each exact for a different subset, which means the
outstanding error is in how a port read is resolved relative to the S-CPU, not
in the stop instant alone: ares synchronizes the S-SMP only at `$2140-$2143`
accesses (`CPU::readAPU/writeAPU`) and pulls the S-CPU forward from inside the
S-SMP's own `SMP::step`. Implementing that direction (the S-SMP advancing the
S-CPU at its access boundaries, instead of the S-CPU advancing the S-SMP) is the
next step; the flags stay at their default (off) so the verified five-title path
is unchanged.

## ares synchronization direction: measured (2026-11-21)

Two further rules were added behind flags, following the access-stop result.

**`SNES_PORT_SYNC` - advance the S-SMP only at `$2140-$2143` accesses**
(ares `CPU::readAPU`/`writeAPU`), holding the credit otherwise. Result: the
S-SMP **starves** - CV4 produces no `KOFF` writes at all (`rc=0`, but zero
`$5C` writes), because the game rarely touches the S-CPU ports. So ares must
synchronize on more than port accesses.

**`SNES_INSTR_SYNC` - additionally flush at the S-CPU instruction end**
(ares' per-instruction `scheduler.synchronize()`), exact-credit stop. Result:
the run is normal (271,640 SPC instructions, same as default) but CV4's phase
is back to the **two-valued** `{+6, -22}` (334/95 over 429 KOFF pairs). So
moving the sync point from per-access to per-instruction with an exact stop does
not change the port resolution at all.

Together with the earlier matrix this pins the requirement: CV4's phase becomes
single-valued only when the S-SMP is allowed to run **ahead of the S-CPU clock**
(overshoot). The sync *point* is irrelevant; the *lead* is what matters. The
remaining question is the S-CPU pull-forward (`SMP::step` ->
`Thread::synchronize(cpu)`) which accompanies that lead in ares and which the
current driver cannot express mid-instruction; access-stop approximates the
lead without it (exact for dkc/fzero/cv4, wrong for zelda/metroid/smw).

All four flags default off, so the verified path is unchanged.

## Pull-forward + access-stop measured; the regression is the payback lag (2026-11-21)

Combining every expressible part of ares' model was measured at 600 frames:

* `SNES_INSTR_SYNC` (flush credit only at instruction ends / port accesses) +
  `SNES_SMP_ACCESS_STOP` (overshoot to the access boundary) gives CV4's phase
  `d=0` on all 429 KOFF pairs, and audio results **bit-identical to access-stop
  alone**:

```
dkc 100.0000%  fzero 100.0000%  cv4 100.0000%
zelda 99.7733%  metroid 62.5180%  smw 90.1613%
smas 50.9452%  ct 31.5413%  mmx2 69.2303%   baseline 212/212
```

So the instruction-end flush adds nothing once the overshoot exists.

* `SNES_NO_PARK` (never park an `$F4-$F7` read) changes **nothing at all**
  (identical percentages). The park path does not even fire, and is not what
  regresses the three titles.

### Why access-stop regresses Zelda/Metroid/SMW

The overshoot is paid back as negative `apu_credit`, which makes the S-SMP's
**plan replay lag the S-CPU** for the next window. A `$F4-$F7` read is therefore
replayed at a later wall-clock instant and observes CPU port writes that ares
would not yet have performed - ares instead pulls the *S-CPU forward*
(`SMP::step` -> `Thread::synchronize(cpu)`) so the two clocks keep their order.

That cannot be fixed by credit arithmetic: "SMP never lags" and "lead stays
bounded" are contradictory unless the **S-CPU clock itself advances**. So the
pull-forward is not expressible in `advance_master_clock` alone; it must move
the system master clock (and the PPU/HDMA/frame timeline with it) when the
S-SMP overshoots. That is a driver/timeline change, not a stop rule.

The evidence for this is that removing the park and moving the sync point both
leave the numbers byte-identical, while the three titles that don't use the
`$049F`-style handshake are the exact ones that regress: the only remaining
variable is the S-SMP's replay position relative to the S-CPU clock.

## S-CPU pull-forward implemented: it inflates the timeline (2026-11-21)

Implemented the pull directly: with `SNES_SMP_ACCESS_STOP`, do not pay the
overshoot back as negative credit; after each `advance`, if the S-SMP's master
clock is ahead of `Snes::clock`, run the S-CPU and its peripherals forward to it
(`advance_peripherals_inner(pull)`), i.e. ares' `SMP::step` ->
`Thread::synchronize(cpu)`.

Result: the emulated timeline inflates. At 600 frames DKC produces

```
329,952 stereo frames (expected 319,828, +3.2%)
master clock 221,176,233 (expected 214,119,xxx)
DKC audio 35.8003% identical
```

The pull advances `Snes::clock` and the PPU by a few clocks per bus access, but
the frame boundary does not re-anchor, so total clocks and DSP samples per frame
grow. ares avoids this because its CPU clock is re-anchored from the PPU
horizontal counter (`CPU::scanline`): a pull shifts *which instruction* runs at
a given clock, not the total number of clocks in a frame.

So the pull-forward is not "add clocks to `Snes::clock`". It needs the timeline
anchor: the S-CPU clock must be derived from the PPU/hcounter at each scanline,
with the S-SMP's lead expressed only in the *ordering* of instructions inside a
fixed clock budget. That is a scheduler/timeline redesign (the CPU suspended and
resumed at scanline boundaries), not a per-access adjustment.

Reverted; all flags default off.

## BS-X verified pixel-identical against ares (2026-11-21)

BS-X was previously only unit/source-tested and boot-checked. ares implements
the BS-X base cartridge (`Coprocessor/MCC`) and its headless capture path works,
so the BIOS can be compared directly. Procedure:

```
# ares (crop hash = same 256x224 pixel set our PRESENT_HASH uses)
ARES_PRESENT_HASH=/tmp/ares.hash tools/snes-oracle/ares-capture.sh \
    /var/home/Lordra/ai/refs/BS-X.bin 600 /tmp/ares-bsx

# ours
FRAMES=600 PRESENT_HASH=/tmp/ours.hash ./target/release/examples/snes-info \
    /var/home/Lordra/ai/refs/BS-X.bin <ipl.rom>
```

Result:

```
600-frame crop hash: ares 600  ours 600  compared 600  matching 600
first diffs: none
```

All **600/600 frames are pixel-identical** (FNV-1a-64 over the same 256x224
RGBA set), and a direct frame-buffer diff at frame 239 gives mean absolute
channel difference 0.0. This covers the MCC register file, the LoROM/HiROM
PSRAM map, BIOS/SRAM/Pack windows and the boot path.

Note: `ares-capture.sh`'s `--dump-trace` hash is the *full* 564x242 frame and
is not comparable to our crop hash; `ARES_PRESENT_HASH` emits the comparable
one (same crop the audio/frame oracles already use).

ST018 still cannot be verified this way: ares has `armdsp` (ST010/ST011) but no
ST018 implementation, and the mesence reference still needs a display server.

## Mutual scheduler probes: parking fires but cannot reorder the latch (2026-11-21)

Three results, all measured, all reverted.

**Parking against the read instant.** `SNES_MUTUAL` was changed to compare the
S-SMP clock with the S-CPU clock at the *start* of the advanced access (ares'
`Thread::synchronize(cpu)` instant) instead of the end. Parks now actually fire:
**447 in 60 frames** for CV4 (0 in the default model). But with
`SNES_SMP_ACCESS_STOP`, all nine titles are byte-identical to access-stop alone
(dkc/fzero/cv4 exact; zelda 99.7733%, metroid 62.5180%, smw 90.1613%, smas
50.9452%, ct 31.5413%, mmx2 69.2303%), and the master clock is unchanged
(21,411,778 at 60 frames), so there is **no timeline inflation**. The park
fires but changes nothing: the port latch holds the *latest* write, so resuming
the read later makes it see newer values, not the value as of the S-SMP clock.

**Timestamped port latches** (`SNES_PORT_STAMP`): S-CPU `$2140-$2143` writes are
queued with their master-clock instant and exposed only when the S-SMP's clock
reaches them, sampled at the deferred `$F4-$F7` read. This is the value-correct
half of the mutual model, but it **starves the handshake**: CV4 produces no
`KOFF` writes at all within 260 frames (`KOFF pairs 0`), so the driver never
progresses.

**Conclusion.** The mutual scheduler is not expressible by (a) choosing the
comparison clock, (b) parking, or (c) timestamping the latch in the current
single-driver architecture. It needs the S-CPU to be genuinely suspended
mid-instruction and resumed *while* the S-SMP continues, with the DSP sample
stream driven by the same reordered clock. That is the redesign; both halves
(CV4's exact phase and the S-CPU's fixed clock budget) are achievable
individually, but not simultaneously with add-on loops.

## Access-stop regressions are a handshake (port-read) effect, not DSP phase (2026-11-21)

Diagnosed the three titles that regress under `SNES_SMP_ACCESS_STOP`.

### First divergent sample (600 frames, access-stop)

```
title     first differing sample   stereo frame   first run   local best alignment
metroid   394,634                 197,317        1 sample    offset 0 (1201/2200), +/- equal
smw       280,498                 140,249        244 samples offset 0 (1956/2200)
zelda      90,136                  45,068        122 samples offset 0 (752/2200; noisy)
```

The best local alignment is **offset 0**, not a shifted stream, so it is not a
continuous timing/drift effect: these are discrete events.

### Whole-run clocks are (near) identical

```
title     spc instructions (default / access-stop)  spc units      master clock
smw       2,651,439 / 2,651,410  (-29)              20469021/22   214389600 both
zelda     2,601,410 / 2,601,408   (-2)              20469022 both 214389610 both
metroid   2,518,198 / 2,518,188  (-10)              20469021/22   214389602 both
```

So the S-SMP clock budget is unchanged and only a handful of instructions (a
few poll/handshake iterations out of ~2.6 million) differ. That is a
**port-read / handshake outcome** difference, which then shifts the DSP write
stream at those points and cascades - not a stop-position drift.

### DSP phase derived from the S-CPU clock: neutral

`SNES_DSP_CPU_PHASE` sets the S-DSP catch-up target from the invariant S-CPU
master clock instead of the S-SMP drain position. On the default path it is
neutral - dkc/zelda/fzero/metroid/smw stay 100%, cv4 stays 98.2383% - and under
`SNES_SMP_ACCESS_STOP` it changes **nothing** (identical percentages). Reason:
the target is clamped with `.max(smp_clock)`, and under access-stop the S-SMP
has overshot the S-CPU, so the clamp keeps the S-SMP value and the override is
bypassed exactly where it would matter.

### Determination

The three regressions are caused by `$F4-$F7` read/ handshake outcomes changing
under the coarser stop rule, not by the DSP write phase and not by drift. The
default per-access exact-credit rule is the one that matches ares for
zelda/metroid/smw, while access-stop matches ares for cv4/dkc/fzero. Resolving
both requires the S-SMP to sample the port at its own clock while the S-CPU is
suspended - the mutual driver - which is why every flag combination so far
splits the titles.

## ST018 reference capture unblocked: mesence runs headless (2026-11-21)

The earlier blocker ("mesence needs a display server") was wrong: the
`mesen-refcap` interop harness runs headless once the ST018 firmware is in its
`Firmware` folder. ares' `cage`-based headless setup was never needed.

```
mkdir -p /var/home/Lordra/ai/refs/mesence/refcap/Firmware
cp "<ares>/System/Super Famicom/st018.rom" \
   /var/home/Lordra/ai/refs/mesence/refcap/Firmware/
cd /var/home/Lordra/ai/refs/mesence/refcap
./mesen-refcap "<shogi rom>" 300 /tmp/mes-st18     # video+audio FNV, raw frame
```

Without `Firmware/st018.rom` mesence renders the S-CPU side only (all-black
frames, constant video hash); with it the game draws (57,336 non-black pixels at
frame 300).

### Comparison against our ST018 implementation

Mesence dumps 256x239 RGBA; our PPU frame is 256x224. The consistent alignment
is `ours[y] = mesence[y + 4]` (verified by an exact-pixel search over offsets).

```
ours FRAMES=101 vs mesence frame 100:
  differing pixels: 760 of 53,568
  bounding box:     x 4..251, y 205..219   (only the bottom status rows)
  playfield rows 4..204: pixel-identical
```

So the ST018-driven playfield (board, pieces, all sprites/BG it controls) is
**pixel-identical** to mesence; the residual is confined to the bottom status
bar, which animates/blinks between frames (the frame index aligns as mesence N
<-> ours N+1, so some of it is phase). Whole-frame parity would need a frame
where the status bar is static.

This replaces the previous "ST018 unverified" status: a reference comparison is
now possible, and the coprocessor's rendered output matches.

## ST018 verified: whole frame 100.0000% pixel-identical to mesence (2026-11-21)

The earlier "playfield only" result was a temporal-alignment artefact: the frame
index maps as **mesence N <-> ours N+3** (our ST018 executes slightly ahead), so
the bottom status bar looked animated. With the correct alignment:

```
mesence frame 100  vs  ours frame 103, vertical offset oy = 7
  identical pixels: 57,344 / 57,344   (100.0000%)
  mesence non-black pixels in frame: 57,344
```

The whole 256x224 frame - playfield and bottom status bar - is **pixel-exact**.
(Black boot frames 59-63 also match trivially; frame 100 is fully drawn.)

Capture commands:

```
mkdir -p refs/mesence/refcap/Firmware
cp "<ares>/System/Super Famicom/st018.rom" refs/mesence/refcap/Firmware/
cd refs/mesence/refcap && ./mesen-refcap "<shogi rom>" 100 /tmp/mes-st18-100
FRAMES=103 ST018_ROM="<...>/st018.rom" FRAME_OUT=/tmp/our-st18-103.frame \
  ./target/release/examples/snes-info "<shogi rom>" ipl.rom
```

So both coprocessor features are now reference-verified: BS-X 600/600 frames
against ares, ST018 a fully-drawn frame 100% against mesence.

## Unclamped CPU-derived DSP phase makes SMAS exact (2026-11-21)

The earlier `SNES_DSP_CPU_PHASE` test clamped the target with `.max(smp_clock)`,
which makes it a no-op exactly under overshoot. Retested **unclamped**: the DSP
catch-up target is the S-CPU clock converted to the shared fixed-point base, even
when the S-SMP is ahead.

600 frames (639,656-sample window):

| title | default | unclamped CPU phase | access-stop + CPU phase |
|---|---|---|---|
| dkc | 100.0000% | **58.2915%** | 58.2915% |
| cv4 | 98.2383% | 98.0890% | 98.3097% |
| smas | 50.9452% | **100.0000%** | **100.0000%** |
| zelda | 100.0000% | - | 93.0372% |
| fzero | 100.0000% | - | 88.0415% |
| metroid | 100.0000% | - | 62.5025% |
| smw | 100.0000% | - | 91.1753% |
| ct | 34.9427% | - | 31.4222% |
| mmx2 | 69.2303% | - | 69.2303% |

**SMAS is bit-exact for the first time**, and it stays exact with or without the
access-stop rule. But the same shift breaks DKC (58.3%) and the other exact
titles, so the untested-clamp variant is a real phase shift of the DSP write
stream, not a free improvement: DKC/Zelda/F-Zero/Metroid/SMW constrain the
default phase and SMAS constrains a shifted one.

This is the strongest evidence yet that the residual is the **DSP register-write
phase** (a few micro-steps), and that the correct model must derive that phase
from the SMP's access instant in a way that matches ares for all titles - the
open item. The flag stays default-off.

## Global DSP phase offsets cannot unify DKC and SMAS (2026-11-21)

`SNES_DSP_PRIME` shifts the S-DSP catch-up target by a constant N micro-steps.
Swept at 300 frames (319,828-sample window; SMAS first diverges at sample
224,916 and CV4 at 578,847, so this window covers SMAS but not CV4):

```
prime   0       2       4       6       8      10      12      16      24      31
dkc   100.00%  83.18%  80.22%  57.70%  56.18%  53.88%  53.70%  53.54%  52.73%  50.37%
smas   70.41%  70.41%  70.41%  70.41%  70.41%  70.41%  70.41%  70.41%  70.41%  69.34%
cv4   100.00% 100.00% 100.00% 100.00% 100.00% 100.00% 100.00% 100.00% 100.00% 100.00%
```

Conclusions:

* **The default DSP phase is exactly right for DKC** - the only offset that
  gives 0 differing is 0, and every other offset degrades monotonically. So a
  "phase rule" cannot be introduced to help SMAS without breaking DKC.
* **SMAS is phase-insensitive**: its diff is 94,638-94,640 samples for every
  offset from 0 to 24. So the SMAS residual is *not* the DSP write phase, and the
  SMAS-exact result under the unclamped CPU target comes from that target's
  *variable advance pattern*, not from a constant phase shift.
* CV4 is exact through 300 frames for every offset; its divergence begins at
  sample 578,847 (stereo frame 289,423) - the `V3VOLR` write already analysed.

So the three remaining titles have three distinct causes and no single phase
parameter addresses them: CV4 is a single late handshake event, SMAS is not
phase-related, and DKC pins the phase exactly.

## SMAS characterised: identical write sequence, two-valued write timing (2026-11-21)

Compared our S-DSP register-write stream against ares' (`ARES_DSP_WRITES`) for
230 frames, around the first audio divergence (DSP sample 112,458).

```
write counts:   ares 15,398   default 15,396   SNES_DSP_CPU_PHASE 15,396
first (reg,value) divergence:  NONE (both variants)
```

The register/value stream is **identical to ares** - there is no control-flow
difference at all in SMAS. The only difference is *when* each write lands
relative to the DSP clock:

```
tick delta (ares - ours):
  default      +3 : 9,418 writes     -5 : 5,963 writes   (11: 15)
  cpu-phase    +8 : 9,201 writes      0 : 5,848 writes   (others: a few)
```

So SMAS has the same **two-valued write-timing** phenomenon already measured for
CV4 (`+6 / -22` mod 32), not a value race, not a sample-count problem, and not a
constant phase. The two values differ by 8 ticks = 16 S-SMP clocks. Around the
first audio divergence (sample 112,458) the delta flips from -5 to +3 (default)
or 0 to +8 (cpu-phase).

`SNES_DSP_CPU_PHASE` makes SMAS exact by shifting this two-valued pattern by
+5 ticks; that is why the constant `SNES_DSP_PRIME` sweep could not reproduce it
(prime shifts the drain target, but the write's sample/micro placement still
rounds on the two-valued pattern), and why the same shift breaks DKC.

Open item: remove the two-valued oscillation itself (our write delta should be a
single value), rather than shifting it.

## The DSP write oscillation flips only at KOFF, constant within a block (2026-11-21)

Analysed the per-write delta (ares dspTicks - our micro-step) across the whole
SMAS trace:

```
delta values:  +3 (9,418 writes)   -5 (5,963)   +11 (15, first write only)
delta changes: 797, and EVERY change (after the first) is at a $5C (KOFF) write
```

The delta is **constant inside each audio block and flips only at the block's
KOFF write**. The two values differ by 8 DSP ticks = 16 S-SMP clocks, i.e. the
same structure measured for CV4 (there 56 S-SMP clocks), just smaller.

Since SMAS's (reg,value) write sequence is byte-identical to ares, the flip is
not a control-flow difference in the writes: between blocks the S-SMP runs an
inter-block wait (a timer/delay loop that produces no DSP writes), and by the
next block our S-SMP clock differs from ares by +/-16 clocks, alternating.
That shifts all of the block's writes by the constant offset, which is enough to
move envelope/volume updates across sample boundaries and change the audio.

So the residual is a **block-boundary timing difference inside the S-SMP's
inter-block wait**, the same class as CV4's Timer-1-gated `$049F` handshake, not
a DSP arithmetic or phase-constant problem.

## SMAS block-boundary difference is one Timer-0 zero-wait iteration (2026-11-21)

Instrumented the first flip (DSP samples 24,363 -> 24,427). Over that interval:

```
S-SMP instructions:  ours 517   ares 519
S-SMP clocks:        ours 4,086 ares 4,102 (= 2,051 DSP ticks x 2)
```

The whole difference is **one extra iteration of a 2-instruction loop** in ares:

```
$0576  EC FD 00   MOV Y, $00FD     ; T0OUT (Timer 0 4-bit counter, read clears it)
$0579  F0 FB      BEQ $0576        ; loop while zero
```

One iteration is 16 S-SMP clocks, exactly the gap. So the S-SMP is waiting for
Timer 0 to become non-zero, and ares observes it one iteration later.

The timer implementation itself checks out against ares line by line:

* frequencies 128/128/16 for T0/T1/T2 (ares `Timer<128>/<128>/<16>`);
* `TIMER_WAIT = [2,4,8,16]` (ares `timerWaitStates`);
* `$FD-$FF` returns `stage3` and clears it (ares `readIO`);
* `$F1` rising enable edge resets `stage2/stage3` (T2 on non-rising, ares);
* `$F0` re-synchronizes stage 1; stage2 == target wraps and increments stage3.

So the 16-clock offset is **not** a timer bug: it is accumulated earlier. Over
the 230-frame run our S-SMP executes 1,017,010 instructions vs ares' 1,017,343
(333 fewer, ~1.4/frame), and those small divergences shift the timer's stage-0
phase, which then changes the zero-wait iteration count at a block boundary and
moves the whole block of DSP writes.

The remaining task is therefore the cumulative S-SMP timing agreement (the same
port/handshake-order issue), observed here through the Timer-0 wait.

## Full S-SMP stream alignment: 799 events, no control-flow divergence (2026-11-21)

Aligned the complete S-SMP instruction streams (ours 1,017,010, ares 1,017,343
entries, 230 frames) with a +-8-instruction resynchronising diff:

```
total divergence events:  799
  IPL upload handshake:     3   ($FFD6/$FFD8, $FFDA/$FFDC/$FFE9)
  Timer-0 zero-wait loop: 796   ($0576 MOV Y,$00FD / $0579 BEQ $0576)
  other:                    0
final alignment: ours exhausted at 1,017,010; ares at 1,017,011
```

So the **program is identical**: there is no control-flow divergence at all.
The previously reported "333 fewer instructions" was a trailing capture-length
artefact - after alignment the streams differ by at most one instruction, and the
remaining 332 ares entries are simply capture time beyond our run.

The 796 Timer-0 events are one-iteration insertions that alternate sides (ours
extra, then ares extra, then ours...), i.e. the block-boundary phase oscillation
already measured, not accumulating drift. The 3 IPL events are the earliest
divergence (instruction 4,299) and are the documented `$FFDA/$FFDC` upload
handshake.

Next: resolve the 3 IPL handshake divergences (the earliest cause) and see
whether removing that SMP-clock perturbation makes the Timer-0 wait parity agree.

## Exact synchronize(smp) at port writes: fixes the IPL events, regresses audio (2026-11-21)

Implemented `SNES_PORT_WRITE_SYNC`: before an S-CPU `$2140-$2143` write lands,
run the S-SMP to the exact write clock (`synchronize_to_master_clock`, consuming
the extra clocks from credit), which is ares' `CPU::writeAPU` ->
`synchronize(smp)`.

Effect on the instruction stream: the first IPL divergence (instruction 4,299,
`$FFD6/$FFD8`) disappears and total alignment events fall 799 -> 736, earliest
remaining event now instruction 72,244 (`$FFDA/$FFDC/$FFE9`).

Effect on audio (600 frames):

```
smas     313,782  (50.9452%)   unchanged
dkc            0  (100.0000%)  unchanged
cv4       69,470  (89.1395%)   worse (was 11,269)
zelda    231,894  (63.7471%)   much worse
metroid  238,681  (62.6860%)   much worse
smw       16,177  (97.4710%)   worse
fzero    463,471  (27.5437%)   much worse
```

So the IPL handshake divergence is **not** the root cause: removing it shifts
the S-SMP clock at every port write and damages six titles without helping SMAS.
The ordering ares gets comes from the mutual pull (S-SMP resumes the S-CPU), not
from a one-directional synchronise at the write. Reverted.

## ARES_THREAD_LOG: the S-CPU leads the S-SMP, it does not lag (2026-11-21)

Captured ares' per-thread clocks (`ARES_THREAD_LOG`, gated to dspTicks
4,558,000-4,561,500, 270 frames of SMAS) and analysed the CPU/SMP interleaving.

```
log lines                    20,933
lines where the S-SMP clock changed   3,509
lines where the S-CPU clock changed  17,423
```

The S-CPU's clock changes about **5x as often** as the S-SMP's, and the S-SMP
advances in 1-cycle increments while the CPU clock is static. So in ares the
**S-CPU runs ahead and the S-SMP catches up in bursts** - the CPU is the
time master during ordinary execution. The absolute lead depends on the
CPU/SMP frequency used for the conversion (the logged clocks are per-thread
scaled), so it cannot be read off exactly here, but the direction is
unambiguous.

This is the opposite of the "DSP-master, S-SMP pulls the S-CPU forward"
hypothesis: during game execution the CPU is not being pulled forward by the
S-SMP; the S-SMP is following the CPU. Our default architecture (CPU advances
the S-SMP by carried credit) already has that direction, which is consistent
with the five exact titles.

So the mutual-scheduler rewrite is not the missing piece for normal execution;
the residual remains the DSP register-write placement relative to the S-SMP
access (the two-valued block timing), not the CPU/SMP lead.

## Timer implementation audit vs ares (2026-11-21)

The SMAS block-boundary difference is one Timer-0 zero-wait iteration
(`$0576 MOV Y,$00FD / $0579 BEQ $0576`). The timer code was audited line by line
against ares `SMP::Timer<Frequency>`:

```
field            ares                 ours            equivalent?
stage0           n8 (wraps at 256)    u16             yes (stage0+clocks < 256
                                                        for Frequency 128/16 and
                                                        timer clocks <= 16)
stage1           n8, ^=1              bool            yes
stage2           n8, ++ then ==target u8, wrapping_add yes
stage3           n4, ++ (wraps 16)    u8, (x+1)&0x0F  yes
line.increment   on line 1->0 only    same            yes
frequencies      128/128/16           128/128/16      yes
timer wait       {2,4,8,16}           TIMER_WAIT same yes
read $FD-$FF     stage3 then clear    same            yes
$F1 reset        rising edge (T2 on non-rising)       yes
$F0 resync       synchronizeStage1   same            yes
```

So the timer itself cannot explain the one-iteration difference: the phase must
differ because the **clock deltas fed to `timer.step` differ somewhere**, or
because the total number of timer clocks per unit of S-SMP time differs.

Concrete next step (bounded, but requires rebuilding the local ares): add a
timer-transition log to ares (`stage3` increments with the SMP clock) and to our
`Timer`, then diff the transition instants over a few frames. That will show the
first clock at which the two timer phases diverge and therefore which access
(and wait-state selector) differs.

## Timer transitions proven identical to ares; the difference is the read instant (2026-11-21)

Added a timer-transition log to both emulators (ares hook `ARES_TIMER_LOG`,
ours `SNES_TIMER_LOG`), recording each Timer-0 `stage3` increment with the DSP
tick and the cumulative timer-clock count.

```
transitions:  ares 119   ours 119   (ares has one extra before its longer capture)
DSP ticks:    ares [779521, 781569, 783617, ...]
              ours [779521, 781569, 783617, ...]   identical
timer clocks: ares [1559040, 1563136, 1567232, ...]
              ours [1559040, 1563136, 1567232, ...] identical
first differing transition: none
```

So the S-SMP timer is **exactly** ares (same logic, same instants on both the DSP
tick and timer-clock axes). The Timer-0 zero-wait loop iteration difference is
therefore not caused by the timer: the `$FD` read must be sampling `stage3` at a
slightly different **S-SMP execution instant** relative to the transition.

That makes the remaining error a pure S-SMP read-timing/position issue (the plan
replay/credit rounded relative to the timer), by a few clocks that alternate -
not timer arithmetic, not DSP phase, not the timer.

Note: the ares timer hook is local to `/var/home/Lordra/ai/refs/ares` and is not
committed; the tree here is unchanged.

## `$FD` read instants: a constant +22 timer-clock offset plus boundary spikes (2026-11-21)

Instrumented every `$FD` (T0OUT) read with the cumulative timer-clock count in
both emulators (`ARES_READ_LOG` / `SNES_READ_LOG`), 60 frames of SMAS:

```
read count:  ares 21,426   ours 21,307
read period: 16 timer clocks (identical)
timer-clock delta (ares - ours):
     +22 : 21,156 reads      <- the dominant, constant offset
     +828 / +1982 / +1738 ... : single reads at block boundaries (238 changes)
```

So relative to the timer, ares' `$FD` read happens **22 timer clocks later** than
ours, from the very first read, while the timer transitions themselves are at
identical cumulative positions. The read period is identical (16), so the offset
is not drift: it is a constant phase difference that spikes by ~+806 (one extra
poll = 16 timer clocks? no - ~806 clocks) at block boundaries and returns.

The +22 is present at the *first* read, so it is introduced during boot/IPL, and
22 = 11 accesses of the 2-clock internal timer wait. That points at a difference
in which accesses step the timer early in the run (the deferred-I/O read path:
the access's clocks are counted in the plan but the timer step happens once at
resume), rather than at the timer or the DSP.

Next: find the 11 accesses whose timer steps differ before the first `$FD` read.

## First timer-step divergence: the IPL `$F4-$F7` split-read loop (2026-11-21)

Logged every timer step (`cumulative clocks`, `step size`, S-SMP PC) in both
emulators (`ARES_ACCESS_LOG` / `SNES_ACCESS_LOG`).

Frame 1 is identical (17,122 accesses, no divergence). The **first divergence is
access 19,170** (timer clock 34,914), inside the IPL upload handshake:

```
k=19164 ares[34903,1] ours[34903,1]     (PC FFD8)
k=19166 ares[34906,2] ours[34906,2]     (ares FFD9, ours FFD8)
k=19168 ares[34910,2] ours[34910,2]     (FFDA)
k=19170 ares[34914,2] ours[34913,1]     <- ares steps 2, ours steps 1
k=19171 ares[34916,2] ours[34914,1]
k=19172 ares[34917,1] ours[34916,2]     <- the 1 and 2 swap
k=19173 ares[34918,1] ours[34918,2]
k=19174 ares[34920,2] ours[34920,2]     <- totals re-converge
```

So the cumulative timer totals re-converge, but the **step sizes are reordered**:
ares applies the 2-clock step before the 1-clock step, ours applies 1 then 2.
The location is `$FFD6 MOV Y,$F4` / `$FFD8 BNE` - the `$F4-$F7` split read, whose
second half (`wait` after `readIO`) is applied one clock early by our deferred
path: the whole read (both halves) is executed at `resume_at = clocks/2`, so the
second half's timer/DSP step happens one plan unit before ares.

This reordering is the earliest divergence and occurs inside the split-read
handling, not in the timer. Fixing it (apply the second half when the plan step
completes) is the next concrete edit.

## Deferring the split-read second half is behaviourally neutral (2026-11-21)

Implemented the identified reordering (`SNES_SPLIT_HALF`): the `$F4-$F7` split
read's second half (`wait` after `readIO`) is stored in `pending_split` and
applied when the plan step completes instead of at `resume_at`, with the S-DSP
and timer stepped at the same instant as ares.

600-frame result - **identical to the default on every title tested**:

```
smas     313,782 (50.9452%)      dkc   0 (100.0000%)
cv4       11,269 (98.2383%)      zelda 0 (100.0000%)
metroid  0 (100.0000%)
```

So the 2-vs-1 timer-step reordering seen at access 19,170 does not affect the
audio: the second half lands inside the same two-clock window either way and the
DSP/timer delta (one clock) is below the granularity that changes any write or
sample. The first *access-level* divergence is therefore not a cause, only a
symptom of the same sub-clock ordering.

Reverted; all flags default off. The residual remains the block-boundary DSP
write timing, now with the timer, DSP phase, CPU/SMP lead, split-read ordering
and IPL handshakes each individually measured and excluded (or neutral).

## Block-boundary trace: one S-SMP-clock offset, timer clocks = 2 x DSP ticks (2026-11-21)

Logged, for every `$F3` write, the DSP micro-step (ours) / `dspTicks` (ares) and
the cumulative timer-clock count, across the first KOFF boundary (55 frames):

```
write sequence: identical (704 writes, no reg/value mismatch)

idx 15 sample 24363 reg 5C  ares[ticks 779632 tc 1559262]  ours[micro 779637 tc 1559272]
                            dtick -5   dtc -10
idx 20 sample 24427 reg 5C  ares[ticks 781683 tc 1563364]  ours[micro 781680 tc 1563358]
                            dtick +3   dtc +6

dtc distribution:   +6 (496 writes)  -10 (193)  +22 (15)
dtick distribution: +3 (496)         -5 (193)   +11 (15)
```

Two facts:

1. **`dtc = 2 * dtick` exactly**, for every write. The S-SMP timer clocks and the
   DSP ticks are locked (one DSP tick = two S-SMP clocks), so there is a single
   underlying quantity: the S-SMP clock at the write differs from ares by
   `+3/-5` DSP ticks = `+6/-10` S-SMP clocks.
2. Across the KOFF boundary between idx 19 and idx 20 (a gap of 59 samples) that
   offset changes from `-10` to `+6`: a **16 S-SMP-clock** flip. That 16 clocks
   is exactly one iteration of the `$0576/$0579` Timer-0 zero-wait loop (2
   instructions x 8 clocks). So the whole residual is that single loop's
   iteration count.

Combined with the earlier measurements (timer transitions identical, `$FD` reads
offset by a constant 22 timer clocks, timer logic exact), the remaining
mechanism is: the S-SMP's `$FD` read/timer phase is offset, so the zero-wait loop
exits one iteration earlier or later, which shifts the following block's writes
by 16 S-SMP clocks (= 8 DSP ticks). Every other candidate has been excluded.
