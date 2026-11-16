#!/usr/bin/env python3
"""Repair iNES headers for known Vs. System dumps and stage them locally.

The source dumps are not committed. Point this script at a directory of
`.nes` files (extract `VS NES Roms.rar` first) and it writes NES 2.0 images
with the Vs. console type, four-screen mirroring, the cabinet type and the
per-game PPU model. The mapper and PRG/CHR sizes are preserved from each
dump's existing header.

Known PPU assignments and cabinet types are from the public NESdev Vs. System
and PPU-palette pages and MAME's Vs. board definitions. No ROM data is
included in this repository.

Usage:
    unrar x "/path/VS NES Roms.rar" /tmp/vs-roms/
    scripts/repair_vs_headers.py /tmp/vs-roms -o test-roms/vs
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

# filename -> (PPU nibble, cabinet nibble, optional mapper override)
# PPU: 0 = 2C03, 2..5 = 2C04A..D, 8..12 = 2C05A..E.
# Cabinet: 0 = UniSystem, 1 = DualSystem, 2 = Raid on Bungeling Bay.
KNOWN = {
    "VS Castlevania.nes": (3, 0, None),  # 2C04-0002
    "VS Clu Clu Land.nes": (5, 0, None),  # 2C04-0004
    "VS Duck Hunt Hack.nes": (0, 0, None),  # 2C03
    "VS Excitebike Hack.nes": (4, 0, None),  # 2C04-0003
    "VS Excitebike.nes": (4, 0, None),  # 2C04-0003
    "VS Gradius Hack.nes": (2, 0, None),  # 2C04-0001
    "VS Ice Climber Hack.nes": (5, 0, None),  # 2C04-0004
    "VS Pinball Hack.nes": (2, 0, None),  # 2C04-0001
    "VS Super Mario Bros Version MAME V1.1A.nes": (5, 0, None),  # 2C04-0004
    "VS TKO Boxing Hack.nes": (4, 0, None),  # 2C04-0003
    "VS The Goonies Hack.nes": (4, 0, 75),  # 2C04-0003; VRC1 (mapper 151 in the dump)
    # Headerless hack: prepend the Vs. Castlevania header (mapper 2).
    "VS Castlevania Hack.nes": (3, 0, 2),
}

# Output names stay recognisable but mark payloads that are known hacks.
OUTPUT_NAMES = {
    "VS Castlevania Hack.nes": "Vs. Castlevania (hack).nes",
    "VS Duck Hunt Hack.nes": "Vs. Duck Hunt (hack).nes",
    "VS Excitebike Hack.nes": "Vs. Excitebike (hack).nes",
    "VS Gradius Hack.nes": "Vs. Gradius (hack).nes",
    "VS Ice Climber Hack.nes": "Vs. Ice Climber (hack).nes",
    "VS Pinball Hack.nes": "Vs. Pinball (hack).nes",
    "VS TKO Boxing Hack.nes": "Vs. TKO Boxing (hack).nes",
    "VS The Goonies Hack.nes": "Vs. The Goonies (hack).nes",
}


def repaired(data: bytes, ppu: int, cabinet: int, mapper_override: int | None) -> bytes:
    if len(data) < 16:
        raise ValueError("file is too short for an iNES header")
    if data[:4] == b"NES\x1a":
        flags6, flags7 = data[6], data[7]
        mapper = (flags6 >> 4) | (flags7 & 0xF0)
        if flags7 & 0x0C == 0x08:
            mapper |= (data[8] & 0x0F) << 8
        body = data[16:]
    else:
        # Headerless dump: use the requested mapper and infer sizes below.
        # The known Castlevania hack carries a 16-byte junk prefix.
        mapper = mapper_override if mapper_override is not None else 0
        body = data[16:] if len(data) % 16384 == 16 else data
    if mapper_override is not None:
        mapper = mapper_override
    if mapper > 0xFF:
        raise ValueError("this repair only handles mappers 0-255")

    prg = data[4] if data[:4] == b"NES\x1a" else (len(body) // 16384)
    chr_ = data[5] if data[:4] == b"NES\x1a" else 0
    header = bytearray(16)
    header[0:4] = b"NES\x1a"
    header[4] = prg
    header[5] = chr_
    # Four-screen mirroring is fixed on the Vs. board. NES 2.0 flag and the
    # Vs. console type live in flags-7.
    header[6] = ((mapper & 0x0F) << 4) | 0x08
    header[7] = (mapper & 0xF0) | 0x08 | 0x01
    header[8] = (mapper >> 8) & 0x0F
    header[9] = 0  # PRG/CHR size high nibbles
    header[10] = 0  # PRG-NVRAM shift
    header[11] = 0  # CHR-NVRAM shift
    header[12] = 0  # NTSC
    header[13] = ((cabinet & 0x0F) << 4) | (ppu & 0x0F)
    return bytes(header) + body


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="directory containing VS .nes files")
    parser.add_argument(
        "-o", "--output", type=Path, default=Path("test-roms/vs"), help="output directory"
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="also stage files without a known PPU assignment (default PPU 2C03)",
    )
    args = parser.parse_args()

    files = sorted(args.source.rglob("*.nes"))
    if not files:
        print(f"no .nes files under {args.source}", file=sys.stderr)
        return 1
    args.output.mkdir(parents=True, exist_ok=True)

    staged = 0
    for path in files:
        spec = KNOWN.get(path.name)
        if spec is None and not args.all:
            print(f"skip (unknown PPU): {path.name}")
            continue
        ppu, cabinet, mapper_override = spec if spec is not None else (0, 0, None)
        try:
            image = repaired(path.read_bytes(), ppu, cabinet, mapper_override)
        except ValueError as err:
            print(f"skip {path.name}: {err}", file=sys.stderr)
            continue
        name = OUTPUT_NAMES.get(path.name, path.name)
        target = args.output / name
        target.write_bytes(image)
        print(f"staged {target} (PPU {ppu}, cabinet {cabinet})")
        staged += 1
    print(f"{staged} file(s) staged in {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
