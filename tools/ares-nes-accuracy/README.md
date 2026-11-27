# ares NES accuracy host

Second NES oracle for the compatibility catalog. It uses the local patched ares
build (`refs/ares/build_headless/desktop-ui/ares`) with:

- `ARES_SYSTEM=Famicom`
- `ARES_AUDIO_RATE=44100`
- the capture patch that writes per-frame NES colour-index hashes in
  `trace.txt` and the raw index sequence next to the RGBA dump

The patch adds `captureGeometryHashes` to `Program` and emits:

```
frame N <rgba-hash> <audio-hash> <index-hash>
```

The third hash is FNV-1a over the PPU's native 9-bit colour indices
(`emphasis << 6 | CGRAM colour`, greyscale applied), two little-endian bytes
per pixel. That matches `unko trace` exactly and ignores the two emulators'
different RGB palettes. The Famicom capture crops NTSC 283x242 to 256x240 and
PAL 283x288 to 256x240 at x=18.

`ares-capture.sh` also writes `frame.raw.idx` (last frame) and
`frame.raw.idxseq` (all frames, `u16` LE, 256x240 each).

Run the current metric from the repository root:

```sh
python3 tools/ares-nes-accuracy/rerun_index.py A          # A entries below 100%
python3 tools/ares-nes-accuracy/rerun_index.py A --all    # every A record
```

It upserts records in `compatibility/accuracy-ares.jsonl`, runs 120 frames, and
records:

- `video_pct` - mean per-pixel index agreement, frames 30+, best `-3..3`
  alignment (the primary score)
- `final_video_pct` - last-frame agreement
- `frame_seq_pct` - fraction of frames whose index hashes are byte-identical

`build_shortfall.py` regenerates `docs/nes-accuracy-not-100.{md,csv}`.

`run_nes.py` and `run_pal.py` are the historical first-pass runners (sorted-RGB
geometry for NTSC, final-frame RGB for PAL). Their records stay in the ledger
until the corresponding letter is re-measured with `rerun_index.py`.
