# NES below-100 campaign — measured triage (2026-11-28)

Scope: the ares ledger `compatibility/accuracy-ares.jsonl` (7,101 records, 7,051 `ok`,
50 `error`) and the campaign policy already implemented in
`tools/ares-nes-accuracy/build_shortfall.py` (unlicensed/pirate/beta/BIOS/aftermarket/proto
dumps excluded from the walk-through).

## What "524 below 100" actually is

The reported number is stale. Re-running the policy on the current ledger:

| measure | count |
|---|---|
| all measured records below 100 | 2,381 |
| excluded by policy (Unl/Pirate/Beta/BIOS/Aftermarket/Proto) | 1,853 |
| **campaign list (policy) below 100** | **528** |
| campaign-list load errors | 27 |

**After the 2026-11-28 full re-measure** (branch `tools/metric-exclude-uniform-frames`,
commits `ad108aa` + `7590387`): campaign list **519**, exact-100 rows **4,670** (was 4,370),
9 rows unmeasured (both engines uniform for all 90 frames), 2 rows newly `error` because ares
itself captures a blank frame. 134 rows rose from warm-up exclusion; 39 fell slightly and
**every one has `excluded_frames == 0`** — those were stale, measured before commit `146fc27`,
and the drop is the NMI change, not the metric. Real target set is now **58 rows below 95%
across 45 base titles**.

So the backlog is **528**, not 524, and the 1,853 excluded rows must never be mixed into it.

A second trap: the ledger spans two metric eras. 5,614 records predate the index metric and
carry `geometry_pct`, which is a naive same-frame score and is below 100 for 5,613 of 5,614
records *by construction* (e.g. Argus: `geometry_pct` 77.5, `steady_geometry_pct` 100.0).
`build_shortfall.py::score()` already prefers `steady_geometry_pct`, then `video_pct`. Any
report that reads `geometry_pct` directly invents thousands of false failures. Do not do that.

## Shape of the 528

| band | rows |
|---|---|
| > 99.99 (roughly 1–50 pixels) | 161 |
| 99.9–99.99 | 67 |
| 99–99.9 | 120 |
| 95–99 | 68 |
| 75–95 | 62 |
| 50–75 | 19 |
| 10–50 | 10 |
| < 10 | 21 |

416 of 528 (79%) are ≥ 95%. 228 are ≥ 99.9%. Only 112 rows are below 95% and therefore
plausibly real content divergence.

Tight-band structure (161 rows > 99.99): all `metric: nes-index`, all 120 frames,
**131 of 161 have `index_shift = -1`**, 137 of 161 have `final_video_pct = 100`, and values
cluster at exactly 99.9999 (48 rows) — one or two differing pixels. This is a frame-phase /
metric-alignment signature, not 161 independent bugs.

## Clusters (base title, not row)

112 rows below 95% collapse to 80 base titles; 19 base titles have multiple variants:

- Zelda (10 rows, all 91.11, shift −1, final 100) — one root cause, flagship.
- Dynamite Bowl (4 rows, 93.0, final 0).
- Swords and Serpents (3 rows, 61–63).
- Rambo (3 rows, 83.33), Terminator 2 (3 rows, known open), Digital Devil Story II (2, known open).
- Hebereke (2, 90.3), Tecmo Super Bowl (2, 81.11), Fantasy Zone (2, 84.42),
  Major League Baseball (2, 65.56), Iron Tank (2, 90–92), Oeka Kids (2), Thexder (2).
- Twenty rows at 0.00 / final 0.0 are *not* one cluster: headers span mapper 12 sub 1/8,
  mapper 1, mapper 4, mapper 23 sub 2, mapper 25 sub 3, mapper 26, mapper 3 sub 2,
  mapper 68 sub 1 and mapper 152. Mapper 152 is implemented and unit-tested, so "0%" here
  means a session/boot or CHR-window problem, not a missing mapper.

## Decision log

