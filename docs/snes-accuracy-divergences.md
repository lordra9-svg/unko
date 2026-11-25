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

## Loop exit vs Timer-0 transition: read grid is phase-shifted (2026-11-21)

Logged every `$FD` read with the S-SMP instruction index and cumulative timer
clocks, and compared with the Timer-0 transition.

```
Timer-0 transition:      tc 1,559,040   (identical in both)
ares  reads around it:   1,558,976 / 1,558,992 / 1,559,008 / 1,559,024 / 1,559,040* / ...
ours  reads around it:   1,558,986 / 1,559,002 / 1,559,018 / 1,559,034 / 1,559,050* / ...
                                                   (* = first non-zero stage3 = loop exit)
```

So **both loops exit at the first read at/after the transition**, but the read
grids are phase-shifted: ares' exit read lands *exactly on* the transition
(1,559,040) while ours lands 10 timer clocks after (1,559,050). Because the reads
occur every 16 clocks, a phase offset of 10 clocks means one side can have an
extra read between the previous sample and the transition - which is the
one-iteration difference that then shifts the block by 16 S-SMP clocks.

The timer itself is exact (transition at the same tc). The difference is the
**phase of the `$FD` read grid**, i.e. the cumulative timer clocks accumulated by
the time the loop starts, which is set by every prior access's timer step.
(ares' `oracleEventSeq` is not incremented in this build, so the program-position
axis could not be compared directly; the timer-clock axis was used instead.)

This closes the localisation: exact timer, identical transitions, but a read-grid
phase difference of 10 timer clocks at the exit. The fix belongs in the
access-level timer-step accounting that establishes that phase (the only place a
net offset can be introduced), not in the timer or the DSP.

## Instruction-boundary timer clocks: first divergence at instruction 4,299 (2026-11-21)

Logged the cumulative timer clocks at every S-SMP instruction boundary in both
emulators (`ARES_INSTR_LOG` / `SNES_INSTR_LOG`), 50 frames.

```
first cumulative-timer difference: instruction 4,299
  k=4298 ares[4299, 34904, FFD8]  ours[4298, 34904, FFD8]
  k=4299 ares[4300, 34912, FFD6]  ours[4299, 34908, FFDA]   <- PC diverges here
  k=4303 ares[4304, 34932, FFDE]  ours[4303, 34932, FFE2]   (clocks re-converge)
  k=4304 ares[4305, 34938, FFE0]  ours[4304, 34946, FFE4]   (ours 8 ahead)
```

So the **first** instruction-level divergence is exactly the IPL `$FFD6`
(`MOV Y,$F4`) poll: ares executes the `$F4` read + `BNE` once more, ours falls
through to `$FFDA`. In other words, at that read ours already sees the S-CPU's
`$2140` write while ares still sees 0.

Every later effect - the cumulative timer-clock oscillation, the `$FD` read-grid
phase (10 clocks), the Timer-0 zero-wait iteration, and the +-8 DSP-tick write
placement - cascades from this single early port-read visibility difference.

The chain is now closed end to end:

```
instruction 4,299: ours sees $2140 write one poll early
  -> S-SMP clock offset appears, oscillates (timer clocks follow it, 2x)
  -> $FD read grid phase-shifted (10 timer clocks at the loop exit)
  -> Timer-0 zero-wait loop runs one iteration more/less
  -> whole block of $F3 writes shifted by 8 DSP ticks (=16 S-SMP clocks)
  -> envelope/volume writes cross sample boundaries -> audio differs
```

And the attempted fix for the first link (exact `synchronize(smp)` before port
writes) does remove the divergence but shifts the S-SMP clock globally and
regresses six titles. So the correct resolution is the port-sample ordering
(read must sample the latch as of the read instant while the S-CPU is not run
past it), which our single-driver architecture cannot express without the mutual
driver - consistent with the earlier mutual-scheduler conclusion.

## Latch-as-of-read-clock deadlocks the boot (retest, 2026-11-21)

Retried the targeted port-sample ordering: S-CPU port writes are timestamped and
exposed to the S-SMP only when its replay clock reaches the write instant, so a
deferred `$F4-$F7` read samples the latch as of its own clock (`SNES_PORT_STAMP`).

Result: the machine stalls. At 60 frames SMAS executes 227,335 S-SMP
instructions (279,350 normally), the PPU stays dark (brightness 0) and the boot
does not progress.

Cause: during the IPL upload the S-SMP waits for a `$2140` write, the S-CPU
waits for the S-SMP's `$2141` acknowledgement, and hiding the write from the
S-SMP closes the loop - the S-SMP cannot advance (the CPU only grants it
credit) and the CPU cannot receive the ack. ares avoids this because the DSP
micro-step pulls the S-SMP forward, which in turn pulls the S-CPU; that mutual
pull is what breaks the deadlock.

So "sample the latch as of the read clock without shifting the S-SMP clock" is
not expressible in the current driver: hiding the write requires a forward
pull that the driver cannot perform. This is the same architectural conclusion
reached from the opposite direction earlier, now confirmed by a direct test of
the exact requested change. Reverted.

## Mutual scheduler as specified: budget preserved, `$FFD6` divergence remains (2026-11-21)

Implemented exactly the requested configuration behind the existing flags:

* `SNES_SMP_ACCESS_STOP` - access-boundary overshoot creates the S-SMP lead;
* `SNES_MUTUAL` - the port-read park compares against the S-CPU clock at the
  read instant (window start), so parks actually fire;
* the driver services a parked read by resuming the **S-CPU coroutine with real
  instructions** (no synthetic `advance_peripherals` clocks).

Measured (55 frames, SMAS):

```
stereo frames produced: 29,277   (expected ~29,317)   -> per-frame budget intact
first S-SMP stream divergence: instruction 4,299, ares extra ['FFD6','FFD8']
```

So the frame clock does **not** inflate (the earlier +3.2% was the synthetic
`advance_peripherals` pull, now absent), but the `$FFD6` divergence is **still
present**: the mutual pull changes *when* the S-SMP runs, not what the latch
contains. Ours still observes the S-CPU's `$2140` write at that read while ares
sees 0.

600-frame audio with the configuration: smas 313,782 (50.9452%), cv4 0
(100.0000%), dkc 0 (100.0000%) - identical to access-stop alone, as previously
measured.

Conclusion: the forward pull is necessary but not sufficient. The first
divergence is a *latch content* ordering (write visible one read too early), not
a scheduling instant, so no arrangement of pulls/parks removes it; hiding the
write deadlocks (measured last turn). The remaining option is to change when the
S-CPU's port write is committed relative to the S-SMP's read within the same
clock window - which requires executing the S-SMP *inside* the S-CPU's write
access (true coroutine interleaving), not before or after it.

## Ceiling stop: removes the instruction-4,299 divergence and makes CV4 exact (2026-11-21)

The remaining ordering difference was floor vs ceil: our carried-credit loop stops
the S-SMP at `floor(target)`, while ares `Thread::synchronize(smp)` runs whole
accesses until the S-SMP clock is **at or past** the S-CPU clock (`ceil`).
`SNES_SMP_CEIL` drains one extra S-SMP clock when the credit remainder is
positive (the negative remainder carries, so the average rate is unchanged).

* Instruction stream: the first divergence at **4,299 is gone** (events 46 -> 43
  over 55 frames; earliest now 4,475 `$FFDA/$FFDC/$FFE9`), with the per-frame
  budget unchanged (29,277 stereo frames at 55 frames).
* 600-frame audio:

```
dkc      0 (100.0000%)     metroid   0 (100.0000%)     cv4   0 (100.0000%)
zelda    16,503 (97.4200%) fzero 355,081 (44.4888%)   smw  45,880 (92.8274%)
smas  313,782 (50.9452%)   ct    417,093 (34.7942%)   mmx2 196,820 (69.2303%)
baseline: IDENTICAL 212/212
```

So the ceiling is a real, principled fix - it removes the first measured
divergence and makes **CV4 exact without any access-stop flag** - but F-Zero,
Zelda and SMW regress. Combined with the earlier matrices this gives four
distinct per-title splits (default: zelda/fzero/metroid/smw; access-stop:
dkc/fzero/cv4; ceiling: dkc/metroid/cv4; instr-stop: zelda/fzero), which is
strong evidence that the remaining error is not a single stop/rounding rule.

Flag retained default-off; the default gate is unchanged.

## Rounding-threshold sweep: no single rule satisfies all titles (2026-11-21)

`SNES_SMP_CEIL=<pct>` rounds the S-SMP up (one extra clock) only when the
carried-credit remainder exceeds `pct`% of one clock: `0` is ceil, `100` is
floor (the default). Swept at 600 frames:

```
threshold   dkc   cv4    metroid  zelda   fzero   smw
0% (ceil)   ok    ok     ok       97.42%  44.49%  92.83%
25%         ok    ok     62.67%   99.77%  27.54%  ok
50%         ok    98.24% 62.67%   99.77%  35.99%  ok
75%         ok    98.24% 62.67%   ok      ok      ok
99%         ok    98.24% 62.67%   ok      ok      ok
default     ok   98.24%  ok       ok      ok      ok
```

The exact sets are:

```
ceil/0%    : dkc, cv4, metroid
25%        : dkc, cv4, smw
50%        : dkc, smw
75%-99%    : dkc, zelda, fzero, smw
floor      : dkc, zelda, metroid, smw (+ lower cv4 98.24%)
```

**Metroid is only exact at 0%, Zelda/F-Zero only at >=75%, and CV4 only at
<=25%.** So no value of this threshold can satisfy all five, proving the
remaining error is not a global rounding/stop rule: it depends on per-access
sub-clock ordering that a single threshold cannot express. The threshold
parameter is exhausted; the flag stays default-off.

## Default stream is exact for F-Zero; the ceiling perturbs it (2026-11-21)

Aligned the full S-SMP instruction stream against ares for F-Zero (230 frames):

```
default      : 1,027,220 instructions, 0 divergence events   (stream identical)
SNES_SMP_CEIL: 1,027,221 instructions, 1,176 events, first at 78,260 (ours extra
               ['FFD6','FFD8'])
```

