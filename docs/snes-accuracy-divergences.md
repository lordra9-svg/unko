# Accuracy divergences — SNES

This ledger records every known place where `unko-snes` does not yet match the
chosen reference (real hardware, then ares, then a public test ROM). It is the
SNES counterpart to `docs/accuracy-divergences.md` and follows the charter in
`docs/accuracy.md`.

Current snapshot: **2026-11-17**, 65C816 + SPC700 + APU + S-DSP + PPU + DMA + HDMA.

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

Ours starts the firmware **~1,350 clocks later**. The mechanism is now clear:
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
