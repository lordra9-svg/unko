# Per-ROM accuracy campaign (started 2026-11-25)

This is the oracle campaign that follows the smoke catalog. It is deliberately
separate from the compatibility statuses: a game can **run** and still have a
low oracle match.

## NES stage — RustyNES historical pass (2026-11-25)

**Primary NES oracle is now ares.** A full 7,101-ROM ares pass is running;
this section records the earlier RustyNES pass and remains as secondary
evidence.

Oracle: a local RustyNES trace host built against `refs/rustynes` (GPL-3.0,
kept outside the MIT tree at `~/ai/refs/rustynes-trace-host`). Its
`run_nes.py` drives one ROM at a time. The host emits the same
`# unko trace v1` records as `unko trace`.

Method, one ROM at a time:

1. Extract the first `.nes` member from the No-Intro ZIP.
2. Run `unko trace <rom> --frames 120 --out ours.trace`.
3. Run the RustyNES host for 120 frames.
4. Compare each frame with the palette-independent **geometry hash** (and the
   raw RGBA hash). The ledger records full-run and steady-state (frame 30+)
   percentages.
5. Append one JSON record to `compatibility/accuracy.jsonl` and `fsync` it.

The ledger is resumable: records already present are skipped. The page loads
the ledger over `games.json` by SHA-256.

Audio is recorded but **not scored as exact PCM**: RustyNES exposes a
DC-blocked frontend stream, not its mixer raw stream, so the sample stream is
not directly comparable to our f32 output.

The NES set is 7,101 headered images. The serial run completed in about 6,206
seconds (~1.14 ROMs/s): **6,726 measured**, **375 errors**, **0 skipped**.

| Metric | Result |
|---|---:|
| Measured ROMs | 6,726 |
| Errors | 375 |
| Steady-state geometry 100% | 4,889 (72.7% of measured) |
| Steady-state geometry >= 98% | 5,143 (76.5%) |
| Steady-state geometry >= 90% | 5,553 (82.6%) |
| Median full-run geometry | 98.3333% |
| Median steady-state geometry | 100.0000% |

The 375 errors are predominantly RustyNES unsupported mappers (336), plus 20
truncated/iNES-invalid images and a handful of IConfig/load failures. Full-run
geometry is never exactly 100% because the first boot frames differ; that is
why the steady window starts at frame 30. Headerless NES images remain
**Unsupported** because they have no iNES header to load.

Games below 100% steady-state geometry, plus oracle/load errors, are listed in
[`nes-accuracy-not-100.md`](nes-accuracy-not-100.md) with the full
[`CSV`](nes-accuracy-not-100.csv).

### Argus (Japan) — RustyNES oracle disagreement

Argus is mapper 87. RustyNES renders a spurious tile-0 overlay on the title
screen; Unko does not. The ares Famicom capture matches Unko at **100%
steady-state geometry** from frame 30. An ares record for Argus is in
`compatibility/accuracy-ares.jsonl`, and Argus is excluded from the NES
below-100 list as a known oracle disagreement rather than an Unko rendering
failure.

### Akuma-kun - Makai no Wana (Japan) — mapper 16 CHR refresh

Akuma-kun is a Bandai FCG mapper-16 title (NES 2.0 submapper 4). Two defects
made the title screen corrupt:

- FCG registers live in `$6000-$7FFF`, but the mapper also accepted the same
  register writes at `$8000-$FFFF`.
- CHR-bank writes did not refresh the PPU's CHR-ROM copy, so the tile bank
  selected by the game was not visible to rendering.

Both are fixed and covered by `mapper16_chr_registers_are_at_the_6000_window
_and_refresh_the_ppu_copy`. The corrected frame matches RustyNES exactly; ares
reports 83.73% exact pixels and 99.71% palette-normalized on the final frame.
The ares record now carries those calibrated values.

### Adventures of Bayou Billy, The (USA/Europe) — MMC1 mid-frame CHR split

Bayou Billy's title screen writes a new CHR bank at scanline ~146 so the logo
and the menu/copyright text use different tile sets. MMC1 latches the bank on
the fifth serial write, but the PPU's CHR copy was only synchronised once per
frame, so the lower half of the screen rendered from the logo bank and came
out blank.

