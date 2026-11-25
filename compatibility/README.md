# Unko compatibility tracker

Static, evidence-based compatibility page for the independent Unko cores.
Published by `.github/workflows/pages.yml` at:

<https://lordra9-svg.github.io/unko/>

The page is grouped by system (NES, FDS, SNES, Nintendo 64, Nintendo 64DD).
Every row links to a
prefilled compatibility report based on
`.github/ISSUE_TEMPLATE/compatibility-report.yml`; no ROMs or firmware are
uploaded to the page or to issues.

## Current catalog (2026-11-25)

The catalog was imported from the local Myrient No-Intro tree after it was
copied to `refs/Minerva_Myrient`. It contains **21,983 archives** across five
systems:

| Set | Archives | Format |
|---|---:|---|
| NES Headered | 7,101 | `.nes` |
| NES Headerless | 7,104 | `.nes` / `.unh` |
| Famicom Disk System | 414 | `.fds` |
| Famicom Disk System QD | 304 | `.qd` |
| SNES | 4,642 | `.sfc` / `.smc` |
| Nintendo 64 BigEndian | 1,193 | `.z64` |
| Nintendo 64 ByteSwapped | 1,186 | `.v64` |
| Nintendo 64DD | 39 | `.ndd` / BIOS |

Each folder was audited against its No-Intro DAT. **15,650 archives matched a
DAT game** and use the DAT game name in `title`; the remaining 6,333 keep their
existing Myrient filename. Of the non-canonical archive names, **124 were
renamed** to the DAT name. A further 1,101 target names already existed and
were left untouched rather than overwriting a duplicate dump.

Every supported archive was then run for 30 frames with the selected core.
Headerless NES images, QD images and Nintendo 64DD disks are catalogued but
marked **Unsupported** until that load path exists. See
[`docs/library-scan.md`](../docs/library-scan.md) for the aggregate results.

## Files

| File | Purpose |
|---|---|
| `index.html` | Page shell, filters and status legend. |
| `styles.css` | Dark responsive theme. |
| `app.js` | Renders `games.json` into per-system sections. |
| `games.json` | Source of truth: systems, status definitions and game rows. |

`games.json` is deliberately plain JSON so it can be regenerated from a scan
or edited by hand:

```json
{
  "id": "snes-0123456789ab",
  "system": "snes",
  "title": "Game (USA)",
  "format": "SNES",
  "region": "USA / Canada",
  "revision": "",
  "mapping": "LoRom",
  "status": "runs",
  "frames": 30,
  "evidence": "Automated 30-frame load/run smoke (2026-11-25).",
  "tested": "2026-11-25",
  "sha256": "0123456789abcdef…",
  "issue": 123
}
```

- `system` must match an entry in `systems`.
- `status` must match an entry in `statuses`.
- `sha256` is the content identity used to join the page to scan results.
- `issue` is optional; when present the row links to that GitHub issue.
- `verified` means a reference gate passed. `runs` means only an automated
  load/run smoke completed; it is not a playability claim.
- `unsupported` means the format or required load path is not implemented yet.
- The page renders 200 rows per system initially; use **Show next 200** to
  reveal more, or search/status-filter to narrow the set.

## Accuracy ledger

`accuracy.jsonl` (RustyNES) and `accuracy-ares.jsonl` (ares, wired and
deferred) are append-only, resumable per-ROM oracle ledgers. The page
loads it over `games.json` by SHA-256 and shows the NES geometry match. The
current NES stage uses a local RustyNES trace host, 120 frames, and the
palette-independent geometry hash; the “steady” percentage starts at frame 30.
Audio is recorded but not scored as exact PCM because RustyNES exposes a
DC-blocked frontend stream. ares uses the calibrated local capture patch,
`ARES_SYSTEM=Famicom`, and the same geometry metric; its full pass is deferred
until after SNES. NES RustyNES completed with 6,726 measured / 375 error;
4,889 measured ROMs (72.7%) reach 100% steady-state geometry. See
[`docs/accuracy-campaign.md`](../docs/accuracy-campaign.md).

## How to test a game

System-level commands are shown on the page. For a single SNES image:

```sh
cargo run --release -p unko-cli -- snes "game.sfc" \
  --frames 600 --ipl ipl.rom --frame-out shot.ppm
```

For a batch library:

```sh
cargo run --release -p xtask -- snes-scan \
  --dir roms/snes --frames 300 --batch 5 --csv out.csv
```

The general smoke runner used for this catalog:

```sh
cargo run --release -p xtask -- compat-scan \
  --system snes --format SNES --core snes \
  --input roms/snes --frames 30 --out smoke.jsonl
```

The N64 real-cartridge gate is documented in
[`docs/n64-validation.md`](../docs/n64-validation.md). Firmware (FDS BIOS,
SNES IPL, CX4/DSP-1/ST018) is user-supplied and is never committed.

## Reporting a result

1. Test with a named commit/release.
2. Open **Report a game** on the page, or use the issue form directly.
3. Include the dump SHA-256, region/revision, mapper or coprocessor, frontend,
   and the furthest verified state.
4. Do not attach ROMs, BIOS/firmware images or commercial save states.

Maintainers can move the row's `status` after triage and link the GitHub issue
number in `issue`. The generated page is static; rebuild/redeploy after editing
`games.json` (the Pages workflow triggers on changes under `compatibility/`).
