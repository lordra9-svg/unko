# Test-ROM inventory and NESdev suite status

The public test ROMs are not committed. `test-roms/` is a local, gitignored
drop point; the committed tests skip cleanly when it is absent.

## Local drop (`test-roms/`, 72 public test-ROM files)

| Group | Files | Status |
|---|---|---|
| CPU | `nestest`, blargg `instr_basics`/`instr_timing`, `cpu_interrupts_v2` singles | `nestest` 8,980/8,980 lines; 13/13 blargg interrupt singles; timing pass |
| PPU | `ppu_vbl_nmi` 12, `ppu_sprites` 19, `oam_read`, `oam_stress`, `sprite_ram` | 12/12 and 19/19 |
| APU | blargg APU/DMC 13, DMC latency/pitch/status, square/sweep/noise/linear-counter | 13/13; the rest are diagnostic |
| Battery | `AccuracyCoin.nes` + catalog | 146/146 non-DRAW (146/151 total, 5 DRAW, 0 failed) |

See `docs/accuracy-status.md` for the dated evidence.

## Local reference trees

`/var/home/Lordra/ai/refs` contains additional local assets; they are not
committed.

### BIOS (`refs/bios`)

| File | Result |
|---|---|
| `disksys.rom` (8 KiB) | FDS BIOS, SHA-256 `99c18490…b0178` (matches MAME `rp2c33a-01a.bin`) |
| `Batocera…/mame/bios/fc_disksys.zip` | Two revisions: `rp2c33-01.bin` (`fdc1a76e…12dd72`) and `rp2c33a-01a.bin` (`99c18490…b0178`) |
| `Batocera…/fdsbios.zip` and `fc_disksys.zip` | `fdsbios.nes` is the same image as `disksys.rom` |
| `Batocera…/mame/bios/playch10.zip` | PlayChoice-10 BIOS files (`pch1-c_8m`, `pch1-c_8te`, …); no PC10 games are present |
| `Batocera…/mame/bios/nes_rob.zip` | R.O.B. firmware (`rfc-cpu10.ic1`) |
| `Batocera…/gamegenie.nes` | Game Genie BIOS ROM |
| `Batocera…/nes.pal` | NES palette file for display tests |
| `Batocera…/MesenNesDB.txt`, `mame/hash/nes*.xml` | Game/mapper databases, not ROMs |

The FDS BIOS revisions boot `fdsirqtestsV7_patched.fds` for 600 frames with
identical traces, but *Akumajou Dracula* differs from frame 5 between
`rp2c33-01` and `rp2c33a-01a`; the revision is observable and must be
selectable.

A local Vs. System drop in `/var/home/Lordra/ai/refs/vspC/` was repaired and
staged on 2026-11-16 (see below). **Vs. Gumshoe**, a 2C05 title and a known-good
DualSystem dump are still absent. PlayChoice-10 game ROMs are present only as
raw MAME sets.

The Miracle Piano's documented SysEx replies and the Datach X24C01 protocol are
implemented from the published protocol/datasheet, so neither firmware nor a
physical dump is required for those.

### Local Vs. System / PlayChoice-10 drop (2026-11-16)

`scripts/repair_vs_headers.py` repairs the local VS images into NES 2.0
headers: Vs. console type, four-screen mirroring, cabinet nibble and per-game
PPU model. It writes only to the gitignored `test-roms/` directory.

Staged under `test-roms/vs/` (12 images):

