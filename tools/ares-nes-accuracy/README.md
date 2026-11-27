# ares NES accuracy host

Second NES oracle for the compatibility catalog. It uses the local patched ares
build (`refs/ares/build_headless/desktop-ui/ares`) with:

- `ARES_SYSTEM=Famicom`
- `ARES_AUDIO_RATE=44100`
- the capture patch that writes a per-frame palette-independent geometry hash
  in `trace.txt`

The patch adds `captureGeometryHashes` to `Program` and emits:

```
frame N <rgba-hash> <audio-hash> <geometry-hash>
```

The geometry hash uses sorted-RGB colour indices, matching `unko trace` even
when ares and Unko have different palette tables. The Famicom capture crops
NTSC 283x242 to 256x240 and PAL 283x288 to 256x240 at x=18.

Run it from the repository root:

```sh
python3 tools/ares-nes-accuracy/run_nes.py [limit]
```

It resumes from `compatibility/accuracy-ares.jsonl`, extracts the first `.nes`
member, runs 120 frames, searches a -3..3 frame alignment, and records both the
same-index and steady-state (frame 30+) geometry percentages. Audio is not
scored because ares' host stream is resampled/filtered differently.

The PAL calibration runner is `tools/ares-nes-accuracy/run_pal.py`: it runs
with `ARES_REGION=PAL`, compares the final frame after the 256x240 crop, and
records exact-pixel and sorted-palette percentages. The full NTSC pass and the
495-ROM PAL calibration are both complete.
