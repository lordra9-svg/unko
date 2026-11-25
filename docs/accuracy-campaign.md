# Per-ROM accuracy campaign (started 2026-11-25)

This is the oracle campaign that follows the smoke catalog. It is deliberately
separate from the compatibility statuses: a game can **run** and still have a
low oracle match.

## NES stage — in progress

Oracle: a local RustyNES trace host built against `refs/rustynes` (GPL-3.0,
kept outside the MIT tree) and invoked by
`tools/snes-oracle`-style orchestration. The host emits the same
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

The NES set is 7,101 headered images. At the measured ~1.2 ROMs/s, the serial
run is about 95 minutes. Headerless NES images are not runnable without an
iNES header and remain **Unsupported**.

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