| Image | Mapper | PPU | Payload |
|---|---|---|---|
| `VS Castlevania.nes` | 2 | 2C04-0002 | known |
| `Vs. Castlevania (hack).nes` | 2 | 2C04-0002 | hack |
| `VS Clu Clu Land.nes` | 99 | 2C04-0004 | known |
| `Vs. Duck Hunt (hack).nes` | 1 | 2C03 | known hack |
| `VS Excitebike.nes` | 99 | 2C04-0003 | known |
| `Vs. Excitebike (hack).nes` | 99 | 2C04-0003 | known hack |
| `Vs. Gradius (hack).nes` | 4 | 2C04-0001 | known hack |
| `Vs. Ice Climber (hack).nes` | 1 | 2C04-0004 | known hack |
| `Vs. Pinball (hack).nes` | 3 | 2C04-0001 | known hack |
| `VS Super Mario Bros Version MAME V1.1A.nes` | 1 | 2C04-0004 | known |
| `Vs. TKO Boxing (hack).nes` | 4 | 2C04-0003 | known hack |
| `Vs. The Goonies (hack).nes` | 75 | 2C04-0003 | known hack |

`crates/unko-nes-dev/tests/vs_staged.rs` loads each one, checks the mapper,
PPU model, UniSystem type and four-screen mirroring, and runs ten frames. The
test skips when `test-roms/vs/` is absent. `VS Castlevania.nes` renders without
an ASCII text screen; the others also pass `unko text`/frame smoke tests.

Raw PC10 MAME sets are staged in `test-roms/pc10/` (`pc_smb`, `pc_duckh`,
`pc_ebike`, `pc_hgaly`, plus `playch10.zip`). They are not iNES PC10 images;
conversion and the second-CPU/INST-ROM layer are still required.

```sh
unrar x "/var/home/Lordra/ai/refs/vspC/VS NES Roms.rar" /tmp/vs-roms/
scripts/repair_vs_headers.py /tmp/vs-roms -o test-roms/vs
cargo test -p unko-nes-dev --test vs_staged
```

### RustyNES test tree (`refs/rustynes/tests/roms`, 341 `.nes`)

This adds the x0000 APU tests, `holy_mapperel`, `extra/{apu,cpu,mappers,ppu}`,
`audio-tests`, `dap`, `mmc5`, `mmc1_a12`, `m22` and a full copy of the blargg
suites. Latest probe run (2026-11-15, screen-text verdicts including the
alternate framebuffer font; the 2026-11-14 run plus the `RTS`/`RTI` dummy-fetch
fix, which moves the two `cpu_exec_space_ppuio` copies to pass):

- **159 passes** in the default NTSC run. Running the PAL singles in PAL mode
  moves all seven to pass (**166**), leaving **4** failures.
- **11 failures**: seven PAL APU singles (region), three MMC3 revision rows
  (selector) and `read_joy3/count_errors` (input).
- **171 without a machine-readable verdict**

Reproducible text-verdict run (2026-11-14, `unko text`, 600 frames, all 341
images): **145 print Pass**, **11 print Fail**, **185 print no ASCII
verdict** (custom fonts, audio-only or input-gated). All 11 failures are
configuration-dependent:

- the three MMC3 rows (`5.MMC3_rev_A`, `6-MMC3_alt`, `6-MMC6`) pass with
  `--mmc3-alt`;
- the seven PAL APU singles pass with `--region pal` (all ten PAL APU images
  pass in PAL mode);
- `read_joy3/count_errors` needs a controller script.

No unexpected failure remains. The difference from the 159/11/171 probe above
is methodological: `unko text` only scores suites that print with the standard
ASCII font.

The x0000 `apu_test_1..10` write-coincidence tests and the two previously open
blargg failures (`dma_4016_read`, `5-branch_delays_irq`) now pass.

`holy_mapperel` runs to the title/menu path but produces a uniform frame in
the headless probe, so it needs an input script or a screenshot oracle.

### Mapper suite matrix (2026-11-15)

`cargo run --release -p unko-nes-dev --example mapper_suite -- <rustynes/tests/roms>`
runs every local mapper-focused suite and records the oracle result:

