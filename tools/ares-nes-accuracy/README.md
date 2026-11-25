# ares NES accuracy host

Second NES oracle for the compatibility catalog. It uses the local patched ares
build (`refs/ares/build_headless/desktop-ui/ares`) with:

- `ARES_SYSTEM=Famicom`
- `ARES_AUDIO_RATE=44100`
- the capture patch that writes a per-frame palette-independent geometry hash
  in `trace.txt`

The patch adds `captureGeometryHashes` to `Program`, computes the same
first-appearance RGBA geometry hash used by `unko trace`, and emits:

```
frame N <rgba-hash> <audio-hash> <geometry-hash>
```

Run it from the repository root:

```sh
python3 tools/ares-nes-accuracy/run_nes.py [limit]
```

It resumes from `compatibility/accuracy-ares.jsonl`, extracts the first `.nes`
member, runs 120 frames, searches a -3..3 frame alignment, and records both the
same-index and steady-state (frame 30+) geometry percentages. Audio is not
scored because ares' host stream is resampled/filtered differently.

**Status:** wired and smoke-tested on three ROMs; the full run is deferred
until after the SNES accuracy stage. At ~2.7 s/ROM the 7,101 headered NES
images are roughly six hours.