**2026-11-28 — boot palette is not scored.** The Zelda cluster was root-caused and dismissed as a
non-bug: 8 fully-differing frames out of the 90 scored frames is exactly 91.111%, and frames
38–119 are byte-identical to ares. The 8 frames are the warm-up blank screen, rendered from
`$3F00` before the guest writes it. Unko boots palette RAM to 0, ares to `cgramBootValue[0] = 0x09`;
NESdev *PPU power up state* says palette RAM is **unspecified** at power-on and the warm-up screen
is a solid colour based on `$3F00`. No hardware evidence prefers either byte.

Decision (user): keep palette boot at 0 (no ares constant baked in) and stop scoring frames whose
content is hardware-unspecified. Metric rule: **a frame is scored only if at least one of the two
frames is non-uniform; if both are uniform the frame leaves the numerator and denominator**. A
uniform-vs-content frame is still scored as 0, so real divergence cannot be hidden.

## Mapper map of the 58 rows below 95% (measured 2026-11-28)

Every remaining hard row resolves to a zip under the No-Intro tree; headers read directly.

> **Correction (2026-11-28):** an earlier version of this table decoded the iNES
> mapper as `(flags6>>4) | (flags7 & 0xF0)` and mislabelled four NES 2.0 titles as
> mapper 12. NES 2.0 adds `(byte8 & 0x0F) << 8`. Those four are **mapper 268
> (Coolboy)**; mapper 12 has **no** rows below 95%. Re-derived with the correct
> decode.

| mapper | sub | rows | titles |
|---|---|---|---|
| 1 | 0 | 12 | 100 Man Dollar Kid (58.9), Bikkuriman World (69.6), Treasure Master (73.1), NTF2 System Cart (77.5), Prince Valiant (89.9), Battle Stadium (90.3), … |
| 3 | 2 | 8 | Minna no Taabou (0.00), Tokoro-san (38.3), Mickey Mouse ×2, Dynamite Bowl ×4 |
| 2 | 2 | 5 | Super Pitfall ×2 (41.5), Swords and Serpents ×3 (61–63), Castelian (79.6) |
| 0 | 0 | 4 | Ice Hockey VC (90.7), Thexder ×2 (93.7), NTF2 Test (94.2) |
| 4 | 0/3 | 4 | Terminator 2 ×3 (75.5–75.9), Super Turrican (84.1) |
| 268 | 0/1/8 | 4 | Rod Land ×2 (0.00), Star Wars LRG (0.00), 8 Eyes (71.3) — **no ares oracle** |
| 19 | 3 | 2 | Digital Devil Story II ×2 (45.4) |
| 69 | 0 | 2 | Hebereke ×2 (90.3) |
| 68 | 1 | 2 | Nantettatte!! Baseball ×2 (0.00) |
| 96 | 0 | 2 | Oeka Kids ×2 (86–88) |
| 66 | 0 | 2 | Super Mario Bros. + Duck Hunt ×2 (92.6) |
| 154, 48, 23/2, 80, 25/1, 5 | | 6 | Devil Man (27.7), Flintstones (90.4), Kid Dracula (0.00), Kyonshiizu 2 (94.9), Racer Mini Yonku (26.6), Suikoden (38.9) |

Mapper 26 (Esper Dream 2, Mouryou Senki Madara) and mapper 152 (Gegege no Kitarou 2,
Pocket Zaurus, Arkanoid II) are no longer in this table: both were fixed and measure 100.000.

**Rejected hypothesis (do not re-run):** the mapper-2/2 cluster is *not* a missing bus
conflict. ares maps mapper 2 to `HVC-UOROM` and `hvc-uxrom.cpp::writePRG` assigns
`programBank = data` with no AND, exactly like `UxRom::prg_write`. Its `readPRG` fixes
`$C000` to the last bank, which unko also does. All six ROMs are 128 KiB PRG / 8 KiB CHR RAM,
vertical, no battery. So the 41–80% scores have a different cause.

## Widened-shift probe — measured 2026-11-28, rejected

A scratch copy of `rerun_index.py` (window −32..+32, writing only to
`/tmp/opencode/shiftprobe/`) re-measured the 51 rows below 95%. It is **not** a
licence to widen the metric.