| Suite | Mapper(s) | Oracle | Result |
|---|---|---|---|
| Holy Mapperel (19 + 23 variants) | 0,1,2,3,4,7,9,10,11,28,34,66,69,78,118,180 | result line `0000` | **42/42** |
| blargg `mmc3_test` | 4 | screen/`$6000` text | **6/6** |
| blargg `mmc3_test_2` | 4 | screen/`$6000` text | **6/6** (incl. `6-MMC3_alt`) |
| blargg `mmc3_irq_tests` | 4 | screen text | **6/6** (rev A uses the alternate revision) |
| MMC5 `mmc5test`/`mmc5exram` | 5 | visual smoke (2-15 colours) | **3/3** |
| VRC2a CHR banking (`m22`) | 22 | visual smoke (4 colours) | **1/1** |
| MMC1 A12 (`mmc1_a12`) | 1 | visual smoke (3 colours) | **1/1** |

**65/65 cases pass, 0 failed, 0 without a verdict.** The per-mapper aggregate
is 0:3, 1:15, 2:2, 3:1, 4:22, 5:3, 7:2, 9:1, 10:2, 11:2, 22:1, 28:2, 34:2,
66:1, 69:2, 78:1, 118:1, 180:2 (mapper 4 includes both the blargg rows and
four Holy Mapperel boards). The runner skips missing suite directories, so a
partial local drop is safe.

### Commercial FDS images

`refs/Minerva_Myrient/No-Intro/Nintendo - Family Computer Disk System (FDS)`
and the `(QD)` folder contain FDS titles, including *Akumajou Dracula*.
The core, CLI (`--bios`) and desktop app can boot them; the desktop pulls the
BIOS from Settings → Paths and persists disk writes.

## NESdev archive (`christopherpow/nes-test-roms`, 263 `.nes`)

Downloaded to a temporary directory and run with:

```sh
cargo run --release -p unko-nes --example testrom -- <rom> 1200
```

`testrom` polls the cartridge work-RAM text at `$6000`, then extracts ASCII
rows from the nametables, and stops on `Passed`/`Failed`. Many NESdev tests
only report by audio, so a missing result is not a failure. `UNKO_REGION=pal`
forces PAL timing and `UNKO_MMC3_ALT=1` selects the alternate MMC3 revision.

Latest run (2026-11-15; the 2026-11-14 run plus the `RTS`/`RTI` dummy-fetch
fix, which moves `cpu_exec_space_ppuio` to pass):

- **137 passes** and **11 failures** in the default NTSC run. The seven PAL APU
  failures all pass with PAL timing selected, leaving **4** failures:
  - three MMC3 revision rows, which pass with the alternate selector,
  - `read_joy3/count_errors`, which needs a controller script.
- **Revision-dependent passes:** `mmc3_irq_tests/5.MMC3_rev_A`,
  `mmc3_test/6-MMC6` and `mmc3_test_2/6-MMC3_alt` pass when the alternate
  MMC3A/NEC revision is selected (`UNKO_MMC3_ALT=1` or Settings -> Advanced).
  Without it they count as the three revision failures.
- **Input-dependent:** `read_joy3/thorough_test` needs a controller script;
  the power-on part of `cpu_reset/registers` now passes and the second part
  waits for an injected reset.
- **115 without a machine-readable result**: visual, audio, input-driven or
  screen-only tests. They load and run without panic; verdicts need a
  screenshot/audio oracle or a controller script.

The MMC3 A12, APU frame-window, DMC-alias and branch-poll fixes (see
`docs/accuracy-status.md`) clear `1-clocking`, `2-details`, `3-A12_clocking`,
`5-MMC3`, `6-MMC3_alt`, `6-MMC6`, the line-239 one-dot boundary, the seven
PAL APU singles in PAL mode, `dma_4016_read` and `5-branch_delays_irq`.

## Still needed

### BIOS / firmware

