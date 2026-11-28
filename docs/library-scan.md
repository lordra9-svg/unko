# No-Intro library import and smoke sweep (2026-11-25)

The local Myrient No-Intro tree was copied from
`~/Downloads/Minerva_Myrient` to `~/ai/refs/Minerva_Myrient` with `rsync -a`
(21,983 archives, 37 GiB source; the existing refs tree was merged, not replaced).

## DAT audit

Every system folder was audited against the No-Intro DAT in
`rom dat/` with `unko-rom-manager`:

```sh
unko-rom-manager --dat DAT --input FOLDER --csv AUDIT.csv
```

| Set | DAT games | Complete | Partial | Missing | ROMs matched | Unknown files | Duplicate files |
|---|---:|---:|---:|---:|---:|---:|---:|
| NES Headered | 4,569 | 4,405 | 2 | 162 | 4,407/4,571 | 1,966 | 728 |
| NES Headerless | 4,571 | 4,452 | 1 | 118 | 4,454/4,573 | 1,890 | 761 |
| FDS | 406 | 403 | 0 | 3 | 403/406 | 2 | 9 |
| FDS QD | 295 | 7 | 0 | 288 | 7/295 | 0 | 2 |
| SNES | 4,143 | 4,089 | 0 | 54 | 4,089/4,143 | 133 | 420 |
| N64 BigEndian | 1,158 | 1,141 | 0 | 17 | 1,141/1,158 | 34 | 18 |
| N64 ByteSwapped | 1,158 | 1,137 | 0 | 21 | 1,137/1,158 | 34 | 15 |
| N64DD | 35 | 13 | 0 | 22 | 13/35 | 0 | 3 |

The DAT covers clean No-Intro dumps; the unknown-file counts include newer
revisions, aftermarket/pirate releases, BIOS images and duplicate content that
the installed DAT version does not name.

## Archive naming

- **15,650 archives matched a DAT game** and use the DAT game name in the
  compatibility page title.
- **124 archives were renamed on disk** where the DAT name was free.
- **1,101 target names already existed**; those duplicate dumps were left
  untouched instead of overwriting an existing file.
- **6,333 archives had no DAT title** and keep their existing Myrient filename.

The compatibility catalog stores only the display title, SHA-256 and smoke
result; no ROM path, ROM data, firmware or BIOS is committed.

## Smoke sweep

Every archive was then loaded and run for **30 frames** with
`xtask compat-scan`, sharded across 16 concurrent processes. The page shows
the per-game result.

| Result | Count |
|---|---:|
| Runs (smoke) | 14,200 |
| Verified reference titles | 7 |
| Nothing (load/run failed) | 329 |
| Unsupported | 7,447 |
| Total archives | 21,983 |

Per set:

| Set | Runs | Nothing | Unsupported |
|---|---:|---:|---:|
| NES Headered | 7,073 | 28 | 0 |
| NES Headerless | 0 | 0 | 7,104 |
| FDS | 414 | 0 | 0 |
| FDS QD | 0 | 0 | 304 |
| SNES | 4,472 | 163 | 0 |
| N64 BigEndian | 1,124 | 69 | 0 |
| N64 ByteSwapped | 1,117 | 69 | 0 |
| N64DD | 0 | 0 | 39 |

Headerless NES images, QD images and Nintendo 64DD disks are catalogued as
**Unsupported**: the current frontend has no headerless-iNES loader, QD reader
or 64DD disk path. They were still hashed and listed so the page has an entry
for every archive.

See [`compatibility/README.md`](../compatibility/README.md) for the catalog
schema and page workflow.