Why it is rejected as-is:

- Agreement skips frames whose shifted partner falls outside 0..119. A +27 shift
  compares only 63 of the 90 scored frames, so the denominator shrinks and a
  static boot screen can score 100. `Battle Stadium` jumped to `100.000 steady`
  with `0.000 final` and `70.00 seq` — a perfect score built from a subset.
- Optima cluster hard on the window edges (`+32`, `−32`, `+27`, `−26`), which
  means the true offset is outside the range or the match is degenerate. A score
  that improves by relaxing the window is not evidence of accuracy.

What it *is* useful for: separating boot-phase from content divergence. Titles
whose score barely moves when the window is widened have a real content bug and
are the right targets:

| title | ±3 | wide | reading |
|---|---|---|---|
| Digital Devil Story II ×2 | 45.36 | 47.19 | real content divergence |
| Terminator 2 (Japan) | 75.88 | 76.45 | real |
| Castelian (Europe) | 79.63 | 79.63 | real |
| Devil Man (Japan) | 27.69 | 27.72 | real |
| Racer Mini Yonku | 26.63 | 26.63 | real |
| Treasure Master | 73.10 | 74.10 | real |
| Swords and Serpents (Europe) | 61.11 | 61.11 | real |
| Battle Stadium / Mickey Mouse / Dynamite Bowl | 90–94 | 96–100 | boot-phase dominated |
| Super Pitfall ×2 | 41.5 | 58.1 at +32 (edge) | inconclusive |

**Decision:** keep the metric at ±3. These titles need a content-anchored
comparison (align on the first frame with content), not a wider blind window.
That is a separate, testable change and must not be slipped in as a score fix.

## Fixes landed (2026-11-28)

| commit | fix | rows |
|---|---|---|
| `3722108` | VRC6b (m26) applied the A0/A1 swap to `$6000-$7FFF` PRG-RAM writes; only `$8000-$FFFF` registers are swapped | Esper Dream 2, Mouryou Senki Madara → 100 |
| `ccb6ff4` | m152 bus-conflict byte now read via `prg_read(address)`, so `$C000-$FFFF` uses the fixed last bank instead of the switchable one | Gegege no Kitarou 2, Pocket Zaurus, Arkanoid II → 100 |
| `0fb005b` | PPU presented the live buffer; the CPU instruction in flight stepped 5–8 dots into the next frame's scanline 0 and overwrote x=0..k. Now latches the completed frame at vblank (scanline 241) | Lolo 2/3, Asmik-kun Land, Parasol Henbee → 100 |

| `dd57116` | m154 (Namco 3453) shares `DxRom` with 76/88/95/206 but sets **CHR A16** on its four 1 KiB banks at `$1000-$1FFF`; the shared core read the lower 64 KiB | Devil Man 27.686 → 100 |

| `3a019fd` | VRC4b (m25 sub 1) wires chip A0/A1 to CPU A1/A0, so registers are `$x000,$x002,$x001,$x003`; unko used the VRC4f order and swapped every CHR nibble pair. Also widened `$9000-$9003` to mirroring 0/1 and swap 2/3, matching ares | Racer 26.63 → 99.994, Gradius II 98.71 → 100, Bio Miracle 97.23 → 100 |

The third was the first *class* fix: 5 of 9 sampled near-miss titles had **every**
differing pixel on scanline 0, x<8, on 2–3 frames of 90. Static scenes hid it;
palette fades exposed it. Verified by removing the latch: the regression test
fails, and the five titles drop back to 99.999x.

Follow-up metric fix (`42b2649`): the warm-up uniformity test ignored the PAL
mask. It tested all 256x240 pixels and anchored the colour comparison at
`x[0]`, which for PAL is a clipped border pixel. ares' 63-pixel border made a
genuinely blank PAL boot frame look non-uniform, so 5 frames were scored
instead of excluded. Zelda PAL 92.031 → 97.444 (the rest is a real PAL content
difference — unko PAL renders the NTSC image, ares PAL does not). NTSC is
unaffected (`mask is None` path unchanged).