| Asset | Needed for | Notes |
|---|---|---|
| `disksys.rom` (FDS BIOS) | `fdsirqtests`, FDS mirroring/`$4023`/audio/`$4030` tests, every `.fds` title | Present locally at `refs/bios/disksys.rom` (two revisions) and in the Myrient BIOS archive; it is **not** in `test-roms/`, and the desktop app reads it from Settings → Paths |
| Vs. System ROMs | `characterize-vs`, VS compatibility | 12 local images staged and booting; **Vs. Gumshoe**, a 2C05 title and a known-good DualSystem dump are still missing. No VS BIOS is required for these boards. |
| PlayChoice-10 ROMs | PC10 compatibility | BIOS present; raw MAME game sets staged. iNES PC10 conversion, INST/PROM packaging and the second-CPU/input layer are still missing. |
| Miracle Piano firmware | MIDI replies | Not required: SysEx 4/11 replies are synthesized from the documented message set |
| Datach external X24C01 dump (Battle Rush) | external EEPROM | Not required: the 128-byte X24C01 is modelled from the Xicor datasheet |

### NESdev suites not yet run to a verdict

- **FDS**: `FdsIrqTests` **21/21 pass**, `FDS-Mirroring-Test` **3/3 pass**,
  `FDS-4023-Test` and `FDS-Audio-Registers` are display-only dumps,
  `FDS-4030D1-Addr` renders `67FF`/`6E25`, exactly matching the hardware
  capture. The images were fetched from
  their public releases for this run; they are not committed. See the dated
  entry in `docs/accuracy-status.md`.
- **Mapper-specific**: `BNTest`, `bxrom_512k_test`, `Holy Mapperel`,
  `mmc3bigchrram`, `mmc3irqtest`, `mmc5ramsize`, `mmc1atest`, `test28`,
  `vrc24test`, `vrc6test`, `serom`, NES 2.0 submapper tests 2/3/7/34,
  `n163_soundram`, `n163_soundram_init`, `fme7acktest`, `fme7ramtest`.
  The archive covers `exram`, `mmc5test`, `mmc5test_v2`, `MMC1_A12` and
  `m22chrbankingtest`; those load but have no machine-readable result.
  Holy Mapperel: **42/42 variants pass `0000`** (see the dated entry in
  `docs/accuracy-status.md`).
- **Input**: `allpads`, `dma_sync_test_v2`, `PaddleTest3`, `vaus`,
  `vaus-test`, `read_joy3`, `Zap Ruder`, `spadtest-nes`, `mset`, `mict`,
  `Telling LYs?`, `ctrltest`, `raw pack2`, zapper tests,
  `powerpadgesture`, `POWERPAD.NES`, `d34test`, `kmbtest`, `Hori Tracker`.
  The archive has `PaddleTest3` and `read_joy3`; the rest need a controller
  script or a manual pass.
- **Audio**: `apu_mixer`, `apu_reset`, `apu_test` singles, `dmc_tests`,
  `dmc_dma_during_read4`, `dpcmletterbox`, `volume_tests`, `pal_apu_tests`,
  `nes-audio-tests`. These need an audio capture and a reference recording,
  not just a text result.
- **Visual / timing**: `color_test`, `blargg_ppu_tests_2005`,
  `full_palette`, `nmi_sync`, `ntsc_torture`, `oamtest3`, `palette`,
  `ppu_read_buffer`, `scanline`, `sprdma_and_dmc_dma`, `tvpassfail`,
  `240pee`, `NEStress`, `oc-r1a`.
- **CPU**: `branch_timing_tests`, `cpu_dummy_reads`, `cpu_dummy_writes`,
  `cpu_exec_space`, `cpu_flag_concurrency`, `cpu_reset`,
  `cpu_timing_test6`, `instr_test-v5`, `ram_retain`. Several of these are in
  the archive and boot; their verdicts are screen-only.

## Tool

`crates/unko-nes/examples/testrom.rs` is the probe used above. It
accepts a ROM and a frame budget and prints the `$6000` text, `$F0`/`$F8`
completion bytes and the number of distinct final-frame colors.
