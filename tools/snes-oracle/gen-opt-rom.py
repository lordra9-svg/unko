#!/usr/bin/env python3
"""Generate a minimal LoROM that exercises Mode 2 offset-per-tile.

BG1 shows three 8-pixel tiles (colours 1 / 2 / 1 at columns 0/1/2) with no
scroll. BG3's tilemap supplies the per-tile offsets:

  BG3 word 0 = $0000        (valid bit clear)
  BG3 word 1 = $2008        (valid for BG1, horizontal offset +8)

The interesting question is which BG3 word BG1's *second* tile uses. ares
looks the word up one tile behind (`offsetX - tileWidth`); the straightforward
"same tile" reading would use BG3 word 1 and shift column 1 onto tile 2. The
frame therefore reads:

  column 0 = red    (first column is exempt from offset-per-tile)
  column 1 = green  if the lookup is one tile behind (ares/FPGA pipeline)
             red    if it uses BG3 word 1 (shift onto tile 2)
  column 2 = red

Hand-assembled here, so the fixture has no external provenance.

Usage: gen-opt-rom.py [out.sfc]
"""

from __future__ import annotations

import struct
import sys

CODE: list[tuple[str, bytes]] = []


def emit(*parts: tuple[str, bytes]) -> None:
    CODE.extend(parts)


def label() -> int:
    return sum(len(b) for _, b in CODE)


def branch(op: int, target: int) -> bytes:
    here = label()
    offset = target - (here + 2)
    assert -128 <= offset <= 127, offset
    return bytes([op, offset & 0xFF])


def write_word(address: int, value: int) -> None:
    """Set the VRAM word address and write one word through $2118/$2119."""
    emit(
        (f"vram addr {address:04X}", bytes([0xA9, address & 0xFF, 0x8D, 0x16, 0x21])),
        (f"vram addr {address:04X} hi", bytes([0xA9, address >> 8, 0x8D, 0x17, 0x21])),
        (f"vram data {value:04X}", bytes([0xA9, value & 0xFF, 0x8D, 0x18, 0x21])),
        (f"vram data {value:04X} hi", bytes([0xA9, value >> 8, 0x8D, 0x19, 0x21])),
    )


def build() -> bytes:
    emit(
        ("sei", bytes([0x78])),
        ("clc", bytes([0x18])),
        ("xce", bytes([0xFB])),
        ("rep #$10", bytes([0xC2, 0x10])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("ldx #$1fff", bytes([0xA2, 0xFF, 0x1F])),
        ("txs", bytes([0x9A])),
        # force blank, mode 2, BG1 map $0400, BG1 chars $0000, BG3 map $0800
        ("lda #$8f", bytes([0xA9, 0x8F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        ("lda #$02", bytes([0xA9, 0x02])),
        ("sta $2105", bytes([0x8D, 0x05, 0x21])),
        ("lda #$04", bytes([0xA9, 0x04])),
        ("sta $2107", bytes([0x8D, 0x07, 0x21])),
        ("stz $210b", bytes([0x9C, 0x0B, 0x21])),
        ("lda #$08", bytes([0xA9, 0x08])),
        ("sta $2109", bytes([0x8D, 0x09, 0x21])),
        # no scroll
        ("stz $210d", bytes([0x9C, 0x0D, 0x21])),
        ("stz $210d", bytes([0x9C, 0x0D, 0x21])),
        ("stz $210e", bytes([0x9C, 0x0E, 0x21])),
        ("stz $210e", bytes([0x9C, 0x0E, 0x21])),
        # BG1 on the main screen
        ("lda #$01", bytes([0xA9, 0x01])),
        ("sta $212c", bytes([0x8D, 0x2C, 0x21])),
        # CGRAM 1 = red, 2 = green
        ("lda #$01", bytes([0xA9, 0x01])),
        ("sta $2121", bytes([0x8D, 0x21, 0x21])),
        ("lda #$1f", bytes([0xA9, 0x1F])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("stz $2122", bytes([0x9C, 0x22, 0x21])),
        ("lda #$02", bytes([0xA9, 0x02])),
        ("sta $2121", bytes([0x8D, 0x21, 0x21])),
        ("lda #$e0", bytes([0xA9, 0xE0])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("lda #$03", bytes([0xA9, 0x03])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        # increment VRAM after the high byte
        ("lda #$80", bytes([0xA9, 0x80])),
        ("sta $2115", bytes([0x8D, 0x15, 0x21])),
    )
    # 4bpp characters: a tile is 16 words, row r at tile*16 + r for planes 0/1
    # and tile*16 + 8 + r for planes 2/3 (left zero).
    # tile 0 -> colour 1, tile 1 -> colour 2, tile 2 -> colour 1.
    for tile, word in ((0, 0x00FF), (1, 0xFF00), (2, 0x00FF)):
        for row in range(8):
            write_word(tile * 16 + row, word)
            write_word(tile * 16 + 8 + row, 0x0000)
    # BG1 map at $0400: tiles 0, 1, 2.
    for col, tile in enumerate((0, 1, 2)):
        write_word(0x0400 + col, tile)
    # BG3 offset words: word 0 invalid, word 1 = valid BG1 + offset 8.
    write_word(0x0800 + 0, 0x0000)
    write_word(0x0800 + 1, 0x2008)
    emit(
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

    header = 0x7FC0
    rom[header : header + 21] = b"OPT TEST ROM".ljust(21, b"\x00")
    rom[header + 0x15] = 0x20  # LoROM, slow
    rom[header + 0x16] = 0x00  # ROM only
    rom[header + 0x17] = 0x08  # 256 KiB
    rom[header + 0x18] = 0x00  # no SRAM
    rom[header + 0x19] = 0x00  # Japan (NTSC)
    rom[header + 0x1A] = 0x00
    rom[header + 0x1B] = 0x00
    rom[header + 0x1C : header + 0x20] = b"\x00\x00\x00\x00"
    rom[header + 0x3C : header + 0x3E] = struct.pack("<H", 0x8000)

    checksum_source = bytearray(rom)
    checksum_source[header + 0x1C : header + 0x20] = b"\x00\x00\x00\x00"
    checksum = sum(checksum_source) & 0xFFFF
    rom[header + 0x1C : header + 0x1E] = struct.pack("<H", (~checksum) & 0xFFFF)
    rom[header + 0x1E : header + 0x20] = struct.pack("<H", checksum)
    return bytes(rom)


def main() -> int:
    out = sys.argv[1] if len(sys.argv) > 1 else "/tmp/opencode/opt.sfc"
    with open(out, "wb") as f:
        f.write(build())
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