Also from the frame-latch fix: `Legend of Zelda, The (USA)` went 98.7952 →
**100.000**. Its single frame-37 miss was the next-frame leakage, not a content
bug — the earlier "real content boundary" reading was wrong.

## Mapper 2 submapper 2 cluster — resolved as non-bugs (2026-11-28)

`Super Pitfall (USA/Japan)` at 41.5% is **CHR-RAM power-on fill**, not a mapper defect.
The guest runs normally (title screen rendered by frame 119); the game clears the nametable
to tile `$FF` and has not yet uploaded that tile. unko's CHR-RAM starts at `0x00`
(`cart.rs:1045`, `vec![0; size]`), so tile `$FF` decodes to colour 0; ares' CHR-RAM reads
`0xFF`, so the same tile decodes to palette entry 3. Frames 22–85 differ for that reason and
the rest is the same content one frame later. A controlled `0x00 → 0xFF` experiment lifted both
titles to 99.955/100.000 and changed nothing else in the cluster.

CHR RAM is **undefined at power-on** just like palette RAM, so this stays `0x00` under the
same policy as the boot-palette decision. Baking `0xFF` to match ares would touch every
CHR-RAM title and encode an oracle convention as truth.

`Castelian (Europe)` at 79.63% is the **PAL content question**, not mapper 2: `Castelian (USA)`
is 100.000 on the same code path, the screen is stable in both engines from frame 9, and no
integer shift aligns the castle region. ares' Europe capture shows garbled font/alphabet tiles
where unko shows coherent art, and ares-Europe is only 75.5% similar to ares-USA while
unko-Europe is 97.5% similar to unko-USA. Worth treating the PAL oracle as suspect before
chasing these rows.

Not bugs, recorded so they are not re-chased:
- `Minna no Taabou` needs `$FF` in work RAM; policy is `$00` (see the divergence ledger).
- Mapper 268 (Rod Land ×2, Star Wars LRG, 8 Eyes) — **ares has no mapper 268**, so
  those rows are oracle-unmeasurable, not failures.
- `100 Man Dollar Kid` 58.89% was the retired geometry metric; it is 99.99% under the index metric.
- Digital Devil Story II: the old "$F639/$F63B/$F63D operand bytes" note is **wrong**.
  Those are normal sprite-loop instructions (`DEC $5C; BMI +3; JMP $F59E; RTS`).
  The real symptom is that the game never writes the nametable at all (cleared once
  at reset, never redrawn) while the CPU runs its main loop and NMI normally.

## Terminator 2 — resolved as power-on RAM (2026-11-28)

`Terminator 2 - Judgment Day (USA/Europe)` and `Terminator 2 (Japan)` at 75.5–75.9% are the
**power-on work-RAM fill**, not a mapper or MMC3 bug.

600-frame capture, USA ROM: ares' first non-uniform frame is 24 and shows the scrolling
`TERMINATOR 2 / JUDGMENT DAY` logo, then the legal screen, cycling past 600. unko's first
non-uniform frame is 23 and jumps straight to the face/logo art, then "THE STORY" — the retail
intro is skipped entirely, which the 120-frame metric window then scores against ares' logo.

Controlled fills: `Zero` 75.92 (intro skipped); `Fill` ($FF) **99.9931** with 562/600 frames
byte-identical; `Random` **identical to Fill** (same 38 differing frames). So the game depends on
RAM being **nonzero**, not on any particular value. The trigger is a contiguous 82-byte region
`$0696–$06E7`; setting any single byte there to `$FF` reproduces ares' intro. Boot code at `$FF8B`
zeroes only `$78/$79` and the warm-up counters, never that region; the first divergent
instruction is `$C749` (`BCC`) on the uninitialised pointer low byte in `$89/$8A`.

