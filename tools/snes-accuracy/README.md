# SNES accuracy sample

Runs the first N SNES archives (default: all) through the calibrated ares
capture pipeline and appends one record per game to
`compatibility/accuracy-snes.jsonl`.

```sh
python3 tools/snes-accuracy/run.py 100
```

Per ROM:

- extract the first `.sfc`/`.smc` member and strip a 512-byte copier header
- run `target/release/examples/snes-info` for the frame/audio capture
- run `tools/snes-oracle/ares-capture.sh` at frame count + 2
- compare the final RGBA frame with `diff-frame.py` (NTSC/PAL/hires flags)
- compare raw S-DSP PCM with `diff-audio-raw.py`

The ledger is resumable by source path. The completed full pass covers 4,642
archives at 120 frames; PAL carts set `ARES_REGION=PAL` and use the 288-line
crop. A future 600-frame pass can deepen the same ledger.
