# SNES library scan — 500 ROMs (2026-11-25)

This is a **load-and-run smoke sweep**, not a pixel/audio accuracy oracle. The
reference gates (ares differential frames and the 600-frame audio gate) remain
the accuracy evidence in
[`snes-accuracy-divergences.md`](snes-accuracy-divergences.md) and the root
`README.md`.

## Dataset

The local No-Intro SNES set contains 4,642 ZIP archives. A deterministic,
region- and mapper-stratified sample was taken from a 1,159-image candidate
sweep. The final selection is **500 unique images** (deduplicated by SHA-256),
with no ROM data committed to the repository.

| Dimension | Distribution |
|---|---|
| Address mapping | LoROM 370 · HiROM 125 · ExHiROM 5 |
| Timing region | NTSC 381 · PAL 119 |
| Header country | Japan 196 · USA/Canada 185 · Europe 119 |
| Header detection | 485 strict standard headers · 15 lenient fallbacks |
| ROM size | 128 KiB – 6 MiB |
| Header offsets exercised | `$7FC0` (LoROM), `$FFC0` (HiROM), `$40FFC0` (ExHiROM) |

The selection deliberately includes ExHiROM titles, SA-1/SuperFX/CX4/SPC7110/
S-DD1/OBC1 cartridges, PAL releases and Japanese releases. Only the 64-byte
S-SMP IPL ROM was supplied; no coprocessor firmware was added, so titles that
need CX4/DSP-1/ST018 firmware exercise the base-console fallback rather than
the coprocessor path.

## Method

```sh
cargo run --release -p xtask -- snes-scan \
  --list /tmp/snes-500.txt \
  --ipl ipl.rom \
  --frames 300 --limit 500 --batch 5 \
  --csv /tmp/snes-500-scan.csv
```

- Release build of `xtask snes-scan`; `--batch 5` reports every five ROMs.
- Each image is loaded with `Cartridge::from_bytes`, with
  `from_bytes_lenient` used only when the strict parser rejects the header.
- Each image runs **300 frames** (approximately five seconds of NTSC video).
- After every completed frame, the scanner hashes the RGBA PPU framebuffer and
  the native-rate stereo S-DSP samples with FNV-1a. A frame that cannot finish
  within the scanner's 5,000,000-step guard is reported as a failure.
- No controller input is injected; the scan measures boot/load stability, not
  gameplay or attract-mode coverage.
- The 500-image list was split into ten 50-image shards and run concurrently
  for wall-clock time; each shard used the same options and batch size.

## Results

| Metric | Result |
|---|---:|
| Images scanned | **500** |
| Loaded and ran 300 frames | **500** |
| Failures / panics / frame timeouts | **0** |
| Unique SHA-256 images | **500** |
| Completed frames | **150,000** |
| Wall-clock time | 11m 22s (ten concurrent shards) |

Mapping / region coverage was preserved exactly as selected:

| Mapping | NTSC | PAL | Total |
|---|---:|---:|---:|
| LoROM | 280 | 90 | 370 |
| HiROM | 96 | 29 | 125 |
| ExHiROM | 5 | 0 | 5 |

All 500 produced non-zero video and audio hashes. Output varied across **454
distinct framebuffer hashes** and **231 distinct audio hashes**; the largest
single video-hash group was 34 images with an identical framebuffer at frame
300 (a static boot/no-input state). That is expected for a boot sweep with no
input and is why the status is *Runs (smoke)*, not *Playable*.

## Candidate exclusions

The 1,159-image candidate sweep had two non-retail exclusions, not emulation
failures:

- `Cassette Kensaki - NUS-you S Cassette (Japan) (En) (Program)`: no valid SNES
  header (a program image, not a bootable cartridge dump).
- `Super 8 (USA) (Unl)`: 8 KiB, below the minimum ROM size accepted by the
  cartridge parser.

## Reproducing

The same runner is part of the repository:

```sh
cargo run --release -p xtask -- snes-scan --dir roms/snes --frames 300 --batch 5
```

`--list` accepts one path per line and supports `archive.zip!member.sfc` to
select a member from a multi-ROM archive. `--csv` records the path, SHA-256,
header fields, mapping, region, country, frames, and the video/audio hashes.