Decision unchanged: `RamClear::Zero` stays. This is the **fourth** cluster in that class
(Minna no Taabou, Super Pitfall ×2, Mickey Mouse ×2, Terminator 2 ×3) — about 8 campaign rows
that are measured against an oracle convention rather than a hardware-defined value.

**Update 2026-11-28:** superseded by the `RamClear::Random` power-on default. The nonzero fill
restores the retail intros (Terminator 2 75.5-75.9 -> 99.99-100; Minna no Taabou 0 -> 100;
Mickey Mousecapade/Fushigi 87.2-87.5 -> 100) while the Super Pitfall rows stay at 41.5.

## Phase B attempt — rejected (2026-11-28)

A builder proposed consuming a deferred NMI latch inside `Cpu::sample_cycle`
(`bus.take_nmi()` + `nmi_suppressed()`), on top of `$2002`/NMI tracing. It compiled,
but **regressed `Tokoro-san` from 38.327 to 9.756**, and came with no test, no
measurement and no written mechanism. Reverted; nothing merged.

Phase B therefore remains open with the same acceptance: 12/12 blargg `vbl_nmi`
ROMs **and** Tokoro-san ≥ 99.99. The next attempt must produce the first divergent
event at dot resolution *before* changing CPU/NMI semantics, and must show the
12-ROM suite passing alongside the measurement.

## Retracted hypothesis: "the runner never passes --region" (2026-11-28)

I claimed `rerun_index.py` ran unko in NTSC against ares in PAL. **Wrong.** Verified:

- `unko trace` has **no** `--region` option — `./target/release/unko trace … --region pal`
  prints `unko: unknown option --region`. The `--region` flag exists only on the `text`
  subcommand. Passing it from Python would break every run.
- `System::header_timing` (`system.rs:772`) already selects PAL/DENDY from NES 2.0 byte 12
  or iNES byte 9, which is the same rule as `rerun_index.detect_region`. So `unko trace`
  already runs these ROMs as PAL; an explicit flag would select the identical timing.
- A header-vs-ledger scan of all 496 PAL + 1 DENDY rows found **0 region mismatches**, and
  re-measuring reproduces the recorded numbers exactly.

So the PAL numbers are correct inputs, and the PAL divergence is a real emulator difference,
not a harness one. Region *does* change output (patching Zelda's timing byte 1→0 changes
59/60 frame hashes), so the lever exists — it is just already applied.

Remaining PAL lead (unresolved): unko PAL output is near-identical to unko NTSC (only the
animated waterfall sprites differ), while ares PAL differs from ares NTSC across the
background. Something makes ares take a different path under PAL.

## Order of work

Batch 0 (this branch): Zelda cluster. Highest rows-per-fix, final frame already identical,
and the answer decides a policy question: if the divergence is intro/fade frame phase, it
applies to ~180 rows with `final_video_pct == 100` and must be resolved as a metric/policy
decision, not 180 title fixes.

Batch 1: the remaining 0.00 / final 0.0 rows, grouped by mapper. Two per mapper first
(Fleet Commander + Minna no Taabou, mapper 3 sub 2; Pocket Zaurus + Gegege no Kitarou 2,
mapper 152), each as its own change.

Batch 2: Dynamite Bowl (4 rows, final 0) and Swords and Serpents (3 rows) — real content,
single base title each.

Batch 3: the > 99.99 band. Only after Batch 0 answers whether −1 shift is harness phase.
If it is, fix the metric/normalisation once; do not chase 131 one-pixel diffs individually.

## Guardrails

- One base title per branch; one builder at a time in this checkout (shared git index).
- Every claim carries `rerun_index.py` before/after output. No ledger edit without measurement.
- No title-specific special-casing, no patching to the answer key, no metric weakening.
- A fix must add a regression test that fails before it and passes after.
- Gates: `cargo test -p unko-nes`, `cargo clippy -p unko-nes --all-targets -D warnings`,
  `cargo fmt --all --check`; workspace gate before any merge.
- Stop and report on: divergence traced to a harness phase artifact (policy decision),
  a mapper/scheduler redesign, missing `test-roms/`, or a failing baseline.
