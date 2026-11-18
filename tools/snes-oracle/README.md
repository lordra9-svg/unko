# SNES oracle toolkit

Headless capture and comparison helpers for `unko-snes`. The primary oracle is
the locally built ares (patched for capture); RustySNES is the second oracle.

## Oracles

| Oracle | Binary | What it gives |
|---|---|---|
| ares | `refs/ares/build_headless/desktop-ui/ares` | per-frame hash trace, last RGBA8 frame, full native-rate stereo PCM |
| RustySNES | `refs/RustySNES/target/release/{vram_dump,port_clock}` | VRAM/ARAM/WRAM/CGRAM/framebuffer dump, S-SMP/port traces |

The ares build in `refs/ares` carries local capture patches:

- `--frames N`, `--dump-trace path`, `--dump-frame path`, `--dump-audio path`
- `ARES_AUDIO_RATE=32040` forces the audio stream to the S-DSP's native rate,
  and `--dump-audio` writes the **whole** run's stereo samples as interleaved
  `f32` (`L,R,...`). The native rate is **32,040 Hz**, not 32,000: ares'
  `apuFrequency = 32040 * 768 = 24,606,720`, and one S-DSP sample is 768 of
  those clocks. Using 32,000 leaves the host resampler active and produces a
  structurally different (filtered) stream.
- `ARES_RAW_AUDIO=<path>` captures the S-DSP's **pre-resampler** output as
  interleaved `i16` (`L,R,...`) from `DSP::sample`, which is the bit-exact
  reference for PCM comparison.
- `ARES_REGION=PAL|NTSC` forces the console region. ares normally picks it from
  the mia game database, which has no entry for homebrew/test images; the forced
  name is `[Nintendo] Super Famicom (PAL)`. PAL capture dumps a 564x288 frame.

Rebuild it with:

```sh
cd refs/ares/build_headless && ninja desktop-ui
```

(`ninja ares` only rebuilds the core library; `desktop-ui` is the app target.)

If CMake regeneration fails after a Homebrew upgrade, the cache references
deleted `Cellar/<pkg>/<version>` paths. Re-point them at the current versions
(a symlink such as `Cellar/dbus/1.16.2_1 -> Cellar/dbus/1.16.4` is enough).

## Capture

```sh
# ares: <outdir>/frame.raw (RGBA8 564x242 NTSC / 564x288 PAL),
#        audio.f32 (stereo f32 at the native S-DSP rate), trace.txt
tools/snes-oracle/ares-capture.sh ROM.sfc 602 /tmp/ares-cap

# PAL: ares selects the region from its game database, so force it here. The
# local build accepts ARES_REGION=PAL (an oracle-only hook).
ARES_REGION=PAL tools/snes-oracle/ares-capture.sh ROM.sfc 602 /tmp/ares-cap-pal

# ours: <outdir>/frame.raw (RGBA8 256x224), audio.f32 (stereo f32 @ 32 kHz)
IPL=.../ipl.rom tools/snes-oracle/ours-capture.sh ROM.sfc 600 /tmp/ours-cap
```

ares' internal frame counter starts two vblanks later than ours, so capture
ares at `N` and ours at `N - 2` (or compare a static screen).

## Synthetic fixtures

`gen-overscan-rom.py` assembles a minimal LoROM that enables PPU overscan
(`SETINI` bit 2) and fills every active line with one colour, so the 240-line
geometry can be diffed against ares. It is hand-assembled in the script (no
external provenance):

```sh
tools/snes-oracle/gen-overscan-rom.py /tmp/overscan.sfc
tools/snes-oracle/ares-capture.sh /tmp/overscan.sfc 122 /tmp/os-ares
FRAMES=120 FRAME_OUT=/tmp/os-ours.raw \
  target/release/examples/snes-info /tmp/overscan.sfc "$IPL"
tools/snes-oracle/diff-frame.py /tmp/os-ours.raw /tmp/os-ares/frame.raw \
  --ours-h 240 --x0 26 --y0 0
```

The overscan frame is **100.000%** exact at crop (26, 0) in the ares 564x242
dump; the top row is the border ares leaves unwritten.

Our core picks the region from the cartridge header country byte
(`Region::Ntsc`/`Region::Pal`), which selects the master clock, the 262/312-line
frame geometry and the `$213F` PAL bit. `unko-snes` reports it as `region:` in
the example output.

## Compare

Video geometry (non-hires):

| Region | ares dump | crop | horizontal scale |
|---|---|---|---|
| NTSC | 564x242 | **(26, 9)** | 2 |
| PAL | 564x288 | **(22, 29)** | 2 |

i.e. our pixel `x` is ares columns `x0 + 2x` and `x0 + 2x + 1`. Both are verified
100.000% exact on a static screen.

