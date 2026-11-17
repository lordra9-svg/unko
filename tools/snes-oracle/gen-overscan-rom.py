#!/usr/bin/env python3
"""Generate a minimal LoROM that enables PPU overscan (SETINI bit 2).

The ROM sets up mode 1 with a single 32x32 background whose tile 0 is colour
15, paints CGRAM colour 15 green, enables overscan and turns the display on.
Every active scanline is therefore solid green, so the 240-line overscan area
(lines 1-239) is visible in the frame; row 0 is the top border.

The program is hand-assembled here so the fixture has no external provenance.

Usage: gen-overscan-rom.py [out.sfc]
"""

from __future__ import annotations

import struct
import sys

# 65816, native mode, 8-bit A, 16-bit X/Y.
CODE: list[tuple[str, bytes]] = []


def emit(*parts: tuple[str, bytes]) -> None:
    CODE.extend(parts)


def label() -> int:
    return sum(len(b) for _, b in CODE)


def branch(op: int, target: int) -> bytes:
    # Offset is relative to the byte after the two-byte branch.
    here = label()
    offset = target - (here + 2)
    assert -128 <= offset <= 127, offset
    return bytes([op, offset & 0xFF])


def build() -> bytes:
    # Entry: native mode, 16-bit index, 8-bit accumulator.
    emit(
        ("sei", bytes([0x78])),
        ("clc", bytes([0x18])),
        ("xce", bytes([0xFB])),
        ("rep #$10", bytes([0xC2, 0x10])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("ldx #$1fff", bytes([0xA2, 0xFF, 0x1F])),
        ("txs", bytes([0x9A])),
        # Force blank + full brightness, then configure mode 1 / BG1.
        ("lda #$8f", bytes([0xA9, 0x8F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        ("lda #$01", bytes([0xA9, 0x01])),
        ("sta $2105", bytes([0x8D, 0x05, 0x21])),
        ("lda #$04", bytes([0xA9, 0x04])),
        ("sta $2107", bytes([0x8D, 0x07, 0x21])),
        ("stz $210b", bytes([0x9C, 0x0B, 0x21])),
        ("lda #$01", bytes([0xA9, 0x01])),
        ("sta $212c", bytes([0x8D, 0x2C, 0x21])),
        # CGRAM 0 = red backdrop, CGRAM 15 = green.
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("lda #$1f", bytes([0xA9, 0x1F])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("lda #$00", bytes([0xA9, 0x00])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("lda #$0f", bytes([0xA9, 0x0F])),
        ("sta $2121", bytes([0x8D, 0x21, 0x21])),
        ("lda #$e0", bytes([0xA9, 0xE0])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("lda #$03", bytes([0xA9, 0x03])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        # VRAM increment after the high byte; tile 0 = all colour 15.
        ("lda #$80", bytes([0xA9, 0x80])),
        ("sta $2115", bytes([0x8D, 0x15, 0x21])),
        ("stz $2116", bytes([0x9C, 0x16, 0x21])),
        ("stz $2117", bytes([0x9C, 0x17, 0x21])),
        ("ldx #$0010", bytes([0xA2, 0x10, 0x00])),
    )
    tile_loop = label()
    emit(
        ("lda #$ff", bytes([0xA9, 0xFF])),
        ("sta $2118", bytes([0x8D, 0x18, 0x21])),
        ("sta $2119", bytes([0x8D, 0x19, 0x21])),
        ("dex", bytes([0xCA])),
    )
    emit(("bne tile", branch(0xD0, tile_loop)))
    # Tilemap at word address $0400: 32x32 entries all pointing at tile 0.
    emit(
        ("stz $2116", bytes([0x9C, 0x16, 0x21])),
        ("lda #$04", bytes([0xA9, 0x04])),
        ("sta $2117", bytes([0x8D, 0x17, 0x21])),
        ("ldx #$0400", bytes([0xA2, 0x00, 0x04])),
    )
    map_loop = label()
    emit(
        ("stz $2118", bytes([0x9C, 0x18, 0x21])),
        ("stz $2119", bytes([0x9C, 0x19, 0x21])),
        ("dex", bytes([0xCA])),
    )
    emit(("bne map", branch(0xD0, map_loop)))
    # SETINI: overscan. Display on.
    emit(
        ("lda #$04", bytes([0xA9, 0x04])),
        ("sta $2133", bytes([0x8D, 0x33, 0x21])),
        ("lda #$0f", bytes([0xA9, 0x0F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
    )
    spin = label()
    emit(("bra spin", branch(0x80, spin)))

    rom = bytearray(0x1_0000)
    offset = 0
    for _, chunk in CODE:
        rom[offset : offset + len(chunk)] = chunk
        offset += len(chunk)

    # LoROM header at $7FC0.
    header = 0x7FC0
    rom[header : header + 21] = b"OVERSCAN TEST ROM".ljust(21, b"\x00")
    rom[header + 0x15] = 0x20  # LoROM, slow
    rom[header + 0x16] = 0x00  # ROM only
    rom[header + 0x17] = 0x08  # 256 KiB
    rom[header + 0x18] = 0x00  # no SRAM
    rom[header + 0x19] = 0x00  # Japan (NTSC)
    rom[header + 0x1A] = 0x00
    rom[header + 0x1B] = 0x00
    rom[header + 0x1C : header + 0x1E] = b"\x00\x00"
    rom[header + 0x1E : header + 0x20] = b"\x00\x00"
    rom[header + 0x3C : header + 0x3E] = struct.pack("<H", 0x8000)

    checksum_source = bytearray(rom)
    checksum_source[header + 0x1C : header + 0x20] = b"\x00\x00\x00\x00"
    checksum = sum(checksum_source) & 0xFFFF
    rom[header + 0x1C : header + 0x1E] = struct.pack("<H", (~checksum) & 0xFFFF)
    rom[header + 0x1E : header + 0x20] = struct.pack("<H", checksum)
    return bytes(rom)


def main() -> int:
    out = sys.argv[1] if len(sys.argv) > 1 else "/tmp/opencode/overscan.sfc"
    with open(out, "wb") as f:
        f.write(build())
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
