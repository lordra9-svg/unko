# unko-rom-manager

A standalone DAT auditing and 1G1R library manager. It has no dependency on
`unko-nes`, `unko-core`, the Unko app or the CLI, so it can be embedded in any
Rust project or used through the `unko-app` ROM Manager window.

## Features

- Logiqx XML DAT parsing (No-Intro/MAME-style `datafile`, `game`, `rom`,
  parent/clone ids, categories, size/CRC/MD5/SHA1/status).
- Loose-file, ZIP, 7z and RAR scanning with CRC-32, MD5 and SHA-1.
- iNES headerless matching: a `.nes` file is hashed whole and with its 16-byte
  header removed.
- Complete/partial/missing game status, unknown files and duplicates.
- 1G1R selection with region, language, verified, retail, parent and revision
  preferences.
- Fixdat and CSV reports.
- Read-only **verify/audit** mode: hashes an existing library or rebuilt
  folder against the DAT, prints per-ROM missing/unknown/duplicate detail and
  exits nonzero unless every game is complete.
- Safe copy/rebuild with per-file hash verification and an optional
  `rebuild_with_progress` callback (per-game counters for a frontend).
- Optional verified **move** (`--move`): a loose source file is deleted only
  after the destination copy is written successfully; archive members are
  never removed from their container.
- Deterministic ZIP output (`--zip`): one archive per game, fixed timestamps.
- Persistent hash cache (`--cache FILE`; the frontend shows its entry count,
  size and path, can disable it per scan and can clear it): unchanged files
  and ZIP members are reused by size and modification time instead of being
  read and hashed again.
- DAT download/update over HTTP (`--dat-url URL`): the body is parsed as a
  Logiqx DAT before it atomically replaces the destination.
- TorrentZip-style output (`--torrentzip`): sorted entries, the 1996-12-24
  23:32 timestamp and no extra fields. It is deterministic but not guaranteed
  byte-identical to reference TorrentZip (the deflate implementation differs).

## Usage

```rust
use unko_rom_manager::{audit, OneGameOneRom, RebuildOptions, ScanOptions, rebuild};

let mut progress = |_: &unko_rom_manager::ScanProgress| {};
let report = audit(
    std::path::Path::new("dat/No-Intro.dat"),
    std::path::Path::new("roms"),
    &ScanOptions::default(),
    &mut progress,
)?;
let selected = OneGameOneRom::default().select(&report.dat);
let result = rebuild(
    &report,
    &selected,
    &RebuildOptions { output: "rebuilt".into(), ..RebuildOptions::default() },
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The crate also ships `unko-rom-manager`, a small headless CLI:

```sh
cargo run -p unko-rom-manager -- \
  --dat "No-Intro.dat" --input roms --1g1r \
  --fixdat missing.xml --csv report.csv --copy rebuilt --move --cache scan.tsv

# Read-only audit: exits 1 unless the set is complete
unko-rom-manager verify --dat DAT --input rebuilt --max-list 20
# Download/update a DAT first
cargo run -p unko-rom-manager -- --dat-url https://example.test/nes.dat \
  --dat-out nes.dat --input roms
# --move deletes loose sources after a verified copy (not with --zip)
# Add --zip to write one deterministic archive per game instead of loose files
```

## Current limits

ZIP, 7z and RAR containers can be read; rebuilt sets can be written as
deterministic ZIPs or TorrentZip-style archives (not byte-exact RVZSTD).
CHD input and ROM patching are future work.
The rebuild operation only copies; it never deletes or moves source files.