```sh
tools/snes-oracle/diff-frame.py /tmp/ours-cap/frame.raw /tmp/ares-cap/frame.raw
# PAL needs the larger dump and the shifted crop:
tools/snes-oracle/diff-frame.py /tmp/ours-cap/frame.raw /tmp/ares-cap-pal/frame.raw \
  --ares-h 288 --x0 22 --y0 29
tools/snes-oracle/diff-audio.py /tmp/ours-cap/audio.f32 /tmp/ares-cap/audio.f32
```

`diff-frame.py` prints the exact-pixel fraction and mean channel error; pass
`--diff out.ppm` for a visual diff. `diff-audio.py` searches a small sample
offset (the two DSP phases can differ by a sample), then reports the first
sample above the tolerance, RMS and peak levels.

## DSP-1 firmware

`unko-snes` never embeds the copyrighted NEC uPD7725 firmware. Point
`DSP1_ROM` at an 8 KiB `dsp1.rom`/`dsp1b.rom` dump when running a DSP-1 title
(the local ares tree has both under `mia/Firmware/Super Famicom/`):

```sh
DSP1_ROM=.../dsp1b.rom tools/snes-oracle/ours-capture.sh \
  "Super Mario Kart (USA).sfc" 600 /tmp/smk
# ares picks the board from its game database; capture the same frame index
# for animated titles (the 0/2-frame rule is only safe for static screens).
ARES_REGION=NTSC tools/snes-oracle/ares-capture.sh \
  "Super Mario Kart (USA).sfc" 600 /tmp/smk-ares
tools/snes-oracle/diff-frame.py /tmp/smk/frame.raw /tmp/smk-ares/frame.raw
```

Super Mario Kart (USA) is **99.981%** exact at the same frame (the 11 residual
pixels are two left-edge sprite columns).

The same core runs the DSP-2/3/4 firmware (8 KiB `dsp2.rom`/`dsp3.rom`/
`dsp4.rom`). Dungeon Master (DSP-2), SD Gundam GX (DSP-3) and Top Gear 3000
(DSP-4) are **100.000%** exact against ares at frame 600 (Top Gear also at
1,500); the local ares database has verification entries forcing their boards
and firmware identifiers.

The bundled `DSP1 Tech Demo` has an empty header table, so ares cannot infer
its board. The local ares database (`mia/Database/Super Famicom.bml`) carries a
verification entry keyed by the ROM's SHA-256 that forces `NEC-LOROM`; with it,
the demo's DSP-driven grid matches and the frame is **95.714%** (the remaining
difference is the demo's own sprite-font decompressor, not DSP arithmetic).

## CX4 firmware

`unko-snes` never embeds the copyrighted Hitachi HG51B data ROM. Point
`CX4_ROM` at a 3 KiB `cx4.rom` dump when running a CX4 title (the local ares
tree has one under `mia/Firmware/Super Famicom/`):

```sh
CX4_ROM=.../cx4.rom tools/snes-oracle/ours-capture.sh \
  "Mega Man X2 (USA).sfc" 600 /tmp/mmx2
ARES_REGION=NTSC tools/snes-oracle/ares-capture.sh \
  "Mega Man X2 (USA).sfc" 600 /tmp/mmx2-ares
tools/snes-oracle/diff-frame.py /tmp/mmx2/frame.raw /tmp/mmx2-ares/frame.raw
```

Mega Man X2 (USA) is **100.000%** exact at frames 30/45/60/90/300/600 and Mega
Man X3 (USA) at 300/600. Attach the firmware only to cartridges that declare a
CX4 (type `$F3`, LoROM); otherwise it would map over that cart's `$6000-$7FFF`
space.

## S-DD1 titles

The S-DD1 needs no firmware: it is detected from map mode `$22`/`$32` and the
decompressor runs against the cartridge's own compressed data.

```sh
tools/snes-oracle/ours-capture.sh "Star Ocean (Japan).sfc" 600 /tmp/so
ARES_REGION=NTSC tools/snes-oracle/ares-capture.sh \
  "Star Ocean (Japan).sfc" 600 /tmp/so-ares
tools/snes-oracle/diff-frame.py /tmp/so/frame.raw /tmp/so-ares/frame.raw
```

Street Fighter Alpha 2 (USA) is **100.000%** exact at frames 120/300/600 and
Star Ocean (Japan) at 120/600. Star Ocean's ENIX logo fade starts a few frames
earlier than ares (frame 90 is 94.046%) but converges by frame 120.

## S-CPU trace diffing against ares

The local ares build carries two more oracle hooks used to localise CPU bugs:

- `ARES_PC_TRACE=<path>` writes the S-CPU program counter for every step, with
  `ARES_PC_TRACE_SKIP` / `ARES_PC_TRACE_TAKE` bounding the window.
- `ARES_SA1_PROBE=<path>` dumps the SA-1's I-RAM and BW-RAM plus the S-CPU
  registers the first time the S-CPU reaches `$00EF34`, and appends a short
  log of the interesting bytes.

`PC_TRACE=<path>` in `snes-info` writes the equivalent trace for our core. The
traces are comparable one-to-one after dropping ares' two power-on entries.
Polling loops (`CMP $2140`/`BNE`-style) legitimately differ in iteration count,
so canonicalise by removing every PC that participates in a repeated
1-3 instruction cycle before diffing.

## SPC7110

No firmware is needed: the program ROM is the first MiB of the image and the
data ROM is the rest. `Momotarou Dentetsu Happy (Japan)` is a good oracle
because the cartridge runs a built-in SPC7110 self-test (press Start) and
prints its own verdict for the register file, the save-RAM buses, the data
port, the multiply/divide units and the decompression length:

```sh
SPC7110_STATE=1 JOYPAD0=4096 JOYPAD0_FROM=150 FRAMES=600 \
  ./target/release/examples/snes-info "Momotarou Dentetsu Happy (Japan).sfc" ipl.rom
```

`ALL OK` means every sub-test passed; the frame is also 100.000% identical to
ares at 120/300/600. Without input the cartridge halts on that screen, so a
frame diff alone only covers the self-test display.

`Super Power League 4` (type `$F5`) and `Tengai Makyou Zero` (type `$F9`, which
adds the Epson RTC at `$4840-$4842`) are both 100.000% identical to ares at
frames 300 and 600 as well. Clear any `time.rtc` save under ares' `Saves`
directory between runs so the RTC starts from the same state.

## Frame-index fixture

`gen-frame-counter-rom.py` builds a 32 KiB first-party LoROM whose NMI handler
increments a 16-bit counter and publishes its low 10 bits into CGRAM colour 0
(blue = bits 0-4, green = bits 5-9). The main screen is off, so the whole frame
is one flat backdrop colour that encodes the frame index - which makes it
possible to check whether `ours-capture.sh`'s `FRAMES=N` and ares'
`--frames N` dump the *same* frame, something no game can tell you because they
are either static or need an oracle.

```sh
tools/snes-oracle/gen-frame-counter-rom.py /tmp/fc.sfc
FRAMES=300 CGRAM_OUT=/tmp/cg.bin ./target/release/examples/snes-info /tmp/fc.sfc ipl.rom
ARES_REGION=NTSC tools/snes-oracle/ares-capture.sh /tmp/fc.sfc 300 /tmp/fc-a300
```

Measured result: our CGRAM colour 0 is `0x12c` / `0x12d` / `0x12e` at frames
300 / 301 / 302, and the captured pixel is identical in both emulators at the
same frame number ((90,74,0) at 300, (99,74,0) at 301). So the two harnesses
agree exactly - any frame-to-frame difference seen between them is emulation,
not capture alignment.

## Regression baseline

`baseline.py` runs every ROM under `test-roms/snes/` for a fixed frame count and
writes a CSV of ROM hash, header, frame hash and audio hash. Run it before and
after a change to catch regressions without re-diffing every ROM by hand.

```sh
tools/snes-oracle/baseline.py --frames 600 --out /tmp/snes-baseline.csv
```

The manifest is intentionally not committed; compare two CSVs instead.

## Current baseline (2026-11-17)

- Video geometry: `spc_timer` at 600 frames vs ares at 602 — **100.000% exact**
  (57,344 / 57,344 pixels, mean |d| 0.000). PAL (`HblankEmuTest.sfc` at 300 vs
  ares `ARES_REGION=PAL` 302) is also **100.000% exact** at crop (22, 29) in
  ares' 564x288 dump, and the overscan fixture at 120 vs ares 122 is
  **100.000% exact** (61,440 / 61,440) at crop (26, 0) in ares' 564x242 dump.
- Audio: `blargg_near_cd_quality.smc` against ares' raw pre-resampler output is
  **bit-exact**: 0 differing samples over 639,700 stereo frames (1,200 frames)
  at a -1-frame alignment. (Before `$00F0-$00FF` reads were deferred to their
  own access clock it differed in 3 samples out of 319,805.)
- Regression manifest: all **212** ROMs under `test-roms/snes/` run to 180
  frames without an emulator error (`baseline.py --frames 180`).

The manifest is a regression guard, not a pass/fail oracle: compare two runs of
the same build inputs to detect changes, then use ares for the actual parity
comparison.