`Mmc1::chr_mapping_write` now reports `$8000-$FFFF` writes, which refreshes the
PPU window immediately; `mmc1_chr_bank_write_refreshes_the_pattern_window_mid_
frame` covers the 8 KiB and 4 KiB modes. The corrected screen matches RustyNES
(99.5% exact); ares renders the scanline-skewed logo differently, so the three
Bayou records now carry calibrated pixel values (79.6-79.8% exact, 83.7%
palette-normalized).

### Crisis Force / Cobra Triangle / Crayon Shin-chan — mapper fixes

Three mapper defects surfaced while re-measuring the C list:

- **VRC4 IRQ (mappers 21/23/25/27/183/308).** The counter was decremented every
  CPU cycle. VRC4 has a scanline mode (341/3 prescaler) and a CPU-cycle mode;
  `$F002` bit 2 selects one and `$F003` re-applies the stored acknowledge bit.
  Crisis Force hung before palette initialisation and now boots.
- **HVC-AxROM (mapper 7).** The board powers on with the last 32 KiB bank and
  has no bus conflict. Starting in bank 0 (and ANDing writes with the ROM)
  left Cobra Triangle in a reset loop. Both are fixed and covered by
  `axrom_powers_on_in_last_bank_and_ignores_bus_conflicts`.
- **Mapper 16 submapper 5.** LZ93D50 boards keep their CHR/PRG/mirroring
  registers at `$8000-$800F`; submapper 4 (FCG) uses `$6000-$7FFF`. Crayon
  Shin-chan now matches ares; the FCG path (Akuma-kun) is unchanged.
- **Mapper 185 (HVC-CNROM-SEC).** Seicross, Mighty Bomb Jack, Spy vs Spy,
  B-Wings, Bird Week, Othello and the Sansuu games gate CHR behind a security
  key and read `$FF` until it is written. The key is not in the iNES header,
  so the core identifies the 16 known images by an FNV hash of PRG+CHR and
  unlocks on the matching write. All 16 now measure 100%.
- **VRC2 raster CHR and save RAM.** VRC2/4 CHR registers now refresh the PPU
  window immediately (Konami games switch banks at raster splits), and VRC2
  boards with 8 KiB WRAM map it at `$6000-$7FFF` instead of returning the
  open-bus latch. Ganbare Goemon Gaiden and its revision went from 65% / 0%
  to 100% / 99.999%, and Gaiden 2 to 100%.

### Air Fortress (Japan) — PPU register writes during warm-up

The core used to drop `$2000`/`$2001`/`$2005` writes during the ~29,658-cycle
post-reset PPU warm-up window. Air Fortress enables NMI in that window and
never writes `$2000` again, so it hung in its frame-wait loop with rendering
off. The registers now latch writes immediately; the warm-up only describes
the unstable output, not ignored registers. The regression test became
`warmup_does_not_swallow_register_writes`, and Air Fortress now matches ares.

### Palette-index metric (2026-11-27)

Sorted-RGB "geometry" still scored visually identical frames near zero when
the two emulators' palettes ordered the same NES colours differently (for
example Adventures of Lolo 3 and Alfred Chicken during fades). The campaign is
being re-run letter by letter with a stricter, palette-independent metric:

- Both cores dump the PPU's **native 9-bit colour index** per pixel
  (`emphasis << 6 | CGRAM colour`, greyscale applied), before either RGB
  palette. Unko exposes it as `unko trace --indices-seq`; the ares capture
  writes `frame.raw.idxseq`.
- `video_pct` is the mean per-pixel index agreement over frames 30-119 using
  the best +/-3 frame alignment.
- `final_video_pct` is the last frame's agreement, and `frame_seq_pct` is the
  fraction of frames whose index hashes are byte-identical (a strict
  diagnostic).

This removes per-emulator RGB differences from the score while still catching
every tile, attribute, emphasis, scroll and sprite difference. Re-runs use
`tools/ares-nes-accuracy/rerun_index.py <letter>`; the older `run_nes.py` and
`run_pal.py` records remain in the ledger until their letter is revisited.

