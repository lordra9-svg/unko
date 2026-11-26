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

The ledger is resumable by source path. The 100-ROM sample uses 120 frames;
the full 4,642-ROM campaign should use 600 frames and is expected to take
multiple days serially.
