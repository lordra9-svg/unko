# Per-ROM accuracy campaign (started 2026-11-25)

This is the oracle campaign that follows the smoke catalog. It is deliberately
separate from the compatibility statuses: a game can **run** and still have a
low oracle match.

## NES stage — complete (2026-11-25)

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

## Second NES oracle: ares (wired, deferred)

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
`compatibility/accuracy-ares.jsonl`. It is smoke-tested on three ROMs but the
full pass is deliberately deferred until after the SNES stage. At ~2.7 s/ROM,
7,101 headered NES images are roughly six hours.

ares audio is not scored: its host stream is resampled/filtered differently
from our f32 output.

## SNES stage — pending

The SNES oracle is the calibrated ares capture pipeline in
`tools/snes-oracle/`. A full 4,642-ROM, 600-frame serial capture is a
multi-day campaign; it starts after NES. The same ledger format will carry
per-ROM video/audio results.

## Page integration

`compatibility/app.js` fetches `accuracy.jsonl` alongside `games.json` and
shows the NES geometry percentage and steady-state percentage in the Accuracy
column. The column is empty for entries without a record. The page is deployed
per system, not per ROM.