The walk-through skips dumps tagged `(Unl)`, `(Pirate)`, `(Beta)`, `(Bios)`,
`(Aftermarket)`, `(Proto)` or `(Demo)`, plus `[BIOS]` images;
`build_shortfall.py` keeps them out of the actionable list but still reports
how many were ignored.

The headless performance benchmark now waits for vblank before enabling
rendering, so it measures the real background renderer. (Previously the PPU
warm-up window swallowed the single `$2001` write, making the benchmark a
backdrop-only fast path.) The CI budget is 200 fps accordingly.

## Primary NES oracle: ares

The local ares capture build now emits a fourth trace field:
`frame N <rgba> <audio> <geometry>`. The geometry hash is the same
first-appearance RGBA index hash used by `unko trace`, so the page can compare
ares and RustyNES on the same metric.

The ares host runs through:

```sh
python3 tools/ares-nes-accuracy/run_nes.py [limit]
```

It uses `ARES_SYSTEM=Famicom`, `ARES_AUDIO_RATE=44100`, 120 frames and a
-3..3 frame alignment search; records append to
`compatibility/accuracy-ares.jsonl`. The full 7,101-ROM pass completed in
about **5h 03m**.

| Metric | Result |
|---|---:|
| Archives | 7,101 |
| Measured | 7,069 |
| Errors | 32 |
| 100% by the relevant metric | 4,370 / 7,069 (61.8%) |
| Best full-run median | 98.3193% |
| Steady median (NTSC) | 100.0% |

PAL was calibrated after the first ares pass. The initial ares geometry hash was
sensitive to colour ordering and reported many PAL titles as 0%. The corrected
PAL runner sets `ARES_REGION=PAL`, uses the 283x288 PAL canvas cropped to
256x240, and records the final-frame exact-pixel percentage (plus a sorted
palette-normalized percentage). NTSC keeps the steady-state geometry metric.

ares is the **primary** NES oracle; RustyNES remains a secondary cross-check
and its records are still shown as **R**. The ares pass has fewer unloadable
images (32 errors vs 375). Argus showed that RustyNES can also render
incorrectly; PAL needed a separate calibrated frame metric.

ares audio is not scored: its host stream is resampled/filtered differently
from our f32 output.

## SNES stage — complete (2026-11-26)

The SNES oracle is the calibrated ares capture pipeline in
`tools/snes-oracle/`; the runner is `tools/snes-accuracy/run.py`.

- All **4,642** SNES archives in title order
- **120 frames**, ares captured at 122 to match its frame counter
- final RGBA frame compared at the calibrated crop/scale
- raw S-DSP PCM compared sample-for-sample
- `ARES_REGION=PAL` and the 288-line crop for the 761 PAL records
- records append to `compatibility/accuracy-snes.jsonl`, resumable

| Metric | Result |
|---|---:|
| Archives | 4,642 |
| Measured | 4,606 |
| Errors | 36 |
| Final-frame exact | 3,379 / 4,606 (73.4%) |
| Final-frame >= 99% | 78.3% |
| Final-frame >= 95% | 84.6% |
| Final-frame median | 100.0% |
| Final-frame mean | 92.539% |
| Audio exact | 4,315 / 4,606 (93.7%) |
| Audio >= 99% | 95.0% |
| Audio median | 100.0% |
| Video + audio both exact | 3,273 / 4,606 (71.0%) |

The 36 errors are 15 unsupported 512x448 interlaced images, 11 archives with
no SNES member, 2 headerless/invalid images, 2 too-small images, 3 missing
ares frame dumps, 2 ares capture stalls and 1 unsupported 512x480 image.

Final-frame video parity is noisy on animated games and mode transitions; low
values are not necessarily rendering errors. Audio parity is much more stable.
The PAL correction deliberately reran all 761 PAL records after fixing country
detection; without it, PAL games were compared with NTSC geometry.

## Page integration

`compatibility/app.js` fetches `accuracy.jsonl` alongside `games.json` and
shows the NES geometry percentage and steady-state percentage in the Accuracy
column. The column is empty for entries without a record. The page is deployed
per system, not per ROM.