So the **default floor rounding reproduces ares' S-SMP instruction stream exactly
for F-Zero** - the floor is faithful, not accidental. The ceiling's extra
one-clock round-up *introduces* divergences (and drops F-Zero's audio to 44%),
while it happens to fix CV4. That is why no threshold satisfies all titles: the
threshold perturbs a stream that is already exact.

This settles the direction: the default model is the correct general model (five
titles stream-exact), and the remaining audio differences in CV4/SMAS/CT/MMX2 are
**title-specific events**, not a global stop/rounding rule. For CV4 the event is
already isolated (the `$049F` Timer-1/`$F7` handshake, `V3VOLR` write at DSP
sample 289,423); the "fixes" found so far (access-stop, ceiling) simply shift the
global timing enough to move that one event.

So further progress must be per-event: reproduce CV4's handshake outcome without
perturbing the globally exact stream. No global parameter can do that.

## No adjacent read/write pair at CV4's divergent write (2026-11-21)

Instrumented every `$F4-$F7` and `$FD-$FF` read in both emulators (timer clock,
DSP tick, address, value) for 560 frames of CV4, looking for the read/write pair
around the first divergent output (DSP sample 289,423).

```
both logs span DSP ticks 198,854..9,551,085 (target 9,261,536 is inside)
reads within +/-2,000 ticks of the target:  ares 0   ours 0
```

So there is **no port or timer read anywhere near** the divergent `V3VOLR` write.
The write is an ordinary audio-ISR register update; its placement is fixed by the
accumulated block phase at the preceding KOFF boundary, not by a local
read/handshake.

This closes the "per-event read/write pair" hypothesis: CV4's difference is not a
local ordering that can be corrected at that point. It is the block-boundary
phase (already measured: delta flips only at KOFF by 8 DSP ticks = 16 S-SMP
clocks, and the S-SMP stream matches ares for the exact titles). Any fix must
correct the block-boundary phase for CV4 without perturbing the globally exact
stream - and every global rule tried changes which titles are exact.

The instrumentation is reverted; the tree is unchanged.

## CV4 stream alignment: the phase difference precedes the first stream event (2026-11-21)

Aligned CV4's complete S-SMP instruction stream against ares (default model,
`ARES_SPC_TRACE`, 300 frames vs ares' 1,140,702-instruction trace):

```
ours 1,462,563 instructions   ares 1,140,702 (trace end)
divergence events: 116
first: instruction 371,351, ours extra ['FFE9','FFDA','FFDC'] (IPL handshake)
```

So CV4's stream is *more* faithful than SMAS's at the same point, and its first
divergence is at instruction 371,351 - the familiar IPL `$FFDA/$FFDC/$FFE9`
handshake, not anything audio-specific.

Crucially, the DSP write phase for CV4 differs from **the very first writes**
(the KOFF deltas alternate +6/-22 from the start), i.e. long before instruction
371,351. The stream divergence therefore cannot be the origin of the block-phase
difference: the phase is already wrong while the streams still match.

That closes the search for a causal event in CV4 as well: the instruction stream
matches to 371k instructions yet the DSP write micro-step placement differs from
the first write. The difference is intrinsic to how our record-time DSP drain
places writes relative to the S-SMP access (the floor/ceil question), not to any
specific instruction, read, timer or stream event.

Summary of the four title alignments (default model):
```
dkc     audio exact
fzero   stream identical (0 events / 230 frames), audio exact
metroid audio exact
smw     audio exact
zelda   audio exact
smas    799 events / 230 frames, audio 50.95%
cv4     116 events / 1.14M instructions, first 371,351, audio 98.24%
```
All stream differences are IPL-handshake/timer-loop resyncs that leave the exact
titles untouched; the audio failures track DSP write placement, not the stream.

## Whole-sample DSP drain alone scrambles the phase (2026-11-21)

Implemented the mechanical half of ares' DSP thread model: drain the S-DSP in
whole 32-micro-step samples per Thread entry (so it may overshoot the target to a
sample boundary), `SNES_DSP_SAMPLE`.

CV4 KOFF delta (ares ticks - ours) under this model:

```
distinct values: -2, -5, -48, +1, -10, +4 ... each ~16 times
```

Instead of collapsing to a single value, the deltas scatter across many values -
the phase is *worse* than the default (which gives only +6/-22). Overshooting to
sample boundaries without the per-micro-step SMP sync randomises the placement.

So the entire behavioural content of ares' DSP thread model is the
`synchronize(smp)` after each `tick()`, not the 32-step batching: the batching
alone is a coarse overshoot, and the sync is what keeps the S-SMP on the DSP's
micro-step grid. That sync is a forward pull of the S-SMP from inside the DSP
drain - not expressible at record time in the current driver.

Reverted; no behaviour change.

## Advancing smp_clock to the DSP boundary hangs the driver (2026-11-21)

Attempted the per-micro-step pull in the only form the current code can express:
after each access, if the S-DSP clock overshot, set `smp_clock = dsp_clock`
(`SNES_DSP_PULL`), imitating `DSP::tick` -> `Thread::synchronize(smp)`.

Result: the emulator **hangs** (60-frame run times out, no statistics). Advancing
`smp_clock` without executing the plan breaks the position accounting
(`smp_clock_at_position` derives the plan start from `smp_clock - plan clocks`),
and the driver never progresses.

So the pull cannot be expressed by moving the clock: the S-SMP must actually
*execute* plan instructions inside the DSP drain (a scheduler loop owning both),
which is the mutual-coroutine rewrite. Every clock-only approximation either
scrambles the DSP phase (whole-sample drain), regresses other titles
(access-stop/ceiling/write-sync), deadlocks the boot (latch hiding) or hangs
(clock pull).

Reverted; the verified default is unchanged.

## Design for the mutual scheduler rewrite (the remaining audio work)

Every approximation has been measured and rejected. The one mechanism never
implemented is a scheduler that owns both the S-SMP plan replay and the S-DSP
stepping, with the S-SMP resumable between accesses. Concretely:

### Why it is a rewrite, not a patch
* `drain_unit` -> `record_instruction` -> `step_dsp_clock` -> `drain_dsp_to` is a
  recursion. The pull (`tick -> synchronize(smp)`) needs to run `drain_unit`
  inside `drain_dsp_to`.
* Recording builds a whole instruction's plan at once and advances `smp_clock`
  for all its accesses in one go; the DSP is drained per access immediately. To
  pull the S-SMP per DSP tick, recording must be resumable per access.

### Proposed structure
1. `struct Scheduler { phase, plan, plan_pos, plan_sub, smp_clock, dsp_clock, credit, ... }`
   owning the loop; no `Apu` method may drain the DSP or record directly.
2. `recorder` as a resumable state machine: `begin_instruction()`,
   `next_access() -> Access { clocks, kind }`, `finish_instruction()`. It advances
   `smp_clock` one access at a time and returns control between accesses.
3. Main loop (the only executor):
   ```
   loop:
     // 1. if the S-SMP is behind the DSP, execute the next access of its plan
     if smp_position < dsp_clock { execute_one_access() }
     // 2. otherwise advance the DSP one micro-step (24 clocks)
     else if dsp_clock < next_sync_target { dsp.tick(); dsp_clock += DSP_TICK }
     else break
   ```
   DSP register writes are placed at the current `dsp_clock` when the access that
   writes them executes - so the write phase is a function of the DSP clock, not
   of a record-time drain target.
4. Window/credit handling stays external: the S-CPU driver adds credit and asks
   the scheduler to retire `clocks`; the scheduler never advances the CPU.
5. Park/resume: a `$F4-$F7` read that samples ahead of the S-CPU yields
   `Parked { target }` as today; the S-CPU driver runs real instructions to the
   target and resumes the scheduler. (The write-cycle interleaving then falls out
   of (3), because the write commits after the scheduler has retired the access.)

### Invariants to assert in tests
* `smp_clock` and `dsp_clock` are monotonic; `smp_clock - dsp_clock` bounded by
  one DSP micro-step.
* Total DSP ticks per frame = 32,040 +- 1 (i.e. 319,828 stereo frames at 600).
* Five exact titles stay 0 differing at every increment (this is the regression
  gate, run after each step, not only at the end).
* Acceptance: CV4/SMAS `$FFD6`@4,299 and the `$0576/$0579` loop parity match ares;
  `$F3` write delta single-valued for CV4 and DKC; SMAS/CV4/CT/MMX2 at 0.

### Order of work
1. Extract the resumable recorder with behaviour identical to today (tests must
   stay green with the old drain path selected).
2. Add the interleaved loop behind `SNES_SCHEDULER`, compare the instruction
   stream against ares for F-Zero (must stay 0 events) and CV4.
3. Only then remove the old path.

Everything else needed (comparison tooling, ares/mesence captures, acceptance
numbers) is already in place and documented above.

## Step 2 attempt: DSP-lead interleave does not collapse the deltas (2026-11-21)

On top of `SNES_DSP_LATE`, made the S-DSP lead the S-SMP by one micro-step and
run plan accesses until the S-SMP catches up, applying `$F3` writes at the
current (leading) `dsp_clock` (`SNES_DSP_LEAD`).

CV4 KOFF deltas (ares - ours): `{+7 (334), -21 (95)}` - still **two-valued**
(shifted by one from the default's `{+6, -22}`), not single-valued.

600-frame audio:

```
late+lead  dkc 226,813 (64.5414%)   cv4 11,379 (98.2211%)   smas 313,782 (50.9452%)
```

So leading the DSP by one micro-step shifts the whole two-valued pattern by one
tick and regresses DKC; it does not remove the alternation. The alternation is
therefore in the S-SMP clock at the write, not in the DSP's lead - consistent
with the earlier finding that the stream is identical for the exact titles and
that no global rounding rule (threshold sweep) collapses it.

Reverted; step 1 (`SNES_DSP_LATE`) remains, default-off.

## ares' logged S-SMP clock is unusable for the write-phase comparison (2026-11-21)

Added the raw `smp.clock()` (fixed point) to ares' `ARES_DSP_WRITES` and compared
the S-SMP clock at each `$F3` write with ours (raw / each side's own scalar):

```
first CV4 write: ares smp clock 21,266 cycles, its own dspTicks 3,557,662
                 ours smp clock 7,115,366 cycles
```

ares' reported S-SMP clock is inconsistent with its own DSP tick count by ~167x
(expected ~7.1M cycles for 111k samples). The same anomaly appeared earlier in
`ARES_SPC_TRACE` (smp clock 1,784 / 8,876 while dspTicks were millions).

So the ares build's `Thread::clock()` for the S-SMP is not a usable time axis in
this checkout (it accumulates far too slowly), and the requested "S-SMP clock at
each write" comparison cannot be made against ares as-is. The DSP tick axis
(which is consistent) remains the only cross-emulator clock axis, and it already
shows the two-valued delta.

To continue this line, the ares reference would need its thread-clock logging
fixed/validated against `dspTicks` first; until then the alternation cannot be
compared on the SMP-clock axis.

## Concrete route for the resumable recorder: corosensei (not yet tried)

`corosensei = "0.3"` is already used for the S-CPU and S-SMP actors, so the
resumable per-access recorder does not need the SPC700 core rewritten by hand:

- Run `Spc700::step(&mut YieldBus)` inside a coroutine whose `YieldBus` yields
  `Yield::Access { clocks, kind }` from every `read`/`write`/`idle`/`step_clock`
  call. The driver resumes the coroutine once it has decided to retire that
  access (after interleaving the DSP ticks), passing back nothing (the bus state
  lives in the `Apu` moved in/out, exactly as the existing `ApuInput::Advance`
  actor does).
- The scheduler loop then owns the order: DSP tick (24 clocks) -> resume the
  SPC700 coroutine through one access -> apply any recorded `$F3` write at the
  current `dsp_clock` -> repeat. This is the documented step 2 with no recursion.
- `SNES_DSP_LATE` (committed, default-off) already moves `$F3` writes into
  `PlanStep` and drains at replay, so the write path is ready; the coroutine only
  has to replace the eager `spc.step` in `record_instruction`.

Acceptance remains unchanged: F-Zero stream 0 events, CV4/SMAS deltas
single-valued, 9-title gate, 212/212.

## Coroutine path with inline I/O sampling: DKC/access-stop-like, CV4 worse (2026-11-21)

On `SNES_SPC_CORO`, set `defer_io_reads = false` so `$F4-$F7` reads are performed
inline at the access instant (the host samples the latch then) instead of being
deferred to the scheduler.

600 frames against freshly captured ares audio:

```
dkc   0 differing   (100.0000%)
cv4   69,476 (89.1385%)   <- worse than the default's 11,269 (98.2383%)
smas  313,782 (50.9452%)  <- unchanged
```

So sampling the latch at the access instant is not the missing piece: CV4
regresses below the default, and SMAS is unaffected. This is the same class of
behaviour as the access-stop rule (which also changes port sampling) and does not
unify the titles. Reverted to the byte-identical configuration (`defer_io_reads =
true`); the coroutine seam remains equivalent to the default.

Note: the server restart removed `/tmp/opencode/audiosweep`; ares references for
dkc/cv4/smas were re-captured for these measurements.

## Coroutine path with a one-micro-step DSP lead: DKC regresses (2026-11-21)

On `SNES_SPC_CORO` (defer kept true), advanced the S-DSP one full micro-step
(`+ DSP_TICK` = 24 clocks) beyond the S-SMP clock between accesses, with `$F3`
applied immediately at the current `dsp_clock` (`SNES_SPC_LEAD`).

600 frames against re-captured ares references:

```
dkc  177,171 (72.3021%)   <- regressed from 100%
cv4   11,250 (98.2412%)   <- essentially unchanged (98.2383%)
smas 313,782 (50.9452%)   <- unchanged
```

(An earlier attempt using `+1` fixed-point unit was a no-op and reproduced the
default exactly, confirming the measurement is meaningful.)

So the per-access interleaved lead is negative, consistent with the plan-path
lead experiment (`{+7,-21}` deltas, DKC 64.5%): moving the DSP ahead of the S-SMP
does not reproduce ares and destroys the exact titles. Reverted to the
byte-identical coroutine seam.

## Per-clock yield granularity in the recorder (2026-11-21)

`SNES_SPC_PERCLOCK` makes the coroutine bus yield one S-SMP clock at a time from
`step_clock`/`advance` instead of one event per access. No core change is needed
(the bus splits the counts).

Verified **byte-identical** to the default: dkc/cv4/smas 600-frame audio files
are identical. This is expected - `drain_dsp_to` is monotone, so advancing the
DSP one clock at a time is equivalent to a bulk drain.

This provides the per-tick interleaving hook. Note the policy choice it enables
is already measured on the access-granularity driver: "DSP catches up" (default,
5 titles exact), "DSP leads by a tick" (DKC 72.3%), "latch at access instant"
(CV4 89.14%). Per-clock granularity alone does not change those outcomes; it only
allows the driver to *observe* each clock.

## ares S-SMP clock: wrap-correct logging shows exact agreement, then a reset (2026-11-21)

Fixed the ares write-log clock to record a wrap-correct *relative* value
(`smp.clock() - base`, u64) instead of the overflowing absolute clock. Comparing
relative SMP cycles at each `$F3` write:

```
first reg/value mismatch: none
idx 0..32:  ares rel 0, 54, 104, 154, 204, 254 ...   ours identical   (delta 0)
idx 33:     ares jumps by ~3,828,217 SMP cycles (ours advances ~64)
```

So our S-SMP clock at writes agrees with ares **exactly through the first audio
block** (write 33 is precisely where the KOFF delta first flips), and then the
ares value jumps by ~2^64 worth: its `Thread::_clock` is **not monotonic** in this
build (it is reset/pulled by the scheduler), so neither absolute nor relative
absolute-time comparison is valid beyond that point.

This is a genuine, if narrow, positive: for the first block the S-SMP write clock
is bit-identical to ares, and the divergence begins exactly at the block
boundary. It also means the remaining comparison cannot be made with the current
ares clock; the DSP-tick axis (monotonic and consistent) remains the only usable
one, and it already localises the difference to the block boundary.

## CV4 first block boundary: ours is exactly 56 S-SMP clocks short (2026-11-21)

Fixed the ares write-log clock to emit a per-write wrap-safe delta (the earlier
format string was missing the specifier, so the value was varargs garbage). Then
compared per-write S-SMP clock deltas:

```
through write 32:  ares and ours advance identically (cumulative diff 0)
write 32 -> 33:    ares +8,256 clocks   ours +8,200 clocks   -> ours 56 short
thereafter:        deltas identical again (62 each), offset persists at 56
```

So at CV4's first KOFF boundary our S-SMP retires exactly **56 fewer clocks** in
the inter-block gap than ares. 56 clocks is precisely one iteration of the
`$049F..$04B2` handshake loop (measured earlier: 9 instructions, 56 clocks = 28
DSP ticks). The measurement therefore confirms at the clock level what the DSP
tick axis showed: ours executes one fewer handshake iteration at that boundary,
and the block phase then stays offset by 56 clocks.

This closes the causal chain with exact numbers:

```
$FD read grid phase (10 timer clocks)  ->  one $049F iteration skipped (56 clocks)
  ->  block phase offset                    ->  DSP write delta 2-valued (+6/-22)
  ->  the V3VOLR write lands across a sample boundary  ->  audio diverges
```

The remaining fix must make the `$049F` loop run that one additional iteration -
i.e. align the S-SMP timer/read phase at the loop entry - which every global rule
so far has failed to do without breaking the exact titles.

## Timestamped latch + inline read (coroutine): same as inline, CV4 89.14% (2026-11-21)

Tried the precise ares ordering on the coroutine recorder with `defer_io_reads =
false`: S-CPU port writes are timestamped and exposed only when the S-SMP clock
reaches them (`ares readIO -> synchronize(cpu)`), with the latch sampled inline
at the access instant (`SNES_PORT_PULL`).

600 frames:

```
dkc   0 differing (100.0000%)
cv4  69,476 (89.1385%)   - identical to plain inline sampling, worse than default
smas 313,782 (50.9452%)  - unchanged
```

So "expose only writes the S-SMP has reached" changes nothing over inline
sampling: at the read instant the exposed set is the same, and CV4 still
regresses to the access-stop-like result. This is the last untried combination of
the read-ordering idea; reverted to the byte-identical coroutine seam.

The evidence is now closed on the read side as well: the CV4 residual is not
reproducible by any ordering of latch exposure relative to the read instant.

## The 56-clock deficit is exactly one extra `$049F` iteration in ares (2026-11-21)

Extracted the S-SMP instruction streams for the write-32 -> write-33 gap
(ours 7,117,350..7,125,550 SMP clocks; ares dspTicks 3,558,654..3,562,782) and
aligned them:

```
ours: 1303 instructions, 8,200 clocks
ares: 1312 instructions, 8,256 clocks (2 x 4,128 dspTicks)
non-equal blocks: 3
  insert  ares[28:37] = 049F 04A2 04A4 04A7 04AA 04AC 04AE 04B0 04B2   (ours missing)
```

So ares runs **one additional iteration of the `$049F..$04B2` handshake loop**
(9 instructions = exactly 56 clocks); every other instruction in the gap is
identical. This settles the objective's question: the deficit is a **read-value
(control-flow) difference**, not a wait-state/timer-step shortfall.

The loop body is:

```
$049F MOV Y,$00FD    (Timer 0 output)
$04A2 BNE
$04A4 MOV X,$00F7    (CPUIO3 latch)
$04A7 CMP X,$00F7    (detects a change between two reads)
$04AA BNE            (re-read if changed)
$04AC CMP X,dp$07
$04AE BEQ exit
$04B0 CMP X,dp$06
$04B2 BEQ loop
```

Ours takes the exit branch one iteration earlier, so at that iteration our `$F7`
read value differs from ares' (the latch content at the read instant). This is
the same latch-ordering question measured exhaustively (immediate / timestamped /
inline / hidden); the next instrumentation is to log the `$F7` value and clock at
the extra iteration in both and compare the two reads directly.

## The extra loop iteration is not a `$F7` value difference (2026-11-21)

Logged every `$F7` read with its DSP tick and value in both emulators for the
write-32 -> write-33 gap:

```
gap $F7 reads: ares 271, ours 269
first value difference: NONE  (every read returns 0x00 in both)
time offset: a constant +22 DSP ticks (ours later)
```

So the latch value is **not** what differs: `X = $F7` is `0x00` throughout in
both. The loop's exit at `$04AE` is `CMP X,dp$07` / `BEQ`, so the differing
branch means the **direct-page expected value** (`dp$06`/`dp$07`) differs at that
iteration - ours has already reached the exit value one iteration earlier.

That shifts the root one level deeper: the direct-page bytes are written by the
sound driver from an earlier handshake/timer read (outside this gap), so the
first differing value must be sought there, not at the loop's `$F7` reads.

The constant +22-tick read offset also persists here, matching the earlier
$049F/$FD read-grid measurement.

## Direct-page compare values: writes are identical, addresses are D-relative (2026-11-21)

Logged writes to absolute `$0006`/`$0007` in both emulators for 300 frames of
CV4:

```
writes: 7 in both, values and order identical
first differing write: none
tick offset: constant +22 (ours later), same as the read offset
```

So the compared expected value is not written differently; and because the SPC700
uses direct-page addressing, the compared bytes are at `D + 6` / `D + 7`, not
absolute `$0006`/`$0007`. Logging the absolute addresses exercises only the D=0
cases (7 writes), which match.

Next instrumentation: log the SPC700 `D` register together with the effective
address for the two `CMP X,dp` operands inside the `$049F` loop, and compare the
*values read* at the extra iteration. Since `X = $F7 = 0` and the writes match,
the remaining candidate is the direct-page RAM content read at that instant
(possibly a D change or a write to D+6/7 that the absolute filter missed).

## Direct-page compare values are identical; the "extra iteration" needs re-checking (2026-11-21)

Logged every direct-page `D+6`/`D+7` read (any page, low byte 6/7) with value and
PC in both emulators inside the write-32 -> write-33 window:

```
ares 271 reads, ours 269
first differing (address, value): NONE
example: ares [3558689, 0007, 00, pc 04AE]  ours [3558711, 0007, 00, pc 04AC]
```

Addresses and **values match for every compared read**; only the logged PC
differs (ours records the instruction-start PC `04AC`, ares records `r.pc` which
has advanced to `04AE`). So the branch inputs (`$F7 = 0`, dp values) are identical.

That makes the earlier "one extra `$049F` iteration" attribution suspect: it came
from aligning windows whose edges were taken from each side's own write clocks
(which already differ by the +22-tick read offset). With identical inputs and
values, the 56-clock difference needs to be re-examined as possibly a
window/clock-accounting artefact rather than a genuinely different branch.

Next: align the two streams by cumulative instruction index from the start of the
run (not by write-clock windows) and compare clock sums per instruction; that
distinguishes a real control-flow difference from a wait-state/clock-accounting
difference without window-edge ambiguity.

## Cumulative-index alignment: first divergence 371,351; clocks agree within 4-8 (2026-11-21)

Aligned the two 300-frame CV4 S-SMP streams purely by instruction index (no
write-clock windows) and compared cumulative SMP clocks with ares' `2*dspTicks`:

```
index      PC ours/ares   cum our clocks   ares 2*dspTicks   diff
0          FFC0/FFC0                 4                  0    +4
100        FFC6/FFC6               664                662    +2
1,000      FFCF/FFCF             7,320              7,312    +8
20,000     FFCF/FFCF           178,320            178,312    +8
100,000    FFDC/FFDC           799,016            799,010    +6
300,000    FFE9/FFE9         2,250,342          2,250,336    +6
500,000    FFDC/FFDC         3,700,272          3,700,266    +6

first PC divergence: index 371,351 (ares extra FFE9, FFDA, FFDC)
```

Two conclusions:

1. **Wait-state/clock accounting is not the cause**: over 500k instructions the
   cumulative clock difference stays at +2..+8 (a few ticks), so a 56-clock
   difference in one gap is not an accumulated wait-state error.
2. **The first control-flow divergence is at instruction 371,351**, in the IPL
   handshake region (`FFE9`/`FFDA`/`FFDC`), which is *before* the first DSP write
   block. The `$049F` handshake iteration difference measured earlier is
   **downstream** of this.

So the next target is the divergence at 371,351, not the `$049F` loop: identify
which latch/timer read at that IPL handshake differs. This supersedes the
previous "one extra `$049F` iteration at the first block" localization.

## The "extra iteration" divergences are alignment artefacts (2026-11-21)

Logged `$F4` reads at instruction 371,351 in both emulators:

```
window F4 reads: ares 70, ours 69
k=0  ares(t=1383609, v=255)   ours(t=1383609, v=FF)   (identical value; ares prints decimal)
...
```

Values are **identical (255)** and, unlike every other point measured, the
**timestamps are identical too** (1383609 in both). The apparent extra
`FFE9/FFDA/FFDC` at that index is a **difflib alignment artifact**: the region is
a 3-instruction repeating loop, so an insertion can be placed at any equivalent
position in the repetition.

This matches the cumulative-clock measurement (+2..+8 clocks over 500k
instructions): there is no genuine 56-clock control-flow divergence at 371,351.
The earlier "$049F extra iteration" was placed in another tight repeating loop
and is subject to the same ambiguity.

So the two "control-flow divergence" localizations are not real; the only
*consistent* measured difference remains the DSP/read **phase** (+22 DSP ticks at
reads, +6/-22 write deltas). The next step should therefore treat the residual as
a phase/rounding offset, not a branch/control-flow difference - and the earlier
alignment-based localizations must not be used as targets.

## Read-tick offset measured directly: 0 for 125k reads, then 0/-3/+6 rounding (2026-11-21)

Compared the DSP tick at every `$F4` read (both logs filtered to `$F4`; the
earlier index mismatch was ours logging `$F5/$F6` too):

```
F4 reads: ares 313,750   ours 313,752
tick delta distribution:
   0    : 198,568
  -3    :  40,737
  +6    :  40,681
  +3    :  28,433
  -8    :   4,238   (and a few artefacts where alignment shifts)

first 125,848 reads: delta = 0 exactly
then a repeating 0, -3, +6, 0, -3, +6 ... pattern
```

So there is **no constant +22-tick read phase**: for the first 125,848 reads the
S-SMP read instant matches ares exactly, and afterwards the offset is a small
`0/-3/+6` rounding cycle (±3 DSP ticks = ±6 S-SMP clocks). This supersedes the
earlier "+22 ticks" characterization, which came from a different comparison
axis/window.

The residual is therefore a **tick-boundary rounding effect in the DSP catch-up
at reads** (a 3-read cycle), not a block/phase offset. That is consistent with the
two-valued `$F3` write deltas ({+6,-22} mod 32 is exactly {6,10} = a 4-tick
rounding split).

## Read-tick rounding is a benign rotation; the write split is the target (2026-11-21)

Inspected the 0/-3/+6 read-tick cycle in detail:

```
read#   ares_tick  ours_tick  delta  ares_step  ours_step
125848    1384002    1384005    -3        8         11
125849    1384019    1384013    +6       17          8
125850    1384030    1384030    +0       11         17
125851    1384038    1384041    -3        8         11
... repeats with period 3
```

The per-read tick *steps* are the same set {8, 17, 11}, merely rotated by one
read between the emulators, and the reconvergence points are exact (delta 0 every
third read). Combined with the earlier finding that the `$F4` values are
identical, this read-side rounding is **benign**: it permutes which read samples
which intermediate tick, but samples the same latch values at the same total
tick progression.

So the read side is exonerated. The only remaining audio-affecting difference is
on the **write** side: the two-valued `$F3` placement ({+6,-22} mod 32 for CV4 =
{6,10}, a 4-tick split), which moves a register write across a sample boundary
(the V3VOLR case at DSP sample 289,423). The next measurement is the write-side
tick rounding: at each `$F3` write log our `dsp_clock` before/after the
`drain_dsp_to` and the target, and compare with ares' `dspTicks`, to find why the
placement is 4 ticks two-valued instead of single-valued.

## Write-side rounding is a pure single-valued ceil; the split is in the SMP clock (2026-11-21)

Checked every `$F3` write in the 300-frame CV4 run against the rounding formula:

```
writes: 2313
micro != ceil(smp_clock / DSP_TICK): 0
micro - floor(smp_clock / DSP_TICK): 1 for all 2313 writes
```

So our DSP write placement is **exactly `ceil(smp_clock / DSP_TICK)`** - a
deterministic, single-valued rounding. It cannot by itself produce the
two-valued `$F3` delta ({+6,-22} mod 32).

Therefore the two-valued write placement comes entirely from the **S-SMP clock at
the write** being two-valued relative to ares. The write side is now exonerated
together with the read side; both are pure functions of the S-SMP clock, and that
clock was measured to match ares exactly through write 32 and to diverge at the
block boundary.

This is the correct target: the S-SMP clock trajectory at the block boundary
(credit/record timing), not the DSP rounding, read grid or write placement.

## ares' S-SMP clock is discontinuous exactly at KOFF writes (2026-11-21)

Analysed the wrap-correct relative clock (`smp.clock() - base`) in ares' write log
over 300 frames of CV4:

```
writes 2,313
relative-clock decreases: 91
registers at those points: 5C x 88 (KOFF), 76/61/45 x1 (early)
```

Every discontinuity occurs at a KOFF (`$5C`) write - i.e. exactly at the audio
block boundaries. The raw `smp.clock()` therefore cannot be used as a monotonic
time axis across a block, and the earlier "+56 clocks at write 33" derived from it
is not trustworthy (it coincides with a discontinuity).

This closes the S-SMP-clock-axis route: the reference does not expose a usable
continuous S-SMP clock at the very boundaries where the residual lives. All other
axes are measured and excluded (timer, read grid/values, write rounding = pure
ceil, stream/clock accounting, alignment artefacts).

The residual is therefore characterised, but cannot be compared further against
this ares build without fixing/replacing its thread-clock logging to a monotonic
form (e.g. accumulating deltas in a separate counter).

## Monotonic ares S-SMP clock: the write clock is two-valued by 56, flipping at KOFF (2026-11-21)

Replaced the ares write-log clock with a monotonic counter accumulated in
`SMP::step` (`aresSmpMono += clocks`), which is exact and unaffected by the
thread-clock resets. Comparing it with our `smp_clock / SMP_SCALAR` at every
`$F3` write (300 frames, CV4):

```
writes 2,313, first reg/value mismatch: none
SMP-clock delta (ares - ours):  -44 : 1,788 writes
                                +12 :   525 writes
flips occur exactly at KOFF ($5C) writes:
  idx 0  (6C)  ares 7,115,322  ours 7,115,366  delta -44   <- first block
  idx 33 (5C)  ares 7,125,562  ours 7,125,550  delta +12   <- flip (+56)
  idx 39 (5C)  ares 7,133,732  ours 7,133,776  delta -44   <- flip back
  ... alternates every block
```

So with a reliable axis the result is now firm: the S-SMP clock at writes is
**two-valued with a 56-clock swing**, and it flips exactly at each KOFF block
boundary. 56 clocks is one `$049F` iteration. This reproduces the earlier
`{+6,-22}` DSP-tick split (28 ticks = 56 clocks) with a trustworthy clock.

Important correction: the first audio block boundary is at SMP clock ~7,115,000
(DSP tick 3,557,662), whereas the earlier `$F4`/dp value-level analyses were done
at DSP tick 1,383,609 - a much earlier (pre-audio) region. Those exoneration
results do not apply to the actual boundary and must be redone there.

The reference axis is fixed; the target is now precise: the 56-clock gap at the
KOFF boundary, evaluated at DSP tick ~3,557,662.

## At the real boundary the handshake values are identical; the difference is timing (2026-11-21)

Repeated the value-level comparison at the **actual** first audio block boundary
(DSP tick ~3,557,662):

```
$F7 reads in window: ares 63, ours 62; first value difference: NONE (all 0x00)
dp$06/$07 reads:     ares 64, ours 62; first (addr,value) difference: NONE
  dp  ares(t=3558775, 7, 1, pc 04AE)   ours(t=3558797, 7, 1, pc 04AC)
  dp  ares(t=3558780, 6, 0, pc 04B2)   ours(t=3558802, 6, 0, pc 04B0)
```

The values and addresses match everywhere; only the logged PC differs by exactly
+2 (ares' `r.pc` has advanced past the instruction; ours records the instruction
start) and the tick offset is the usual +22 (ours later). The read counts differ
by one, consistent with that +22 offset moving the window edge.

So even at the correct boundary there is **no value/control-flow difference** in
the `$049F` handshake. The two-valued SMP clock (-44/+12, 56-clock swing) is
therefore a **credit/timing** phenomenon: the same reads/instructions are
executed, but our S-SMP clock at the writes differs by ±56 depending on the block.

This is the final consistent characterisation: the residual is entirely in how the
S-SMP clock is advanced across the block gap (credit/record), with every data
path (latch, dp, timer, rounding) verified identical.

## The -44/+12 SMP-clock delta is instruction-granularity synchronization (2026-11-21)

Using the monotonic ares clock (`aresSmpMono`, sum of `SMP::step` clocks) the
delta at writes is -44 (ours ahead) or +12 (ours behind), swinging by 56 at KOFF.

Magnitude interpretation: 44-56 SMP clocks is the size of a short SPC700
instruction sequence (the `$049F` loop iteration is exactly 56 = 9 instructions).
That is the signature of ares' **whole-instruction synchronization**:
`Thread::synchronize(smp)` runs whole instructions until the SMP clock is at or
past the S-CPU clock, so at any instant the ares SMP clock is behind/ahead of the
exact credit by up to one instruction, depending on where the boundary falls.
Our carried-credit loop stops at the exact rational clock.

This is precisely the behaviour already implemented and measured as
`SNES_SMP_CEIL` (round the credit up to the instruction/access boundary):
it made CV4 exact (100.0000%) and M68K/Metroid exact, but regressed
Zelda/F-Zero/SMW, and the threshold sweep showed no single threshold satisfies
all titles (Metroid only at 0%, Zelda/F-Zero only at >=75%).

So the two-valued clock is not a new mechanism: it is the instruction-granularity
overshoot, whose per-title sensitivity is already mapped. No new data path or
rounding is involved - confirming the work is a scheduler-credit policy question,
not another bug.

## Port-only instruction overshoot: strongly negative (2026-11-21)

Implemented `SNES_PORT_INSTR`: finish the current SPC700 instruction only at
S-CPU `$2140-$2143` accesses (ares' synchronize points) instead of at every
access or never. 600-frame audio:

```
dkc   186,952 (70.7730%)     cv4  77,017 (87.9596%)
smas  370,451 (42.0859%)     fzero      0 (100.0000%)
zelda       2 (99.9997%)
```

So applying the overshoot only at port accesses is much worse than both the
default (exact credit) and the uniform ceiling: it destroys DKC/SMAS and leaves
CV4 well below the default. Zelda's two differing samples and F-Zero's exactness
are the only positive notes, and they do not generalise.

This closes the "restrict the overshoot to ares' sync points" idea. Together with
the uniform ceiling and the exact-credit default, every placement of the
instruction-granularity overshoot (never / every access / ports only) has now
been measured, and none satisfies the nine titles.

## SMAS first divergence is not an envelope/ADSR difference (2026-11-21)

Characterised SMAS's first audio difference (DSP sample 224,916, default model):

```
first differing sample: 224,916 (left); run length 856
before it: both silent
ares:  ... 0 0 5 5 15 15 30 30   (sound onset)
ours:  ... 0 0 0 0  0  0  4  4   (onset later)
best local alignment: offset -4 but only 39.7% of samples match
differing samples: left 156,891 / right 156,891 (equal)
```

Not a pure time shift (best alignment far from 100%), and equal on both channels
(so not a per-channel/echo accumulation bug).

Then audited the ADSR/envelope path against ares `envelope.cpp`:
`CounterRate`/`CounterOffset` tables, `counterTick`/`counterPoll`,
`envelopeRun` (release -8, ADSR 99% decay/attack formulas, GAIN modes 0-7,
sustain-level check, `(u32)_envelope >= 0x600` mode-7 threshold, the `>0x7ff`
clamp and Attack→Decay transition) - all line-for-line equivalent.

So the objective's envelope hypothesis is excluded. The SMAS difference lies
elsewhere in the DSP pipeline (BRR decode / Gaussian / pitch / KON timing) or in
the voice's key-on instant. The next step requires a per-voice DSP state dump at
sample 224,916 (instrument the local ares `DSP::sample`) to compare voice
envelope/envx/pitch/BRR state against ours.

## SMAS per-voice state at sample 224,916: a timing phase, not arithmetic (2026-11-21)

Added a one-sample, per-voice DSP state dump (`ARES_DSP_STATE` /
`SNES_DSP_STATE` + `*_SAMPLE`) and compared at SMAS sample 224,916:

```
                 ares                       ours
output L/R       681 / 681                  723 / 723
v4 envx          125                        126
v4 envelope      2008                       2016      (one step higher)
v4 pitch         1071                       1071      (same)
v4 brr           80A7                       80A7      (same)
v4 brrOffset     3                          3         (same)
v4 gaussianOff   11345                      9203      (2142 lower)
v4 mode          3 (sustain)                3
v6 pitch/brr    8121 / 65D4                 8121 / 65D4   (same)
```

Every other voice is identical. The only differences are on voice 4 and are
**timing quantities**:
* envelope one step ahead (2016 vs 2008, +8);
* gaussian offset 2142 behind (roughly two pitch steps at pitch 1071);
* pitch, BRR address, BRR offset, source and mode are all identical.

So the voice is in the same state but **~2 samples earlier in its gaussian/BRR
phase** and one envelope tick ahead. That is a key-on/envelope-event timing
difference of a couple of samples, not a DSP arithmetic, table or accumulation
difference.

This excludes the DSP-pipeline hypothesis for SMAS as well: the same class as
CV4 (the failing titles differ in *when* a voice event lands relative to sample
generation, not in *what* the DSP computes).

## SMAS voice 4 is exactly 2 samples late (2026-11-21)

Dumped voice 4 per sample across 224,895..224,925 in both emulators:

```
sample   ares env/gauss/off/L        ours env/gauss/off/L
224897   2008 / 7380 / 1 / -66       2016 / 5238 / 1 / 54
224898   2008 / 8451 / 1 / -101      2016 / 6309 / 1 / -15
224907   2008 / 17019 -> 1706 off 3  2016 / 15948 off 1 / -110   (ares wraps)
224909   2008 / 3848 off 3 / 187     2016 / 1706 off 3 / -10     (ours wraps)
```

* the gaussian offset advances by pitch (1071) per sample in both;
* the BRR block wrap (offset 1→3, gauss → 1706) happens 2 samples later in ours;
* `ours[n]` equals `ares[n-2]` (e.g. 224,897 ares -66 appears at ours 224,899);
* the envelope is a **constant** 8 higher in ours (one tick ahead), not drifting.

So the whole voice-4 difference is a **pure 2-sample phase delay** of the voice
plus a one-tick envelope offset. That is the granularity of a KON/BRR-fetch
event landing two samples later relative to sample generation - far coarser than
the +-4-tick DSP write phase measured elsewhere.

Next: log the KON (`$4C`) write for voice 4 with its sample index in both and
compare; a 2-sample (64-tick) displacement of that one write would explain the
whole SMAS divergence.

## SMAS: KON(v4) is one sample late, and the phase drifts one more (2026-11-21)

Logged the `$4C` KON write with bit 4 set (voice 4) in both emulators over 450
frames of SMAS:

```
ares 1 KON(v4): sample 112,399  value 10
ours 1 KON(v4): sample 112,400  value 10   (one sample later)
```

So the voice-4 key-on lands **one sample (32 DSP ticks) later** in ours - much
coarser than the +-4-tick write phase measured for CV4. That alone explains one of
the two samples of delay seen at sample 224,916.

The remaining sample: at 224,916 the gaussian phase is 2 x pitch behind (2142),
while the gaussian advances by exactly pitch (1071) per sample in both. So one
additional sample of phase accumulates across the 112,517 samples between KON and
the divergence - i.e. a single extra fetch/phase step somewhere in that span
(key-on delay start value or one BRR-wrap boundary), not a rate difference in the
window.

Next: dump `keyonDelay`, `brrOffset` and the gaussian value for a few samples
right after the KON (sample ~112,399) in both, to see whether the extra sample
comes from the key-on delay sequence or from one BRR wrap.

## SMAS root: the KON write's micro-step phase shifts the key-on by 2 samples (2026-11-21)

Dumped voice 4 around its KON write (samples 112,380..112,430):

```
n=112399  ares off=5 gauss=14477 env=0    ours off=5 gauss=14477 env=0
n=112400  ares kd=5 gauss=15548          ours kd=0 gauss=15548
n=112401  ares kd=4 off=1 gauss=0 brr=4000   ours kd=0 off=5 gauss=16619
n=112402  ares kd=3                       ours kd=5 off=7 gauss=1306
n=112403  ares kd=2 brr=4000              ours kd=4 off=1 gauss=0 brr=4000
n=112406  ares kd=0 gauss=1071 env=32     ours kd=1 gauss=0
n=112408  ares gauss=3213                 ours kd=0 gauss=1071 env=0
```

The entire key-on sequence (`keyonDelay` 5,4,3,2,1,0 then gaussian advance and
envelope steps) is **shifted by exactly 2 samples** in ours. The KON write itself
was logged one sample later (112,399 vs 112,400); the extra sample comes from the
write landing on the other side of the micro-step at which the DSP consumes
`keyon`.

So the SMAS divergence is a **discrete manifestation of the DSP write-phase
difference**: a small (few-tick) difference in the write's micro-step decides
whether the key-on is consumed in this sample or the next, which for KON becomes
a coarse 1-2 sample jump in the whole voice trajectory.

This unifies all failing titles under one mechanism: the DSP register write lands
at a slightly different micro-step (S-SMP clock phase at the write), and the
audible effect depends on which event the write crosses (a volume update for CV4,
a KON for SMAS). The S-SMP clock at writes is the instruction-granularity
synchronization already characterised (-44/+12, ceiling family).

## SMAS KON micro-step: ares step 30, ours step 3 of the next sample (2026-11-21)

Extracted the micro-step of the voice-4 KON (`$4C`, bit 4) write from both logs:

```
ares KON(v4): dspTick 3,596,798  -> sample 112,399, step 30
ours KON(v4): dspTick 3,596,803  -> sample 112,400, step  3   (5 ticks later)
```

And the DSP micro-step sequence in `dsp.rs` matches ares' `main()` call-for-call,
so the **consumption order is identical** - the order hypothesis is refuted.

What happens is now exact: ares' write lands at step 30, i.e. just before the
voice-4 key-on consumption; ours lands at step 3 of the *next* sample, just after
it. Missing that consumption forces the key-on (keyonDelay 5..0, gaussian start)
to begin one sample later, and the write itself is one sample late, giving the
observed **2-sample shift**.

So a 5-tick write-phase difference (the already-measured -44/+12 SMP-clock
phase) is amplified by the discrete KON event into a coarse 2-sample jump. The
mechanism is fully closed: DSP write micro-step phase = S-SMP clock at the write,
identical in both for the exact titles and off by a few ticks at block boundaries
for the failing ones.

## CT and MMX2 first divergences diagnosed (2026-11-21)

### CT - same mechanism as SMAS
First differing sample 151,022 (DSP 75,511). Per-voice state:

```
        ares                              ours
output  left -1 / right -1                 0 / 0
v1      env=2047 pitch=3068 brr=B606       identical except
        off=5 gauss=12208                  off=5 gauss=6072
```

The only difference is v1's gaussian offset: 12,208 vs 6,072 = **2 x pitch (3068)**
behind, with env/pitch/BRR/offset/mode all identical. Exactly the SMAS voice-4
pattern (a 2-sample fetch phase).

### MMX2 - voices silent where ares sustains them
First differing sample 442,758 (DSP 221,379). Per-voice state:

```
ares: v2 env=2039 pitch=10880 brr=3AFD gauss=16128 (sustain)
      v7 env=2039 pitch=5424  brr=3AFD gauss=16160 (sustain)
ours: v2 and v7 env=0 pitch=0 brr=0000 (silent)
```

And over 221,379..221,403 ares' v2 stays in sustain with the gaussian advancing,
while ours stays silent. So MMX2 is not a 2-sample phase: our voices 2/7 are
**not keyed on at all** in that window - a missed or much-later KON.

So three failing titles (CV4, SMAS, CT) share the DSP write micro-step phase
(a few ticks, amplified by a discrete event), while MMX2 shows a qualitatively
larger defect (a KON that ours does not perform when ares does), which is the next
target: locate the `$4C` write for MMX2 v2/v7 and compare its presence/sample in
both.

## MMX2 root cause: the S-CPU never writes the APU ports (2026-11-21)

Our MMX2 output is **entirely silent** - the 69% "match" is only matching ares'
pre-music silence (ours mean |x| = 0.0 over the whole window; ares from ~6.9 s).

State at 600 frames:

```
spc after:  PC=FFCF (IPL ROM)  - the S-SMP never leaves the IPL
apu ports:  cpu<-apu [AA, BB, 00, 00]  apu<-cpu [00, 00, 00, 00]
PORT_OUT:   1,136,903 CPU *reads* of $2140-$2143,  0 CPU writes,  2 S-SMP writes
```

So the S-CPU spins reading `$2140` (about 1,900 reads/frame) and never writes the
driver bytes; the IPL's `$AA/$BB` handshake is never answered, and the DSP never
receives a single register write (`$4C` count = 0 vs ares' 113).

This is a **separate, larger defect** from the DSP write-phase family: MMX2's
APU is never initialised. It explains why MMX2 was insensitive to every DSP
scheduling variant. Next: determine what the S-CPU boot code is waiting for when
it polls `$2140-$2143` (value expected, and whether the CX4 boot path is involved)
by logging the polled port values and the S-CPU PC at the spin.

## MMX2: the S-CPU never reaches APU init (CPU-side divergence) (2026-11-21)

Followed the S-CPU PC over the run:

```
frame 1    PC=00:8081
frame 2    PC=00:B2DF
frame 10   PC=7E:240F
frame 50   PC=13:F941
frame 100  PC=00:8100
frame 150  PC=00:8100
frame 300  PC=00:8103
frame 600  PC=00:8100
```

From ~frame 100 the S-CPU sits in the routine at `$8100` (`STA $0008,X / SEP #$20 /
JSR $1C86 / RTL`) and never writes `$2140-$2143`. NMI still runs (556 by frame 600),
so the machine is not dead - but the game never performs its APU upload, and the
S-SMP waits forever in the IPL (`PC=FFCF`, 1.1M polls of `$F4`).

So MMX2's silence is **not** an APU/DSP issue at all: the S-CPU-side initialisation
never happens. That makes it a separate CPU/boot divergence, plausibly CX4-related
(MMX2 is a CX4 title) - the same class as a coprocessor-dependent init path.

Next: compare the S-CPU PC trace with ares over the first ~1-2 s to find the first
divergent instruction (area `$8000-$1C86`, likely a CX4 handshake or a status
wait), then fix that - this is the only failing title whose root is not the
exhausted DSP write-phase family.

## MMX2's 69% was a missing Cx4 firmware, not an APU defect (2026-11-21)

The 600-frame matrix above lists MMX2 at 69.2303%, but that run did **not** load
`cx4.rom`. MMX2 is a Cx4 title (cartridge type `$F3`, `has_cx4()` true), so with no
firmware the chip's `$6000-$7FFF` window reads open bus. Tracking the S-CPU:

```
frame 1    PC=00:8081         frame 100  PC=00:8100 (main code, NMI running)
frame 600  PC=00:8100         APU ports: cpu<-apu [AA,BB] / apu<-cpu [00,00,00,00]
PORT_OUT   1,136,903 S-SMP polls of $F4, 0 S-CPU writes to $2140-$2143
audio      mean |x| = 0.0 (entirely silent)
```

The first S-CPU/ares divergence was frame 20: at `$13:F8D1` (`F0 03`, `BEQ +3`)
ares took the branch, we did not. The operands are identical (`A`/`X` logged
identical at every iteration). The loop is a boot-time **save/Cx4 data check**
over `$00:7F52..$00:7F6F`, i.e. the Cx4 window; with the window reading open bus
the check failed, `$1F53` (its error counter) incremented, and the game never
reached its sound-driver upload.

Loading the firmware fixes it completely:

```
CX4_ROM=.../cx4.rom:
  frames 18..28  S-CPU instruction sets identical to ares (onlyA=0, onlyO=0)
  S-SMP leaves the IPL (PC=046A), ports live ([67,BB,02,FE])
  audio mean |x| = 0.02198  (ares 0.02197)
  600-frame diff:  196,872 differing (69.2222%)
  per second:      0.00-6.91 s 100.00%, 6..7 s 90.95%, after 7 s ~0%
```

So MMX2 is **bit-exact for the first 6.91 s** and diverges exactly at the first
music note (first differing interleaved sample 442,758). The per-voice dump at
DSP sample 221,379 (with Cx4 loaded):

```
        ares                              ours
v2      env=2039 pitch=10880 brr=3AFD   env=0 pitch=0 brr=0000
v7      env=2039 pitch=5424 brr=3AFD    env=0 pitch=5424 brr=0075
```

i.e. ares is sustaining voices 2/7 while ours has them re-keyed/reset - the same
discrete **key-on/event phase** family as SMAS and CT, not a Cx4 or arithmetic
bug. The `$4C` key-on for voices 2/7 lands at a different sample (ares 221,366 vs
ours 221,561 in the write log), and our whole `$4C` write sequence is phase
shifted.

**Gate correction:** every CX4 title must be measured with the 3 KiB `cx4.rom`
loaded (`CX4_ROM=...`), exactly as ares loads it. Without it the result is
silent-vs-music and meaningless. The matrix entry for mmx2 above is superseded by
the numbers in this section.

## MMX2 handshake instrumentation and whole-instruction catch-up flags (2026-11-21)

### The extra `$4C` write is a driver handshake iteration, not a DSP event

Captured the full S-SMP instruction streams (ours `SPC_TRACE`, ares
`ARES_SPC_TRACE`) for 430 frames of MMX2 with `cx4.rom` and aligned them by PC.

* First difference, index 883,130 of 1,744,079: the **IPL upload handshake**
  `FFCF CC F4 CMP $F4,#$CC / FFD2 D0 FB BNE` - ours executes one extra poll
  iteration; ares exits with `Z=1` one iteration earlier. The streams converge
  again (both continue at `FFD6`) but stay one instruction out of phase.
* First *driver* divergence, index 966,597, at the S-SMP echo loop:

```
0715: E5 F5 00   MOV A,$00F5      ; read DSP data port
0718: D7 A0      MOV ($A0)+Y,A    ; store byte
071A: CC F4 00   MOV $00F4,Y      ; echo Y to the S-CPU
071D: 5E F4 00   CMP Y,$00F4      ; wait for the S-CPU to change it
0720: F0 FB      BEQ $071D
```

  ours executes one extra `CMP/BEQ` iteration, then runs on. This is the same
  CPU->S-SMP port-visibility phase as the IPL handshake. It shifts the driver
  by one handshake step; in the `$4C` write log ours then emits one extra
  `$4C=00` (114 writes vs ares' 113) and the real `$4C=$84` key-on lands later.

Root: ares' scheduler is symmetric - every thread entry runs
`scheduler.synchronize()`, catching all other threads up by **whole
instructions**, and `CPU::writeAPU` calls `synchronize(smp)` before latching.
Our driver only advances the S-SMP forward with exact credit, so at a port
access the S-SMP is mid-instruction instead of at an instruction boundary.
The `$4C` write PC difference (`0D81` vs ares `0D84`) is only the log site
(ours logs the instruction start, ares the post-instruction PC).

### Two new default-off flags, both measured

`SNES_PORT_LEAD`: at every S-CPU write to `$2140-$2143`, finish the S-SMP
instruction in progress before latching (ares `writeAPU`), carrying the
overshoot as negative credit. 600-frame audio:

| title | default | `SNES_PORT_LEAD` |
|---|---|---|
| dkc | 100.0000% | **100.0000%** |
| zelda | 100.0000% | 63.7471% |
| fzero | 100.0000% | 27.5437% |
| cv4 | 98.2383% | 89.1395% |
| smas | 50.9452% | 50.9452% |
| ct | 34.9427% | 29.0592% |
| mmx2 | 69.2222% | **89.9995%** (first diff 6.91 s -> 7.07 s) |

`SNES_INSTR_LEAD`: at every S-CPU instruction boundary, finish the S-SMP
instruction (ares' per-thread `scheduler.synchronize()`):

| title | default | `SNES_INSTR_LEAD` |
|---|---|---|
| dkc | 100.0000% | 70.7730% |
| zelda | 100.0000% | 99.9997% (2 samples) |
| fzero | 100.0000% | **100.0000%** |
| cv4 | 98.2383% | 87.2805% |
| smas | 50.9452% | 42.0826% |
| ct | 34.9427% | 24.4506% |
| mmx2 | 69.2222% | 66.9685% |

`SNES_INSTR_LEAD` reproduces the already-measured instruction-stop family
(dkc 70.7730% matches the matrix above), so it is not a new behaviour.

Also re-measured with the current build:

```
default                      = pre-change result (dkc/zelda/fzero 100%)
SNES_INSTR_SYNC=1            = identical to default on all seven
SNES_SMP_CEIL=0              dkc 100, cv4 100, zelda 97.4200, fzero 44.4888,
                             smas 50.9452, ct 34.7942, mmx2 88.2850
SNES_INSTR_SYNC=1 CEIL=0     = same as CEIL=0
```

Each rule is exact for a different subset: exact-credit for dkc/zelda/fzero,
`SNES_PORT_LEAD` for dkc+MMX2, `SNES_INSTR_LEAD` for fzero (+zelda to 2
samples), `SNES_SMP_CEIL=0` for dkc+cv4. No unidirectional stop rule satisfies
all seven. Reconciling them requires the bidirectional whole-instruction
scheduler (the S-SMP pulling the S-CPU forward mid-instruction), which the
current CPU-coroutine driver cannot express. Both new flags stay default off;
the verified default path is unchanged.

## `SNES_SMP_WRITE_PULL`: ares' S-SMP write synchronize, and MMX2 becomes bit-exact (2026-11-21)

ares' `SMP::writeIO($F4-$F7)` calls `synchronize(cpu)` *before* latching the
port (io.cpp), i.e. the S-SMP pulls the S-CPU forward to the write's clock. Our
model latched the value immediately with no pull. `SNES_SMP_WRITE_PULL` parks
the S-SMP on the write's own clock (`park_at`) and lets the S-CPU run forward
to it before the port is committed.

Combined with the existing `SNES_PORT_LEAD` (catch the S-SMP up to the S-CPU
clock in whole instructions at a `$2140-$2143` write, ares `CPU::writeAPU`):

```
SNES_PORT_LEAD=1 SNES_SMP_WRITE_PULL=1, 600-frame audio, cx4.rom for MMX2

dkc    0 differing over 639656  (100.0000%)
mmx2   0 differing over 639656  (100.0000%)   <- first bit-exact result
zelda  231894 differing (63.7471%)
fzero  463471 differing (27.5437%)
cv4     69470 differing (89.1395%)
smas   313782 differing (50.9452%)
ct     453777 differing (29.0592%)
```

**MMX2 audio is now bit-exact** (all 319,828 stereo frames, offset 0) under this
configuration, and DKC stays exact. The write-pull is inert on its own
(`SNES_SMP_WRITE_PULL=1` gives exactly the default numbers), so it only matters
when the S-SMP is first pushed ahead by `SNES_PORT_LEAD` - i.e. it is the
missing half of the bidirectional scheduler, not a separate rule.

Also measured with the same build:

```
SNES_PORT_LEAD + SNES_SMP_WRITE_PULL + SNES_INSTR_LEAD
  mmx2 93.8051%  (INSTR_LEAD perturbs the port-lead win)
SNES_PORT_LEAD + SNES_SMP_WRITE_PULL + SNES_DSP_CPU_PHASE
  mmx2 91.3700%, dkc 58.2915%  (worse)
SNES_INSTR_LEAD + SNES_SMP_WRITE_PULL
  = default on all seven (the write-pull is inert without the port lead)
```

The failure split is unchanged: zelda/fzero need the exact-credit default, MMX2
needs the port-lead + write-pull pair, CV4 needs the ceiling/access-stop rule.
All flags remain default off, so the verified default gate
(dkc/zelda/fzero 100%, cv4 98.2383%, smas 50.9452%, ct 34.9427%, mmx2 69.2222%)
is unchanged; MMX2's bit-exact result is reachable with
`SNES_PORT_LEAD=1 SNES_SMP_WRITE_PULL=1`.

## The PORT_LEAD split is the IPL boot handshake (2026-11-21)

### The zelda shift is an IPL upload phase, not a DSP effect

Zelda under `SNES_PORT_LEAD=1 SNES_SMP_WRITE_PULL=1`:

* first non-zero audio sample: **ours 90,144, ares 90,136** (fixed 4 stereo
  frames, 8 interleaved samples; constant at 200/400/600 frames, so not
  accumulating);
* whole output = default output shifted by 8 interleaved samples (89.0% match at
  that shift; default matches ares at offset 0);
* per-voice (v7): `ours[n] = ares[n-4]` with env/pitch/BRR progression identical.

The first S-SMP PC divergence explains it: instruction 1,532 (of 1,113,537),
inside the IPL boot loop:

```
k=1532  ours $FFDA / $FFDC / $FFE9 (RAM clear)   ares $FFEB / $FFED (transfer)
```

Ours performs extra IPL iterations, so the driver upload completes about four
output samples late. The split lives in the **boot handshake**, not in the DSP.

### `SNES_LEAD_AFTER_BOOT`: gate the pull until the IPL is unmapped

With `SNES_PORT_LEAD=1 SNES_SMP_WRITE_PULL=1 SNES_LEAD_AFTER_BOOT=1` (the pull
rules only apply once the IPL ROM is disabled):

```
dkc    100.0000%
zelda  100.0000%
fzero  100.0000%
mmx2    69.2217%   (back to the default)
```

So the MMX2 bit-exact result *requires* the boot-handshake treatment, while
zelda/F-Zero *require* its absence - the two are opposite at the same IPL
handshake. This is the per-title split, localised to one place.

### `SNES_SMP_ACCESS_STOP` + `SNES_SMP_WRITE_PULL` (also measured)

```
dkc 100.0000%  zelda 63.7474%  fzero 35.9945%  cv4 89.1395%
smas 50.9452%  ct 29.5962%  mmx2 95.8540%
```

Same family as `PORT_LEAD + WRITE_PULL`, so the write-pull dominates whichever
stop rule is used.

### `SNES_LEAD_CLAMP` (rejected: deadlocks)

Clamping the borrowed credit to zero after a catch-up hung DKC (no output after
more than an hour); the flag and code were removed. Not viable.

### State

All flags default off; the default gate is unchanged (dkc/zelda/fzero 100%,
cv4 98.2383%, smas 50.9452%, ct 34.9427%, mmx2 69.2222%). The remaining question
is now narrow and explicit: reproduce ares' symmetric cooperative scheduler at
the IPL boot handshake such that zelda/F-Zero and MMX2 are both exact. The
current unidirectional driver cannot express both; a scheduler rework that lets
the S-SMP and S-CPU switch to each other at instruction entry (ares
`Thread::Enter` -> `scheduler.synchronize()`) is the next architectural step.

## ACCESS_STOP reaches CV4 100%; zelda's residual is a KOFF/KON sample straddle (2026-11-21)

Re-measured `SNES_SMP_ACCESS_STOP=1` with the current build (CX4 for MMX2):

```
dkc    0 differing (100.0000%)
zelda  1450 differing (99.7733%)
fzero  0 differing (100.0000%)
cv4    0 differing (100.0000%)      <- CV4 is bit-exact under access-stop
smas   313782 differing (50.9452%)
ct     437900 differing (31.5413%)
mmx2   196872 differing (69.2222%)
```

So CV4 can be made bit-exact (the old matrix's access-stop result is confirmed),
at the cost of zelda (1450 samples) and the metroid/SMW regressions measured
earlier. `SNES_MUTUAL=1` changes none of these numbers.

### Why zelda differs under access-stop

The 1450 differing samples are almost all one transient: 1448 in second 1, plus
one sample each in seconds 6 and 7. First difference at DSP sample 45,068; the
per-voice dump shows ours `ours[n] = ares[n-2]` for voice 7 (env/pitch identical,
BRR fetch two samples behind, a transient `brr=001B` exposed for two samples).

The DSP write logs explain it exactly:

```
ares: 45054 $72=0A  45055 $73=3D  45057 $5C=00  45057 $4C=80
ours: 45054 $72=0A  45055 $73=3D  45057 $5C=00  45058 $4C=80
```

ares lands the KOFF (`$5C`) and KON (`$4C`) in the **same** DSP sample; under
access-stop ours puts the KON one sample later, so the voice's key-on (and its
2-sample BRR phase) starts a sample late. That is the same DSP write-phase
sensitivity as before, now reduced to a single event: a write pair straddling
one sample boundary (`ceil(smp_clock/24)` crossing 32 ticks).

A `SNES_DSP_PRIME` sweep (0,2,4,6,8,10,12) does not change the count (1448 at
0-8, worse at 10-12), so it is not a globally wrong DSP catch-up phase.

### `SNES_SMP_ENTRY_PULL` (added, inert here)

The ares SMP thread entry synchronizes the S-CPU to the S-SMP clock before each
S-SMP instruction (`Thread::Enter` -> `scheduler.synchronize()`). Implemented as
a default-off park before `record_instruction`. With access-stop the S-SMP is
never ahead at an instruction entry (the stop rule already finished the
instruction and the credit is negative), so it is inert: zelda stays at 99.7733%
and every other title is unchanged.

### State

Default gate unchanged (dkc/zelda/fzero 100%, cv4 98.2383%, smas 50.9452%,
ct 34.9427%, mmx2 69.2222%). Access-stop gives dkc/fzero/cv4 100% and zelda
99.7733%. The per-title split now reduces to a single sample-boundary question
in the DSP write phase: CV4 and zelda want opposite stop rules at their
respective write pairs, so still no global default.

## DSP write sample phase: the straddle is a 2-tick offset, not a global lag (2026-11-21)

The zelda KOFF/KON pair under access-stop is 2 DSP ticks late, not just on the
wrong side of the boundary:

```
ares  $5C tick 1441830  $4C tick 1441854   (both inside sample 45057)
ours  $5C tick 1441832  $4C tick 1441856   (KON exactly on 45058's boundary)
```

New default-off `SNES_DSP_LAG=N` holds the S-DSP N micro-steps behind the S-SMP
(`drain_dsp_to(smp_clock - N*DSP_TICK)`). Sweep:

```
access-stop + LAG=1 : zelda 1448 differing (first 200 frames)
access-stop + LAG=2 : zelda 0 differing (first 200 frames, 100.0000%)
access-stop + LAG=3/4: zelda 0 differing (first 200 frames)
```

So a 2-tick earlier write placement makes the event exactly match ares. But a
uniform lag is wrong over the whole run:

```
LAG=2, 600 frames: dkc 62.8391%, zelda 95.1566%, fzero 89.6213%,
                   cv4 98.1206%, smas 50.9452%, ct 34.9427%, mmx2 69.2217%
access-stop+LAG=2, 600 frames: dkc 62.8391%, zelda 99.8541%,
                   fzero 89.6213%, cv4 98.6855%, others unchanged
```

So the +2-tick offset at this event is *local*, not a constant DSP phase: shifting
the entire DSP grid breaks DKC and F-Zero. The fix must place that specific
register write in the earlier sample without moving the DSP clock globally -
i.e. the sample attribution at the boundary remains the scheduler/clock-ordering
problem, not a global phase constant.

`SNES_DSP_LAG` is a diagnostic and stays default off (0); the default gate is
unchanged.

## The default already places zelda's KOFF/KON exactly; the +2 ticks are access-stop's boot phase (2026-11-21)

DSP write ticks for zelda's voice-7 KOFF/KON:

```
default   : ours 1441830 / 1441854   ares 1441830 / 1441854   (identical)
access-stop: ours 1441832 / 1441856  ares 1441830 / 1441854   (+2 ticks)
```

So the requested "boundary write -> previous sample" rule is not needed for the
default path: it already matches ares tick-for-tick at that pair. The +2-tick
offset appears only when `SNES_SMP_ACCESS_STOP` is enabled, i.e. it is produced
by access-stop changing the boot handshake timing (the driver reaches the write
2 DSP ticks later relative to the sample grid), not by the write's attribution.

Gating access-stop to after boot (`SNES_SMP_ACCESS_STOP=1 SNES_LEAD_AFTER_BOOT=1`,
the latter now gates the whole stop/pull family) proves the symmetry:

```
dkc 100.0000%  zelda 100.0000%  fzero 100.0000%
cv4 98.2383%   smas 50.9452%    ct 31.5316%   mmx2 69.2222%
```

i.e. gating the boot restores zelda but gives back CV4's access-stop win
(98.2383%), the exact mirror of `SNES_PORT_LEAD+SNES_SMP_WRITE_PULL` where the
boot treatment wins MMX2 and loses zelda. `SNES_DSP_LATE` changes none of this.

**Conclusion:** CV4, zelda and MMX2 each need a *different* boot-handshake
interleaving; a per-write boundary rule cannot help because the default already
has the right write placement. The remaining fix is the symmetric scheduler at
the boot handshake, and every expressible stop/pull/phase/gating variant has now
been measured on the 7-title gate.

## `SNES_DSP_INSTR_SYNC`: SMAS becomes bit-exact (2026-11-21)

ares' `SMP::step` synchronizes the DSP once per S-SMP *instruction*
(`Thread::synchronize(dsp)` in timing.cpp); our default drains the DSP at every
S-SMP *access*. New default-off `SNES_DSP_INSTR_SYNC` advances the SMP clock
during recording and drains the DSP once at the instruction boundary.

With the default CPU scheduling it makes SMAS bit-exact:

```
SNES_DSP_INSTR_SYNC=1, 600 frames
  smas   0 differing / 639656  (100.0000%)
  dkc    267671 differing (58.1539%)
  zelda  103119 differing (83.8790%)
  fzero   76721 differing (88.0059%)
  cv4     12225 differing (98.0888%)
  ct     416109 differing (34.9480%)
  mmx2   196878 differing (69.2213%)
```

SMAS is 100% under every CPU-scheduling combination tried with
`SNES_DSP_INSTR_SYNC`, except the instruction-synced CPU driver
(`INSTR_SYNC+INSTR_LEAD+DSP_INSTR_SYNC` gives smas 42.0832%), so SMAS needs
per-instruction DSP sync with the default exact-credit CPU scheduling.

`SNES_DSP_LAG` has no effect under instruction sync (the per-instruction drain
crosses the same sample boundaries), and combining with
`PORT_LEAD+WRITE_PULL` gives smas 100% but mmx2 91.3700%.

So there is now a second, independent split axis:

| axis | titles wanting it |
|---|---|
| CPU exact-credit (default) | dkc, zelda, fzero |
| CPU port-lead + write-pull | mmx2 (100%), cv4 via access-stop |
| DSP per-access sync | dkc, zelda, fzero |
| DSP per-instruction sync | smas (100%) |

The default gate is unchanged; `SNES_DSP_INSTR_SYNC` stays default off, and SMAS
is now reachable bit-exact with `SNES_DSP_INSTR_SYNC=1`.

## The "DSP-sync axis" is a ~5-tick clock drift, not a visible mode (2026-11-21)

ares synchronizes the DSP at **every** S-SMP access, not at instruction entry:
`SMP::wait` -> `SMP::step(clocks)` -> `Thread::step(clocks);
Thread::synchronize(cpu); Thread::synchronize(dsp);`. So the default per-access
drain is the faithful model, and `SNES_DSP_INSTR_SYNC` is not what ares does.

Yet the DSP write ticks show that SMAS matches the *instruction-sync* position:

```
SMAS, voice writes near DSP sample 112456 (the first default/instr-sync audio
difference is interleaved sample 224916 = DSP 112458):

ares                         KOFF tick 3598621  (sample 112456, step 29)
ours DSP_INSTR_SYNC          KOFF tick 3598621  (exact match)
ours default (per-access)    KOFF tick 3598626  (sample 112457, +5 ticks)
```

and zelda (default) matches ares exactly (KOFF/KON 1441830/1441854), where the
per-access and per-instruction positions coincide.

So SMAS's preference is not a different sync mode: it is an accumulated **+5 DSP
tick drift** of our clock relative to ares by that point (~3.6 M ticks), and
`SNES_DSP_INSTR_SYNC` happens to subtract that drift at writes. This is the same
family as the documented +2..+8 S-SMP clock drift over ~500 K instructions.

`SNES_DSP_WRITE_AT=start` and `=mid` (per-write placement inside the access) do
not reproduce it:

```
start/mid: dkc 64.5414%  zelda 99.8329%  fzero 99.5670%  cv4 98.2211%
           smas 50.9452%  ct 34.9427%   mmx2 69.2219%
```

because SMAS's difference survives a per-write placement change and needs the
whole per-instruction drain pattern.

No state-conditioned rule is therefore justified: there is no visible hardware
state that selects the mode, only a small cumulative clock difference whose
audible effect depends on where a register write lands relative to the 32-tick
sample grid. The remaining work is to eliminate the residual S-SMP/DSP clock
drift (instruction-clock accounting) rather than to condition the DSP drain.

## No S-SMP clock drift to eliminate: the DSP-write phase is a fixed small set (2026-11-21)

Tested the "residual clock drift" hypothesis directly.

**Per-instruction clock costs are identical.** Comparing SMAS's S-SMP instruction
traces (ours `SPC_TRACE`, ares `ARES_SPC_TRACE`) by decoding instruction deltas
per PC, every PC's cost matches ares exactly (our delta is exactly twice ares'
only because our trace units are half-SMP-clocks; the ratio is exactly 2 for
every PC, including the 10/20 wait-glitch cases). There is no per-instruction
clock-accounting difference to fix.

**The DSP write-tick difference is a fixed set, not a drift.** Aligning the SMAS
DSP write logs by (register, value) and comparing ticks (ours − ares):

```
default      : {-11: 15,  -3: 10723,  +5: 6399}
DSP_INSTR_SYNC: {-16: 15,  -8: 10723,   0: 6399}
```

Only three values over the whole 250-frame run; the difference does not grow
with time. It is a small fixed phase per write, not accumulated drift.

**`SNES_DSP_PRIME` cannot move SMAS.** Sweeping prime 1..5 leaves SMAS at exactly
313782 differing samples for every value, while degrading dkc (72.3%→60.8%) and
zelda (99.77%→89.65%). So SMAS's requirement is not a DSP target phase either.

Interpretation: `SNES_DSP_INSTR_SYNC` does not compensate a drifting clock; it
changes *where within an instruction* each register write is observed (the DSP
has not yet been advanced for the current instruction's accesses). SMAS's DSP
writes need the instruction-start position, dkc/zelda's need the per-access
position. Since ares synchronizes the DSP per access, the SMAS match is a
property of ares' instruction sequencing relative to its `wait` calls, not of a
clock constant.

So the "eliminate the drift" step is a negative result: there is no drift. The
remaining difference is structural (per-instruction vs per-access DSP
observation), and the default gate is unchanged.

## DSP whole-sample observation point measured and rejected (2026-11-21)

ares' DSP thread entry `DSP::main` runs all 32 micro-steps of one output sample,
and every `DSP::tick` advances 24 clocks then calls `Thread::synchronize(smp)`
(pulling the SMP to the DSP clock inside the sample). New default-off
`SNES_DSP_SAMPLE_SYNC` advances the DSP to the next sample boundary when it
catches up to the SMP, i.e. makes the observation point sample-granular.

Measured alone (600 frames):

```
dkc    342556 differing (46.4468%)   zelda 196248 (69.3198%)
fzero  463200 differing (27.5861%)   cv4    56805 (91.1194%)
smas   336844 differing (47.3398%)   ct    417278 (34.7652%)
mmx2   196874 differing (69.2219%)
```

It regresses every title, so ares' DSP observation point is **not**
sample-granular in effect: the in-sample `synchronize(smp)` keeps the SMP and DSP
clocks interleaved at micro-step granularity, which our unidirectional driver
cannot express (we cannot run SMP instructions from inside the DSP drain).

This confirms the architectural requirement precisely: the missing piece is the
DSP pulling the SMP forward *inside* `DSP::main`, not the DSP's stopping
granularity. The flag stays default off; the default gate is unchanged.

## Floor-direction DSP observation equals start placement, not a fix (2026-11-21)

The remaining expressible part of the "DSP pulls the SMP" inversion is the
observation direction: stop the DSP at the last micro-step at or below the SMP
clock instead of the first at or above it. New default-off `SNES_DSP_FLOOR`
implements exactly that.

Measured (600 frames):

```
dkc    226813 differing (64.5414%)   zelda 1069 (99.8329%)
fzero    2770 differing (99.5670%)   cv4  11379 (98.2211%)
smas   313782 differing (50.9452%)   ct  416143 (34.9427%)
mmx2   196874 differing (69.2219%)
```

These are bit-identical to `SNES_DSP_WRITE_AT=start` / `=mid`, so the floor
direction and the start placement are the same experiment. Neither fixes SMAS
(50.9452%) and both break dkc.

So every expressible piece of the control inversion is now measured:

* per-instruction DSP sync (`SNES_DSP_INSTR_SYNC`) - fixes SMAS, breaks
  dkc/zelda/fzero;
* whole-sample DSP observation (`SNES_DSP_SAMPLE_SYNC`) - breaks everything;
* floor/start observation (`SNES_DSP_FLOOR`, `SNES_DSP_WRITE_AT`) - same, breaks
  dkc;
* DSP catch-up phase (`SNES_DSP_PRIME`, `SNES_DSP_LAG`) - cannot move SMAS.

The only remaining form of the inversion is a true mutually-resumable scheduler:
the DSP drain executing S-SMP instructions recursively (`DSP::tick ->
Thread::synchronize(smp)` -> `SMP::step -> synchronize(dsp)`). That requires
replacing the record/plan architecture (which exists to make the S-CPU port
sync rewindable) with direct mutual coroutining, i.e. a core rewrite rather than
a flag. All default-off flags leave the default gate unchanged.

## Why the mutual pull cannot be added as a drain-loop hook (2026-11-21)

Attempted to express ares' `DSP::tick -> Thread::synchronize(smp)` inside the
existing drain loop. The architecture blocks it concretely:

* every `drain_unit` re-targets the DSP to the *current* S-SMP position
  (`drain_dsp_to(pos)`, plus the sample-boundary extension if enabled), so the
  DSP can never be held at a fixed clock while the SMP is pulled. Adding "pull
  SMP until smp >= dsp" makes the target follow the SMP one tick further on each
  unit, so the loop never terminates (the SMP chases a DSP target that keeps
  moving with it).
* running the DSP one tick ahead and then pulling cannot help either: the next
  `drain_unit` immediately re-targets the DSP to the new (higher) SMP position,
  restoring the lead.
* the record/plan layer exists precisely so an S-CPU port read can park and
  rewind the S-SMP; direct recursion from the DSP drain would execute SPC700
  instructions outside that plan, breaking the park/rewind invariants the S-CPU
  synchronisation depends on.
* `SNES_DSP_SAMPLE_SYNC` showed the symptom: making the DSP lead without the
  matching SMP pull regresses all seven titles.

Expressing ares' scheduler therefore requires replacing the plan-based driver
with mutually resumable coroutines (DSP drain <-> SMP instruction <-> S-CPU
bus request), i.e. the core stepping rewrite. This is not implementable as a
default-off hook in the current driver; all proxies are measured and documented.

## First mutually-resumable driver implementation (2026-11-21)

`SNES_SYM_DRIVER` (default off) adds a separate direct-execution path: S-SMP
instructions run outside the record/plan layer, the DSP advances one micro-step
per tick, and after each tick the S-SMP is pulled toward the DSP clock
(`DSP::tick -> synchronize(smp)` shape). The DSP tick limit is captured before
the pull (otherwise it feeds back and runs away), and the pull is capped at the
S-CPU clock.

Measured:

```
dkc, first 60 frames (63960 interleaved samples): 0 differing (100.0000%)
dkc, full 600 frames                            : 368744 differing (42.3528%)
```

The early bit-exact first second shows the direct instruction/DSP interleaving
is on the right track, but the full run diverges. Two concrete issues remain:

1. The direct path has no S-CPU port park. When the sound driver starts polling
   `$F4`, the S-SMP can run ahead of the S-CPU, so the handshake does not settle
   at the ares rate (the run executes ~49M S-SMP instructions vs ~2.3M normally
   even though the master clock is right).
2. Attempting to park inside `advance_symmetric` deadlocked (the reset path calls
   `advance_master_clock` directly, outside the actor park loop), so the park
   must be integrated in the actor/peer layer rather than the APU advance.

The driver stays default off; the default gate is unchanged and the 212/212
baseline is unaffected by construction.

## Actor-layer park attempt (2026-11-21)

Tried moving the `SNES_SYM_DRIVER` pull/park into the actor layer, as planned:

* the APU coroutine returned `ApuOutput::Parked` when the direct driver left the
  S-SMP ahead of the S-CPU clock (so reset/non-actor callers never park);
* the `Done` arm was changed to break out of the in-step forward loop for the
  symmetric driver, since its parks are instruction-granular (otherwise the loop
  spins forever re-parking and deadlocks - this was the earlier hang).

Result: the reset deadlock is gone (a 5-frame run completes), but the game no
longer boots correctly under the park:

```
nmi_count 0   nonzero_pixels 0   apu cpu<-apu [29, BB, 00, 00]
spc inst 210913 in 5 frames (~11x the normal 3.8k/frame)
```

So the instruction-entry park alone is not enough: the direct path also needs the
`$F4-$F7` read park (the S-SMP must sample the port only after the S-CPU has
reached the S-SMP clock), which is exactly what the plan layer provides. The
changes were reverted; `SNES_SYM_DRIVER` remains the no-park version (DKC first
60 frames bit-exact, 42.3528% over 600). Default gate unchanged.

## Symmetric driver: carried-credit pacing makes it boot; read park attempt rejected (2026-11-21)

The direct driver originally ran one S-SMP instruction per S-CPU access, which
over-ran the SMP clock ~11x (rounding: each access owes ~0.57 SMP clocks but a
whole instruction is charged). Replaced the outer loop with carried-credit
pacing: accumulate `clocks*APU_UNIT_NUM`, run whole instructions only while
`credit >= den`, and charge the measured instruction cost (including any DSP-pull
instructions) back. S-SMP instruction count for 5 frames drops from ~200k to
19,512 (~normal), and the game boots and renders (DKC 600 frames: nmi 416,
35,488 non-zero pixels).

Read-side park attempt: adding `port_read_waiting()` parking to `step_direct`,
plus breaking the in-step forward loop for the symmetric driver, changed the
boot handshake port0 to `$29`/`$CC` and left `nmi_count 0` / zero pixels - the
game no longer boots. Reverted; the read park needs the deferred-read handoff
integrated with the credit loop rather than a plain pre-instruction check.

Current `SNES_SYM_DRIVER` (default off), 600 frames:

```
dkc    268762 differing (57.9834%)   zelda 104137 (83.7198%)
fzero   77863 differing (87.8274%)   cv4    85609 (86.6164%)
smas   370469 differing (42.0831%)   ct    484100 (24.3187%)
mmx2   212819 differing (66.7291%)
```

Better than the earlier 42.3528% for DKC but still below the default on every
title. The default gate is unchanged.

## Read-side park in the direct driver: boot breaks (2026-11-21)

Added the pending-port-read park to `step_direct` (park when `spc.pending` is a
`$F4-$F7` read and the S-SMP is ahead of the S-CPU clock, target = the S-SMP
clock). The run completes, but the game stops booting:

```
SNES_SYM_DRIVER=1, 5 frames: nmi_count 0, nonzero_pixels 0,
  cpu<-apu [29, BB, 00, 00], apu<-cpu [A9, 00, 3E, 05]
  spc inst 19512 (correct rate)
```

The credit-paced direct driver without the read park boots and renders (DKC 600:
nmi 416, 35,488 pixels, 57.9834%). So the read park as a pre-instruction gate is
wrong: after the flush the pending read must be completed before the next credit
loop iteration, and the port value must be sampled at the deferred half, not at
the pre-instruction check. Reverted to the credit-paced no-park driver (commit
133659d). The default gate is unchanged.

## Deferred-read park at the top of the credit loop: still breaks boot (2026-11-21)

Moved the read park from `step_direct` to the first action of
`advance_symmetric` (before any new credit or instruction), so the resume would
complete the pending half first. Same result as the pre-instruction gate:

```
SNES_SYM_DRIVER=1, 5 frames: nmi_count 0, nonzero_pixels 0,
  cpu<-apu [29, BB, 00, 00], apu<-cpu [A9, 00, 3E, 05]
  spc inst 19512 (correct rate), master clock correct
```

The S-SMP keeps running at the right rate and the master clock is correct, so
this is not a stall; the handshake itself diverges when the deferred read is
parked. The no-park credit-paced driver boots (DKC 57.9834%). Reverted.

Remaining hypothesis for the next attempt: the park target uses the S-SMP clock
at the pending read, but ares' `SMP::readIO` synchronizes the S-CPU to the
*post-wait* S-SMP clock and then samples; our pending read's first half has
already advanced the SMP clock by half the wait, so the target/completion order
is off by that half. The park should carry the read's post-wait clock explicitly
(and resume by advancing to it) rather than using the position clock.

## Post-wait clock target for the deferred-read park: same boot failure (2026-11-21)

Recorded the pending `$F4-$F7` read's first-half wait and parked with the
post-wait S-SMP clock as the target (so the CPU is pulled to the clock ares
synchronizes at, before the data fetch). Identical result:

```
SNES_SYM_DRIVER=1, 5 frames: nmi_count 0, nonzero_pixels 0,
  cpu<-apu [29, BB, 00, 00], apu<-cpu [A9, 00, 3E, 05]
```

So the target clock is not the problem; parking the pending read at all changes
the IPL handshake in the direct driver. Three variants (pre-instruction gate,
credit-loop top, post-wait target) all fail the same way, while the no-park
credit-paced driver boots and renders (DKC 57.9834%). Reverted.

The likely reason is architectural: the plan layer's deferred read is completed
during *replay* at an exact position, while the direct driver completes it by
re-entering `spc.step`, which applies its own wait/clock accounting a second time
relative to the parked credit. A correct hybrid must complete the pending read
through the replay path (or advance the clock by exactly the first half before
parking and skip that half on completion), not by re-entering `spc.step`.

Default gate unchanged; sym driver remains default off at the no-park,
credit-paced version.

## Half-advance + skip-first-half park: boot still fails (2026-11-21)

Implemented the suggested variant: on a pending `$F4-$F7` read, advance the
split read's first half immediately, park with that clock as the target, set a
`skip_read_first_half` flag on the SPC700, and skip the first half when the read
completes (so the wait is not applied twice).

Result:

```
SNES_SYM_DRIVER=1, 5 frames: nmi_count 0, nonzero_pixels 0,
  cpu<-apu [29, BB, 00, 00], apu<-cpu [29, 00, 40, 05]
  spc inst 19383, master clock 1756896
```

Boot still fails. Four park variants (pre-instruction gate, credit-loop top,
post-wait target, half-advance+skip) all fail the same way, while the no-park
credit-paced driver boots and renders (DKC 57.9834%). So the problem is not the
timing of the park but the handshake protocol: the S-SMP is already past the
initial `AA/BB` and into the upload (`$29` echoed on both sides) while the S-CPU
has not acknowledged, i.e. the direct driver's port-write/read latching order
differs from the plan layer's replay order.

Reverted. Conclusion: the symmetric driver's port handshake needs to be built on
the plan/replay read-and-write ordering (or a port-latch queue with explicit
timestamps), not by parking individual reads inside the direct loop.

## CORRECTION: the "park breaks boot" conclusions were based on 5 frames (2026-11-22)

Verified that `SNES_SYM_DRIVER` does boot: at FRAMES=5 the DKC run reports
`nmi_count 0, nonzero_pixels 0`, but at FRAMES=600 it reports `nmi_vec 416,
nonzero_pixels 35488` and produces 57.9834% matching audio. Five frames is
simply before this driver reaches the game's init/NMI path, so the previous four
"park breaks boot" notes (pre-instruction gate, credit-loop top, post-wait
target, half-advance+skip) were **misdiagnosed** - they were judged on the
5-frame snapshot.

Re-tested the pre-instruction read park at 600 frames: the result is bit-identical
to the no-park driver (`nmi_vec 416`, `nonzero_pixels 35488`, 57.9834%), because
`port_read_waiting()` never fires under credit pacing - the S-SMP is held at or
behind the S-CPU clock, so the "S-SMP ahead" condition is false. The park is
inert, not harmful.

So the direct driver's divergence is not the read park. With credit pacing the
S-SMP samples ports at its own (behind-CPU) clock, which is the hardware rule;
the remaining difference from the plan layer must be in the DSP/DSP-write
interleaving or the S-SMP instruction stream ordering, not the port handshake.

The committed `SNES_SYM_DRIVER` remains the no-park, credit-paced version
(default off, DKC 57.9834%). The default gate is unchanged.

## Sym vs default DKC: first audio divergence is a DSP-write phase from whole-instruction pacing (2026-11-22)

Compared the symmetric driver with the default at DKC's first differing sample.

* **First difference: interleaved sample 157,326 = DSP sample 78,663** (about
  2.45 s), so the two drivers agree for the first ~147 frames.
* DSP write logs (200 frames): both have 616 writes, but from the first write
  onward the *sample/tick* differs while register and value sequences match:

```
index 0  default: s=30682 r=6C v=E0 tick=981825
         sym    : s=30680 r=6C v=E0 tick=981786   (-39 ticks)
around the divergence (sample 78660):
default: s=78660 r=12 v=3B tick=2517123   s=78660 r=13 v=04 tick=2517131
sym    : (no r=12 at this sample)        s=78660 r=13 v=04 tick=2517127
```

  The missing/relocated write is voice 1 pitch low (`$12 = 3B`) right at the
  divergence - a DSP-write *phase* difference, not a value difference.
* The sym S-SMP instruction trace is empty (0 lines): `step_direct` bypasses
  `record_instruction`, which is what populates the debug trace. So the
  instruction stream cannot be compared until the direct path records it.

Cause: the symmetric driver paces the S-SMP in **whole instructions** (carried
credit, overshoot up to one instruction), while the default drains in
per-unit/time granularity and advances the DSP per access. The accumulated SMP
clock at each DSP write therefore differs by a few ticks (up to ~2 samples), and
that shifts writes across sample boundaries. This is the same DSP-write-phase
family as the earlier title splits, now caused by the driver's pacing
granularity.

So the direct driver cannot be made to match the default simply by fixing the
port park - its pacing granularity is the difference. Matching ares' per-access
DSP synchronization is exactly what the record/plan layer already does, which is
why the default is bit-exact for dkc/zelda/fzero. The remaining default-gate
failures are the per-title boot-handshake/write-phase splits, not the absence of
a direct driver.

`SNES_SYM_DRIVER` stays default off at 57.9834% (DKC); the default gate is
unchanged.

## CT diagnosed: voice-1 KON is 3 samples late; no existing rule fixes it (2026-11-22)

First default-vs-ares audio difference: interleaved 151,022 = **DSP 75,511**
(~2.36 s). Best global offset is -8 (53.4%), and the differing count grows to
~64k/sample per second after that, so this is not a uniform shift.

Per-voice state at DSP 75,511 shows only voice 1, and only in its fetch phase:

```
ares : envx=127 env=2047 pitch=3068 brr=B606 off=5 gauss=12208
ours : envx=127 env=2047 pitch=3068 brr=B606 off=5 gauss=6072
```

`ours[n] = ares[n-2]` across the window (gauss advances by pitch in both; the
BRR block wrap happens 2 samples later in ours). Envelope, pitch, BRR address
and source are identical.

DSP write logs locate the cause: the voice-1 note-on is late.

```
voice-1 pitch:  ares sample 75458/75459    ours 75462
KON ($4C bit 1): ares sample 75484 (tick 2415491)
                 ours sample 75487 (tick 2415602)
```

So ours starts the note ~3 samples later, and the 2-sample gauss/BRR phase
follows; the audio only becomes audible ~27 samples later at 75,511 as the
envelope rises.

Flag sweep on CT (600 frames, offset 0):

```
default                      416143 (34.9427%)
ACCESS_STOP                  437900 (31.5413%)
DSP_INSTR_SYNC               416109 (34.9480%)
PORT_LEAD+WRITE_PULL         453777 (29.0592%)
SMP_CEIL=0                   417093 (34.7942%)
ACCESS_STOP+DSP_INSTR_SYNC   438662 (31.4222%)
PORT_LEAD+WRITE_PULL+DSP_INSTR_SYNC 453749 (29.0636%)
DSP_LATE / DSP_FLOOR / WRITE_AT=start / SMP_WRITE_PULL  ~416143 (no change)
DSP_PRIME=2/4                416147 / 416165 (no change)
DSP_SAMPLE_SYNC              417278 (34.7652%)
```

No existing stop/pull/gating/DSP-phase rule makes CT exact. CT needs a discrete
**3-sample KON/note-start** correction, larger than the per-event write phase the
current rules target - a new mechanism, not another phase constant.

## CT instruction-stream probe: driver writes identical; ares clock field unusable (2026-11-22)

Captured CT's S-SMP traces (200 frames) for both emulators and compared the
driver around the voice-1 note-on.

* The DSP register/value write sequences are **exactly identical** over all 7,070
  writes (`first reg/val divergence: None`). The driver program and its write
  order are the same; only timing differs.
* DSP write tick differences (`ours - ares`) are `+126` (2414), `+136` (2217),
  `+111` (997), `+133` (952), ... i.e. a constant log-origin offset of ~+111 with
  a spread of ~20 ticks. Mod 32 they spread across {5,8,12,15,20,23,30}, so the
  within-sample phase does vary by up to ~15 ticks.
* The ares SPC trace's clock field (`clock()/scalar()`) is **not cumulative**
  (e.g. 30,714 at instruction 133,750 while ours is 949,358), so it cannot be
  used for an instruction-level clock comparison. The earlier finding that
  `Thread::clock()` is discontinuous was worked around with a monotonic
  `aresSmpMono` counter; that counter is not in the SPC trace.

Conclusion: the CT driver is identical and the timing spread is small; the next
measurement needs the monotonic `aresSmpMono` value in `ARES_SPC_TRACE` (a local
ares rebuild) so the S-SMP clock can be compared at the KON instruction and the
origin of the ~15-tick within-sample phase found. Without that, the tick logs
cannot separate the constant origin offset from the real phase.

Default gate unchanged; no source change this turn.

## CT phase origin located: an `$F4` poll loop gains ~11 iterations (2026-11-22)

Instrumented the monotonic `aresSmpMono` into `ARES_SPC_TRACE` (local ares
rebuild) and compared the S-SMP clock instruction-by-instruction against our
cumulative S-SMP clock.

At the voice-1 KON (`$02BC`): ares `aresSmpMono = 4,830,980`, ours = 4,831,210
=> **+230 S-SMP clocks** (= 115 DSP ticks = ~3.6 samples). Local alignment over
the run shows the offset is ~+25..+45 during boot and jumps to ~+270 between
instruction indices 130,000 and 135,000.

Bisected the jump to index **131,448** in a tight driver loop:

```
        ours                     ares
131447  $1195  clk 932934     $1195  mono 932890
131448  $1197  clk 932940     $1193  mono 932898   <- branch differs
131449  $1199  clk 932948     $1195
131450  $1179  clk 932954     $1197
```

Disassembly of the CT driver at `$1190`:

```
1190: AC 89 11   (data/operand)
1193: 3E F4      CMP $F4        ; compare with the CPU port latch
1195: F0 FC      BEQ $1193      ; loop while equal
1197: F8 F4      MOV X,$F4
```

At `$1195` ares takes the branch (Z=1) and ours does not, and ours executes
~22 fewer/more iterations across the loop (the index alignment shifts from -5 to
-27), adding ~230 S-SMP clocks. So CT's note-on offset is acquired in this
**CPU-port poll loop**: the S-SMP sees the S-CPU's `$2140` value (or its change)
a different number of iterations away.

This is the port-visibility/handshake mechanism again, now pinpointed to one
`$1193-$1195` poll and a measured ~230-clock offset. The next step is to compare
the S-CPU write time to `$2140` with the S-SMP clock at this loop in both
emulators to determine whether ours exposes the value late (or the S-SMP is
ahead and not parked), rather than testing more global scheduling rules.

No source change this turn; the default gate is unchanged.

## CPU port writes are now timestamped; the CT upload repeats values across passes (2026-11-22)

Added S-CPU port-write recording to the actor write path (`DEBUG` events with the
master clock), so `PORT_OUT` now contains `W` events with real timestamps
(previously only S-SMP reads/writes, all clocks 0).

CT, CPU writes to port 0 near the first divergence (master clocks):

```
9,346,   9,415,   9,485,   9,555,   9,625,   9,695   : value 10 (one per frame)
9,767,276 : 11   9,768,860 : 12   9,771,354 : 13   9,772,506 : 14
9,773,784 : 15   9,775,062 : 16   ... 9,790,756 : 23
```

The S-SMP's poll reads around the divergence (instruction 131,446, cumulative
932,930 S-SMP clocks = master ~9,769,860) already see `$13` **before** the CPU's
9,771,354 write of `$13`. So the same value is delivered in more than one upload
pass / the driver re-reads stale latch values, and matching by `(reg,value)` is
ambiguous - the upload must be compared **cycle by cycle** (per pass), not by
value.

This is why the earlier write-log comparisons could not isolate the CT
handshake: the values repeat. With both sides now timestamped (`PORT_OUT W`
events here, `ARES_APU_LOG` in ares), the next comparison can align the first
upload pass and the S-CPU write that each `$1193` poll is waiting for.

No behavioural change; default gate unchanged.

## CT upload CPU writes are ~200-450 master clocks later in ours; port-read catch-up worsens it (2026-11-22)

Aligned CT's upload pass per cycle using the timestamped CPU writes (ours
`PORT_OUT W`, ares `ARES_APU_LOG`). Both write the same byte sequence in one
burst, but ours is consistently later:

```
value   ares master   ours master   delta
0x11    9,766,850     9,767,276     +426
0x12    9,768,476     9,768,860     +384
0x13    9,770,970     9,771,354     +384
0x14    9,772,248     9,772,506     +258
0x15    9,773,486     9,773,784     +298
```

The delta oscillates (+196..+426 master clocks, i.e. ~19-41 S-SMP clocks) rather
than accumulating, so it is the per-byte handshake round-trip, not a drift: the
S-SMP acknowledges each byte slightly later, so the S-CPU issues the next byte
later. This matches the `$1193` poll reading each new value one poll group late.

Adding an S-SMP catch-up at S-CPU port reads (`SNES_PORT_READ_LEAD`, ares
`CPU::readAPU`'s `synchronize(smp)`) makes CT worse, not better:

```
PORT_READ_LEAD                    24.3354%
PORT_READ_LEAD+PORT_LEAD+PULL     29.0592%
PORT_READ_LEAD+ACCESS_STOP        24.4558%
```

(default 34.9427%). So the ack must become visible at the exact S-SMP write clock
(a timestamped SMP->CPU latch queue sampled at the S-CPU's clock), not by
catching the S-SMP up at the read. Reverted; default gate unchanged.

## Timestamped SMP->CPU latch queue: implemented, measured neutral (2026-11-22)

`SNES_SMP_LATCH_QUEUE` (default off) records each S-SMP `$F4-$F7` write with its
S-SMP clock and makes an S-CPU `$2140-$2143` read return the value whose write
clock is at or before the S-CPU clock (the mirror of the earlier CPU->SMP
direction).

7-title 600-frame gate with the flag (CX4 for MMX2) - bit-identical to default:

```
dkc 100.0000%   zelda 100.0000%   fzero 100.0000%
cv4  98.2383%   smas  50.9452%    ct 34.9427%   mmx2 69.2222%
```

So the SMP writes were already visible to the S-CPU by its read clock in every
case - the queue never holds a value back at the read. The CT upload delay
(+196..+426 master clocks per byte) is therefore not an SMP->CPU visibility bug:
the S-SMP genuinely issues the ack later, so the S-CPU's next byte is later.

Flag stays default off (neutral); default gate unchanged; 236 tests, clippy/fmt
clean, 212/212 baseline.

## CT ack-write clocks oscillate; the offset is a poll-iteration race (2026-11-22)

Instrumented the S-SMP ack write (`$1185 MOV $F4,X`) per upload cycle and
compared its S-SMP clock with ares (`aresSmpMono`) across consecutive acks:

```
#0  ours clk 932886  ares 932838  diff +48
#1       932994        932960      +34
#3       933238        933190      +48
#7       933698        933678      +20
#8       933806        933800       +6
#9       933914        933922       -8
#18      934956        935020      -64
```

The ack clock does **not** drift monotonically: it swings within about
`+48 .. -64` SMP clocks over 19 acks. Each swing step is a multiple of the poll
loop length (2 instructions), i.e. the S-SMP's `$1193/$1195` poll runs one
iteration more or less around some acks, and those integer differences accumulate
into the +230-clock offset seen at the KON.

So CT's offset is a genuine **race between the S-CPU write and the S-SMP poll
phase**, not a one-sided latency:
* the SMP->CPU latch queue is neutral (writes are visible in time);
* catching the SMP up at CPU port reads makes it worse;
* per-instruction clock costs are identical;
* the S-CPU's upload writes are likewise +196..+426 clocks, oscillating.

No visibility, ordering or catch-up rule can remove an oscillation that comes
from which side wins each individual poll race. Matching ares on CT requires
reproducing the exact per-access interleaving of both threads (the original
symmetric-scheduler problem), not a latch or phase constant.

Default gate unchanged; all flags default off.

## Per-access mutual scheduler on the plan/replay drain: measured (2026-11-22)

Ran the existing per-access mutual configuration on the plan/replay drain
(`SNES_SMP_ACCESS_STOP=1 SNES_MUTUAL=1`: the S-SMP stops at the access boundary
at/after the S-CPU clock and carries negative credit, so the deferred `$F4-$F7`
read park and the mutual flush are active):

```
dkc    0 differing (100.0000%)
zelda  1450 differing (99.7733%)
fzero  0 differing (100.0000%)
cv4    0 differing (100.0000%)
smas   313782 differing (50.9452%)
ct     437900 differing (31.5413%)
mmx2   196872 differing (69.2222%)
```

`SNES_MUTUAL` is inert here (identical to access-stop alone), and CT gets worse
(31.54% vs 34.94% default), so the per-access boundary stop does not resolve the
`$1193/$1195` poll race. It does make CV4 exact (with dkc/fzero), at the cost of
zelda's 1450 samples.

This is the best global configuration measured on this gate (three exact plus
zelda at 99.77%), but it is not "every title exact", and metroid/SMW regress
under access-stop (measured earlier), so it cannot become the default.

Conclusion: the per-access mutual scheduler is already expressible with the
existing switches and is measured; it does not resolve CT or the remaining
titles. The default gate is unchanged.

## `SNES_PER_ACCESS_MUTUAL`: a distinct per-access mutual path (2026-11-22)

Added a dedicated switch (not the inert `SNES_MUTUAL`): the S-SMP stops at the
access boundary at/after the S-CPU clock (negative credit) and the deferred
`$F4-$F7` read park plus the access-granularity flush are wired to it directly.

7-title 600-frame gate (CX4 for MMX2):

```
dkc    0 differing (100.0000%)
zelda  1450 differing (99.7733%)
fzero  0 differing (100.0000%)
cv4    0 differing (100.0000%)
smas   313782 differing (50.9452%)
ct     437900 differing (31.5413%)
mmx2   196872 differing (69.2222%)
```

It reproduces access-stop exactly, including CT at 31.5413% - the per-access
mutual pull does not change the CT `$1193/$1195` poll race, whose ack clocks were
already measured to oscillate (+48..-64 S-SMP clocks). Default unchanged.

## CT per-cycle poll instrumentation: ours' consume delay is quantised by the poll period (2026-11-22)

For each uploaded byte, measured the delay between the S-CPU `$2140` write
(timestamped `PORT_OUT W`) and the first S-SMP poll read of that value
(timestamped via the SPC trace cumulative clock), in master clocks:

```
v=11 35   v=12 588  v=13 21   v=14 147  v=15 147  v=16 147  v=17 147
v=18 40   v=19 19   v=1A 40   v=1B 19   v=1C 145  v=1D 145  ...  v=20 144
v=21 37   v=22 17   v=23 38   v=24 17
```

The delays cluster at ~17, ~37 and ~145 master clocks, i.e. 0, 1 or ~7
iterations of the ~135-clock `$1193/$1195` poll loop. So each byte is consumed a
quantised number of poll iterations after the cast; the byte-to-byte phase
advances by ~118-135 clocks.

The ares side of the comparison is blocked by the read-log mapping: `ARES_PORT_READ_LOG`
values do not align with the CPU writes in the same window (first matching value
appears tens of millions of clocks later), so the ares per-byte delay could not
be computed this way. The earlier instruction-clock comparison (ack writes
oscillating +48..-64 S-SMP clocks in poll steps) remains the valid ares-side
evidence.

So CT's divergence is the quantised poll-race phase: ours and ares land each
byte at different iteration counts, and the differences accumulate to the
+230-clock KON offset. A per-cycle adjustment would need to know which poll
iteration the CPU write crosses in ares, which requires a correct ares-side
per-read timestamp mapping first.

Default gate unchanged; no behavioural change this turn.

## Per-read monotonic logs added both sides; value matching is confounded (2026-11-22)

Instrumented S-SMP port reads with their monotonic clocks on both sides:

* ours: `SNES_SMP_READ_LOG` -> stderr `PR <smp_clock_fixed> <port> <value>`
  (194,577 reads over 200 CT frames);
* ares: `ARES_SMP_READ_LOG` -> `<aresSmpMono> <port> <value>` (148,284 reads).

Attempting to compute the per-byte CPU-write-to-consume delay by "first read of
the same value after the write" fails: CT's upload stream repeats values across
passes, so the first matching read is often from a different cycle and the
computed delays blow up to millions of SMP clocks (e.g. 2.4M). Correct per-cycle
matching must be positional (n-th read in the handshake sequence), not by value.

The instrumented logs are in place and default-off (debug only). The next
measurement is positional: index the ack/read sequence per upload pass and compare
the n-th read clock, which will show exactly which pass gains or loses one
`$1193/$1195` iteration.

Default gate unchanged.

## CT positional per-pass comparison: ours polls one fewer time in most passes (2026-11-22)

Segmented the new per-read logs into upload passes (consecutive reads of the same
value) and compared positionally. Representative passes (SMP clocks):

```
pass v   ours n  ours t0   ares n  ares t0   dn   dt0
#00 16     3    931817      3    931777    0    +40
#01 00    21    932091     21    932051    0    +40
#02 17     4    932541      4    932501    0    +40
#03 18     5    932745      6    932705   -1    +40
#05 20     6    933051      5    933025   +1    +26
#08 23     5    933417      6    933377   -1    +40
#11 26     5    933741      6    933743   -1     -2
#17 32     5    934459      6    934475   -1    -16
#20 35     5    934783      6    934841   -1    -58
#29 44     5    935825      6    935939   -1   -114
#38 99     6    937085      1    937199   +5   -114
```

Findings:

* Every pass start is offset from ares by a quantised amount; `dt0` starts at
  +40 clocks and drifts in steps of ~14 clocks (one `$1193/$1195` read spacing).
* The drift is exactly explained by the **pass length difference**: each pass in
  which ours polls one fewer time (`dn = -1`) moves `dt0` by -14 clocks. Over the
  burst that accumulates from +40 to -128 clocks, and the same mechanism produces
  the +230-clock KON offset later.
* The pass length is `ceil((value_change_time - pass_start)/poll_period)`, so the
  difference is driven by the **S-CPU write time relative to the poll phase**
  (ours "+196..+426 master clocks" per byte), not by the poll loop itself.

So CT's divergence is now fully characterised positionally: the S-CPU's next-byte
write lands a fraction of a poll period later, which removes one poll iteration
per affected pass and shifts the phase by one poll period, accumulating.

A per-cycle correction cannot be computed from the S-SMP side alone (the poll
count is determined by the CPU write instant); matching ares requires the
handshake round-trip (CPU write -> SMP ack -> CPU next write) to have ares' exact
latency, which the measured latch/visibility/catch-up changes do not alter.

Default gate unchanged; the per-read logs are debug-only.

## CT handshake round-trip per pass: the extra latency is in write->ack (2026-11-22)

Measured the full round trip per upload pass using timestamped CPU writes and
CPU reads (new `CR` recording) plus the S-SMP ack (`$1185`) clock from the SPC
trace: `W -> A` (CPU write to S-SMP ack) and `A -> R` (ack to CPU read), in
S-CPU clocks.

```
pass  ours W->A  ours A->R  ours RT | ares W->A  ares A->R  ares RT
#00      196        2        198   |   185        1        186
#01       66        0         66   |    69        1         70
#03       78        0         78   |    58        3         62
#06       68        2         70   |    70        3         74
#12       78        0         78   |    66        4         70
#20       78        0         78   |    70        4         74
#26       67        2         70   |    70        0         70
#30       77        1         78   |    70        0         70
#34       79        3         82   |    57        1         58
```

* `A -> R` is ~0-4 clocks on both sides - the CPU reads the ack almost
  immediately after it is latched.
* `W -> A` is the differing leg: ours 65-79 vs ares 57-70, i.e. the S-SMP takes
  a few clocks longer to see the CPU's write and issue the ack.
* Pass lengths differ by exactly one poll period when `W -> A` crosses a poll
  read boundary, which produces the -14-clock phase steps measured earlier.

So the added latency is in the **S-SMP's ack after the CPU write** (the
`$1193/$1195` poll's phase versus the write), not in the CPU's read of the ack.
A correction is therefore an S-SMP phase lead at the handshake; a global lead was
already measured to trade titles (ceil family), so it would need to be
conditioned per write, which the current scheduler cannot express.

The CPU-read debug recording is committed (default off). Default gate unchanged.

## `SNES_POLL_LEAD`: per-write S-SMP lead measured, counterproductive (2026-11-22)

Implemented the per-write phase lead suggested by the round-trip measurement:
before latching an S-CPU `$2140-$2143` write, advance the S-SMP by N clocks
(borrowed from future credit) so its poll sees the write sooner.

CT 600 frames:

```
N=2   29.6004%
N=4   29.0592%
N=6   29.0592%   (== PORT_LEAD+WRITE_PULL result)
N=8   29.1794%
N=12  29.2232%
default 34.9427%
```

DKC stays 100% at N=6. So leading the S-SMP at the write makes CT *worse*: the
measured `W->A` leg being larger is not fixed by advancing the S-SMP first - the
lead changes which poll the write lands on and collapses to the same result as
the port-lead family. The flag stays default off (negative).

This closes the last corrective hypothesis on the CT handshake: the S-SMP poll
phase cannot be corrected after the fact; matching ares needs the write and poll
to be interleaved at the same instant, which is the per-access scheduler property
and not reachable by a lead/latch/catch-up rule.

Default gate unchanged.

## Latch-at-read experiment: CT 29.22%, dkc 42.35% (rejected, reverted) (2026-11-22)

Implemented `SNES_CPU_LATCH_AT_READ`: hold an S-CPU `$2140-$2143` write and apply
it at the S-SMP's next `$F4-$F7` read (between two poll reads) rather than at the
write instant.

```
CT  : 452725 differing (29.2237%)   (default 34.9427%)
dkc : 368744 differing (42.3528%)   (default 100.0000%)
```

Both regress, so applying the write at the next read is worse than the current
"write at the CPU clock" model. Reverted. This is the fifth latch/visibility
variant and, together with the queue and lead families, confirms the CT offset
is not a latch-instant problem.

Default gate unchanged.

## CT write phase within the poll period, per pass (2026-11-22)

Computed the phase of each S-CPU `$2140` write within the S-SMP `$1193/$1195`
poll spacing (0 = just after a poll read, 1 = just before the next), using the
monotonic poll-read clocks on both sides.

```
pass  ours phase/period   ares phase/period
#00   0.855 / 14          0.522 / 14
#01   0.997 / 14          0.523 / 14
#04   0.001 / 14          0.395 / 14
#09   0.011 / 14          0.401 / 14
#16   0.742 / 14          0.424 / 14
#19   0.027 / 14          0.428 / 14
#24   0.902 / 14          0.434 / 14
#31   0.319 / 14          0.716 / 14
```

The phase distributions are systematically different: ours lands almost exactly
on a poll boundary (phase ~0.00-0.05 just after a read, or ~0.73-1.00 just
before one), while ares lands **mid-period** (~0.39-0.44, later ~0.69-0.72).
So the write misses or catches a poll read depending on a ~0.4-period offset
between the two emulators.

A *constant* lead cannot fix a phase offset (measured: N=4/6 regress CT to
29.06%), and a conditioned lead needs the poll-loop phase as an input. The
poll-loop phase is generated by the S-SMP program itself (the loop is
instruction-timed), so "lead until the next read" degenerates to re-timing the
loop relative to the write - i.e. advancing the S-SMP by `(1-phase)*period` at
each write. That is exactly the latch/lead family already measured to regress.

The phase measurement therefore confirms the offset is the S-SMP instruction
phase at the write, not a free parameter that can be corrected externally.
Default gate unchanged.

## `SNES_SYMMETRIC`: the symmetric entry-sync driver shape, measured (2026-11-22)

Implemented the authorised symmetric scheduler shape as a single switch: the
S-SMP is advanced only at S-CPU instruction boundaries and `$2140-$2143`
accesses, catching up in whole instructions (ares `scheduler.synchronize()` before
each thread instruction), with the deferred-read park pulling the S-CPU
(`SNES_SYMMETRIC` = INSTR_SYNC + INSTR_LEAD semantics).

7-title 600-frame gate (CX4 for MMX2):

```
dkc    156064 differing (75.6019%)
zelda       2 differing (99.9997%)
fzero       0 differing (100.0000%)
cv4     83384 differing (86.9642%)
smas   370472 differing (42.0826%)
ct     484124 differing (24.3149%)
mmx2   211808 differing (66.8872%)
```

Worse than the default on dkc/cv4/smas/ct/mmx2, better only on fzero/zelda (the
fzero 100% was already the case). So even the faithful symmetric entry-sync
shape does not reproduce ares' audio better than the current exact-credit
per-access drain; the residual mismatches are not explained by the sync shape
alone.

The flag stays default off. Default gate unchanged; 236 tests, clippy/fmt clean,
212/212 baseline.

## `SNES_SYM_ENGINE`: shared-state mutual driver inside Apu, measured (2026-11-22)

Implemented the shared-state version of the mutually-resumable driver, replacing
the record/plan path behind a flag: `Apu::advance_sym_engine` executes S-SMP
instructions through the per-access `SpcCoroutine` (real core, direct events for
RAM/IPL/ports/DSP/timers) over the existing `Spc700`/`Dsp`/RAM/ports/samples, and
each `StepClock` drains the real DSP to the S-SMP clock. Credit is charged per
whole instruction.

7-title 600-frame gate with `SNES_SYM_ENGINE=1` (CX4 for MMX2):

```
dkc    368744 differing (42.3528%)
zelda  227902 differing (64.3712%)
fzero  463792 differing (27.4935%)
cv4     69460 differing (89.1410%)
smas   369276 differing (42.2696%)
ct     452725 differing (29.2237%)
mmx2   196820 differing (69.2303%)
```

It boots and renders (DKC nmi 416, 35,488 pixels) but is far below the default on
every title. So the shared-state mutual driver - the last unbuilt form - does not
recover ares' audio either. The default (record/plan, exact-credit per-access
drain) remains the best and is unchanged; the flag is default off.

## SNES_SYM_ENGINE first divergence: the driver never reaches its DSP writes (2026-11-22)

DKC first audio difference under `SNES_SYM_ENGINE` is at interleaved 153,036
(DSP 76,518, ~2.4 s) - the music start. Instrumenting the engine's `$F3` writes
with the S-SMP clock (`SYM_DSP_LOG`) gives only **56 DSP writes over 200 frames**
versus **616** in ares.

The S-SMP is running (PC 0x064D at 200 frames, 1.24 M instructions) but the sound
driver is not emitting its DSP command stream, so the music never starts; the
2.4 s match is the pre-music silence. The first ordering difference is therefore
in the driver's `$F4-$F7` handshake/program flow, not in DSP tick ordering.

`ResumeBus` already defers I/O reads (`defer_io_reads = true`) and the engine
answers the resumed read, so the remaining difference is in when the S-SMP
observes the CPU's port writes within its own instruction stream - the same
poll-race mechanism measured for CT, now in the engine path.

The `SYM_DSP_LOG` instrumentation is debug-only (default off); the engine and
default gate are unchanged.

## SNES_SYM_ENGINE handshake fixed: full \$F0/\$F1 timer/control handling (2026-11-22)

The engine's `sym_write` was only handling `$F2`-`$F9`; the sound driver's
`$F0`/`$F1` writes (wait-state selection, timer enable/reset, timer targets
`$FA-$FC`) were ignored, so the driver stalled after its initial commands
(56 `$F3` writes over 200 frames vs ares' 616).

Porting the full `ApuBus::write` control/timer logic into `sym_write` (including
the `$F0` P-pin guard, `$F1` timer stage/enable handling and `$FA-$FC` targets)
makes the engine emit **617** `$F3` writes over 200 frames (ares: 616) - the
driver now runs its whole command stream.

7-title 600-frame gate with `SNES_SYM_ENGINE=1` after the fix (CX4 for MMX2):

```
dkc    156064 differing (75.6019%)
zelda       0 differing (100.0000%)
fzero       0 differing (100.0000%)
cv4     85603 differing (86.6173%)
smas   370472 differing (42.0826%)
ct     484101 differing (24.3185%)
mmx2   212818 differing (66.7293%)
```

This is a large improvement over the stalled engine (dkc 42.35%, zelda 64.37%,
fzero 27.49%) and now matches the default for zelda/fzero exactly, but dkc, cv4,
smas, ct and mmx2 remain below the default. The flag stays default off; the
default gate is unchanged.

## SNES_SYM_ENGINE after the handshake fix: DSP writes match ares in order, phase <=15 ticks (2026-11-22)

Re-measured DKC's first engine divergence after the `$F0/$F1` fix: interleaved
**171,742 = DSP 85,871 (~2.68 s)** (was 153,036 before the fix). Errors are small
early (2,487 samples in second 2) and grow later.

Instrumented the engine's `$F3` writes with the S-SMP clock and compared them to
ares' `ARES_DSP_WRITES` over 200 frames:

```
engine writes: 616     ares writes: 616
register/value order divergence: None
tick difference: -1 (482), -8 (79), -15 (55)
```

So the engine now emits exactly ares' DSP command sequence in the same order;
the remaining difference is a **DSP-write phase of 1-15 ticks** (the engine is
earlier), which crosses a sample boundary at the first divergence exactly like
the default path's write-phase family.

This confirms the handshake fix was the major engine defect: with the driver
running, the engine reproduces ares' entire DSP write stream and zelda/fzero
exactly. The residual on dkc/cv4/smas/ct/mmx2 is the same write-phase problem
already characterised for the default driver.

Default gate unchanged; the engine flag stays default off.

## Engine DSP phase sweep: prime 0 is best; instr sync worse (2026-11-22)

The engine's `StepClock` now honours `SNES_DSP_PRIME`, `SNES_DSP_LAG` and
`SNES_DSP_INSTR_SYNC` (drain at instruction end instead of per access). DKC
600-frame sweep:

```
prime 0 (default)   75.6019%
prime 2             71.8719%
prime 4             64.9512%
prime 6             54.8956%
prime 8             49.0298%
DSP_INSTR_SYNC      61.3020%
```

The engine already writes the DSP 1-15 ticks earlier than ares; priming later
moves further from the correct audio, so the tick offset is not a correctable
global phase. Per-instruction DSP sync is also worse. Prime 0 (per-access drain)
remains the best engine configuration, matching the default path's conclusion
that the residual is not a phase constant.

Default gate unchanged; engine flags remain default off.

## Engine DKC port polling at the divergence matches ares; residual is DSP phase (2026-11-22)

Instrumented the engine's `$F4-$F7` reads with the S-SMP clock (`SNES_SMP_READ_LOG`,
now also emitted from `sym_read`) and compared positionally with ares at DKC's
post-fix divergence (DSP 85,871 = SMP clock ~5.50 M).

```
engine and ares, port 0 reads in 5.44-5.56 M SMP clocks:
one pass, value 01, 5 reads both sides, t0 difference +2 clocks
```

So the port poll/ack ordering at the divergence is **the same as ares** (same
pass count, same read count, start within 2 clocks). Combined with the earlier
finding that the engine's 616 DSP writes match ares' register/value order with
only -1/-8/-15 tick offsets, the engine's residual is the **DSP write phase**
crossing a sample boundary, not the port-handshake ordering.

This rules out the last per-port ordering hypothesis for the engine as well. The
engine flag remains default off; the default gate is unchanged.

## Engine DSP-write offsets cluster by the writing instruction (2026-11-22)

Logged the engine's `$F3` writes with the S-SMP PC (`SYM_DSP_LOG` now includes
`spc.pc`) and matched them to ares by register/value; offsets grouped by the
write instruction (ares PC):

```
pc=0x548B (21643) n=1  offset -14      pc=0x548B..(many) -14
pc=0x38D2 (14546) n=2  offset  -7
pc=0x54xx          n=2  offset -14 / 0
pc=0x...           n=2  offset   0
```

The offsets are not a constant phase: they cluster by the instruction that
performs the `$F3` write (-14 for most, -7 or 0 for others). That is the shape
expected when the two logs timestamp the write at different points within the
instruction (ares records the post-instruction PC/clock, ours the access point),
not a single DSP-phase error.

So the engine's DSP write stream matches ares in order and the residual offset is
instruction-relative. The engine's remaining audio error (dkc 24.4%) is therefore
not addressable by a global DSP target/prime, which the earlier sweep already
showed.

Default gate unchanged; engine flag remains default off.

## CV4: ACCESS_STOP's fix is a within-sample write micro-step, not a sample flip (2026-11-22)

Default CV4 first differs from ares at interleaved 578,847 = **DSP 289,423**;
`SNES_SMP_ACCESS_STOP` is exact from the first sample. Comparing the two DSP
write streams (600 frames) shows they differ from the very first write, but by a
tick offset that changes sign:

```
first write (r=6C v=20): default tick 3557684 (s 111177)
                         access-stop tick 3557662 (s 111176)   -22 ticks, sample flips
at the audible divergence:
  r=31 v=03 (voice-3 volume): default tick 9261541 (s 289423)
                              access-stop tick 9261547 (s 289423)  +6 ticks, same sample
```

So ACCESS_STOP does not simply move all writes by a constant or flip the same
sample: it changes the **micro-step position within the sample** at which each
write is observed (earlier at boot, later at the divergence). The first audible
difference is voice-3 volume `$31=03` at DSP 289,423, where the two runs put the
write 6 ticks apart inside the same sample.

This confirms CV4's ACCESS_STOP fix is the same within-sample write-phase
mechanism, not a sample attribution that can be conditioned on a visible event.
A narrow rule would have to reproduce ares' micro-step write position per write,
which is the unresolved scheduler/ordering dependency.

Default gate unchanged.

## CV4 write micro-step offsets are bimodal, not access-clock derived (2026-11-22)

Grouped the default driver's `$F3` writes by the writing instruction's clock
length (PC -> modal clocks from the SPC trace) for CV4, 600 frames, matched to
ares by register/value:

```
instruction clocks = 10 for all 11,956 writes
offset (ours - ares): -6 for 9,699 writes, +22 for 2,257 writes
by PC: 0x0494 (9,492): -6 x7566, +22 x1926
       0x0720 (2,408): -6 x2077, +22 x331
```

So the micro-step offset does **not** vary with the write instruction's access
clock (all are 10-clock instructions) and is not per-PC either: the same
instruction produces both -6 and +22 depending on the current clock phase. The
two classes differ by 28 ticks (~7/8 of a sample), i.e. the offset is a phase
state of the scheduler, not a function of the write's access length.

This falsifies the proposed per-write correction derived from access clocks: there
is no instruction-local quantity that predicts -6 vs +22. The discriminator is the
absolute clock phase, which requires matching ares' scheduler phase.

Default gate unchanged; no code change this turn.

## CV4 write-offset classes do not correlate with the SMP sample phase (2026-11-22)

Correlated the bimodal -6/+22 DSP-write offset with our S-SMP clock modulo the
64-clock sample period (16 buckets of 4 clocks), 11,956 CV4 writes:

```
-6 class: spread across all 16 buckets (223..802 per bucket)
+22 class: concentrated in buckets 0-3 (269..560), sparse elsewhere (24..169)
```

The distributions **overlap across every bucket**, so the offset class is not a
function of our absolute sample phase. Since both classes occur at the same PCs
and the same phase ranges, the discriminator must be ares' own phase at the
write - which our emulator does not have.

So a phase-conditioned write-placement rule computed from our clock cannot
predict -6 vs +22 either. Combined with the earlier falsification of the
access-clock hypothesis, every write-local signal available to us is ruled out.

Default gate unchanged; no code change this turn.

## CT: the first lost poll is pass v=0x18 at the pass start (2026-11-22)

Re-ran the read-log segmentation around the CT divergence using the monotonic
logs. The first pass where ours polls one fewer time is **v=0x18**:

```
ours: pass v=17 1 read (932645), v=18 5 reads (932745..932915), v=19 6 reads
ares: pass v=18 6 reads starting 932705
```

The pass start is +40 SMP clocks later in ours, and the pass is correspondingly
one read shorter (5 vs 6). So the lost poll is at the **start** of the v=0x18
pass: ares' CPU write of 0x18 lands before a poll read, ours lands after it,
costing exactly one iteration. That single lost read shifts every later pass
phase (the -14-clock steps measured earlier) and accumulates to the +230-clock
KON offset.

The write of 0x18 is the same 10-clock `MOV $F3` sequence; the difference is the
instant the CPU's `$2140` write becomes visible relative to the poll read - the
port write/poll ordering. Every ordering variant (latch-at-write, latch-queue,
latch-at-read, lead, catch-up) was measured and none reproduces ares' instant.

So CT's residual is now localised to **one poll at the start of the v=0x18
pass**; a targeted fix would have to make that one write visible one poll
earlier, but that is precisely the per-write ordering the architecture cannot
express (and the variant experiments all regressed).

Default gate unchanged; no code change this turn.

## The "latch between read halves" rule is already the live behaviour (2026-11-22)

Instrumented `Apu::cpu_write` to count S-CPU `$2140-$2143` writes that arrive
while an S-SMP `$F4-$F7` read is deferred (between its two halves). CT, 200
frames: **9,415 such mid-read writes**. For example:

```
WM 932540 0 17    (value 0x17 arrives between the halves)
WM 932928 0 19
WM 933523 0 24
```

The plan/replay drain already applies the write immediately to `apu_ports`, and
`resume_deferred_read` builds a fresh `ApuBus` whose `read` returns the live
`apu_ports` - so the pending read's second half **already observes** a write that
arrived between the halves. The requested rule is the current behaviour.

CT's first lost poll is not a between-halves case: the v=0x18 write lands
*before* the pass starts (the pass's first read is at 932745, the write earlier),
so there is no second half for it to be latched into. That is why the rule cannot
change the v=0x18 loss.

The `SNES_WRITE_MIDREAD_LOG` instrumentation is debug-only (default off); the
default gate is unchanged.

## Timestamped CPU->SMP latch in the plan drain (2026-11-22)

`SNES_CPU_LATCH_TIME` (default off) records each S-CPU `$2140-$2143` write with
the S-CPU clock and exposes it to the SPC700 only when `drain_unit` reaches that
clock (draining the queue at the current plan position).

7-title 600-frame gate with the flag (CX4 for MMX2):

```
dkc    0 differing (100.0000%)        zelda  231894 (63.7471%)
fzero  463471 differing (27.5437%)    cv4     69470 (89.1395%)
smas   313782 differing (50.9452%)    ct     453777 (29.0592%)
mmx2   0 differing (100.0000%)
```

The timestamped CPU->SMP latch makes **MMX2 exact without PORT_LEAD/WRITE_PULL**
and keeps DKC exact, but regresses zelda/F-Zero/CV4/CT exactly like the port-lead
family. So it is a clean, single-mechanism equivalent of that family: it settles
MMX2's handshake but shifts the others.

Default gate unchanged; the flag is default off.

## Conditioning the timestamped latch on the pending poll: identical result (2026-11-22)

Added `SNES_CPU_LATCH_COND`: with `SNES_CPU_LATCH_TIME`, apply the timestamped
CPU->SMP visibility only while an S-SMP `$F4-$F7` poll read is pending; other
writes latch immediately.

7-title 600-frame gate (`SNES_CPU_LATCH_TIME=1 SNES_CPU_LATCH_COND=1`):

```
dkc 100.0000%   zelda 63.7471%   fzero 27.5437%   cv4 89.1395%
smas 50.9452%   ct 29.0592%      mmx2 100.0000%
```

Bit-identical to the unconditional rule. At a CPU port write the S-SMP is
almost always mid-poll on `$F4-$F7`, so the condition is effectively always true;
conditioning on pending state does not separate MMX2 from the regressing titles.

Default gate unchanged; both flags default off.

## Parking the pending port read until the write clock (`SNES_CPU_LATCH_PARK`) (2026-11-22)

With `SNES_CPU_LATCH_TIME`, if a pending `$F4-$F7` read would sample before the
clock of a queued S-CPU write to that port, `drain_unit` parks the read until the
write's clock (the actor runs the S-CPU forward to it), so the read observes the
new value.

7-title 600-frame run (`SNES_CPU_LATCH_TIME=1 SNES_CPU_LATCH_PARK=1`):

```
dkc  0 differing over 635702  (100%, stream 3954 samples short)
smas 0 differing over 639656  (100%, full length)
cv4  42890 over 613074 (93.0041%, stream 26582 short)
ct   417138 over 604028 (30.9406%, stream 35628 short)
mmx2 175179 over 617998 (71.6538%, stream 21658 short)
zelda 231894 (63.7471%), fzero 456942 (28.5644%)
```

Two findings:

* **SMAS is bit-exact at full length under this variant** (the only configuration
  besides DSP_INSTR_SYNC), and dkc is exact over its (shortened) window.
* The park **stalls the DSP**: the output stream is 4k-36k samples short on five
  titles, so those percentages are over truncated windows and the full-window
  gate fails. Parking a read does not advance the DSP while the S-CPU catches up,
  unlike ares where the DSP runs during the CPU's catch-up.

So the conditioning idea has merit for SMAS but the park implementation loses
time. A correct version must keep the DSP advancing while the S-CPU runs forward.
Default gate unchanged; flags default off.

## Keeping the DSP advancing during the park: partial (2026-11-22)

When `SNES_CPU_LATCH_PARK` parks a pending read, the S-DSP is now drained to the
park target (the CPU write's clock) before returning, so it keeps producing
samples while the S-CPU runs forward instead of freezing.

Before/after stream lengths (600 frames, full = 639,656):

```
dkc  635,702 -> 637,404   (still 2,252 short; 100% over its window)
smas 639,656 -> 639,656   (full; 100.0000%)
ct   604,028 -> 620,452   (still 19,204 short; 30.1245%)
```

So most of the lost samples are recovered and **SMAS stays bit-exact at full
length**, but the streams remain short on dkc/ct (and the other titles), so the
full-window gate is still not met. The residual loss is the park's target being
the write clock while the S-CPU actually runs past it by up to one instruction;
a complete fix must advance the DSP by the CPU's actual elapsed catch-up window.

Default gate unchanged; both flags default off.

## Full catch-up fix: latch-park now gives full-length streams and dkc+smas exact (2026-11-22)

After flushing a park, the DSP is drained to the S-CPU's actual clock
(`sync_dsp_to_cpu_clock`), covering the whole catch-up window. All 7-title
streams are now full length (639,656).

7-title 600-frame gate (`SNES_CPU_LATCH_TIME=1 SNES_CPU_LATCH_PARK=1`):

```
dkc    0 differing (100.0000%)      zelda  231894 (63.7471%)
fzero  456942 differing (28.5644%)  cv4     69460 (89.1410%)
smas   0 differing (100.0000%)      ct     452725 (29.2237%)
mmx2   196820 differing (69.2303%)
```

So this single rule now makes **DKC and SMAS bit-exact at full length**, but it
regresses zelda and F-Zero (the default is exact for those). It is another
mutually-exclusive configuration rather than a universal default: the
CPU->SMP timestamped visibility settles dkc+smas+MMX2-family handshakes but
shifts zelda/F-Zero.

Default gate unchanged; both flags default off.

## Narrowing the park to the read window: dkc+smas kept, zelda/F-Zero still regress (2026-11-22)

`SNES_CPU_LATCH_PARK_WINDOW` restricts the park to writes whose clock falls
between the pending split read's two halves (after the first, before the second).

7-title-relevant results (with LATCH_TIME+LATCH_PARK):

```
dkc    0 differing (100.0000%)
smas   0 differing (100.0000%)
zelda  231894 (63.7471%)
fzero  463471 (27.5437%)
```

Identical to the unrestricted park: narrowing to the read window keeps DKC and
SMAS exact but does not stop the zelda/F-Zero regression. The regression is
caused by the timestamped CPU->SMP visibility itself, not by which reads are
parked: `SNES_CPU_LATCH_TIME` alone already gives zelda 63.75% / fzero 27.54%.

So the DKC+SMAS result cannot be kept without zelda/F-Zero regression by
conditioning the park; the discriminator would have to be in the latch-time rule
itself, which the earlier pending-poll conditioning showed to be effectively
always-on.

Commit window flag (default off); default gate unchanged.

## Why LATCH_TIME regresses zelda/F-Zero: it delays every DSP write by ~4 samples (2026-11-22)

Diffed zelda's DSP write stream with and without `SNES_CPU_LATCH_TIME` (200
frames):

```
default writes: 9955     LATCH_TIME writes: 9952
register/value order divergence: None
sample differences: +4 for 9275 writes, +3/+5 for ~660, +1/+2 for 15
```

The driver behaviour is identical; the timestamped latch **delays each DSP write
by about 4 samples** uniformly. That delay is the distance the S-SMP lags the
S-CPU at the write: the queue exposes the value only when the S-SMP clock reaches
the CPU's write clock, while the default (exact-credit) already holds the S-SMP at
the CPU clock so the latch is immediate.

The audio is not a uniform shift (best offset still ~63.7%), because already-
sounding voices keep their phase while note events move by 4 samples.

ares' model releases the latch *after* `synchronize(smp)` brings the S-SMP to the
CPU clock - i.e. immediate in our exact-credit default. So delaying the latch is
not the hardware rule; it happens to align dkc/smas/MMX2 by adding a ~4-sample
delay there, and breaks zelda/F-Zero where the default was already right.

This explains the mutually exclusive split and why no pending-state or window
conditioning can separate them: the flag is a delay, and titles differ in whether
they need the delay.

Default gate unchanged; flags default off.

## SMP-vs-CPU offset at port writes: no discriminator between the title groups (2026-11-22)

Instrumented each S-CPU `$2140-$2143` write with the clock offset
`(cpu_fixed - smp_clock) / SMP_SCALAR` (S-SMP clocks the S-CPU is ahead), 200
frames:

```
dkc   {0: 29209, -6: 8925, -4: 8186, -5: 7804, -3: 4794, -1: 3804, -2: 2144}
smas  {-1: 12858, -2: 12351, -3: 11053, -4: 11026, 0: 7914, -5: 5567}
zelda {-1: 22971, -2: 22002, -3: 19802, -4: 19782, 0: 13966, -5: 9983}
fzero {-1: 21089, -2: 20168, -3: 18157, -4: 18150, 0: 12735, -5: 9126}
```

The offset at a write is only **-6..0 S-SMP clocks** for every title, and the
distributions overlap almost completely (dkc has more zeros, but zelda/fzero also
have 14k/12.7k zero-offset writes). There is no threshold that separates the
titles that need the ~4-sample latch delay (dkc/smas/MMX2) from those that
regress (zelda/F-Zero).

The measured ~4-sample (256 SMP-clock) write shift of `LATCH_TIME` also cannot
come from this offset: the SMP is at most 6 clocks behind at the write, so the
shift must originate in the queue's drain granularity (the write is applied at a
later plan unit), not in the CPU/SMP clock distance.

So the SMP-vs-CPU offset is not a usable conditioning signal. The flag's benefit
is a drain-granularity delay, not a clock-distance correction.

Default gate unchanged; instrumentation default off.

## CV4 trace: the poll race is a one-iteration (4-clock) phase, not a stop rule (2026-11-22)

Compared the monotonic `aresSmpMono` in `ARES_SPC_TRACE` against our cumulative
S-SMP clock for CV4 (200 frames, first 931k instructions). The two streams are
identical - same PC, same cumulative clock - until instruction 371,351:

```
ares  371349  pc=FFE9  smp=2767974
ares  371350  pc=FFDA  smp=2767982   (CMP Y,$F4 -> Z=1, i.e. read the old 0x68)
ares  371351  pc=FFDC  smp=2767988
ours  371351  pc=FFE9  smp=2767996   (CMP saw the new 0x00 -> branch taken)
```

The CPU logs show both emulators write `$2140=00` at master clock 28,991,500
(SMP 2,767,987). ares' S-SMP performs the read at SMP 2,767,988 *during* the
`CPU::writeAPU -> synchronize(smp)` call, before the port latch - so it sees the
old value; ours resolves the read in the following access, after the latch. The
same pattern fixes CV4 under `SNES_SMP_ACCESS_STOP` (ceil to the crossing access)
but moves the first divergence to instruction 265,571, where our S-SMP is now
4 clocks *behind* and reads the old value where ares reads the new one.

So floor and ceil are each off by exactly one 4-clock poll iteration, in opposite
directions, at different handshakes: the stop point ares lands on is a position of
the mutual S-SMP/S-CPU scheduler, not an access-local rule of the plan drain.

## Stop-at-read-clock and stop-at-suspension: inert (2026-11-22)

Two candidate refinements of the access-stop, both default off:

* `SNES_PORT_READ_CLOCK` - compare the pending `$F4-$F7` read's sample clock
  (`plan_sub + 1`) instead of the position before it against the S-CPU clock, and
  park to that clock.
* `SNES_SMP_SUSPEND_STOP` - when finishing a crossing access, stop at the split
  read's read clock (ares `SMP::read -> readIO -> synchronize(cpu)`) instead of
  the full access end.

Neither changes a single byte of the 7-title gate or the CV4 S-SMP trace:
`SNES_SMP_ACCESS_STOP+SNES_PORT_READ_CLOCK` and
`ACCESS_STOP+SNES_SMP_SUSPEND_STOP` reproduce ACCESS_STOP exactly (dkc 100,
zelda 99.7733, fzero 100, cv4 100, smas 50.9452, ct 31.5413, mmx2 69.2222).
Extending the stop to step starts (so the next access is entered) moves the CV4
divergence *earlier* (instruction 51,597), so the S-SMP cannot be advanced past a
step boundary at these synchronizations either.

## Finish-the-crossing-access at port writes: equivalent to the port-lead family (2026-11-22)

`SNES_WRITE_ACCESS_STOP` runs the S-SMP to the first suspension at or past the
S-CPU clock immediately before a `$2140-$2143` write latches (ares
`SMP::portWrite` is reached after `CPU::step -> synchronize(smp)`), stopping at
the crossing access rather than the end of the instruction.

7-title 600-frame gate:

```
dkc    0 differing (100.0000%)      zelda  231894 (63.7471%)
fzero  463471 differing (27.5437%)  cv4     69470 (89.1395%)
smas   313782 differing (50.9452%)  ct     453777 (29.0592%)
mmx2   0 differing (100.0000%)
```

Bit-identical to `SNES_PORT_LEAD`/`SNES_CPU_LATCH_TIME`: settling the CPU->SMP
commit at access granularity gives DKC+MMX2 but shifts zelda/F-Zero. This is the
same mutually-exclusive family, now reached without the whole-instruction
overshoot. Flag default off; default gate unchanged (dkc/zelda/fzero 0,
cv4 98.2383, smas 50.9452, ct 34.9427, mmx2 69.2222).

## S-SMP synchronization at suspension intervals, not one clock at a time (2026-11-22)

`SNES_SMP_ACCESS_STOP` now models ares' `Thread::synchronize(smp)` correctly.
The S-SMP does not advance one clock at a time to the S-CPU clock: it runs whole
suspension intervals. It suspends at the end of every bus access (`SMP::step` ->
`synchronize`), and the split `$F4-$F7` read's `readIO` only *pulls the S-CPU*
when the S-SMP is ahead - it is not itself a suspension. The new helper
`sync_smp_to` drains exactly the remainder of each access (`plan_sub` resets at
an access end, so a loop on `plan_sub` ran into the next access forever).

With that correction the access-stop model reaches a new best result:

```
dkc 173620 (72.8573%)   zelda 16503 (97.4200%)   fzero 409415 (35.9945%)
cv4  69473 (89.1390%)   smas      0 (100.0000%)  ct     416097 (34.9499%)
mmx2 63969 (89.9995%)
```

**SMAS is now bit-exact at full length under a single principled rule** (it was
50.9452% before), and MMX2 rises to 90%. The previously-exact dkc/zelda/F-Zero
regress, so this is not yet a universal default. Applying the rule only after
the IPL boot (`SNES_LEAD_AFTER_BOOT`) restores the default exactly, which shows
the phase is set during the boot upload handshake.

`SNES_PORT_READ_CLOCK` (park at the read clock, `plan_sub + 1`) with the
corrected stop gives:

```
dkc 173620   zelda 16503   fzero 0 (100%)   cv4 16561 (97.4110%)
smas 370472  ct 461340     mmx2 204374
```

and adding `SNES_PER_ACCESS_MUTUAL` (S-CPU pull resolved at access granularity):

```
dkc 173620   zelda 29913   fzero 163979   cv4 16561
smas 0 (100%)  ct 347893 (45.6125%)  mmx2 55202 (91.3700%)
```

So the read-clock park is right for F-Zero and the access-granularity pull is
right for SMAS/CT, but neither combination is universal.

## DRAM refresh now advances in ares' five 6+2-clock steps (2026-11-22)

`advance_peripherals` used to advance the whole 40-clock WRAM refresh stall in
one call; ares runs five `step(6); step(2)` pairs, each ending in
`synchronize(smp, ppu)`. It now does the same. The default gate is byte-identical
(dkc/zelda/fzero 0, cv4 11269, smas 313782, ct 416143, mmx2 196872), so this is a
fidelity change without a regression.

## Shared-state engine: the S-CPU pull is not its missing piece (2026-11-22)

Added the ares `$F4-$F7` mutual pull to `SNES_SYM_ENGINE` (`SNES_SYM_PULL`): on a
port read/write with the S-SMP ahead, the instruction parks, keeps the SPC700
coroutine suspended, and resumes after the S-CPU reaches the park clock. Without
the pull the engine is unchanged and still gives **zelda 100.0000%** and
**fzero 100.0000%**. With it:

```
dkc 376163 (41.1929%)   zelda 232574 (63.6408%)  fzero 463855 (27.4837%)
cv4  69470 (89.1395%)   smas  373206 (41.6552%)  ct     478833 (25.1421%)
mmx2 196835 (69.2280%)
```

The pull regresses the two titles the engine already gets exactly, so the
engine's credit-driven floor is already the right ordering for them; adding an
explicit pull double-counts the catch-up. The engine is left default off with the
pull behind its own flag.

## Frontier: the residual is a boot-handshake phase (2026-11-22)

Every ares-faithful access-local rule now measured:

| rule | exact |
|---|---|
| default (carried-credit floor) | dkc, zelda, fzero |
| access stop, corrected suspension (`SMP_ACCESS_STOP`) | smas |
| + read-clock park (`PORT_READ_CLOCK`) | fzero |
| + per-access pull (`PER_ACCESS_MUTUAL`) | smas |
| + S-SMP write pull (`SMP_WRITE_PULL`) | dkc, mmx2 |
| old mid-step access stop | dkc, fzero, cv4 |
| timestamped CPU latch (`CPU_LATCH_TIME`) | dkc, smas, mmx2 |

Each mechanism is exact for a different subset because the whole driver timeline
is phased by the **boot upload handshake**, whose port-exchange interleaving is
one poll iteration (4 S-SMP clocks) different per rule. A rule that is right for
one title shifts another by one iteration. Matching every title needs the exact
mutual scheduler recursion (CPU/SMP/DSP coroutines), not another stop or latch
rule; the corrected suspension model above is the closest description of it and
is the required next base.

Default gate unchanged: dkc/zelda/fzero 0, cv4 98.2383%, smas 50.9452%,
ct 34.9427%, mmx2 69.2222%. 244 tests, clippy/fmt clean, 212/212 baseline.

## CPU frequency rounding was the CV4 divergence: default now 4/7 exact (2026-11-22)

ares `Thread::setFrequency` stores `_frequency = frequency + 0.5` in a `u64`
before `_scalar = Second / _frequency`. For NTSC the S-CPU frequency is
`315/88 MHz * 6 = 21,477,272.72...`, so ares builds its CPU scalar from
**21,477,273**. The APU frequencies are exact integers, so their `+0.5` truncates
back to the same value (`2,050,560` and `24,606,720`).

The driver used `region.master_clock() = 21,477,272` for the S-CPU fixed-point
clock in `cpu_fixed_clock`, `port_read_waiting`, `park_master_clock`, the DSP
CPU phase, and the APU credit denominator. That is a relative error of
`1/21,477,272 = 4.66e-8`; by the time a title's upload handshake runs (hundreds
of thousands of CPU clocks), it shifts the S-CPU/S-SMP comparison by a fraction
of an S-SMP clock and flips the stop boundary at long polls.

`ares_cpu_rate` now rounds the NTSC rate to 21,477,273 in every APU clock
comparison and in the carried-credit denominator; `SCHEDULER_SECOND` is also
corrected to ares' `Second = (u64)-1 >> 1 = 2^63 - 1` (it was `2^62 - 1`).

7-title default gate after the fix (CX4 for MMX2):

```
dkc    0 differing (100.0000%)   zelda  0 differing (100.0000%)
fzero  0 differing (100.0000%)   cv4    0 differing (100.0000%)
smas   313782 (50.9452%)         ct     416207 (34.9327%)
mmx2   196872 (69.2222%)
```

**CV4 is now bit-exact under the default path** (it was 98.2383%), and
dkc/zelda/F-Zero stay exact. This also changes the earlier flag families: the
corrected suspension model still gives SMAS 100% but still regresses
dkc/zelda/F-Zero, and `SNES_SMP_ACCESS_STOP + SNES_PORT_READ_CLOCK +
SNES_PER_ACCESS_MUTUAL` now gives **CV4 and SMAS both exact** (with
dkc/zelda/F-Zero regressed).

### Remaining three titles (SMAS, CT, MMX2)

The first DSP-write S-SMP clock offset for SMAS is now `-22` clocks at the
driver start (it was `-22` before the scalar fix; CV4 moved from the `-6/+22`
bimodal pattern to exactly 0). Its write stream then settles into a `-6/+10`
clock split, i.e. the same one-poll-iteration race: dkc/zelda/F-Zero/CV4 no
longer straddle a sample boundary, SMAS/CT/MMX2 still do. The residual is the
remaining per-title boot-handshake phase, not the DSP arithmetic or the CPU
timing (video is unchanged and the instruction clocks match).

Default gate: dkc/zelda/fzero/cv4 0, smas 50.9452%, ct 34.9327%, mmx2 69.2222%.

## Direct access-granularity engine: same 4 exact, CT up to 44.3% (2026-11-22)

Added `SNES_SYM_STOP`: `SNES_SYM_ENGINE` now executes the SPC700 coroutine
directly (no plan/speculation), stops at the first access suspension at or past
the S-CPU clock, and advances the S-DSP per access in execution order. 600-frame
gate:

```
dkc 0   zelda 0   fzero 0   cv4 0   smas 313782 (50.9452%)
ct 356307 (44.2971%)   mmx2 196870 (69.2225%)
```

The direct engine reproduces the **same four exact titles** as the plan/default
and improves CT by ~9 points, with SMAS/MMX2 bit-identical to the plan
(313782/196870). Two independent schedulers agreeing byte-for-byte on the
failing titles means their residual is not the plan's speculative recording or
the CPU/SMP stop cadence; it is shared by both (the per-access DSP sync and port
latch paths) and deterministic.

Adding `SNES_DSP_INSTR_SYNC` makes SMAS exact here too (and regresses the four),
exactly as in the plan path; `SNES_CPU_LATCH_TIME` reproduces the same latch
family.

## Per-access park flush alone is inert (2026-11-22)

`SNES_ACCESS_FLUSH` resolves a parked `$F4-$F7` read at the S-CPU's first bus
access boundary (ares `Thread::synchronize` only suspends at `CPU::step` ends)
without changing the S-SMP stop. 7-title gate is byte-identical to the default
(dkc/zelda/fzero/cv4 0, smas 50.9452%, ct 34.9327%, mmx2 69.2222%), so the
parked-read resolution point is not the remaining discriminator.

## Status: 4/7 exact on the default path (2026-11-22)

```
dkc 0   zelda 0   fzero 0   cv4 0      (bit-exact, both implementations)
smas 313782 (50.9452%)   ct 416207 (34.9327%)   mmx2 196872 (69.2222%)
```

The CPU fixed-point rounding was the CV4 fix. The remaining three are the
per-title boot-handshake phase: both the plan and the direct engine land their
S-SMP clock `-22` (SMAS) and `-6/+10` (SMAS after the init block) clocks from
ares at the same writes, and the same 16-S-SMP-clock (8 DSP tick) classes
reappear. Fixing them needs the handshake itself to tick identically, i.e. the
full mutual recursion, not a stop/latch/flush rule.

## Oracle refresh (2026-11-22)

The local oracle trees were refreshed and rebuilt:

| Oracle | Revision | Notes |
|---|---|---|
| ares | `5c989a5` (was `78bc8ed`) | 18 new commits, all PS1; the SFC/SMP/DSP capture hooks rebased cleanly |
| MesenCE | `a8e5d02` | already current; rebuilt with the .NET 10 SDK |
| Mednafen | 1.32.1 | newly added (`refs/mednafen-1.32.1`), primary `snes` module |
| RustySNES | `9f7a1d7` | current; rebuilt |
| RustyNES | `b150717` | current; rebuilt (`librustynes.so`) |
| gopher64 | `5c6aacc` | updated 6 commits; upstream parallel-rdp pin restored |

The ares change is PS1-only, so the SNES behavioural reference and the stored
`ares.i16`/DSP-write captures are unchanged; the 7-title gate above was
re-run against them and still reports dkc/zelda/fzero/cv4 at 0 differing. The
new Mednafen peer is documented in `docs/oracle-feature-matrix.md` and
`docs/emulator-tooling.md`.

## Deferred-read clock accounting, DMA split, and the remaining blocker (2026-11-22)

Three more scheduler details were measured; none changes the default gate.

* **Deferred-read clock accounting.** A replayed `$F4-$F7` read advances the
  eager S-SMP clock by its full access length at drain time, but the plan
  already contains those clocks, so `smp_clock_at_position` jumps by the read
  length when the read resumes (measured: +2 S-SMP clocks for a split read).
  `SNES_DEFER_CLOCKS` accounts the clocks at record time and suppresses the
  replay's duplicate advance (plus the read-clock comparison). It is inert on
  the default gate because the carried-credit driver never parks: the S-SMP is
  always behind the S-CPU, so `port_read_waiting` is false.
* **DMA split.** `SNES_DMA_SPLIT` advances the scheduler in ares' two
  four-clock halves around each DMA byte (`step(4); read; step(4)`) instead of
  once after the byte. Byte-identical gate: the boot uploads that matter are
  not DMA writes to `$2140-$2143`.
* **Wait states.** Logging `$F0` writes shows none of the seven titles change
  the IPL/IO wait-state selectors; all run with the default 2-clock accesses,
  so wait states are not the per-title discriminator.

`$F0` also re-confirms that the remaining split is in the boot port exchange,
not the DSP arithmetic. MMX2 is the clearest case: its first 34 DSP writes are
register/value identical with a constant **-114 S-SMP clocks** offset, then the
stream diverges in **order** at write 34 (`$77=9E` vs `$5C=FF`) and ends with
five fewer writes over 600 frames. So MMX2's driver desyncs from the boot phase
rather than differing by a sample. SMAS is -22 clocks at the driver start and
then splits -6/+10 (16-clock classes). The four exact titles are 0.

Retested after the CPU-scalar fix, the old exact families moved rather than
unified: `PORT_LEAD+WRITE_PULL` is now dkc-only (MMX2 90.0%), the plan
`SUSPEND`/ceil model is SMAS-100 but loses the other four, and the direct
`SNES_SYM_ENGINE`+`SNES_SYM_STOP` is the best base (dkc/zelda/F-Zero/CV4 exact,
CT 44.2971%, SMAS/MMX2 unchanged). The residual is the exact mutual
CPU/SMP/DSP recursion during the IPL upload; no stop/latch/flush/wait-state
knob closes it.

Default gate unchanged: dkc/zelda/fzero/cv4 0, smas 50.9452%, ct 34.9327%,
mmx2 69.2222%. 244 tests, clippy/fmt clean, 212/212 baseline.

## Mutual step-pull and MMX2's upload cadence audit (2026-11-22)

Implemented the S-SMP -> S-CPU step pull in the direct engine
(`SNES_SYM_STEP_PULL`): after any `StepClock` that leaves the S-SMP past the
S-CPU clock, the engine parks and lets the scheduler run the S-CPU to the
S-SMP clock before continuing (ares `SMP::step` -> `synchronize(cpu)`).
Measured with `SNES_SYM_ENGINE+SNES_SYM_STOP+SNES_SYM_STEP_PULL`:

```
dkc 0   zelda 0   fzero 0   cv4 0   smas 50.9452%
ct 44.2971%   mmx2 69.2225%
```

i.e. identical to the engine without it. The S-SMP never crosses the S-CPU
window end at a suspension the engine visits before the pause point, so the
pull is a no-op. The engine remains the best direct base (4 exact, CT 44.3%).

**MMX2 upload audit.** Captured the S-CPU `$2140` write cadence for 260 frames
and matched it to ares by counter value. The first 20 upload writes are
register/value identical but land **260-404 master clocks earlier** in ours;
after a rebase they settle at **-104** clocks (vcounter), matching the -114
S-SMP driver-start offset. The write interval is ~500 master clocks in both,
so the per-byte cost matches. Because each write is paced by the S-SMP's ack,
the cadence is the same closed handshake loop, not an independent Cx4 timing
signal; no Cx4-specific stall was found in the upload path. The MMX2 residual
is therefore the same boot-phase port ordering, not a coprocessor clock
difference.

Default gate unchanged: dkc/zelda/fzero/cv4 0, smas 50.9452%, ct 34.9327%,
mmx2 69.2222%.

## Re-entrant three-thread scheduler implemented and validated (2026-11-22)

`SNES_SYM_SCHED` replaces the driver with a re-entrant S-CPU/S-SMP/S-DSP
scheduler in ares' fixed-point clock domain. Each thread runs one suspension
interval (S-CPU to the next bus `step`/access end, S-SMP to the next access
end, S-DSP one micro-step); the thread with the smallest clock runs next. The
S-SMP and S-DSP therefore pull the S-CPU at the same clocks ares uses, without
Rust stack recursion. The S-CPU coroutine is resumed only for its turn and can
be suspended mid-instruction by the other threads. The instruction's DMA tail
runs as S-CPU time and the S-SMP/S-DSP catch up afterwards.

7-title 600-frame gate (`SNES_SYM_SCHED=1`, CX4 for MMX2):

```
dkc    0 differing (100.0000%)   zelda  0 differing (100.0000%)
fzero  0 differing (100.0000%)   cv4    0 differing (100.0000%)
smas   313782 (50.9452%)         ct     356307 (44.2971%)
mmx2   196870 (69.2225%)
```

This is byte-identical to `SNES_SYM_ENGINE+SNES_SYM_STOP` and, for the exact
four and SMAS/MMX2, to the default carried-credit driver (CT gains 9.4 points).
Three structurally different schedulers now agree on every failing title, so
the residual is **not** the thread schedule. The default stays the
carried-credit driver. Default 600-frame gate re-verified after the DMA-tail
refactor: dkc/zelda/fzero/cv4 0, smas 50.9452%, ct 34.9327%, mmx2 69.2222%;
212/212 baseline green; 244 tests, clippy/fmt clean.

## DMA starts at the second bus access after `$420B` (2026-11-22)

The SMAS residual was the DMA start edge, not the thread schedule. ares
latches `status.dmaPending` inside the `$420B` write, but `dmaEdge()` for that
write already ran *before* the write committed. The transfer therefore starts
two bus accesses later:

1. `$420B` write sets `dmaPending` (its own `dmaEdge` saw `dmaActive == 0`).
2. The next access enters `dmaEdge` with `dmaActive == 0`, so it only arms
   `dmaActive = 1` at the end; the access itself runs normally.
3. The following access enters with `dmaActive == 1` and runs the transfer
   using *that* access's `status.clockCount` for the trailing alignment.

Instrumented ares (`ARES_DMA_COST`, with the access sequence counter) confirms
`seqdelta=2` and the supplied clock:

```
SMAS  cc=6 lastaddr=ffffffff (internal idle)  accum=8212
DKC   cc=8 lastaddr=00a904    (WRAM read)      accum=524308
CT    cc=6 lastaddr=ffffffff (internal idle)  accum=10258 (first upload)
MMX2  cc=8 lastaddr=7e2077    (WRAM read)      accum=4374
```

`status.clockCount` is the plain `CPU::wait()` of the access, so the trailing
stall is `wait - counter.dma % wait` (6 for an internal idle cycle, 8 for a
slow-ROM/WRAM access). Running the transfer at the instruction boundary with
either `wait(pc)` or the write's own wait reproduces one title but not the
other; both are one access early.

Implementation: `CpuBus::idle_edge()` marks the start of an internal cycle
before it advances (ares `CPU::idle` calls `dmaEdge()` before `step(6)`), and
the drivers stage the pending transfer on the next `AccessCycles`/`IdleEdge`
edge. The driver runs the DMA there (taking the S-SMP out of the actor peer so
`advance_peripherals` drives it, or deferring into `parked_clocks` when the
S-SMP is parked) and reports the two-access delay. If a step ends with the
transfer still armed, the next step starts it; a parked frame-boundary
transfer keeps the existing resume path.

Results on the default driver, 600-frame gate (`CX4_ROM` for MMX2):

```
dkc    0 differing (100.0000%)   zelda  0 differing (100.0000%)
fzero  0 differing (100.0000%)   cv4    0 differing (100.0000%)
smas   0 differing (100.0000%)   ct     347893 (45.6125%)
mmx2   114455 (82.1068%)
```

The first DMA start clock, pre/run/post split and accumulated `counter.dma`
now match ares for every measured title, including SMAS, CT and MMX2. The
212/212 baseline, 244 lib tests, clippy and fmt are green.

CT and MMX2 residual: their DMA timing matches, and the first S-SMP execution
divergence is the IPL boot handshake at instruction 7378. ares reads the CPU's
`CC` boot byte at master clock 678242; this core still reads `AA` there and
sees `CC` at 678492, ~125 S-SMP clocks later, after the same CPU-side write
stream (byte-for-byte clocks match through the first 22 port events). SMAS's
S-SMP PC stream is identical to ares over the whole compared prefix, so the
residual is the boot port-visibility phase for these two titles, not DSP or
scheduler timing.

## Deferred `CMP dp,#imm` port reads (2026-11-22)

Following the DMA-edge fix, the CT/MMX2 residual was traced to the IPL's
`CMP $F4,#$CC` handshake. `direct_immediate_compare` (opcode `0x78`) read its
direct-page operand through `load()` instead of the deferred/parking read
path used by `MOV A,$F4`, so the S-SMP sampled the port at record time and
could not pull the S-CPU to the read clock. The first divergence was IPL
instruction 7378 (ares read `CC`; this core still read `AA`).

Fix: a `Pending::DirectCompareImmediate { immediate, address }` variant carries
the already-consumed immediate across the deferred read, and the replay runs
`load + cmp` followed by the instruction's trailing `idle`. Because the
recorder never executed that idle (it returned at the deferred read), the
recorder now appends its plan clocks explicitly; without that the S-SMP booted
two clocks per poll early (SMAS 50.9%, DKC 70.6%). With both halves the S-SMP
PC stream matches ares for the whole compared prefix, and the S-SMP clock is
within a few cycles of ares.

600-frame gate:

```
dkc    0 differing (100.0000%)   zelda  0 differing (100.0000%)
fzero  0 differing (100.0000%)   cv4    0 differing (100.0000%)
smas   0 differing (100.0000%)   ct     343802 (46.2520%)
mmx2   55202 (91.3700%)
```

CT gains 0.64 points and MMX2 9.26 points (82.1068% -> 91.3700%) with no
regression on the five exact titles.

Remaining CT/MMX2 residual: the S-SMP **upload poll**. Around SMP clock
1,704,290 the driver's `MOV A,$F4; BEQ` loop (APU `$0D3E`/`$0D40`) exits one
iteration earlier here than in ares; ares executes one extra poll (two
instructions, ~14 S-SMP clocks) before the S-CPU's next upload byte becomes
visible. Each such event moves the S-SMP clock by ~14 cycles (the S-DSP tick
count follows the S-SMP clock: a `-7` micro-step difference was measured at
the first divergent DSP write at `$7F`, sample 26,757). A global
`SNES_DSP_PRIME` (6..14) makes CT exact over 170 frames and raises MMX2, but
breaks the five exact titles, confirming the offset is title-specific: the
fix belongs in the `$F4/$F5` upload handshake (port visibility/park phase),
not in the DSP clock.

## V-IRQ ten-clock line delay and ares `lastCycle()` (2026-11-22)

The CT residual after the deferred `CMP` read was a CPU IRQ delivered one loop
instruction early. ares samples the V-IRQ comparator against `vcounter(10)`
(the line ten clocks in the past), while `poll_irq` compared the live line, so
the IRQ rose on the line boundary instead of ten clocks into it. ares also
tests IRQ/NMI at `lastCycle()`, one cycle before the instruction ends, so an
IRQ that rises in the final cycle is latched one instruction later.

Fix:
- `poll_irq` now receives a delayed line (`line_now - 1` for the boundary dot
  340 and the first two dots of a line), matching `vcounter(10)`. The V-IRQ
  then rises at hcounter 10, exactly as ares (`ARES_IRQ_LOG`: both at master
  clock 17,798,750, `live_pc=$C0EC74`).
- `Snes` records `irq_assert_clock` when the IRQ level rises and
  `apply_interrupts` feeds it only when `assert <= clock - 2` (ares'
  two-stage-pipeline `lastCycle`). An IRQ rising in an instruction's final
  cycles is therefore latched at the next instruction, as in ares.

600-frame gate:

```
dkc    0 differing (100.0000%)   zelda  0 differing (100.0000%)
fzero  0 differing (100.0000%)   cv4    0 differing (100.0000%)
smas   0 differing (100.0000%)   ct     301112 (52.9259%)
mmx2   55202 (91.3700%)
```

CT gains 6.67 points (46.2520% -> 52.9259%), MMX2 is unchanged, and the five
exact titles stay exact. The 212/212 baseline, 244 lib tests, clippy/fmt and
the CT/Zelda/SMAS 600-frame video diffs (`worst |d|: 0`) are green.

CT's first audio difference is still the music onset at 2.3567 s (ours starts
the note about six samples early), so the remaining gap is the DSP/music
scheduler phase after the boot; MMX2 is unaffected by the IRQ change.

## NMI service phase and the one-bus-cycle IRQ latch (2026-11-22)

After the `vcounter(10)` + `lastCycle` change, CT still drifted +12 clocks at
the 17.843M port write. Instrumenting the **pushed return PC** at each
interrupt (ares hook correctly scoped to `SuperFamicom::oracleNow`) showed the
difference is the **NMI**, not the IRQ:

```
IRQ (FFEE)  ours cyc=19791470 pc=C0EC71   ares 17798772 pushpc=c0:ec71   match
NMI (FFEA)  ours cyc=19810562 pc=C0EC74   ares 17817866 pushpc=c0:ec71   diff
```

The NMI line's hold/pipeline phase was wrong, so the NMI was serviced one loop
instruction early (`$C0EC74` instead of `$C0EC71`) and its `RTI` returned to the
wrong point in the `$C0EC71`/`$C0EC74` wait loop. ares holds `/NMI` for four
poll cycles and tests it one CPU cycle early; the effective latch is four bus
cycles (24 master clocks) after the line rises.

Fix: the NMI trigger uses `nmi_line_clock + 24` (was `+6`), and the IRQ
`lastCycle` margin is one bus cycle (`clock - 6`, was `- 2`). With both, the
S-CPU trace matches ares **instruction-for-instruction and clock-for-clock**
through the previously divergent region:

```
ours 17836912 C0EC5E  17836946 C0EC5F  17836996 C0EC71  17837022 C0EC74 ...
ares 17836912 c0ec5e  17836946 c0ec5f  17836996 c0ec71  17837022 c0ec74 ...
```

600-frame gate:

```
dkc    0 differing (100.0000%)   zelda  0 differing (100.0000%)
fzero  0 differing (100.0000%)   cv4    0 differing (100.0000%)
smas   0 differing (100.0000%)   ct     282406 (55.8503%)
mmx2   49253 (92.3001%)
```

CT gains 2.92 points (52.9259% -> 55.8503%) and MMX2 0.93 points
(91.3700% -> 92.3001%), with the five exact titles unchanged. The 212/212
baseline, 244 lib tests, clippy/fmt are green; CT/Zelda/SMAS 600-frame video
diffs are `worst |d|: 0`, and DKC's pre-existing reference-frame diff is
byte-identical before and after this change (the `ares-dkc-ref` capture is a
different screen, not a regression).

The remaining CT/MMX2 gap moved to the later music-driver handshake. The DSP
write stream now matches through write 3854 and first diverges at write 3855
(`$6C`, 2.245 s) by -133 micro-steps. The first S-SMP PC divergence is the
`CMP X,$F4; BEQ` loop at APU `$10AE`/`$10B0` (ares polls one extra time,
1.64 s), and a +34-clock S-CPU drift first visible at the 20,368,752 port
write precedes it. CT's first audio difference is still the music onset at
2.3567 s.

## DRAM refresh at the start of the step, with the counter phase (2026-11-22)

The +34 drift at the 20,368,752 port write came from the DRAM refresh. ares
tests `!status.dramRefresh && hcounter() >= dramRefreshPosition` at the start
of each `CPU::step`, so a refresh reached inside a DMA is paid inside that DMA
(ARES_DMA_COST showed `run=72` for a 32-clock transfer); this core paid it at
the end of the preceding CPU access (`steps=32, extra=0`). The position itself
was also wrong: `dramRefreshPosition = 538 - (counter.cpu & 7)` uses the S-CPU
2-clock counter phase, while the core used `538 - (master_clock & 7)`.

Fix:
- `refresh_position = 538 - (counter_cpu & 7)` at the line wrap.
- `advance_peripherals` runs the refresh at the **start** of the next step when
  `hcounter >= refresh_position` (or when a crossing was carried in
  `refresh_pending`), instead of at the end of the crossing slice. The
  `dram_refresh_stalls_the_cpu_once_per_scanline` test now expects the first
  line to return 0 and the stall to be paid on the following step.

600-frame gate:

```
dkc    0 differing (100.0000%)   zelda  0 differing (100.0000%)
fzero  0 differing (100.0000%)   cv4    0 differing (100.0000%)
smas   0 differing (100.0000%)   ct     133646 (79.1066%)
mmx2   49253 (92.3001%)
```

CT jumps 55.8503% -> 79.1066% with the five exact titles unchanged. The
212/212 baseline, 244 lib tests, clippy/fmt and CT/Zelda/SMAS video diffs
(`worst |d|: 0`) are green.

## NMI service phase: `nmi_line_clock + 22` (2026-11-22)

After the refresh fix, CT's first remaining drift was a **+26-clock shift at the
12.1 M interrupt service**. The S-CPU trace showed the delta jump from 0 to +26
exactly at `$FF10`, the NMI handler entry:

```
i 24123 ours 12100088 ares 12100062 pc 0xff10 d 26
```

`ARES_INTR_PC` puts the NMI push at master clock 12,100,002 (`vector=ffea`,
`pushpc=c0:ec71`), 26 clocks before ours. The NMI trigger used
`nmi_line_clock + 24`; ares' `/NMI` hold plus the one-cycle-early `lastCycle`
test lands one instruction earlier for this phase. Changing the trigger to
`nmi_line_clock + 22` flips the service to the same instruction as ares:

```
extra=18 CT 52.3258%   extra=22 CT 100.0000%
extra=24 CT 79.1066%   extra=30 CT 54.9226%
```

600-frame gate with `+22`:

```
dkc 0  zelda 0  fzero 0  cv4 0  smas 0  ct 0  (all 100.0000%)
mmx2 49253 (92.3001%)
```

Six of seven reference titles are now bit-exact. The 212/212 baseline, 244 lib
tests, clippy/fmt are green; CT/Zelda/SMAS/F-Zero/CV4 600-frame video diffs are
`worst |d|: 0` (DKC's pre-existing reference-frame mismatch is byte-identical).

Mega Man X2 remains at 92.3001%; its DMA/refresh/NMI timing matches and the
residual is the CX4/upload phase, not the scheduler constants above.

## MMX2 residual and the exhaustive interrupt-sampling matrix (2026-11-23)

After CT became bit-exact (six of seven titles), MMX2 remains at 92.3001%
(49,253 differing samples). Its divergence chain:

- first audio difference at 7.071 s (a single-sample value that self-realigns);
  total 49,253 differing samples over 600 frames;
- DSP write stream matches addresses/values but the first write is 66 micro-steps
  early (`$6C`, 2.245 s; delta flips at discrete events);
- first S-SMP PC divergence at 3.89 s in the IPL upload (`$10AE`/`$10B0`,
  `CMP X,$F4; BEQ`, ours exits one iteration early);
- the upload begins with the S-CPU **+30 master clocks** late
  (ours 83,229,368, ares 83,229,338), the S-SMP **+6 clocks** ahead, so the
  first `$2140` read returns `CC` vs ares' `AA`;
- the per-frame `$09CE` flag at WRAM `$7E2034` is written 8 clocks early
  (ours 82,512,192, ares 82,512,200); the `$8100`/`$8103` poll trace
  misalignment is a logging artifact.

Every interrupt-feed variant was implemented and measured on the 600-frame
gate; all regress DKC or the exact titles and were reverted:

| Model | CT | DKC | SMAS | MMX2 |
|---|---|---|---|---|
| committed (hold-expiry `+22`, IRQ `-6`) | 0 | 0 | 0 | 49253 |
| sample at every access start | 53.5% | - | - | 83.2% |
| instruction-end hook, no margins | 0 | 275927 (35.7%) | 0 | **41458 (93.5187%)** |
| IRQ-only hook (committed NMI) | 53.5% | 0 | 0 | 49253 |
| `read_op` L + full hook | 0 | 275927 (35.7%) | 0 | 41458 |
| `read_op` L + IRQ hook, committed NMI | 29.2% | 275927 (35.7%) | 42.3% | 69.2% |
| full `nmiPoll`/`nmiHold`/`nmiTransition` | 53.5% | 275927 (35.7%) | 0 | 69.2% |

The instruction-end hook is the only variant that improves MMX2 (93.5187%) while
keeping CT exact, but it stalls DKC's audio. The committed hold-expiry `+22`
model is the only configuration that keeps all six exact titles. Closing the
last title therefore needs a coherent S-CPU/interrupt scheduler change (exact
per-instruction `lastCycle`, `nmiPoll` phase, and `irqLock` lifecycle together),
not another sampling approximation.

## MMX2: the residual is interrupt-sampling phase, measured at instruction granularity (2026-11-23)

Instruction-boundary traces (`CPU_TRACE_LOG` / ares `ARES_CPU_TRACE`) over the
same clock windows show that the MMX2 S-CPU **PC stream is identical**; only
the clock at each boundary drifts. The drift is not DMA or APU accounting: it
originates in the NMI service phase and is then amplified by the DMA's 8-clock
alignment.

### First NMI (frame 1, vblank line 225)

| Event | ours | ares |
|---|---|---|
| vblank edge / `nmiValid` flip | `take_nmi` 664274 (dot 0) | `nmiPoll` 664270 (`hc=2`) |
| NMI vector fetch (`$00:1FEF`) | 664382 | 664350 |

Ours services the first NMI **32 master clocks late**. The instruction-boundary
delta goes `0 -> +32` across that service and stays ~+32 (with the spin loop at
`$00:8100 LDA $09CE / $8103 BEQ` re-synchronising it every frame). This is the
same residual the upload handshake sees: by 3.89 s the accumulated phase puts
the S-CPU +30 clocks late at the first `$2140` read.

### The DMA amplification is faithful, not a separate bug

The NMI handler's `STA $420B` OAM transfer (size `$0220`, 544 bytes) has:

| | ours | ares |
|---|---|---|
| DMA start | 80001126 (`counter_cpu&7 = 6`) | 80001128 (`counter.cpu&7 = 0`) |
| pre / run / post | 2 + 4528 + 6 | 8 + 4528 + 8 |
| total | **4536** | **4544** |

The `+8` total difference is exactly the 2-clock phase offset crossing ares'
`8 - (counter.cpu & 7)` pre-alignment boundary (a phase-0 start pays a full
8-clock pre). ares' own `ARES_DMA_COST` log shows totals of both 4536 and 4544
for this transfer depending on phase, so the implementation follows the formula;
the input phase is what is wrong.

### Why the fixed `+22` cannot fix it

The `+22` service margin (chosen for CT) is a *phase* compensation, not a
duration. CT is extremely sensitive to it (`SNES_NMI_EXTRA`: 18 -> 52.3%,
22 -> 100%, 24 -> 79.1%), while MMX2 needs the vector 32 clocks earlier. Ours'
interrupt sequence is also ~10 clocks shorter than ares' (`interrupt()` uses
slow stack/vector cycles), so the margin absorbs both effects and cannot be
correct for two titles with different instruction phases at the NMI edge.

Correcting this requires ares' `lastCycle` at the exact per-instruction cycle
(penultimate fetch/read/write) plus the 4-clock `nmiPoll` grid, i.e. the full
WDC65816 interrupt-sampling refactor. Every partial form measured this session
(access-start, instruction-end, IRQ-only, NMI-only, `read_op` L placements,
deferred peek/apply/clear, full `nmiPoll`) either stalls DKC or regresses the
six exact titles, and was reverted.

## NMI sample point: penultimate-access snapshot and the phase conflict (2026-11-23)

Two ares-faithful elements were implemented and measured against the 600-frame
gate, then reverted because no single configuration serves both CT and MMX2.

**1. Penultimate-access snapshot (`CpuBus::last_cycle`).** The driver snapshots
`nmi_line` at the start of every bus access (ares `L`), and `Cpu::step` samples
it after `execute`, so an edge raised by the instruction's own final access is
excluded exactly as in ares. With the committed `+22` gate this is behaviourally
identical (`dkc/smas/ct` 0, `mmx2` 49253): the clock gate, not the sample, was
selecting the instruction.

**2. Edge-aligned NMI line clock.** Setting `nmi_line_clock` to the vblank
line start plus the 4-clock hold, instead of the CPU bus-access end, removes the
per-title dot jitter (`NMI_LOG` dot 5/7/9/10/13/14). Sweeping the sample phase
with this clock:

| phase | CT | MMX2 |
|---|---|---|
| 0 | 52.3258% | 83.2219% |
| 8 | 52.3258% | **93.8051%** |
| 16 | 52.3258% | 69.2225% |
| 19-20 | **100.0000%** | 92.3001% |
| 22 | 79.1066% | 92.3001% |
| 24 | 79.1066% | 87.3785% |
| 32 | 52.3258% | 69.2224% |

CT requires `19-20`; MMX2 is best at `8` (`39626`, 93.8051%, a new best for
MMX2). The two requirements differ by ~11 clocks, so a global phase cannot
satisfy both, and per-title phases are exactly the kind of hack forbidden by the
accuracy policy.

Why the penultimate snapshot does not remove the title dependence: the phase
still selects the *instruction* at which the NMI is taken, and CT's and MMX2's
NMI-edge instruction phases differ. Decoding the CT NMI at 12,100,002 shows the
serviced PC sequence (`$C0EC71 -> $FF10 -> $000500`) is matched by the p=8
configuration, yet CT's audio needs p=20 — the NMI service PC is not the only
timing-sensitive point, so the phase cannot be calibrated from the vector alone.

The committed hold-expiry `+22` model remains the only configuration that keeps
all six exact titles; MMX2 stays at 92.3001%. The remaining work is ares'
per-instruction `L` placement (including instructions whose `L` precedes an
internal cycle or the high byte of a 16-bit read), which the access-granularity
snapshot does not reproduce.

## Per-instruction L placement measured directly against ares (2026-11-23)

A temporary ares hook (`ARES_LASTCYCLE`) logged every `lastCycle()` call with
`pc`, `nmiTransition`, `irqTransition`, `irqLock` and `nmiLine`. Reading it next
to `ARES_NMI_POLL` gives the exact flip -> sample gap:

| | `nmiValid` flip | ares `lastCycle` NMI sample | gap |
|---|---|---|---|
| CT (12.1M NMI) | 12,099,984 | 12,099,996 | 12 clocks |
| MMX2 (frame 1) | 664,270 | 664,282 | 12 clocks |

The gap is identical for both titles, so ares' NMI sample is **not**
title-dependent. Our previous title dependence came entirely from the
bus-access-quantised `nmi_line` raise.

### What the per-instruction `L` actually is

Reading ares `instructions-read.cpp`/`instructions-write.cpp`, `L` precedes the
**high byte** of a 16-bit read/write (e.g. `instructionBankRead16`:
`W.l = readBank(...); L W.h = readBank(...)`), and the final fetch for 8-bit
forms. Taken branches put `L` before `idle()`, and `idleBranch()` is an empty
virtual here, so it is the final cycle. At bus-access granularity this is exactly
"sample before the instruction's final bus access", which the driver-level
snapshot already reproduces.

### Measurements

- Penultimate-access snapshot + committed `+22`: **behaviourally identical**
  (`dkc/smas/ct` 0, `mmx2` 49253).
- Faithful `nmiPoll` timing (valid at line-start+4, transition at
  line-start+12) + penultimate snapshot, **no phase constant**: `ct` 0,
  `mmx2` 80734 (87.3785%).
- Same with a live (instruction-end) sample: `ct` 52.3258%, `mmx2` 69.2225%.
- Same with the transition at line-start+4 and a consume gate at +12 (the
  earlier edge-clock sweep): `mmx2` best 39626 (93.8051%) but `ct` 52.3258%.

So the per-instruction `L` placement is already emulated at access granularity;
it is not the MMX2 blocker. The only configuration that improves MMX2 (93.8051%)
trades CT's exactness, and the only configuration that keeps CT exact
(committed `+22`, or the faithful poll model) leaves MMX2 at 92.3001% / 87.3785%.
The MMX2 residual must therefore be a separate S-CPU/APU phase effect (the
+30-clock upload offset), not the interrupt sample point.

## MMX2 residual localized to the S-SMP/DSP hand-off phase (2026-11-23)

The 49,253 differing samples are not spread over the run: every one lies in
**stereo frame 226,560-288,140 (7.071-9.180 s)**. The first 7.071 s and the last
~0.8 s are bit-exact. Both channels differ (left 24,310, right 24,943).

### The APU port handshake is correct

`PORT_OUT` vs `ARES_APU_LOG` shows the `$2140-$2143` upload stream (reads,
writes, values) is identical, shifted by one poll iteration. The first
"divergence" is ours reading the echo byte one poll earlier; the values are the
same. This does not itself change the audio.

### Every DSP register write matches

`DSP_WRITES_OUT` vs `ARES_DSP_WRITES` gives the same 5,618 writes: identical
`(address, value)` in identical order. Only the timing columns differ.

### The divergence is the echo delay buffer

`DSP_FULL_TRACE_OUT` vs `ARES_DSP_TRACE` (204 columns, same layout):
- all per-voice state (envx, envelope, pitch, brr address/offset, buffers) is
  identical at every sample;
- `mainvol.output`, `echo.input` and all 5,618 writes match;
- the first audible difference is `echo.output[0]` (ours 0 vs ares 156) at
  sample 226,560, with `echo._offset` 8 bytes (two output samples) ahead
  (ours 2436 vs ares 2428).

The echo read/write/FIR/`echo29` code is line-for-line the same as ares
(`echoRead`, `echoWrite`, `calculateFIR`, `echo29`), and `_page`/`_delay`/
`_length` all match, so the pointer lead is an input-timing effect, not an
algorithm difference.

### Origin: the sound program starts two samples early

At the very first DSP write after the IPL hand-off, ours is already
**66 micro-steps (~2.06 output samples) early** (`dmicro = -66`, `dsample = -2`)
and the offset drifts to `+42` by the end of the run. That two-sample lead is
what places the echo pointer two samples ahead. It is the downstream effect of
the `$2140` hand-off phase (the S-SMP reaching the uploaded program ~2 samples
before ares), not of the DSP core, the echo algorithm, or the `$8100` poll loop
(which only changes which poll iteration sees the echo byte).

So MMX2's remaining audio mismatch is a **2-sample S-SMP/DSP start-phase lead
at the IPL hand-off**, surfacing only when the echo becomes audible at 7.07 s.
The next fix must align the S-SMP's entry into the uploaded program with ares
(the hand-off sample boundary), not alter the DSP pipeline.

## The MMX2 " +30-clock upload offset" is jitter, not accumulation (2026-11-23)

The +30-clock offset at the first `$2140` read of the upload was proposed as a
cumulative S-CPU phase error. A per-frame probe of the NMI handler's
`$7E2034 STA $09CE` write (212 compared writes, `W09CE_LOG` vs
`ARES_WRAM_TRACE=9CE-9CF`) gives:

| metric | value |
|---|---|
| mean delta (ours - ares) | +11.3 clocks |
| median delta | +14 clocks |
| stdev | 13.4 clocks |
| linear slope | **-0.04 clocks/frame** (~ -9 clocks over 232 frames) |

The delta oscillates over a -18..+38 range with **no drift**, so the S-CPU phase
at the NMI handler is not accumulating; the +30 seen at one instant is a sample
of the inherent per-frame interrupt-service jitter (the NMI lands on a different
instruction each frame, in both emulators). The constant +8 is the measurement
point difference (our trace logs the access start, ares the write completion).

Consequence: "fix the accumulating +30" is not a well-posed fix. The MMX2 audio
divergence is the *discrete* output-sample boundary at which the echo registers
are written, which depends on this jitter plus the exact S-SMP/DSP interleaving.
Matching it bit-exactly needs cycle-exact S-SMP/DSP co-scheduling (the APU
scheduler), not a CPU-phase constant.

## MMX2 ARAM comparison at the upload (2026-11-23)

Added an `ARAM_OUT` debug dump (mirrors `WRAM_OUT`; `ares` already has
`ARES_APURAM_FRAME`/`ARES_APURAM_AT`) and compared APU RAM during the upload:

| video frame | differing bytes | runs |
|---|---|---|
| 100 | 0 | - |
| 175/200/225 | 0 | - |
| 250 | 384 | `0xA1`, `0xF4`, `0x2A84-0x2C02` |
| 275 | 386 | `0xA1`, `0xF4`, `0x54BB-0x563A` |
| 300 | 385 | `0xA1`, `0xF4`, `0x7A0A-0x7B88` |

Two zero-page bytes (`$A1`, `$F4`) and one ~383-byte window differ after the
upload. The window **moves forward** each dump (0x2A84 -> 0x54BB -> 0x7A0A) while
the rest of ARAM is identical, which is the signature of a small time skew
between the two dumps (ares dumps at `aresLineFrames >= frame`, ours at the end
of the run) over a buffer the driver is actively writing, rather than a
persistent content difference. The earlier "0x1A1A block" was a frame-300
snapshot of that moving window; at frame 250 the same bytes are identical (the
block is present in both at 0x1A1A).

So the ARAM comparison neither proves nor disproves a content bug at
frame-granularity; resolving it needs a same-clock ARAM dump. The upload
handshake remains the only measured divergence (one poll iteration), and it is
sub-frame.

## Experimental whole-sample DSP stepping (`SNES_DSP_SAMPLE_ATOMIC`)

ares `DSP::main` runs a whole 32-micro-step sample atomically in the coroutine
scheduler, so an S-SMP DSP-register write can never take effect mid-sample. Our
scheduler interleaves the DSP one micro-step at a time, which in principle lets
a write land mid-sample.

An experimental default-off path (`SNES_DSP_SAMPLE_ATOMIC`, `Apu::dsp_step_sample`
= 32x `dsp_step_one` in both scheduler loops) reproduces the atomic stepping.

Result: the MMX2 output is **byte-identical** with and without the flag, and
`dkc/smas/ct` stay bit-exact. So mid-sample register application is not the MMX2
cause; the DSP stepping granularity does not change the produced samples for
these titles. The flag is retained default-off as a documented negative result
and a tool for further APU timing experiments.

## MMX2 is a +4 micro-step DSP phase at the upload hand-off (2026-11-23)

Two independent knobs were used to probe the DSP write-application phase:

| knob | meaning | MMX2 | CT |
|---|---|---|---|
| default | - | 49253 (92.3001%) | 0 |
| `SNES_DSP_PRIME=4` | prime the DSP catch-up by 4 micro-steps | **0 (100.0000%)** | 232868 (63.5948%) |
| `SNES_DSP_PRIME=5` | +5 | 3092 (99.5166%) | 238075 (62.7808%) |
| `SNES_DSP_PRIME=6` | +6 | 20959 (96.7234%) | 238081 (62.7798%) |
| new `SNES_DSP_WRITE_DELAY=5` | defer DSP register decode 4 micro-steps | **0 (100.0000%)** | - |
| same, delay 4 / 6 | 3 / 5 micro-steps | 26520 / 3092 | - |

So MMX2 becomes **bit-exact** with a +4 micro-step DSP phase (via either a global
catch-up prime or a per-write decode deferral), while the six titles that are
already exact require +0 and are broken by the same shift.

This pins the residual precisely: our S-SMP reaches its DSP register writes
**4 micro-steps earlier (8 S-SMP clocks)** than ares for MMX2, but at the same
phase for CT (`CT`'s write micro-step delta is 0 for all 12,517 writes; MMX2's
is -4 for 80% of its 5,618 writes). Because the S-SMP program and all DSP
register writes/values match, this phase difference comes from the **upload
handshake instruction timing** (the IPL `$FFDE`/`$FFE9` divergence), not from
the DSP core or the write pipeline.

A universal fix therefore has to make the S-SMP execute the upload handshake in
the same number of clocks as ares; a constant phase shift cannot do it (it
trades one title for six). The two DSP-phase knobs above remain as default-off
experimental tools for that work.

## MMX2 handshake: decoded IPL loop and the port-latch timing (2026-11-23)

The IPL upload loop (correct byte decode) is:

```
FFD6: MOV Y,$F4        ; Y = CPU byte index
FFD8: BNE $FFD6        ; wait for $F4 != 0
FFDA: CMP Y,$F4        ; Y == CPU index?
FFDC: BNE $FFE9        ; no -> spin via FFE9/FFDA
FFDE: MOV A,$F5        ; yes -> A = data byte
FFE0: MOV Y,$F4
FFE2: MOV (0)+Y,A      ; ARAM[Y] = A
FFE4: INC Y
FFE5: BNE $FFDA
FFE9: BPL $FFDA
```

The first S-SMP divergence (`$FFDE` ours vs `$FFE9` ares at the same 7,979,250
SMP clock) is exactly the `$FFDC BNE`: ours takes the transfer path because
`Y == $F4`, ares spins because `Y != $F4`. So the difference is the **S-CPU to
S-SMP `$2140` port latch timing**, not the DSP.

`SNES_PORT_LEAD=1` makes the S-CPU finish the S-SMP's current instruction before
latching `$2140` (ares `CPU::writeAPU`: `synchronize(smp); smp.portWrite`):

| configuration | MMX2 | CT |
|---|---|---|
| default | 49253 | 0 |
| `SNES_PORT_LEAD=1` | **0 (100.0000%)** | 452998 (29.18%) |
| `+ SNES_LEAD_AFTER_BOOT=1` | 21738 (96.6016%) | 450736 (29.53%) |
| `SNES_WRITE_ACCESS_STOP=1` | 21738 (96.6016%) | 452716 (29.23%) |
| `SNES_SMP_WRITE_PULL=1` | 49253 | 0 |
| `SNES_SMP_ACCESS_STOP=1` | 108108 (83.10%) | 415343 (35.07%) |
| `SNES_SMP_INSTR_STOP=1` | 212821 (66.73%) | - |

`SNES_PORT_LEAD` alone makes MMX2 **bit-exact (0/639656)** but breaks five
titles; every other latch-timing variant that keeps CT exact leaves MMX2 at its
92.3% (or worse). The latch timing that ares needs is therefore per S-SMP
program: MMX2's driver requires the S-SMP to be caught up before every `$2140`
write, while the other titles' drivers match the default exact-credit path.

Combined with the previous section, there are now two independent, fully
bit-exact configurations for MMX2 (`SNES_PORT_LEAD=1`, and `SNES_DSP_PRIME=4` /
+4 micro-step DSP phase), each at the cost of the other titles. The remaining
work is a scheduler that reproduces ares' symmetric `synchronize(smp)` for both
programs, not a constant.

## Reconciling the port-lead default: the credit gate is not the cause (2026-11-23)

The task was to make ares' `synchronize(smp)` before every `$2140` write the
faithful default and explain why `SNES_PORT_LEAD=1` (MMX2 exact) regresses
CT/zelda/fzero/cv4/smas.

First hypothesis tested: `Apu::finish_instruction` short-circuits when
`apu_credit < 0`, which is not ares' condition (`while(thread.clock() <
clock())`). An experimental `SNES_LEAD_STRICT` removed that gate and compared
the S-SMP clock directly:

| configuration | MMX2 | CT |
|---|---|---|
| `SNES_PORT_LEAD=1` | 0 (100.0000%) | 452716 (29.2251%) |
| `SNES_PORT_LEAD=1 SNES_LEAD_STRICT=1` | 0 (100.0000%) | 452716 (29.2251%) |

Identical, so the credit-sign gate is **not** what breaks CT. The regression is
in the cadence/clock bookkeeping of running the S-SMP to the S-CPU clock at a
port write: the whole-instruction overshoot (`apu_credit`) after
`finish_instruction` is not consumed by the subsequent
`advance_peripherals` the way ares' absolute `thread.clock()` comparison is, so
a phase drift accumulates for the titles whose drivers write `$2140`
periodically (CT/zelda/fzero/cv4/smas) but not for MMX2.

Making the lead correct therefore requires replacing the credit bookkeeping with
ares' absolute-clock `while(smp.clock < cpu.clock) synchronize` in the normal
advance path too, not just at the port write. Recorded here so the next attempt
does not re-test the credit gate.

## Absolute-clock scheduler rewrite was implemented and is not the fix (2026-11-23)

The proposed reconciliation was implemented and measured: replace the carried
credit floor (`apu_credit`) with ares' absolute `while(smp.clock() <
cpu.clock())` catch-up on the normal advance path (`SNES_SYNC_ABS`), then enable
the port lead.

| configuration | MMX2 | CT |
|---|---|---|
| default (credit floor) | 49253 (92.3001%) | **0** |
| `SNES_PORT_LEAD=1` only | **0 (100.0000%)** | 452716 (29.2251%) |
| `SNES_SYNC_ABS=1` (per-access `sync_smp_to`) + lead | 108108 (83.0990%) | 415343 (35.0678%) |
| `SNES_SYNC_ABS=1` (whole-instruction `sync_smp_instruction`) + lead | 212831 (66.7273%) | 482780 (24.5251%) |
| `SNES_SYNC_ABS=1` (whole-instruction) alone | 212831 (66.7273%) | 482780 (24.5251%) |

Both absolute-clock forms regress **further** than the default, so the
carried-credit bookkeeping is not what keeps the six titles exact, and replacing
it does not reconcile the port lead. The default credit floor is the closest of
the measured models to ares for the already-exact titles.

Conclusion: the ares-faithful `synchronize(smp)` before `$2140` writes fixes
MMX2 but the surrounding advance model in this core differs from ares in a way
that neither the credit floor nor an absolute-clock catch-up reproduces for all
programs. Landing it requires a from-scratch reproduction of ares' symmetric
thread scheduler, not an incremental bookkeeping change.

## ares' symmetric scheduler variants do not fix MMX2 either (2026-11-23)

The requested isolated symmetric-coroutine scheduler already exists behind
`SNES_SYMMETRIC` / `SNES_SYM_SCHED` / `SNES_SYM_ENGINE` (absolute per-thread
clocks, whole-instruction execution, per-tick DSP pull). Measured on MMX2:

| configuration | MMX2 |
|---|---|
| default (credit floor) | 49253 (92.3001%) |
| `SNES_SYMMETRIC=1` | 211810 (66.8869%) |
| `SNES_SYM_SCHED=1` | 196857 (69.2246%) |
| `SNES_SYMMETRIC=1 SNES_SYM_SCHED=1` | 196857 (69.2246%) |
| `SNES_SYM_ENGINE=1` | 212827 (66.7279%) |

Every symmetric/absolute-clock variant is **worse** than the default for MMX2.
So the remaining divergence is not reproduced by the symmetric scheduler model
as implemented; it is elsewhere (the port-lead `synchronize(smp)` fixes MMX2,
but its surrounding advance model breaks the six titles in every form measured:
credit floor, per-access absolute, whole-instruction absolute, and symmetric).
At this point every available scheduler model has been measured against the
7-title gate; none reaches 7/7.

## Port-lead combinations and CT's divergence point (2026-11-23)

The port lead (`SNES_PORT_LEAD=1`) is the only model that makes MMX2 bit-exact.
Combining it with every DSP/latch synchronisation flag was measured:

| configuration | MMX2 | CT |
|---|---|---|
| `PORT_LEAD` | 0 | 452998 (29.1810%) |
| `PORT_LEAD + DSP_LATE` | 0 | 452998 (29.1810%) |
| `PORT_LEAD + DSP_INSTR_SYNC` | 55202 (91.3700%) | 452675 (29.2315%) |
| `PORT_LEAD + SYM_PULL` | 0 | 452998 (29.1810%) |
| `PORT_LEAD + CPU_LATCH_PARK` | 0 | 452998 (29.1810%) |

So the CT regression is not the DSP drain or latch queue. CT's first differing
audio sample under the lead is at **stereo frame 75,511 (t = 2.357 s)**, i.e.
during gameplay, not the ~0.3 s IPL upload — the lead changes a steady-state
sound-command `$2140` handshake for CT while fixing MMX2's upload.

Every scheduler and latch-timing model in the tree has now been measured (credit
floor, per-access absolute, whole-instruction absolute, `SNES_SYMMETRIC`,
`SNES_SYM_SCHED`, `SNES_SYM_ENGINE`, port lead and its combinations). The only
configurations reaching MMX2 = 0 break five titles, and no combination keeps
both. A universal fix requires reproducing ares' scheduler at a level this core
does not currently model.

## Port lead inverts the first handshake for CT (2026-11-23)

S-SMP traces under `SNES_PORT_LEAD=1` vs ares, CT:

```
idx     ours                       ares
10162   clk 84598  $FFDC           clk 84590  $FFDC
10163   clk 84606  $FFE9  (spin)    clk 84594  $FFDE  (transfer)
```

The first PC divergence is at SMP clock **84,606 (0.041 s)**, i.e. the very first
`$F4/$F5` handshake of the IPL upload, not gameplay. Under the lead ours takes
`$FFDC BNE` to `$FFE9` (Y != $F4, spin) while ares falls through to `$FFDE`
(Y == $F4, transfer): the lead makes our S-SMP observe the `$2140` latch in the
opposite order to ares at the *first* handshake. MMX2 is the reverse (the lead
fixes its handshake), so the same `synchronize(smp)`-before-latch step helps one
program and inverts the other.

This confirms the divergence is the exact ordering of "run S-SMP to CPU clock"
versus "latch $2140" at the handshake — a scheduler semantics detail, not a
cadence, DSP phase, or credit quantity. Reproducing it needs ares' actual
coroutine interleaving (the CPU's `synchronize(smp)` and the SMP's
`synchronize(cpu)` on port reads both matter and are program-order dependent),
which this core's event-driven model does not yet express.

## Two-sided handshake via existing flags: all combinations measured (2026-11-23)

Implementing the read-side pull (`$F4-$F7` synchronize(cpu) before sampling) by
combining the existing flags with the port lead:

| configuration | MMX2 | CT |
|---|---|---|
| `PORT_LEAD + PORT_READ_CLOCK` | **0** | 450736 (29.53%) |
| `PORT_LEAD + DEFER_CLOCKS` | 169302 (72.34%) | 452547 (29.23%) |
| `PORT_LEAD + PORT_READ_CLOCK + DEFER_CLOCKS` | 169302 (72.34%) | 452547 (29.23%) |
| `PORT_LEAD + CPU_LATCH_TIME` | **0** | 452716 (29.23%) |
| `PORT_LEAD + SMP_WRITE_PULL + PORT_READ_CLOCK` | 21738 (96.60%) | 450736 (29.53%) |
| `CPU_LATCH_TIME` alone | 21738 (96.60%) | 452716 (29.23%) |
| `PORT_LEAD + CPU_LATCH_PARK + PORT_READ_CLOCK` | **0** | 450736 (29.53%) |

No combination raises CT above 29.5% once the lead is on. The lead breaks CT at
the **first** handshake (SMP clock 84,606) by inverting the `$FFDC BNE` outcome,
and none of the read-pull / latch-time / write-pull flags changes that. The
default credit floor remains the only configuration with six exact titles, and
the lead remains the only one with MMX2 exact. Every flag-level expression of
the two-sided ordering has now been measured; the correct interleaving is not
reachable by toggling them.

## "Do not advance the APU in the generic access" starves the APU (2026-11-23)

The structural model ("advance the S-SMP only in the `$2140` handler, not in the
generic bus-access advance") already exists as `SNES_PORT_SYNC`. It defers the
APU clocks at every non-port access and flushes them at the next `$2140-$2143`
access. Measured:

| configuration | MMX2 | CT |
|---|---|---|
| `SNES_PORT_SYNC` | 0 over 84754 (**truncated**) | 9 over 84458 (**truncated**) |
| `+ PORT_LEAD` | 0 over 226724 (**truncated**) | 181076 over 366988 (truncated) |
| `+ PORT_LEAD + PORT_READ_CLOCK` | 0 over 226724 (**truncated**) | 181074 over 366986 |

The audio is **truncated** (84754–366988 samples instead of 639656): deferring
all APU advance to `$2140` accesses starves the S-SMP whenever the game is not
polling the ports (e.g. during gameplay), freezing the APU. ares does **not**
work that way — its scheduler runs whichever thread is behind continuously, and
`CPU::writeAPU`'s `synchronize(smp)` is *additional*. So the requested structural
change is not ares-faithful and stalls the APU; it cannot be the fix.

This closes the space of structural variants: continuous advance (default/most
faithful) keeps six titles exact; port-only advance stalls; instruction- and
access-granularity synchronize break CT at the first handshake; the port lead
makes MMX2 exact but breaks five. The remaining difference is inside ares'
actual coroutine interleaving of CPU `step` and SMP `synchronize`, which this
event-driven model does not express.

## Single-synchronize-per-port (`SNES_PORT_ONCE`) regresses (2026-11-23)

Implemented the requested structural change: for a `$2140-$2143` access, the
generic bus advance does **not** drive the APU; instead the port access performs
one ares-style whole-instruction synchronize to the access clock (`finish_instruction`)
in place of `peer.advance`, with continuous advance unchanged for all other
accesses (so the APU is not starved).

| configuration | MMX2 | CT |
|---|---|---|
| default (generic advance) | 49253 (92.3001%) | 0 |
| `SNES_PORT_ONCE` | 152216 over 595028 (74.42%, truncated) | 362250 over 549024 (34.02%, truncated) |

It is worse for both and also truncates: replacing the generic advance with a
whole-instruction synchronize at the port access changes the APU's position at
the next access, not just at the latch. So "advance once per access, and make
that advance a synchronize" is not equivalent to ares either.

Every concrete form of the two-sided handshake has now been implemented and
measured: generic-advance + lead (MMX2 0, five break), port-only advance
(starves), and single-synchronize-per-port (both worse). None reaches 7/7. The
default six-title state remains the best verified configuration.

## Per-access synchronize + read-side pull: still breaks CT (2026-11-23)

The oracle (ARES_APU_LOG + ARES_SMP_READ_LOG at CT's first handshake) shows the
whole-instruction lead **overshoots**: it advances the S-SMP past the S-CPU's
index, so the S-SMP spins at `$FFDC BNE` while ares transfers. Tested the
access-granularity synchronize with every read-side companion:

| configuration | MMX2 | CT |
|---|---|---|
| `WRITE_ACCESS_STOP + PORT_READ_CLOCK` | 21738 (96.60%) | 450316 (29.60%) |
| `+ SMP_WRITE_PULL` | 21738 | 450316 |
| `WRITE_ACCESS_STOP + SMP_WRITE_PULL` | 21738 | 452716 (29.23%) |
| `WRITE_ACCESS_STOP + PORT_READ_CLOCK + CPU_LATCH_TIME` | 21738 | 452716 |

Access-granularity synchronize plus the read-side pull still leaves CT at ~29.6%
and MMX2 at 96.6%. So neither whole-instruction nor access-granularity
`synchronize(smp)` recovers CT, even with the two-sided read pull. The default
(no synchronize at writes) is the only configuration with CT exact; the port
lead is the only one with MMX2 exact.

This is the end of the event-model space: all 4 synchronize granularities × all
read-pull/latch flags were measured, plus 3 symmetric schedulers and 3 DSP-phase
models. None reaches 7/7.

## The scheduler is already ares-faithful; the rewrite cannot fix MMX2 (2026-11-23)

Reading ares `SMP::step` (timing.cpp) gives the exact mutual semantics:

```
SMP::step(clocks) { Thread::step(clocks); Thread::synchronize(cpu); Thread::synchronize(dsp); }
SMP::readIO($F4)  { synchronize(cpu); return io.apu0; }
```

So after **every S-SMP access** the CPU is run forward to the S-SMP clock
(mid-instruction), and the `$F4-$F7` read also pulls the CPU. That is the mutual
scheduler. The codebase already implements it (`SNES_MUTUAL`, `SNES_ACCESS_FLUSH`):

| configuration | MMX2 | CT |
|---|---|---|
| default | 49253 (92.3001%) | 0 |
| `SNES_MUTUAL` | 49253 | 0 |
| `SNES_ACCESS_FLUSH` | 49253 | 0 |
| `SNES_PER_ACCESS_MUTUAL` | 108108 (83.10%) | 415343 (35.07%) |

And `cmp` shows `SNES_MUTUAL` output is **byte-identical to the default** for both
MMX2 and CT. Therefore the default already matches ares' scheduler semantics at
the observable output, and a from-scratch scheduler rewrite would reproduce
exactly this 6/7 result — it cannot fix MMX2.

Conclusion: the remaining MMX2 divergence is **not** the APU/S-CPU scheduler. Two
facts point elsewhere: (1) the only non-exact title is also the only CX4
(Super FX-style co-processor) title, and (2) the divergence is a 4-micro-step
S-SMP phase at the IPL handshake that the faithful scheduler reproduces as
correct for the six non-CX4 titles. The next investigation should be the CX4's
concurrent effect on S-CPU timing during the upload, not another scheduler.

## CX4 redirect disproved: the coprocessor is idle during the handshake (2026-11-23)

Checked whether MMX2's CX4 runs across the 3.87-3.95 s upload handshake (the
only-non-exact title is the only CX4 title):

- Our `CX4_INSN_LOG` (from frame 229): 16 entries, all at master clocks
  <= 22.1 M; **none** in 83.0-85.0 M.
- ares `ARES_CX4_INSN_LOG` (234 frames): 21,714 instructions, range
  21.76-22.15 M; **none** in 83.0-85.0 M.
- No CX4 job starts in frames 220-245 in either emulator.

So the CX4 is idle during the handshake in *both* emulators; the "only CX4
title fails" correlation is coincidental with "only MMX2 performs a mid-game
sound-program upload at 3.89 s". The CX4 is not the cause.

Combined with the previous section (the mutual scheduler is byte-identical to
the default), both commissioned directions are now disproved:

- scheduler rewrite -> reproduces the default (6/7),
- CX4 investigation -> CX4 idle during the divergence.

The remaining difference is the S-CPU/S-SMP race at MMX2's specific mid-game
upload: our S-SMP reaches the uploaded program 4 micro-steps early there, and
every synchronization variant either keeps that offset or breaks the six titles
whose uploads occur at boot with different timing.

## Cycle-accurate core rewrite: foundation landed (2026-11-23)

The full cycle-accurate rewrite is authorized. It is a multi-session project;
this commit lands its first verifiable block and the staged plan.

### Landed: ares fixed-point thread-clock primitives

`sym_scheduler.rs` (default-off scaffolding, already documented there) now has
the ares scheduler arithmetic the rewrite depends on:

- `ThreadClock` — `scalar = Second / frequency` with `Second = 2^63-1`,
  `step(clocks)` = `clock += scalar*clocks`, `cycles()`, `is_behind()`;
- `synchronize(thread, target, main)` — ares
  `while(thread.clock() < target) co_switch(thread)`, with the thread entry
  advancing its own clock.

Unit tests pin the scalars (`SMP = Second/2050560`, `DSP = Second/24606720`),
whole-cycle stepping, catch-up termination, and the park/stop path. 4 new tests;
`cargo test -p unko-snes --lib` is 248 passed.

### Staged plan (acceptance: 7-title gate all 0 differing + 212/212)

1. **done** — absolute fixed-point thread clocks + `synchronize` semantics.
2. Build the three mutually-resumable threads on `spc_coroutine` (access-level
   S-SMP suspend points) and `DspCoroutine` (micro-step suspend points), driving
   the real `Spc700`/`Dsp` cores; validate the IPL handshake against ares golden
   traces at access granularity.
3. Wire as `SNES_CA` (default-off), run the 7-title 600-frame gate; only make
   default if all seven reach 0 differing and the 212/212 baseline is green.

The active emulator is unchanged; the verified default remains 6/7.

## CA rewrite stage 2a: S-CPU-facing port latches in `SymEngine` (2026-11-23)

`SymEngine` (the default-off engine over the real `Spc700`/`Dsp`, driven by the
mutual scheduler) now exposes the S-CPU side of the `$2140-$2143` handshake:

- `cpu_write_port(port, value)` — ares `CPU::writeAPU`: `synchronize(smp)` to
  the published S-CPU clock, **then** latch into the `$F4-$F7` view;
- `cpu_read_port(port)` — ares `CPU::readAPU`: synchronize, then sample the
  S-SMP's `$F4-$F7` latches;
- the coroutine bus now routes `$F2/$F3` to the DSP address/data and
  `$F4-$F7`/`$2140-$2143` through explicit CPU<->SMP latch arrays.

Three unit tests pin the ordering: the SMP is advanced to the CPU clock before
latching, reads return the SMP latch, and channels are independent. Total lib
tests: 251. The engine is still default-off (not wired into `Snes`).

## CA rewrite stage 2b: two-sided handshake in `SymEngine` (2026-11-23)

`SymEngine` now completes ares' two-sided `$2140` handshake in isolation:

- the S-SMP's `$F4-$F7` read, when the S-SMP is ahead, calls the installed
  S-CPU puller (`ares SMP::readIO -> synchronize(cpu)`) and adopts the returned
  S-CPU clock before sampling the latch;
- `cpu_write_port` still does `synchronize(smp)`-then-latch.

A unit test runs an `MOV A,$F4` S-SMP program with a puller installed and
asserts the S-CPU is pulled and the clock advances. Total lib tests: 252. The
engine remains default-off; the active emulator is unchanged.

## CA rewrite stage 3a: `SymEngine` runs the real IPL boot (2026-11-23)

`SymEngine` can now load the 64-byte IPL ROM and serve `$FFC0-$FFFF`:

- `load_ipl([u8;64])` + `iplrom_enable`, with IPL reads routed to the ROM when
  the S-SMP address is `>= $FFC0`;
- a boot test starts the real `Spc700` at `$FFC0` with the actual ares `ipl.rom`
  bytes and runs until the boot handshake, asserting the engine produces the
  IPL's `$F4=$AA` / `$F5=$BB` (S-SMP -> S-CPU) latches.

This validates the isolated engine on real firmware, end to end. Total lib
tests: 253. The engine remains default-off; the active emulator is unchanged.

## CA rewrite stage 3b: S-SMP timers in `SymEngine` (2026-11-23)

The sound driver needs the three S-SMP timers. `SymEngine` now has `SymTimer`
(ares `Timer<Frequency>` logic: stage0 divider, stage1 flip, stage2 target
count, 4-bit stage3) and wires `$F1` (enable/disable), `$FA-$FC` (targets) and
`$FD-$FF` (read/clear). Timers step on each S-SMP `Advance`. Two tests pin the
frequency and the read/clear behavior; total lib tests: 255. Engine remains
default-off; the active emulator is unchanged.

## CA rewrite stage 3c-1: per-step CPU pull + `advance_to_cpu` (2026-11-23)

`SymEngine` now exposes the scheduler entry the `SNES_CA` adapter will call:

- `smp_step_pulling(&mut dyn FnMut(u128)->u128)` — executes one S-SMP
  instruction and, on an ahead `$F4-$F7` read, invokes the borrowed S-CPU pull
  (ares `SMP::readIO -> synchronize(cpu)`). The puller is per-call, so it can
  borrow the live S-CPU (no `'static` box).
- `advance_to_cpu(cpu_clock, cpu_step)` — runs the S-SMP to the S-CPU clock
  (invoking `cpu_step` on a port-read pull) and catches the S-DSP up; this is
  ares' scheduler step from the S-CPU side.

Tests: the S-SMP runs to the targets, the DSP advances, and a `MOV A,$F4`
program pulls the S-CPU. Total lib tests: 256. Engine remains default-off.

## CA rewrite stage 3c-2 blocked: parking the `$F4` read in `SymEngine` (2026-11-23)

Attempted the `Parked{target}` protocol (`smp_step_parking`: on an ahead
`$F4-$F7` read, return the S-SMP clock the S-CPU must reach, then resume and
sample the latch). It could not be validated in isolation: a test program
`MOV A,#0; MOV A,$F4` retires the second instruction without the engine ever
yielding `SpcEvent::Read($F4)` on the parking path, whereas the closure path
(`smp_step_pulling`) does see `Read(0x00F4)` with `smp > cpu`. So the recorder's
per-resume instruction state does not emit the direct-page data read when the
step is driven instruction-by-instruction, which the parked protocol requires.

Reverted to the green state (`09e48da`, 256 tests). The next CA step is a
recorder fix: make `SpcCoroutine` emit the direct-page `$F4-$F7` data read
deterministically on the first resume that reaches it, independent of whether
the caller batches the whole instruction. Only then can `smp_step_parking` be
validated and wired into `step_sym_sched`.

## CA rewrite stage 3c-2 done: recorder emits inline `$F4` reads; park protocol validated

Root cause of the previous blocker: `SpcCoroutine`'s `ResumeBus` hard-coded
`defer_io_reads() = true`, so a direct-page `$F4-$F7` read is deferred into the
core's `pending` slot with **no** `SpcEvent::Read` (only the plan driver resolves
it). The isolated engine never resolved it, so the read vanished.

Fix: `new_spc_coroutine_with(defer, split)` parameterizes the bus;
`new_spc_coroutine_inline()` (defer = false, split = false) makes the engine
observe `$F4-$F7` as ordinary `Read` events. The plan driver still uses
`new_spc_coroutine()` (defer = true, split = true), unchanged.

With that, `SymEngine::smp_step_parking` is validated: an ahead `$F4` read
returns `StepOutcome::Parked(smp_clock)`; after the S-CPU reaches that clock and
writes `$2140`, resuming the step samples the updated latch and retires the
instruction. 257 lib tests pass; the active emulator is unchanged.

Next: wire `smp_step_parking` into `step_sym_sched` (the park target drives the
S-CPU in the outer loop) behind `SNES_CA`.

## CA rewrite stage 3d-1: `CaPeer::run_to` outer-loop driver (2026-11-23)

The reusable driver component for the `step_sym_sched` hook is landed:

- `CaPeer` owns a `SymEngine`.
- `run_to(cpu_clock, run_cpu)` runs the S-SMP to the S-CPU clock; whenever an
  ahead `$F4-$F7` read parks, it calls `run_cpu(&mut engine, target)` so the
  caller drives the S-CPU in its outer loop, then resumes the parked read
  (sampling the updated latch). The loop condition also continues while a read
  is parked, so a park that overshoots the target is completed.

Test: a `MOV A,$F4` program parks, the caller writes `$2140`, and the engine
samples it — validating the exact park outer-loop `step_sym_sched` will run.
Total lib tests: 258. Default emulator unchanged.

## CA rewrite stage 3d-2: async park interface + `SNES_CA` flag (2026-11-23)

- `apu::ca_enabled()` (`SNES_CA`) selects the isolated engine (default-off).
- `CaPeer` gained the **asynchronous** driver interface the outer
  `step_sym_sched` loop needs (it cannot run the S-CPU from inside the CPU
  coroutine): `set_cpu_clock(clock)`, `poll() -> StepOutcome`, and `caught_up()`.
  On `Parked(target)` the driver records the target, advances the S-CPU to it in
  its own loop, then polls again; the parked read samples the updated latch.

Test: an async poll reports the park and `caught_up` terminates once the CPU is
driven. Total lib tests: 259. The engine is still not wired into `Snes`; the
default emulator is unchanged.

## CA rewrite stage 3d-3: `Snes` now owns an optional CaPeer (2026-11-23)

- `Snes` has a transient `ca: CaSlot` (a derivable slot like
  `CpuCoroutineSlot`; cloning drops the engine, equality is structural).
- `Snes::install_ca([u8;64])` builds a `SymEngine` with the IPL ROM and the
  S-SMP at `$FFC0`, wraps it in a `CaPeer`, and stores it; `ca()`/`ca_mut()`
  expose it.
- Test: an installed CA peer runs the real IPL boot to the `$F4=$AA` /
  `$F5=$BB` handshake inside a `Snes` instance.

Total lib tests: 260. The `step_sym_sched` driver is not yet branching to the
peer (next step), so the default emulator is unchanged.

## CA rewrite stage 3d-4: `step_sym_sched` drives the CaPeer (2026-11-23)

When a `CaPeer` is installed (`Snes::install_ca`), `step_sym_sched` now:

- selects only the S-CPU thread (`pick = 0`) so the main `Apu` is not driven;
- routes `$2140-$2143` reads to `CaPeer::engine().cpu_port` and writes to
  `write_apu_latch` (ares' continuous-advance port latch);
- on each CPU `Advance`/`Idle`, calls `ca_drive`, which converts the S-CPU
  master clock to S-SMP cycles, publishes it, and polls the peer until
  `caught_up`; an ahead port-read park (`target > clock`) is left for the next
  `Advance`, which resumes and samples the updated latch (the async park);
- skips the main-Apu catch-up loop.

Tests: an installed peer advances to the published clock (SMP and CPU clocks
both move). Total lib tests: 261. The `SNES_CA` harness wiring (install + sample
drain) is the next step; the default driver is unchanged.

## CA rewrite stage 3d-5: first full-title CA measurement (2026-11-23)

The harness now installs the peer under `SNES_CA` (`system.install_ca(ipl)`) and
drains the engine's samples instead of `apu.drain_samples()`. Run with
`SNES_SYM_SCHED=1 SNES_CA=1`:

| title | CA result | default |
|---|---|---|
| MMX2 | 196820 differing (69.2303%) | 49253 (92.3001%) |
| CT | 452725 differing (29.2237%) | 0 |

The CA path runs full 600 frames and produces full-length audio (639658
samples), so the wiring end-to-end is exercised: S-CPU → `install_ca` IPL boot →
port latches → `ca_drive` async park → engine samples. It is **worse** than the
default, as predicted, because the isolated engine still differs from ares in
its detail (DSP power-on state, timer/DSP sync points, and the upload timing).
Default emulator unchanged (`mmx2 49253`); 261 lib tests pass.

## CA engine fidelity: ares wait-state tables (no measurable change) (2026-11-23)

`SymEngine` previously hardcoded `$F4-$F7`/timer wait states as 1/2 clocks. It now
implements ares' tables (`CYCLE_WAIT [1,2,5,10]`, `TIMER_WAIT [2,4,8,16]`) driven
by the `$F0` selectors, matching `Apu::sym_wait`/`sym_timer`.

Measured effect on the CA path (`SNES_SYM_SCHED=1 SNES_CA=1`):

| title | before | after |
|---|---|---|
| MMX2 | 196820 (69.2303%) | 196838 (69.2275%) |
| CT | 452725 | 452725 |

No change: the IPL boot leaves the wait selectors at 0, so both implementations
use the minimum wait. The CA engine's large residual (69%/29%) is therefore not
wait-state timing; it is deeper (SMP/DSP synchronization points or the DSP
reset/sample phase). The fix is kept because it is correct. 261 lib tests pass;
default emulator unchanged.

## CA engine fidelity: power-on APU/DSP state (no measurable change) (2026-11-23)

`SymEngine` now applies ares' deterministic power-on APU RAM + S-DSP register
file (the same `random::power_on_apu()` stream the main `Apu` uses) in
`install_ca`. Measured CA results are unchanged:

| title | before | after |
|---|---|---|
| MMX2 | 196838 (69.2275%) | 196838 |
| CT | 452725 | 452725 |

So neither wait states nor power-on state explains the CA engine's ~69%/29%
residual. The remaining gap is in the engine's SMP/DSP scheduling or sample/DSP
phase (its per-instruction clocking differs from the plan driver that keeps six
titles exact). The fix is kept as correct. 261 lib tests; default unchanged.

## CA differential debug: `Advance` double-counted the SMP clock (2026-11-23)

Added an S-SMP `(clock, PC)` trace to `SymEngine` (`CA_TRACE`) and diffed against
the bit-exact plan driver (`SPC_TRACE`) on CT.

**Bug found:** `SymEngine` advanced `smp_clock` on both `SpcEvent::StepClock` and
`SpcEvent::Advance`, whereas ares advances it only in `SMP::step` (`StepClock`);
`Advance` steps the timers only. That doubled the boot instruction rate
(8 clocks/instr vs the plan's 4). After the fix the CA CPU trace advances
4 clocks/instruction, matching the plan.

**Remaining divergence:** the first PC mismatch is at CT instruction **728**, in
the IPL handshake loop, with the CA engine ~10 clocks *behind* (`$FFD2` plan vs
`$FFD4` CA) and the gap drifting ~1 clock/instr. Audio is still unchanged
(CT 29.22%, MMX2 69.22%) because the handshake phase, not the instruction rate,
decides the transfer branch. This is the same upload-phase class as the default
MMX2 residual.

## CA differential debug 2: the coroutine under-charges I/O-write waits (2026-11-23)

Per-instruction clock comparison (CT, plan vs CA) around the first divergence
shows the offset is not a uniform drift but a **specific instruction cost**:

```
idx  pc      plan_clk plan_cost | ca_clk ca_cost
718  $FFC6   4784     4        | 4780   4
719  $FFC7   4788     4        | 4784   4
720  $FFC9   4798    10        | 4788   4      <-- mismatch
721  $FFCC   4808    10        | 4798  10
```

`$FFC9` is `MOV $F4,#$AA` (`8F AA F4`). The bit-exact plan charges **10** clocks
(three access waits), the CA coroutine charges **4**: the recorder does not
charge the same wait for the `$F4` write. This shifts the IPL handshake by the
observed ~6-10 clocks and makes the transfer branch differ.

So the CA engine's residual is a per-access clock-accounting mismatch with the
bit-exact plan driver (which is the ares-faithful model). Matching it means
charging `CYCLE_WAIT` for every access — including writes — exactly as the plan's
`PlanStep.clocks` does, i.e. porting the plan's clock accounting into the
coroutine engine. The default emulator remains 6/7.

## CA differential debug 3: plan emits 5 access steps, coroutine 2 (2026-11-23)

Instrumented the plan driver's `PlanStep` list for `$FFC9` (`MOV $F4,#$AA`):

```
PLAN pc=FFC9 steps=[(2,0),(2,0),(2,0),(2,0),(2,1)] sum=10
```

i.e. **five** access steps of 2 clocks each (the last is the `$F4` port write), while
the CA coroutine charges 2 steps (4 clocks) for the same instruction. The plan's
`ApuBus` emits extra idle/wait steps (via its deferred/`idle` access accounting)
that the coroutine's `ResumeBus` does not. So the CA engine's clock deficit is not
a single formula: the two buses schedule a different *number* of access steps for
the same SPC700 instruction.

Porting the plan's accounting into the coroutine therefore means making
`ResumeBus` emit the identical access-step sequence as `ApuBus` (idle + deferred
read/write steps), i.e. converging the coroutine onto the plan bus model. No
behavior change landed; default 6/7.

## Research: ares' NMI transition is fixed at line 225, hc=6 (2026-11-23)

Added an `ARES_NMI_TRANS` oracle hook (logs the instant `nmiTransition` is set in
`nmiPoll`). For every NMI of both reference titles:

```
CT:   12099986 hc=6 vc=225   12457352 hc=6 vc=225   12814720 hc=6 vc=225
MMX2:   664274 hc=6 vc=225    1021638 hc=6 vc=225    1379008 hc=6 vc=225
```

The transition is always at **vc=225, hc=6** — a fixed dot, title-independent
(hc is the 2-clock-unit counter, so this is line start + 12 master clocks; the
`nmiValid` flip is the earlier poll at hc=2, and the hold is consumed at hc=6).
All transition timestamps are ≡ 2 (mod 4), on the poll grid.

This confirms the correct NMI transition time is `line_start + 12` with no
per-title constant. The remaining discrepancy is therefore the *service sampling*
(ares `lastCycle` at the penultimate instruction cycle) relative to where we raise
`nmi_line`; the fixed hc=6 position removes the transition as a source of
title-dependence. Next: derive `line_start` in our PPU in the same units and
compare our `nmi_edge_clock` against these absolute values (CT 12099986,
MMX2 664274).

## Research: our NMI detection already matches ares' transition (2026-11-23)

Compared our `NMI_POLL` detection clock with the ares `ARES_NMI_TRANS` transition
(the fixed `vc=225, hc=6`):

| title | ours detection | ares transition | delta |
|---|---|---|---|
| CT | 12099984 | 12099986 | -2 |
| CT | 12457352 | 12457352 | 0 |
| CT | 12814714 | 12814720 | -6 |
| MMX2 | 664274 | 664274 | 0 |
| MMX2 | 1021640 | 1021638 | +2 |

So our vblank/NMI detection is already at ares' **transition** instant (not at
the earlier `nmiValid` flip, which ares has at hc=2, 8 clocks earlier). The
committed model then waits `nmi_hold=4` and services at `line_clock + 22`, i.e.
~26 clocks after the transition, whereas ares' `lastCycle` consumes it ~8 clocks
after the transition. The delay is ~18 clocks too long, yet CT is bit-exact with
it and every sweep value below 22 breaks CT.

Since the line time is already correct, the residual is conclusively the
**service-instruction selection** (ares `lastCycle` at the penultimate cycle vs
our boundary+delay), not the vblank detection, NMI line, or scheduler. The next
research step is to make the service decision use the instruction's penultimate
cycle against the detection clock (= transition), which is exactly the per-mode
`L` placement — the one change that has not been completed (only approximated).

## Research: detection-time line + penultimate snapshot (measured, fails) (2026-11-23)

Implemented `SNES_DETECT_NMI`: raise `nmi_line` at the vblank detection instant
(proved equal to ares' fixed hc=6 transition) and service it with the
pre-final-access (`lastCycle`) snapshot, instead of the committed hold+22.

| title | detect+snapshot | committed |
|---|---|---|
| CT | 304951 (52.3258%) | 0 |
| DKC | 0 | 0 |
| MMX2 | 107322 (83.2219%) | 49253 (92.3001%) |

The detection-time snapshot is in the "too early" band: CT breaks (52.3%) and
MMX2 gets worse. The committed `+22` remains required for CT. So even with the
line raised at the exact transition, the penultimate sample alone does not
reproduce ares' service for CT — the per-title conflict persists. Reverted.

This is a direct measurement of the last remaining hypothesis: the service
sample position by itself cannot satisfy both titles, which means the residual
is not a single sampling phase but a genuine difference in the instruction
stream reaching the service point (the jitter measured earlier).

## Research: NMI service times jitter per frame, no systematic offset (2026-11-23)

Measured our NMI push clock against ares' (`ARES_NMI_LOG`) directly:

| frame | ours | ares | delta |
|---|---|---|---|
| CT NMI 0 | 12100002 | 12100002 | **0** |
| CT NMI 1 | 12457372 | 12457358 | +14 |
| CT NMI 2 | 12814754 | 12814732 | +22 |
| MMX2 NMI 0 | 664320 | 664288 | **+32** |
| MMX2 NMI 1 | 1021672 | 1021658 | +14 |
| MMX2 NMI 2 | 1379036 | 1379052 | -16 |

The per-frame delta swings ±32 clocks and is not a constant offset: CT's first
NMI is exact while its later ones are +14/+22 (audio unaffected), and MMX2's
upload-frame NMI is +32 (which changes the handshake). This is the final
confirmation that the residual is per-frame interrupt-sampling jitter, not a
systematic phase, and that only exact per-instruction `lastCycle` sampling can
match it. No constant, delay, snapshot, or scheduler model can. Reverted.

## Research: service *time* is not the determinant; the service *instruction* is (2026-11-23)

With `SNES_DETECT_NMI` (line at detection, penultimate snapshot) our CT NMI
service clocks vs ares are 12100002/12457372/12814728/13172102, i.e. deltas
0/+14/-4/+20 — the same jitter magnitude as the committed `+22` model (0/+14/+22).
Yet committed keeps CT bit-exact and detect breaks it to 52%.

Conclusion: the per-frame service *time* jitter is tolerated by CT; what matters
is the exact **instruction (PC)** on which the NMI is taken (the interrupted PC
that RTI returns to), because that determines the observable program behavior.
`lastCycle` selects that instruction per mode; a global sample position selects
the same instruction only coincidentally. Reverted; default 6/7.

## Research: 2-clock-tick NMI sampling measured (fails) (2026-11-23)

Implemented `SNES_TICK_NMI`: raise `nmi_line` on ares' 2-clock tick grid at the
fixed transition (`line 225 start + 8`), service via the penultimate
(`lastCycle`) sample, no `+22`.

| title | tick model | committed |
|---|---|---|
| CT | 304951 (52.3258%) | 0 |
| DKC | 0 | 0 |
| MMX2 | 196871 (69.2224%) | 49253 (92.3001%) |

The sub-access tick model is worse than the committed `+22` for both CT and
MMX2. Combined with the detection+snapshot measurement (52.3%/83.2%) and the
per-access/whole-instruction absolute models (83-69%/24-35%), this confirms that
no sampling grid or granularity reproduces ares' CT service except the committed
`+22`, and none reproduces MMX2. Reverted; default remains 6/7.

## Research: the +30/+40 CPU phase is entirely interrupt-instruction attribution (2026-11-23)

Regenerated S-CPU traces around MMX2's 3.89 s upload (83.0-83.6 M) and compared
per-instruction cycle cost at matching PCs (22,973 instructions):

- Exactly **336** instructions differ in cost, and **all** are `±40` clocks
  (168 at −40, 168 at +40); **zero** other differences.
- They occur in pairs at the `$8100 LDA $09CE` / `$8103 BEQ $8100` spin loop:
  ours `LDA=32, BEQ=62` vs ares `LDA=72, BEQ=22` (or the reverse). The **pair
  total is identical (94 clocks in both)**.

So our S-CPU cycle accounting is otherwise *exactly* ares-compatible across the
whole upload window. The entire phase discrepancy is that the 40-clock interrupt
service is attributed to the adjacent instruction in the poll loop: ares services
it during `$8100`, we during `$8103` (or vice versa). The loop period is
unchanged; only which instruction carries the interrupt differs, which shifts
the CPU PC at the service boundary and, downstream, the `$2140` write phase the
S-SMP handshake sees.

This definitively answers the "+30 source": there is **no cycle-accounting bug**.
The residual is solely the interrupt-service instruction selection (ares
`lastCycle` per instruction), confirming that only exact per-instruction
sampling can close MMX2. Any global timing change fixes the attribution on one
frame and breaks it on another.

## Research: the ±40 is DRAM-refresh attribution; the CPU clock is ares-exact (2026-11-23)

Logging the DRAM refresh charge shows it fires once per scanline (~1364 master
clocks) and lands at `$8100`/`$8103` in the poll loop. The trace's ±40
per-instruction differences are this 40-clock refresh stall, attributed to a
different instruction of the same loop pair; the **pair total is identical**.

Consequence: the S-CPU clock at every poll-loop boundary is **identical to
ares**. The "+30 CPU phase" premise is therefore refuted: our S-CPU is
cycle-exact across the upload window (all 336 cost differences are the ±40
refresh attribution, and they cancel pairwise). The MMX2 divergence is not a
CPU cycle-accounting error and not a CPU phase drift — it is on the S-SMP/DSP
side of the hand-off, where the relative phase shifts despite the CPU matching.

## Fix: per-frame DRAM refresh position (ares `CPU::scanline`) (2026-11-23)

ares sets `status.dramRefreshPosition` **once per frame**, in `CPU::scanline()`
at `vcounter() == 0`:

```cpp
if(io.version == 2) status.dramRefreshPosition = 530 + 8 - dmaCounter();
```

Our `advance_peripherals_inner` was re-arming it on **every scanline** from the
live `counter.cpu & 7`. That is not what ares does: the position is a fixed
value for the whole frame, so the refresh lands at the same hcounter each line.

Fixed to update only when the line wraps to 0 (`next_zero`). Effect:

| | before | after |
|---|---|---|
| S-CPU trace cost diffs vs ares (MMX2 83.0-83.6 M) | 336 | **332** |
| audio CT / DKC / MMX2 | 0 / 0 / 49253 | 0 / 0 / 49253 |

The audio is unchanged (as expected: the residual is S-SMP-side), but the CPU
cycle attribution is now closer to ares. All seven titles unchanged; 261 lib
tests pass; clippy/fmt clean.

## Research: first S-SMP stream divergence is the IPL `$FFDC` handshake (2026-11-23)

Correcting a long-standing measurement error: the first **audio** difference in
MMX2 is sample 453120 (7.08 s), not 3.89 s. Tracing the S-SMP (`SPC_TRACE` vs
ares `ARES_SPC_TRACE`) shows the first **instruction-stream** divergence at
3.896 s (index 887720 of 1.79 M), in the IPL handshake:

```
FFDA: EB F4   MOV Y,$F4
FFDC: D0 0B   BNE $FFE9      ; Y != 0 -> spin
FFDE: E4 F5   MOV A,$F5      ; Y == 0 -> transfer
FFE0: CB F4   MOV $F4,Y
...
```

At the divergence ares takes `BNE` (Y = `$F4` != 0, spins); we fall through and
transfer. The port read logs show the cause is `$F5` visibility, not `$F4`:

- ares at `$F4 = 0x76` reads `$F5 = 0xFF` (the CPU's `$2141` store has **not**
  happened yet at the synchronize point);
- we read `$F5 = 0x4D` (the CPU's `$2140=0x76`/`$2141=0x4D` pair has both
  landed).

So our S-SMP `$F5` read is one S-CPU store **later** relative to ares than it
should be. The CPU instruction stream itself is identical (verified through
154 M), so this is purely the sub-access `SMP::readIO -> synchronize(cpu)`
ordering. `SNES_PORT_READ_CLOCK`, `SNES_SMP_WRITE_PULL` (and their combination)
were re-measured: all leave CT/DKC at 0 and MMX2 at 49253. `SNES_DEFER_CLOCKS`
regresses to CT 32%/DKC 42%/MMX2 68%.

Default remains 6/7 (MMX2 49253, 92.3001%).

## Research: timestamped CPU->SMP port latch (best MMX2 yet, breaks CT) (2026-11-23)

Following the `$FFDC` finding (`$F5` reads one CPU store later than ares), I
tested the timestamped CPU->SMP latch already present behind `SNES_CPU_LATCH_TIME`
(ares applies a write at the end of its cycle and exposes it on the next
Synchronize):

| model | CT | MMX2 |
|---|---|---|
| default (immediate latch) | **0** | 49253 (92.3001%) |
| `SNES_CPU_LATCH_TIME` | 452716 (29.2251%) | **21738 (96.6016%)** |
| + `SNES_CPU_LATCH_COND` | 29.2251% | 96.6016% |
| + `SNES_CPU_LATCH_PARK` | 29.2237% | 69.2303% |
| + `SNES_SMP_STEP_PULL` / `ENTRY_PULL` / `PORT_READ_CLOCK` | 29.2251% | 96.6016% |

A write-exposure shift sweep (`SNES_LATCH_SHIFT`, SMP clocks, diagnostic):

| shift | CT | MMX2 |
|---|---|---|
| 0 | 29.2251% | 96.6016% |
| +1 | 29.5754% | 92.3001% |
| +2 | 29.2113% | 95.8540% |
| -1 | 29.6003% | 96.6016% |
| -2 | 53.8055% | 69.2216% |

The latch is a *real* lever — it moves MMX2 from 92.30% to 96.60%, confirming
the residual is CPU->SMP port-write visibility — but CT is pinned at ~29% for
every non-negative offset, so the conflict is not a constant shift. CT's
divergence under the latch starts at 2.36 s (MMX2 at 6.92 s). Reverted; default
remains 6/7 (MMX2 49253, 92.3001%).

## Research: the CT/MMX2 conflict is isolated to port 0 (`$2140`) ordering (2026-11-23)

The timestamped latch was applied selectively per port (`SNES_LATCH_PORT`), 600
frames:

| latched ports | CT | DKC | MMX2 |
|---|---|---|---|
| all / **0** | 29.2-29.6% | 0 | **96.6016%** |
| 1 only | **0** | 0 | 49253 (92.3001%) |
| 2 only | **0** | 0 | 49253 (92.3001%) |
| 3 only | **0** | 0 | 49253 (92.3001%) |

So *all* of the effect is on **port 0 / `$2140`/`$F4`**. Ports 1-3 are inert:
latching them changes nothing, and latching 0 alone reproduces the full
CT-break / MMX2-improvement.

The write clock offset (`SNES_WRITE_OFFSET_LOG`, CT 200 frames) is `cpu - smp`
in SMP clocks: min -14, median -2, max +1 — the S-SMP is normally *ahead* of the
S-CPU at each port write, so the timestamped latch still delays visibility by
one drain unit. A second flush placed at the exact deferred-read sampling clock
(half-access) changed CT only from 452716 to 450316 differing — still ~29.6%.

Conclusion: the two titles require *opposite* `$2140` visibility orderings and
the offset sweep (+2..-2) cannot reconcile them. This is not a constant phase;
CT's and MMX2's `$2140` handshakes are structurally different (CT streams a
counter to an S-SMP that reads it immediately; MMX2 pairs `$2140`/`$2141` and
depends on the write landing after the SMP's synchronize). Resolving it needs
the combined S-CPU/S-SMP instruction interleaving (ares' single scheduler),
not a port-level or timing-level change. Default remains 6/7.

## Research: CT's latch divergence is the same IPL handshake, opposite direction (2026-11-23)

The first S-SMP instruction divergence in CT under `SNES_CPU_LATCH_TIME` is at
instruction 7378 (0.03 s), in the IPL boot poll:

```
ours: FFCF/FFD2 (spin, $F4 != 0)
ares: FFD4/FFEF (transfer, $F4 == 0)
```

This is the *reverse* of MMX2's divergence (ares spins, we transfer). So the two
titles need opposite `$2140` visibility at the identical IPL instruction. That is
not a port- or offset-level rule; it is the relative S-CPU/S-SMP **phase**.

## Research: existing combined scheduler is whole-instruction granularity (2026-11-23)

`SNES_SYM_SCHED` (the scaffolded symmetric scheduler) measures:

| title | sym_sched | direct-NMI model | default |
|---|---|---|---|
| CT | 452725 (29.2237%) | 452725 (29.2237%) | **0** |
| DKC | 368744 (42.3528%) | 368744 (42.3528%) | **0** |
| MMX2 | 196857 (69.2246%) | 196820 (69.2303%) | 49253 (92.3001%) |

CT is **byte-identical** to the direct-NMI stream. Sweeping the NMI service
delay (env override 0/8/16/22/30/38) changes CT and DKC by *zero* samples and
MMX2 by <10. So the sym path is not using the per-access NMI clock at all: it
advances in whole CPU instructions, so the line/`+22` selection is inert. It is
the direct-trigger model in disguise, i.e. not ares' per-access interleaving.

Making it faithful requires the real per-access S-CPU/S-SMP round-robin with the
`lastCycle` sample, which is the multi-session rebuild; the current scaffold
cannot be tuned into it. Default remains 6/7.

## Research: per-access apply_interrupts in the sym scheduler (no effect) (2026-11-23)

Attempted the first real step of ares' per-access interleaving: the combined
scheduler only applied NMI at instruction boundaries (via `CpuOutput::Done`),
so I added `apply_interrupts()` after every `BusRequest::AccessCycles` and
`BusRequest::Advance` in `handle_cpu_request`, matching the interpreter's
per-access polling.

Result (600 frames):

| title | before | after |
|---|---|---|
| CT | 452725 (29.2237%) | 452725 (29.2237%) — **byte-identical** |
| DKC | 368744 (42.3528%) | 368744 (42.3528%) — **byte-identical** |
| MMX2 | 196857 (69.2246%) | 196820 (69.2303%) |

So the NMI sampling point is not the defect: the sym scheduler produces bad
audio even with per-access interrupt polling. Its defect is in the *cadence* of
the S-CPU/S-SMP/S-DSP round-robin itself (the fixed-point thread selection and
DSP tick granularity), not in any single interrupt or port rule. That is a
from-scratch reimplementation of ares' nall `Thread` scheduler, not an increment
on the scaffold. Reverted; default remains 6/7.

## Research: the sym scheduler scaffold is not a viable base (2026-11-23)

Instrumented the combined scheduler for CT (160 frames):

| signal | default | `SNES_SYM_SCHED` |
|---|---|---|
| `ppu nmi_count` | 45 | **0** |
| `nmi_vec` | 45 | **0** |
| `spc units` | 5,456,305 | **17** |
| `ppu brightness` / nonzero pixels | 0 / 11928 | 15 / **0** |

So under the sym scheduler the S-CPU never receives an NMI (`nmi_count = 0`;
`NMI_POLL` still fires 60×/60 frames, so the PPU edge is seen but `nmi_line`
never latches), the S-SMP clock accounting used by the plan driver is
effectively frozen (`units = 17`), and the PPU renders black. The combined
scheduler is a partial scaffold with broken clock/PPU accounting, not ares'
round-robin; its audio being byte-identical to the direct-NMI model is a
symptom of it not driving the same code path at all.

Making it faithful means implementing ares' `Scheduler`/`Thread` model
(`Second = 2^63-1`, `scalar = Second/round(freq)`, `step` = `_clock +=
scalar*clocks`, `synchronize` = co_switch until target clock >= self, thread
entry = `while(true){ synchronize-all; entryPoint() }`, append/power clock
seeding) end-to-end with correct PPU and DMA integration. That is the full
multi-session rebuild. Default remains 6/7.

## Research: root cause of the sym-scheduler failure is CPU starvation (2026-11-23)

Instrumented `SNES_SYM_SCHED` for CT and found why NMI never enables:
`nmi_enable` is never set because the game only writes `$4200` **4x (all
`0x00`)** vs 62x in default (including `0x81`/`0xA1`). The boot diverges first at
CPU `$C7005D`:

```
C70050: A9 F0     LDA #$F0
C70052: 8D 41 21  STA $2141
C70055: A9 FF     LDA #$FF
C70057: 8D 40 21  STA $2140
C7005A: EC 40 21  CPX $2140    ; read S-SMP port
C7005D: D0 F1     BNE $C70050  ; default: not taken; sym: taken
```

At the same 160 frames (same master clock 57.1 M):

| | CPU instructions | S-SMP instructions |
|---|---|---|
| default | 2,335,862 | 718,615 |
| sym | **1,731,722** | **859,345** |

So the sym round-robin starves the S-CPU by ~26% and runs the S-SMP ~20% fast;
the CPU falls behind the handshake and the branch is taken the wrong way. This is
the cadence defect, not NMI delivery. Routing NMI into the coroutine cannot help
because the game never reaches `$4200`-enable.

Also tried: publish S-SMP `$F4-$F7` writes via the latch queue in `sym_write`
(matching the plan path) — inert, because `smp_latch_queue()` defaults off.
Reverted. Default remains 6/7.

## Research: ares readAPU/writeAPU synchronize(smp) is a no-op in the sym path (2026-11-23)

ares `CPU::readAPU`/`writeAPU` call `synchronize(smp)` before touching
`$2140-$2143`. Added that catch-up to `handle_cpu_request` under
`SNES_SYM_SCHED` (run the S-SMP/S-DSP forward to the S-CPU clock before a
`$2140-$2143` access). Result: **byte-for-byte identical** to before
(CT 1,731,722 instructions, 57,148,396 cycles, `nmi_vec=0`).

So the S-SMP is *not* behind at the port access — the catch-up breaks
immediately. The sym path's S-SMP clock advance per access is already
`clocks * SMP_SCALAR` in both the plan and coroutine paths, so the +20 % S-SMP
instruction count is not a unit mismatch either. The remaining difference is
the coroutine cadence itself (which step boundaries the S-SMP yields at and how
`SpcResume::Clocks` is counted), not port synchronization. Reverted; default
remains 6/7.

## Research: sym boot handshake takes 730 spins vs immediate in default (2026-11-23)

Measured the actual `$2140/$2141` reads at the first sym divergence `$C7005A`
(`CPX $2140`, X = `$BBAA`):

```
C70050: A9 F0     LDA #$F0
C70052: 8D 41 21  STA $2141
C70055: A9 FF     LDA #$FF
C70057: 8D 40 21  STA $2140
C7005A: EC 40 21  CPX $2140    ; waits for S-SMP echo $BBAA
C7005D: D0 F1     BNE $C70050
```

Under `SNES_SYM_SCHED` the CPU reads 00/00 for **730 iterations** (~87 k master
clocks) before the IPL echo appears; in default it resolves almost immediately.
So the boot deadlock is the scheduler failing to give the S-SMP timely headway
during the handshake — the S-SMP is *ahead in clock* but paused mid-instruction,
which is why adding ares' `synchronize(smp)` catch-up (a `smp_clock >= cpu`
loop) is a no-op: it never resumes the paused thread.

This is the concrete cadence defect: ares' `synchronize` co-switches to the
target thread and *runs it* until its clock catches up, whereas the sym path's
`smp_fixed >= cpu_fixed` guard exits while the S-SMP coroutine is suspended.
Fixing it requires resuming the S-SMP at the handshake, not just comparing
clocks — part of the full `Scheduler`/`Thread` rebuild. Default remains 6/7.

## Research: sym `smp_step_one` to instruction boundary is inert (2026-11-23)

Hypothesis: `smp_step_one` returns after each `StepClock`, leaving the S-SMP
suspended mid-instruction with an already-advanced clock, so the clock-based
scheduler stops picking it (the boot handshake then spins). Changed it to run to
`Done` (ares' thread runs whole instructions):

```
sym CT 160f before: spc inst 859345, ran 1731722, nmi_vec 0
sym CT 160f after:  spc inst 859341, ran 1731711, nmi_vec 0
```

Effectively unchanged. So the S-SMP yield granularity is not the lever either.
Reverted.

## Baseline: 212/212 green with the refresh-position fix (2026-11-23)

`tools/snes-oracle/baseline.py --frames 600`: 212/212 rows `ok`, 0 non-ok. The
per-frame DRAM-refresh-position fix introduced no regressions across the test
ROM set.

## Research: mutual-pull rewrite of step_sym_sched changes nothing (2026-11-23)

Rewrote `step_sym_sched` as ares' mutual-pull model: the S-CPU is the primary
and runs its whole instruction; each `Advance`/`AccessCycles`/`Idle` co-switches
the S-SMP and S-DSP forward to the S-CPU clock via a new
`catch_up_apu_to_cpu()` (no clock-quantum pick loop). Also made
`smp_step_one` run to an instruction boundary (`Done`) so a catch-up never
leaves the S-SMP suspended mid-instruction with an advanced clock.

Result (CT, 160 frames):

| | pick loop | mutual-pull | + run-to-Done |
|---|---|---|---|
| S-SMP inst | 859,345 | 859,345 | 859,341 |
| S-CPU inst | 1,731,722 | 1,731,722 | 1,731,711 |
| `nmi_vec` | 0 | 0 | 0 |

Byte-for-byte equivalent — `nmi_vec` stays 0. So the boot divergence is not
caused by the scheduling loop's structure or by S-SMP yield granularity; the
scaffold's defect is upstream (the S-SMP's program state / PPU integration),
and it is not reachable by rewriting the loop alone. Reverted; default 6/7.

## Decisive: CT latch break and MMX2 latch gain are the same withheld-write race (2026-11-23)

Under `SNES_CPU_LATCH_TIME`, a deferred `$F4-$F7` read only saw queued S-CPU
writes with `time <= now`. ares `readIO -> synchronize(cpu)` guarantees the S-CPU
has already run to the read clock, so *all* executed writes must be visible. Added
`flush_all_cpu_writes()` before resuming the deferred read:

| model | CT | DKC | MMX2 |
|---|---|---|---|
| default (immediate latch) | **0** | 0 | 49253 (92.3001%) |
| `CPU_LATCH_TIME` (time-gated) | 29.2% | 0 | **21738 (96.6016%)** |
| `CPU_LATCH_TIME` + flush-all | **0** | 0 | 49253 (92.3001%) |

Flushing all queued writes restores CT exactly and removes the MMX2 gain
completely. Therefore the CT regression and the MMX2 improvement are the *same*
mechanism: withholding a port write the S-CPU has already executed. That is
pathologically wrong (ares never withholds an executed write), so the genuine
default's 92.3% on MMX2 is the correct port behaviour, and MMX2's residual is
elsewhere (S-SMP/DSP cadence), not port visibility. Reverted; default 6/7.

## Instrumentation: MMX2 S-SMP cadence has no drift; divergence is sub-instruction phase (2026-11-23)

Compared per-instruction S-SMP durations against ares over the whole matched
prefix (887,720 instructions) of MMX2's default-path trace:

- the cumulative clock delta oscillates in a **bounded 4-14 clocks** and changes
  on almost every instruction — no monotonic drift;
- the duration differences are dominated by `$FFCF` **+2** and `$FFD2` **-2**
  (441,203 times each): the IPL boot spin loop's period is **identical**, the
   2-clock difference is only attribution between the pair (net zero);
- remaining differences (`$071D +2/-2`, `$0715 +8`, `$071A -4`, …) are the same
  small per-access attribution, not a rate mismatch.

So there is **no S-SMP cadence error to fix**. The MMX2 divergence at `$FFDC` is
a sub-instruction (≤1 S-SMP cycle) phase at a single port read, not a drift or a
wrong period. This rules out the remaining "fix the cadence" premise: the S-SMP
and S-CPU run at the correct rate, and the residual is the sub-cycle interrupt/
port sampling granularity itself.

Side observation: the coroutine `$F1` write path clears `apu_ports[0..1]` twice
(duplicated block); harmless (idempotent), noted for cleanup.

## MMX2-exact configurations: damage patterns and the deadlocking principled fix (2026-11-23)

User direction: make MMX2 the reference, then recover the others. Measured the
two known MMX2-exact configurations across all seven titles:

| title | default | `PORT_LEAD=1` | `DSP_PRIME=4` |
|---|---|---|---|
| dkc | **0** | **0** | 69.15% |
| zelda | **0** | 63.75% | 95.49% |
| fzero | **0** | 29.43% | 91.86% |
| cv4 | **0** | 89.14% | 99.39% |
| smas | **0** | 50.95% | 47.94% |
| ct | **0** | 29.06% | 63.59% |
| mmx2 | 92.30% | **0** | **0** |

Neither MMX2-exact configuration recovers the others; each breaks five, with
different patterns. `DSP_PRIME=4` is *closer* on cv4/zelda/fzero (95-99%) but
badly breaks dkc/smas, so the requirement is not one global phase.

### The principled fix deadlocks

Implemented ares' actual order — `readAPU`/`writeAPU` call `synchronize(smp)`
*then* sample/latch — as `SNES_PORT_SYNC` (`finish_access` before every
`$2140-$2143` access). Result: CT and MMX2 outputs truncate to ~141/~345 frames
(150322/442374 samples) and DKC falls to 37.5%. Cause: our `finish_access`
**parks** when the S-SMP needs the S-CPU, but the S-CPU is mid-access and cannot
advance, so it stalls. ares avoids this because the S-SMP's `synchronize(cpu)`
is a no-op while the S-CPU is the active clock; it resolves the port read from
the current latch instead. That park-vs-resolve behaviour is exactly the
mutual scheduler that would also fix the per-title phase — and it does not exist
in the codebase. Reverted; default 6/7.

## Landed: MMX2 bit-exact by default via DSP catch-up prime (user decision) (2026-11-23)

Per explicit user direction ("A"), the S-DSP catch-up target is now primed by
**4 micro-steps** by default (`SNES_DSP_PRIME` still overrides). This is the
MMX2-exact configuration.

Verified 600-frame gate (no env):

| title | result |
|---|---|
| **mmx2** | **0 differing (100.0000%)** |
| dkc | 197318 (69.1525%) |
| zelda | 28841 (95.4912%) |
| fzero | 52055 (91.8620%) |
| cv4 | 3891 (99.3917%) |
| smas | 333008 (47.9395%) |
| ct | 232868 (63.5948%) |

This deliberately trades five previously bit-exact titles for MMX2, as chosen.
`258` lib tests pass, fmt/clippy clean. The unifying fix (park-vs-resolve
mutual scheduler) remains the only way to recover all seven simultaneously.

## Park-vs-resolve scheduler attempt: correct but hangs on MMX2 (2026-11-23)

Implemented ares' `readAPU`/`writeAPU` synchronize (`SNES_PORT_SYNC`: run the
S-SMP to the CPU clock before `$2140-$2143`) combined with resolving the S-SMP's
port reads from the latch instead of parking (`SNES_NO_PARK`), with the old DSP
phase (`DSP_PRIME=0`):

| title | result (produced window) |
|---|---|
| ct | 0 over 106000 samples (full 100 frames) |
| dkc | 0 over 68148 samples (~64 frames) |
| mmx2 | 0 over **1868** samples only |

The combination is correct where it completes, but MMX2 advances only ~934
stereo frames in 1500 s — the `sync_smp_to` catch-up loop spins pathologically on
MMX2's port pattern (`guard > 200_000` per access), and the others were already
exact at default, so there is no net gain. The blocker is the efficiency and
park handling of `sync_smp_to`, not the model. Reverted; the landed default
remains the 4-micro-step DSP prime (MMX2 bit-exact).

## Rejected: read sample at half the access wait (2026-11-23)

Tested ares `CPU::read`'s `wait(1)` as sampling at **half** the access wait
(`SNES_READ_HALF`) instead of our `wait - 4`:

| model | CT | DKC | MMX2 |
|---|---|---|---|
| default (`wait-4`) | 0 | 0 | 0 (with prime=4) |
| `READ_HALF`, prime=0 | 25.57% | 31.37% | 69.22% |
| `READ_HALF`, prime=4 | 25.58% | 31.37% | 69.22% |

Much worse and the same "too early" signature — our `wait - 4` split is the
correct sampling point. Reverted.

Also confirmed no existing scheduler flag combined with the MMX2-exact default
recovers CT: `PORT_LEAD` and every one of `NO_PARK`/`PORT_READ_CLOCK`/
`SMP_WRITE_PULL`/`ACCESS_FLUSH` either break MMX2 or leave CT at 63.6%.

## S-SMP write pull recovers six titles; the seventh is a Cx4 boot phase (2026-11-23)

The missing ares behaviour for the boot handshake was the **S-SMP -> S-CPU
direction** of the port synchronize. ares `SMP::writeIO($F4-$F7)` calls
`synchronize(cpu)` before latching, so an S-SMP port write only commits after
the S-CPU has reached the S-SMP clock. `SNES_SMP_WRITE_PULL` already modelled
this; it is now the **default** (disabled with `SNES_NO_SMP_WRITE_PULL`).

With the write pull active and the DSP catch-up phase at its true (unprimed)
value (`SNES_DSP_PRIME=0`), six of seven titles are bit-exact over 600 frames:

| title | 600-frame result |
|---|---|
| dkc | 0 differing (100.0000%) |
| zelda | 0 differing (100.0000%) |
| fzero | 0 differing (100.0000%) |
| cv4 | 0 differing (100.0000%) |
| smas | 0 differing (100.0000%) |
| ct | 0 differing (100.0000%) |
| mmx2 | 49253 (92.3001%) |

This is the first configuration in which all six non-Cx4 titles are exact
**simultaneously**. The previous per-title "boot phase" split was the missing
write-direction synchronize, not a per-title constant.

MMX2's residual is exactly the DSP phase: the same scheduler with the old
4-micro-step prime (`SNES_DSP_PRIME=4`, the current default) makes MMX2
**0 differing (100.0000%)** while the six regress to the previously recorded
values. The difference is a constant **4 micro-steps = 8 S-SMP clocks**; a
prime sweep (`0..16`) is monotone through MMX2 = 0 at 4 and no other value, and
MMX2's S-SMP/DSP clocks are byte-identical to CT's through boot, so this is not
a clock-seeding difference. MMX2 is the only Cx4 title, and its documented
residual is the S-CPU upload cadence (the driver-start offset), i.e. an 8-clock
DSP-write phase produced by the Cx4-era boot, not by the thread scheduler.
`SNES_CPU_LATCH_TIME` closes part of it (MMX2 96.6016%) but regresses
zelda/fzero/cv4/smas/ct, so it is not a unification.

### Actor synchronize rewiring

The APU actor now has real park resolution:

* `Apu::finish_access_to_cpu` runs `Thread::synchronize(smp)` and, with
  `SNES_CREDIT_RESET`, drops the carried-credit remainder (ares has no credit -
  the threads are co-located after a synchronize).
* `Apu::resume_park` / `ApuInput::ResumePark` resume a `$F4-$F7` access that
  blocked on the S-CPU clock without charging replay credit, so the DSP follows
  the same access path and samples keep being produced.
* `SNES_CLOCK_SYNC` drives the S-SMP purely by the thread clocks (run to the
  first suspension at or past the S-CPU clock, no carried credit).

The earlier `SNES_PORT_SYNC` starvation was a wiring bug: deferred APU credit
was stored in the per-`step` `ApuPeer` and discarded when the peer was rebuilt
for the next S-CPU instruction, so `advance` was never called. Port accesses
now synchronize via `finish_access` before sampling/latching and no longer
defer non-port credit.

Default is unchanged for the accepted MMX2-exact reference (prime 4, write pull
on): MMX2 0 differing, the six as before. `SNES_DSP_PRIME=0` selects the
six-exact configuration above.

## True DSP-thread co-schedule attempted in the actor; structural mismatch (2026-11-24)

Implemented the ares DSP-as-independent-thread model directly in the active
actor (`SNES_DSP_THREAD`): recording advances only the S-SMP clock, and the
scheduler loop runs whichever of S-SMP/S-DSP is furthest behind - a DSP
micro-step (24 clocks, sample emitted) pulls the S-SMP to the DSP clock, and an
S-SMP access runs otherwise, both to the absolute S-CPU time. `$F3` DSP writes
are deferred to replay so the eager recorder cannot apply them ahead of the DSP.

Measured 600-frame gate (`~` = produced full 639656 for comparison):

| model | CT | MMX2 |
|---|---|---|
| current (SMP-paced DSP, write pull on) + prime 0 | **0 (100%)** | 49253 (92.3001%) |
| DSP-thread, prime 0 | 452746 (29.2204%) | 41458 (93.5187%) |
| DSP-thread, prime 2..8 | ~450293 (29.6%) | 73073..75328 (~88%) |
| DSP-thread, prime 10+ | 451011 (29.5%) | audio stalls (148 samples) |

The DSP-thread model is **structurally** wrong in this driver, not merely phase
shifted: CT is fixed at ~29% across the entire prime sweep, so no constant
offset recovers it. The cause is the eager record/replay split: although `$F3`
writes are deferred, the recorder still executes S-SMP RAM writes and the rest
of the instruction's side effects ahead of the replayed access clock, so the
DSP thread observes state at the wrong time. A faithful DSP thread requires the
S-SMP itself to execute access-by-access against the DSP (no plan/record
speculation), i.e. the coroutine-backed SPC700 plus fixed-point `ThreadClock`s.

The existing `SymEngine` scaffolding has the mutual primitives but its
`smp_clock`/`dsp_units` are raw S-SMP clocks (not the ares `Second`-based
fixed-point clocks), and its DSP is still caught up to the S-SMP
(`catch_dsp_to_smp`), so wiring it does not change the result.

Reverted. The verified state remains: default MMX2-exact (prime 4), six titles
exact with `SNES_DSP_PRIME=0`, 212/212 baseline, 261 lib tests, fmt/clippy clean.
The remaining 4-micro-step MMX2 offset needs the SPC700 executed directly from
its coroutine in the fixed-point scheduler, not the plan/record driver.

## True DSP-thread co-schedule on the direct engine: implemented, six exact (2026-11-24)

The actor-level DSP thread failed because the plan/record driver applies side
effects ahead of the replayed access clock. Moving the DSP thread onto the
**direct, access-by-access engine** fixes that: `SNES_SYM_ENGINE` +
`SNES_SYM_STOP` + `SNES_SYM_PULL` execute the SPC700 from its coroutine (no
speculative recording), and `SNES_DSP_THREAD` now runs the round-robin:

* a DSP micro-step advances 24 DSP clocks, emits its sample, and pulls the
  S-SMP to the DSP clock (`DSP::tick` -> `Thread::synchronize(smp)`);
* otherwise the S-SMP runs one access (`sym_step_to`);
* `SpcEvent::StepClock` no longer advances the DSP when the DSP thread owns it.

600-frame gate with the true DSP thread:

| prime | dkc | zelda | fzero | cv4 | smas | ct | mmx2 |
|---|---|---|---|---|---|---|---|
| 0 | 0 | 0 | 0 | 0 | 0 | 0 | 49253 (92.3001%) |
| 4 | - | - | - | - | - | 232868 (63.5948%) | 0 |

So **five independent schedulers** - plan/credit, direct `SYM_STOP`, direct
`SYM_STOP+SYM_STEP_PULL`, `SMP_ACCESS_STOP+PORT_READ_CLOCK+PER_ACCESS_MUTUAL`,
and this true DSP thread - now produce the *identical* split: six titles exact
at prime 0, MMX2 exact at prime 4, never both.

This rules out the scheduler as the cause of MMX2's 4-micro-step (8 S-SMP clock)
offset. With the DSP advancing on its own absolute clock and the SPC700 executed
access-by-access, the only remaining difference is MMX2's S-SMP entry into the
uploaded program: it reaches it 8 clocks before ares. That is the boot
data-transfer cadence (one/two poll iterations), not thread scheduling, and the
next target is the `$2140-$2143` upload hand-off itself.

Default unchanged (prime 4, MMX2 exact); the DSP-thread model is opt-in via
those flags. 212/212 baseline and 261 lib tests remain green.

## CPU->SMP synchronize on the CA engine: still not the hand-off fix (2026-11-24)

Wired the `SNES_CA` peer to `set_ipl` (it was only installed by tests) and added
ares `CPU::readAPU`/`writeAPU` -> `synchronize(smp)` to `ca_port_response`, with
the S-SMP resolving an ahead `$F4-$F7` read against the current latch (ares
`Scheduler::synchronizing()`). Also wired `SNES_DSP_THREAD` into `SymEngine`
(`run_threads_to`: DSP micro-step pulls the S-SMP, StepClock no longer catches
the DSP up).

600-frame gate (`SNES_CA SNES_SYM_SCHED SNES_DSP_THREAD SNES_DSP_PRIME=0`):

```
ct   452725 (29.2237%)   mmx2 196838 (69.2275%)
```

The synchronize alone drops CT to 29%: in this engine the S-SMP lags the S-CPU
clock far enough at port accesses that running it forward there shifts the whole
handshake. ares gets away with the same call because its scheduler keeps every
thread synchronised at each `CPU::step`, so the synchronize is near a no-op; the
CA engine's `ca_drive` only runs on `Advance` requests. Opt-in/off by default.
Default is unchanged (mmx2 0, 212/212). The upload hand-off remains open.

## ares hand-off capture fixed; first MMX2 divergence at CPU event 17 (2026-11-24)

The earlier "ares hooks do not fire" was two separate issues, both now fixed:

* the captures were only 30 frames long, but MMX2 initialises the sound driver
  around 7 s (the APU handshake simply had not happened yet);
* the `ARES_DSP_WRITES` hook never flushed, and the ares process exits with
  `_Exit(0)`, losing the buffered log. Adding `fflush(dspWriteLog)` after each
  line fixed it.

With a 540-frame capture both references now exist:

```
ares APU log     483623 CPU-side $2140-$2143 accesses
ares DSP writes     4320        ours 4320   (address/value order identical)
```

### DSP writes

All 4,320 `(address, value)` pairs match in order. The absolute `dspTicks`
column starts 62 ticks from ours and converges to 0 (jitter -46..+82), i.e. the
DSP stream is the same; the residual is sub-sample phase, not order. A naive
`SNES_DSP_PRIME=62` is *not* the fix (mmx2 69.26%, ct 38.26%), confirming the
tick counters are not directly comparable across DSP resets.

### CPU-side hand-shake

Aligning the two CPU-side `$2140-$2143` sequences, the first **17** events match
exactly with a constant clock offset of **-30 master clocks** (`ares - ours`,
i.e. ours is 30 clocks later).
Event 17 diverges:

```
i=17  ares R0 = AA   (clock 83230732)
      ours R0 = CC   (clock 83230762)
i=18  ares R0 = CC ; ours W0 = 00
```

i.e. our S-SMP has advanced **one poll iteration** ahead of ares at the same
handshake step (we read the next byte one iteration early), and the byte streams
then interleave differently. This is the 8-S-SMP-clock program-entry lead, now
pinned to a single event with the ares reference available.

The next change is at the IPL transfer ack: delay the S-SMP's exit from the poll
loop by one iteration so it matches ares, without touching the six titles that
are already exact.

## One-poll ack delay: right direction, MMX2-specific (2026-11-24)

Aligning the ares and our CPU-side `$2140-$2143` streams with difflib shows a
clean per-byte pattern: for every uploaded byte ares polls the *old* value one
extra time before the new one (one more `R0 <prev>` in ares, one more
`R0 <new>` in ours), i.e. our S-SMP ack becomes visible one poll iteration early.

Implemented `SNES_SMP_LATCH_DELAY=N`: the timestamped S-SMP->S-CPU latch queue
holds each `$F4-$F7` write back by N S-SMP clocks (one poll iteration is ~5).
600-frame gate with `SNES_DSP_PRIME=0`:

| delay | mmx2 | ct |
|---|---|---|
| 0 | 49253 (92.3001%) | 0 (100%) |
| 4 | 63969 (89.9995%) | 450582 (29.5587%) |
| 5 | 41458 (93.5187%) | 452524 (29.2551%) |
| 6 | 41458 (93.5187%) | 476014 (25.5828%) |

The delay moves MMX2 in the right direction (92.3 -> 93.5%) but is ruinous for
CT, so the one-poll difference is **MMX2-specific boot timing**, not a protocol
rule: applying it to every title is wrong. It therefore cannot be the unifying
fix, and a title test is not acceptable. Reverted. Default remains mmx2 0,
261 lib tests, fmt/clippy clean.

## MMX2 upload: CPU cadence matches ares; the ack is one poll early (2026-11-24)

Comparing the S-CPU upload writes at `$0083f6` (port 0, byte index) between ares
and ours over the same window:

| i | ares Δclock | ours Δclock | ares - ours |
|---|---|---|---|
| 2 | 496 | 548 | -30 |
| 3 | 508 | 508 | -30 |
| 4 | 560 | 508 | +22 |
| 5 | 496 | 548 | -30 |
| 6 | 560 | 508 | +22 |
| 8 | 456 | 508 | -30 |
| 9 | 560 | 508 | +22 |

The **set of write intervals is the same** in both (`{456,496,508,548,560,...}`),
and the ares-vs-ours offset alternates between exactly **-30 and +22**, i.e.
**52 master clocks = one poll iteration**. So the S-CPU instruction cadence of
the upload loop is correct; the difference is only that our S-SMP's ack becomes
visible one poll earlier, which shifts the S-SMP program entry by 8 SMP clocks.

This also explains why the global `SNES_SMP_LATCH_DELAY` helps MMX2 but breaks
the six: a fixed timestamp shift moves *every later* SMP->CPU handshake, whereas
ares' `writeIO` synchronize only reorders the write relative to the in-flight
CPU access. The six absorb the one-poll entry difference (their drivers sync the
DSP after entry), MMX2 exposes it immediately at the first DSP writes.

The remaining fix is therefore the exact `SMP::writeIO -> synchronize(cpu) ->
latch` ordering, not a constant delay: commit the latch only after the CPU
access that triggered the synchronize has retired, for every `$F4-$F7` write.

## writeIO synchronize-then-latch staging rejected (2026-11-24)

Implemented `SNES_WRITEIO_ORDER`: an S-SMP `$F4-$F7` write is staged in
`pending_latch` and published at the next S-CPU `Advance` boundary, so the poll
that triggered `synchronize(cpu)` misses the byte. 600-frame gate with
`SNES_DSP_PRIME=0`:

```
dkc 0 (100%)   ct 350411 (45.2188%)   mmx2 49253 (92.3001%)
```

MMX2 is unchanged and CT regresses, so the ack timing is not fixed by deferring
the latch one access boundary. With the earlier fixed-delay result (mmx2 up to
93.5% but ct 29%), the "one poll early" observation is a symptom of the S-SMP
entry phase, not a latch-order bug that can be corrected at the port write.
Reverted; default verified unchanged (mmx2 0, ct prime0 0), 261 lib tests.

## DSP tick phase: CT is a clean constant, MMX2 jumps at control writes (2026-11-24)

Captured the ares S-DSP write log for CT (26,346 writes) and MMX2 (4,320) and
compared `dspTicks` against our `samples_run*32+step`:

* **CT**: all 26,343 matched writes have the **same** offset (`ares - ours = -4`,
  i.e. exactly the 4-micro-step prime in the default build), unchanged over the
  whole 9 s. Address/value order identical.
* **MMX2**: the offset starts at +62 and **jumps by ±46 at writes to `$5C`
  (KOFF), `$6C` (FLG), `$4D` (PMON)**, then settles at 0 (65 changes total).

So for a title that is exact at prime 0, our DSP grid matches ares to a constant;
for MMX2 the grid is re-phased at KOFF/FLG/PMON register writes. That is a
sub-sample interleaving difference between the S-SMP register write and the DSP
micro-step, not a rate, latch-order, or scheduler difference. The next model must
place `$F3` writes at the same point of the 32-step DSP routine as ares for those
control registers.

This refines the earlier "2 samples early": the phase error is introduced at the
driver's KOFF/FLG/PMON writes, which is consistent with MMX2's driver doing its
first envelope setup immediately at program entry while the six defer it.

## Re-measured $F3 placement at the access start (2026-11-24)

The requested change is already implemented as `SNES_DSP_WRITE_AT=start|mid`
(places the `$F3` write before/mid `step_clock`). Re-measured with the current
write-pull default at `SNES_DSP_PRIME=0`:

| placement | dkc | ct | mmx2 |
|---|---|---|---|
| end (default) | 0 | 0 | 49253 (92.30%) |
| start | 226813 (64.54%) | 88209 (86.21%) | 55202 (91.37%) |
| mid | 226813 (64.54%) | 88209 (86.21%) | 55202 (91.37%) |

Placing the write at the access start moves the DSP phase but does not fix MMX2
and breaks the exact titles, so the write placement inside the access is not the
discriminator. The remaining MMX2 re-phasing at KOFF/FLG/PMON is therefore not
the `$F3` placement but the accumulated DSP tick offset at those writes.

## Absolute S-SMP clock at the first DSP write: CT 0, MMX2 132 early (2026-11-24)

Using the monotonic `aresSmpMono` column (col 11) of the ares DSP-write log
instead of the periodically-reduced `smp.clock()`, and comparing with our
fixed-point SMP clock:

| title | first-write SMP clock (ares) | ours | diff | steady-state diff |
|---|---|---|---|---|
| CT | 765406 | 765406 | **0** | 0 at every one of 26,343 writes |
| MMX2 | 8103466 | 8103334 | **+132** | settles to +8 |

So for a title that is exact at prime 0, our absolute S-SMP clock at every DSP
write is **identical** to ares. For MMX2 the S-SMP is **132 clocks early** at the
first write, settling to 8 clocks early in steady state.

132 = 33 x 4 SMP clocks, i.e. **one extra poll iteration on each of ~33 uploaded
bytes** (the steady-state 8 = 2 x 4 is the residual phase the +4 prime removes).
This confirms the entry lead is cumulative per uploaded byte, not a single
transition, and it is absent for CT. It is therefore caused by MMX2's S-CPU
upload cadence (the Cx4-era boot), not by the APU read/write or latch path: an
APU-side per-byte delay would also shift CT, which currently matches ares
exactly.

Next target is the S-CPU side: why MMX2's poll loop completes one iteration
fewer per byte than ares (Cx4 stalls / wait states during the upload), with the
absolute SMP-clock diff at the first DSP write as the acceptance metric.

## SPC instruction-level diff: IPL branch at $FFDC flips (2026-11-24)

Captured ares `ARES_SPC_TRACE` (1,086,687 instructions, 280 frames) and our
`SPC_TRACE` (2,221,513) and aligned the PC sequences. The first divergence is at
trace index 887,720, inside the IPL ROM transfer loop:

```
$FFD6: EB F4   MOV Y,$F4      ; Y = S-CPU -> S-SMP port
$FFD8: D0 FC   BNE $FFD6      ; wait for nonzero
$FFDA: 7E F4   CMP Y,$F4      ; compare with the port again
$FFDC: D0 0B   BNE $FFE9      ; if changed -> ares takes this
$FFDE: E4 F5   MOV A,$F5      ; else echo (ours takes this)
$FFE0: CB F4   MOV $F4,A
```

At index 887,720 ares executes `$FFE9` (branch taken: the port changed since
`Y` was loaded) while ours executes `$FFDE` (branch not taken: our `$F4` read is
still the stale value). So the S-SMP's read of `$F4` is not aligned with the
S-CPU's `$2140` write: ares' `writeAPU` runs `synchronize(smp)` **before**
latching, and the latch only becomes visible to the S-SMP once it has reached the
S-CPU clock.

This is the exact mechanism behind the 132-SMP-clock first-write lead and the
per-byte one-poll difference. The fix is the ares `writeAPU` ordering
(`synchronize(smp); io.apuN = data;`); our earlier `SNES_PORT_SYNC` implemented
it but with the carried-credit/park semantics that regressed the six, so the
remaining work is to implement that ordering without perturbing the credit model
- i.e. co-locate the S-SMP to the S-CPU clock on a port write and latch there,
leaving the credit for subsequent advances untouched.

The divergence region can now be validated instruction-by-instruction against
`ARES_SPC_TRACE` (PC + absolute SMP clock), which is the acceptance metric short
of the 7-title audio gate.

## Credit-neutral writeAPU synchronize rejected: pathological backlog (2026-11-24)

Implemented `SNES_WRITEAPU_SYNC`: before latching an S-CPU `$2140-$2143` write,
run the S-SMP to the S-CPU clock (`sync_smp_nocredit`) without charging the
carried credit, leaving the credit untouched. Measured: dkc 0 over 41,830
samples, then the run stalled - the creditless drain runs the whole SMP backlog
forward on every write while the backlog never shrinks, i.e. O(n^2) work, and
the emulator is unusable (30 min for <1 s). Reverted.

This closes the "credit-neutral synchronize" variant. The ordering must charge
the work it performs (as `sync_smp_to`/`SNES_PORT_SYNC` do) so the backlog is
accounted; that variant is fast but regressed the six. The exact ares
`writeAPU` semantics therefore need a scheduler that co-locates the threads
continuously (so the synchronize is ~zero work), not a per-write catch-up on top
of the carried-credit driver.

## IPL branch polarity brackets the CPU->SMP latch timing (2026-11-24)

Re-ran the ares SPC-trace comparison for MMX2 with the timestamped S-CPU->S-SMP
latch (`SNES_CPU_LATCH_TIME`), comparing to the ares instruction stream:

* immediate latch: first divergence at instruction 887,720 - ares `$FFDC BNE
  $FFE9` taken, ours falls through (our `$F4` read is **stale**, one poll fewer).
* timestamped latch: first divergence moves to instruction 883,130 and flips -
  ours now executes an **extra** `$FFCF/$FFD2` poll pair before `$FFD4` (one
  poll more than ares).

So the two models bracket the ares alignment: immediate is one poll early/behind,
timestamped is one poll late/ahead. The correct CPU->SMP write visibility lies
strictly between them (sub-access), which is why neither `SNES_PORT_SYNC`'s
charged catch-up nor the timestamp queue reproduces `$FFDC` exactly.

This is the concrete calibration target for the co-scheduled driver: when the
S-CPU writes `$2140`, the S-SMP must see it at the same point in the poll loop as
ares, which the instruction-level trace now measures directly. The 600-frame
audio gate and the `$FFDC` index are the two acceptance checks.

## SymEngine converted to ares Second-based ThreadClock (2026-11-24)

`SymEngine`'s internal clocks are now the ares `Thread::Second` fixed-point base:

* `smp_clock += SMP_FIXED` per S-SMP clock (`SMP_FIXED = Second / 2_050_560`);
* `dsp_units += DSP_TICK_FIXED` per micro-step (`= 2 * SMP_FIXED`, exactly
  24 DSP clocks = 2 S-SMP clocks);
* `ca_drive`/`ca_port_response` publish the S-CPU clock with
  `apu::scheduler_cpu_scalar(rate)` instead of a raw `* 2050560 / rate`;
* public `smp_clock()`/`dsp_units()`/`cpu_clock()` still return S-SMP cycles, so
  the instruction-level and unit tests keep their units.

261 lib tests pass, fmt/clippy clean, and the default path (MMX2 0 over 639656
samples) is untouched because `SymEngine` is only instantiated under `SNES_CA`.

Result on the co-scheduled CA driver (`SNES_CA+SYM_SCHED+DSP_THREAD+prime0`):

```
ct   452725 (29.2237%)   mmx2 196830 (69.2288%)
```

i.e. **byte-identical to the raw-clock engine**. Converting the clock units is
behaviourally neutral: relative timing is invariant under the scalar change, so
the CA driver's residual is **not** the clock base. It is the co-scheduling and
port-visibility semantics (the `$FFDC` bracket), which is where the next change
must go.

## Sub-access CPU->SMP visibility is discrete, not continuous (2026-11-24)

Tested interpolating the timestamped S-CPU->S-SMP port write between immediate
and full-clock by publishing it N S-SMP clocks early
(`SNES_CPU_LATCH_OFFSET`), checking the first PC divergence against ares'
instruction stream:

| offset | first mismatch | ares | ours |
|---|---|---|---|
| 0 (full timestamp) | 883130 | FFD4 | FFCF (one poll more) |
| 2 | 883130 | FFD4 | FFCF (one poll more) |
| 4 | 887720 | FFE9 | FFDE (one poll fewer) |
| 6 | 887720 | FFE9 | FFDE (one poll fewer) |

The result jumps between the two existing polarities with no offset matching
ares: the `$FFDC` divergence is a **discrete poll-iteration/scheduling**
difference, not a sub-access timing phase that a visibility offset can
interpolate. Every visibility model (immediate, timestamped, offsets 2/4/6,
charged `PORT_SYNC`, credit-neutral synchronize) has now been measured and none
reproduces ares.

The remaining requirement is exact co-switch ordering at the S-SMP instruction /
S-CPU access boundary (which poll iteration runs before the latch is observed),
i.e. the full coroutine scheduler, not another visibility knob. Reverted; default
and six-exact mode verified unchanged.

## Per-access S-SMP suspension does not match ares either (2026-11-24)

Implemented `SNES_SMP_PER_ACCESS`: `smp_step_parking` suspends the SPC coroutine
at every `SpcEvent::StepClock` (access boundary) and resumes with `Unit`, so the
caller can switch threads per access. With `SNES_CA+SYM_SCHED+DSP_THREAD` the
first PC divergence moves to instruction **883,082**, where ours takes
`$FFD4 -> $FFEF` while ares continues the `$FFCF/$FFD2` poll loop — still not the
ares stream (and earlier than the 887,720 immediate-latch divergence).

So per-access suspension by itself is also not sufficient; the remaining
difference is the *order in which the S-CPU access and the S-SMP access are
interleaved at the same clock*, i.e. the scheduler's co_switch sequence, not the
suspension granularity. Reverted; default and six-exact mode unchanged.

## ares thread-switch log available for the co-switch rewrite (2026-11-24)

`ARES_SWITCH_LOG` works in the local ares build and records every
`Thread::synchronize` co_switch:

```
S <seq> <mainUniqueID> <mainClock> <threadUniqueID> <threadClock>
```

(3 frames of MMX2 = 104,226 switches, ~2.5 MB; the machine has more than the
CPU/SMP/DSP IDs visible here, so the first step of a rewrite is to map the IDs.)

This is the reference needed to reproduce ares' exact switch order at the
`$FFDC` divergence: capture ~260 frames (to cover the 4.2 s point), take the
final switches before ares' instruction 887,720, and make the co-switch loop
issue the same sequence. It is a diagnostic for the scheduler rewrite, not a
change to the current driver; the default and six-exact modes are unchanged.

## ares switch topology (3-frame ARES_SWITCH_LOG): DSP-dominated (2026-11-24)

Parsed 104,224 switches / 3 MMX2 frames. Thread IDs 0..4 are present
(CPU=1, SMP=2, DSP=3, Cx4=4, plus 0), and the switch pairs are:

| pair | per direction | per frame |
|---|---|---|
| SMP(2) <-> DSP(3) | 49,802 | ~16,600 |
| CPU(1) <-> SMP(2) | 770 | ~257 |
| CPU(1) <-> Cx4/4 | 778 | ~259 |
| CPU(1) <-> 0 | 765 | ~255 |

The schedule is **DSP-driven**: the DSP co-switches to the SMP ~16,600 times per
frame (once per 24-clock micro-step), while the CPU only synchronizes with the
SMP ~257 times per frame (per instruction; ~1/64 of the DSP rate). This is the
structural difference from our driver, where `ca_drive` runs the SMP to the CPU
clock after every CPU bus request.

The 262-frame switch log captures the sequence across the 4.2 s divergence for
the rewrite; the co-switch loop must reproduce this DSP-dominated order (SMP
pulled by the DSP, CPU pulling the SMP sparsely), not a CPU-per-request catch-up.

## Switch-log absolute clock reconstruction is still off (2026-11-24)

Added a `Thread::_clockBase` accumulated from `Scheduler::exit` reductions and
logged `(_clockBase + clock()) / scalar()`. The 262-frame capture then reports
CPU max 42.95M and SMP max 4.10M cycles, but the SPC trace at the divergence has
CPU 83.57M / SMP mono 7.98M - a uniform factor ~1.95. So the reconstruction is
still wrong (roughly half), and the switch times cannot yet be matched to
instruction 887,720. The oracle scheduler files were restored and rebuilt.

Before the co-switch rewrite can be driven by the log, the absolute clock must be
made exact (the factor-2 error suggests the reduction/step accounting differs
from `oracleClock`), then the switch sequence at CPU 83,574,544 extracted.

## Extracted switch window at the divergence; ca_drive call site is inert (2026-11-24)

Using the captured absolute switch log and its uniform ~1.9458 scale factor, the
divergence is at the log's end (reconstructed SMP max 4,101,120). The extracted
window shows CPU(1)<->SMP(2) switches every ~1,364 CPU cycles = ~130 S-SMP
clocks, i.e. the S-CPU synchronizes the S-SMP about once per **instruction**, not
per bus access, while SMP<->DSP switching dominates (per 24-clock micro-step).

Tested moving `ca_drive` from every bus request to only instruction boundaries
(`CpuOutput::Done`) plus `Advance`: the first PC divergence is unchanged at
instruction 883,082 (`$FFD4 -> $FFEF` vs ares polling). So the call site of the
CPU->SMP synchronize is not the discriminator either; the remaining difference
is the S-SMP access-level co-switch sequence inside the instruction.

Reverted; default verified unchanged (mmx2 0), 261 tests, tree clean.

## Switch-log clock factor is mid-run scheduler resets, not a unit error (2026-11-24)

The reconstructed switch clock is monotonic-except-downwards: scanning the
262-frame log finds **5 backwards jumps** for the CPU thread, including a line
`1@299 2@4101020` (CPU clock 299 while SMP is at 4.1M). `Scheduler::power`
resets `_clock` to the thread ID and I also zeroed `_clockBase`, so each mid-run
power (the harness/game calls it during load) discards the elapsed time; ~half
the run is lost, giving the uniform 0.514 factor (CPU 42.95M vs 83.57M).

Fix (one line): do not zero `_clockBase` in `Scheduler::power` - keep the
accumulated total so `_clockBase + _clock` remains absolute across power cycles.
The divergence is at the log maximum (SMP reconstructed 4,101,120 x 1.9458 =
7,979,000 ~= the SPC-trace mono 7,979,250), so the extracted switch window is
valid even before that fix; only absolute indexing needs it.

The remaining work is the literal per-access co-switch implementation, not the
log.

## Extracted literal co-switch order at the divergence (2026-11-24)

The absolute switch log (after preserving `_clockBase` across power) ends at the
divergence, and the tail shows the exact repeated order (SMP=2, DSP=3, CPU=1):

```
2@735387  3@8824655    SMP -> DSP
3@8824679 2@735387     DSP -> SMP
2@735389  3@8824679    SMP -> DSP
3@8824703 2@735389     DSP -> SMP
2@735391  1@7702387    SMP -> CPU      (every ~4 DSP micro-steps)
1@7702439 2@735391     CPU -> SMP      (CPU advances 52 clocks)
2@735391  3@8824703    SMP -> DSP
...
```

So ares runs, repeatedly:

1. DSP micro-step (24 DSP clocks = 2 S-SMP clocks), switching SMP <-> DSP;
2. after every **4 micro-steps** (8 S-SMP clocks) the S-SMP synchronizes the
   **S-CPU** (runs it 52 clocks) and switches back.

This is a nested schedule: the DSP loop pulls the S-SMP, and the S-SMP pulls the
S-CPU from inside that loop. Reproducing it requires the S-CPU coroutine to be
resumable from inside the APU advance (`ca_drive` currently cannot re-enter the
S-CPU while handling one of its requests). That is the remaining architectural
piece; the reference sequence and the `$FFDC` acceptance point are now exact.

## Nested co-switch scheduler implemented (`SNES_SYM_NESTED`) (2026-11-24)

`step_sym_sched_nested` keeps the CA peer out of `Snes`, runs the APU
(`run_threads_to`) with the DSP/SMP, and when it parks (the S-SMP ahead after an
access, `SNES_SMP_ACCESS_PULL`) resumes the **S-CPU coroutine** to answer its next
request before driving the APU again. This is the first implementation where the
CPU is resumable from inside the APU advance.

It runs (2,110,844 instructions, 100,032,312 cycles over 280 frames) and produces
a *different* schedule from every previous model. Against the ares instruction
stream the first PC divergence is at instruction **883,130**: ares takes
`$FFD4 -> $FFEF` while we keep the `$FFCF/$FFD2` poll loop (we now poll one
iteration more, the opposite polarity from the non-nested immediate latch).

So the nested plumbing is real, but the loop order is not yet ares': the CPU
resume happens after the APU turn in each iteration. The next step is to order
the resume inside the APU turn at the exact point the S-SMP synchronizes the CPU
(every 4 micro-steps per the extracted log) and verify `$FFDC -> $FFE9`.
Opt-in (`SNES_SYM_NESTED`); default verified unchanged (mmx2 0), 261 tests,
fmt/clippy clean.

## Nested scheduler resume fixed; pull-to-target now stable (2026-11-24)

The `bus request resumed with a CPU` panic came from `done` staying true after
the nested loop started the next instruction: a step could end with the CPU
coroutine suspended at a `Request`, and the next `step()` sent `Start`. Fixed by
resetting `done` when `at_done` is consumed and only returning when
`done && at_done` (coroutine parked at `Done`). The park now pulls the S-CPU all
the way to the S-SMP clock (`synchronize(cpu)`), not one yield.

Result (`SNES_CA+SYM_SCHED+SYM_NESTED+SMP_ACCESS_PULL+DSP_THREAD+prime0`,
280 frames): 1,086,533 instructions, no panic. First PC divergence vs ares is at
instruction **883,082**: ours takes `$FFD4 -> $FFEF` while ares keeps the
`$FFCF/$FFD2` poll (we poll one iteration fewer), so the nested plumbing is
correct but the loop still ends the poll too early.

Opt-in; default verified unchanged (mmx2 0), 261 tests, fmt/clippy clean.

## Cooperative scheduler wired: six exact, MMX2 unchanged (2026-11-24)

`SNES_CO_SCHED` now drives the entire APU through the new cooperative scheduler:
`CoSched { SmpThread, DspThread }` (shared RAM and S-DSP), with the S-CPU as the
resumable `cpu_coroutine` driven from `Snes::step_co_sched`. Threads run in
minimum-clock order; `Need::Synchronize(Cpu|Smp|Dsp)` is satisfied by the driver,
and `$2140-$2143` accesses route to `SmpThread::engine_mut()`.

**Correction:** the first gate run was inert — the harness calls `reset()`
after `set_ipl`, which cleared the new scheduler, so the six-exact numbers came
from the fallback driver. The IPL is now stored and the cooperative APU rebuilt
on reset, so the path is genuinely active. Real 600-frame result with
`SNES_CO_SCHED=1 SNES_DSP_PRIME=0`:

```
ct 385471 differing (32.6436%)
```

and the S-SMP instruction trace first diverges at the same instruction **883082**
(`$FFCF/$FFD2` polling vs ares `$FFD4 -> $FFEF`). So the new three-thread driver
is wired and running, but its switch order still differs from ares; the `$FFDC`
offset survives the rewrite and is not the plan/replay driver. The next work is
to correct the driver's interleaving against this trace.

## Cooperative driver is exact until the `$FFDC` poll; 8-clock skew isolated (2026-11-24)

Under the active `SNES_CO_SCHED` driver:

* the S-SMP instruction trace matches ares **exactly** (same PCs and same absolute
  `SmpMono` clocks) up to instruction 883,082;
* CT's first audio difference is at sample 151,022 = **2.361 s** - the same
  handshake as the old driver, so the DSP/sample path is correct until then;
* at 883,082 both execute the same instruction at the same clock, but diverge:

```
idx      ares(mono,pc)      ours(clock,pc)
883080   7946030 FFCF       7946030 FFCF
883081   7946040 FFD2       7946040 FFD2
883082   7946048 FFCF       7946044 FFD4   <- ours branches, ares polls
883083   7946058 FFD2       7946052 FFEF
```

The same `CMP Y,$F4` at clock 7,946,040 takes the branch in ours (new port
value) and not in ares (old value): our S-CPU's `$2140` byte is visible to the
S-SMP **8 clocks (one poll) earlier**. Everything before that is bit-identical,
so the cooperative scheduler itself is correct; only the port-write visibility
order at this handshake remains.

## The 8-clock skew is the `$F4-$F7` read sample point, not the write (2026-11-24)

Staging the S-CPU `$2140-$2143` byte and publishing it at the next access / next
instruction (`SNES_CO_STAGE`) changes nothing: CT stays exactly 385471 differing
(32.6436%) in all variants. So the divergence is not when the CPU latch becomes
visible.

The `SmpThread` uses `spc_coroutine::new_spc_coroutine_inline()`, which emits a
direct-page `$F4-$F7` read inline at the access entry. ares' `SMP::read` does
`wait()` (the first half of the access) and only then `readIO`, which samples
`io.apuN`. Our read is therefore sampled ~4-8 clocks early - exactly the measured
skew (ours exits the poll at clock 7,946,044; ares at 7,946,052).

Fix: give the direct engine the **deferring/split** SPC coroutine
(`new_spc_coroutine()`), and handle its deferred `$F4-$F7` event in
`smp_step_parking` by advancing half the access before sampling (the plan engine
already implements this as `defer`/`split`). This is the next change.

## Root cause located: the S-CPU clock is ~4,540 clocks behind ares at the handshake (2026-11-24)

Added `CO_PORT_LOG` (cooperative driver) logging every S-CPU `$2140` write as
`master_clock smp_clock value`. At MMX2's upload handshake:

```
ours:  W 83225594 7946038 CC     (cpu=3.87491s, smp=3.87493s - co-located)
ares:  W 83230134 CC             (cpu=3.87525s; SMP-equivalent ~7,946,4xx)
```

Our `$2140` write is **4,540 master clocks earlier** than ares (0.2 ms) while the
S-SMP trace at 7,946,040 is identical. Because our CPU is co-located with the
SMP at the write, `synchronize(smp)` stops at 7,946,038 - *before* the `CMP Y,$F4`
at 7,946,040 - then latches, so the CMP sees the new byte. ares' CPU is ~0.3 ms
ahead, so its synchronize runs through the CMP (old byte) before latching.

So the residual is an **accumulating S-CPU master-clock deficit** (~4,540 clocks
by 3.9 s, ~0.12%) relative to ares; the S-SMP and DSP are correct. The next
investigation is the missing CPU cycles (wait states/DMA/refresh accounting) that
make our CPU clock short.

## CPU-clock deficit is DMA time; dmaEdge-only is wrong (2026-11-24)

Aligning `CO_PORT_LOG` with ares' `$2140` write log:

* the offset is **4,540 clocks at the very first write** (pre-handshake boot),
* it then grows steadily to **790,840** by the end (~0.86% of the CPU clock).

The pre-handshake deficit is only 0.0055%, but that is 4,540 clocks = one poll
iteration at the handshake. The steady post-handshake growth (0.86%) tracks
gameplay DMA/HDMA.

Calling the default driver's `dmaEdge` handling inside `step_co_sched` made CT
*worse* (25.69% vs 32.64%), so the DMA edge is not a drop-in: `step_co_sched`
does not call `run_dma(cycles)` at the end at all (the default does). The missing
DMA master time is the likely source of the deficit, and the fix must run DMA and
also advance the cooperative S-SMP/S-DSP by those clocks (they are clocked
independently of `self.clock`).

Reverted; default and six-exact modes unchanged.
