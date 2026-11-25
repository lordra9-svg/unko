# Unko compatibility tracker

Static, evidence-based compatibility page for the independent Unko cores.
Published by `.github/workflows/pages.yml` at:

<https://lordra9-svg.github.io/unko/>

The page is grouped by system (NES, SNES, Nintendo 64). Every row links to a
prefilled compatibility report based on
`.github/ISSUE_TEMPLATE/compatibility-report.yml`; no ROMs or firmware are
uploaded to the page or to issues.

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
  "region": "USA / Canada",
  "revision": "",
  "mapping": "LoRom",
  "status": "runs",
  "evidence": "Automated 300-frame load/run sweep (2026-11-25).",
  "tested": "2026-11-25",
  "sha256": "0123456789abcdef…",
  "issue": 123
}
```

- `system` must match an entry in `systems`.
- `status` must match an entry in `statuses`.
- `sha256` is the deduplication key for imported library scans.
- `issue` is optional; when present the row links to that GitHub issue.
- `verified` means a reference gate passed. `runs` means only an automated
  load/run smoke completed; it is not a playability claim.

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
