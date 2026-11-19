#!/usr/bin/env python3
"""Generate a minimal LoROM that exercises the MSU1 register window.

Phase 1 (first visible frames) sets the backdrop to the two bytes read from
`$2001` after a data seek to zero, so the colour is `0x2211` when the data ROM
maps and reads correctly.

Phase 2 (after about ten frames) sets the backdrop to the `$2000` status byte,
which must be `0x32` (revision 2 + playing + repeat) once track 1 (a valid
`track-1.pcm`) is selected and played with the repeat bit set.

The program is hand-assembled here so the fixture has no external provenance.
Attach `msu1.data.rom` (bytes `11 22 ...`) and `track-1.pcm` next to the ROM.

Usage: gen-msu1-rom.py [out.sfc]
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


def build() -> bytes:
    emit(
        ("sei", bytes([0x78])),
        ("clc", bytes([0x18])),
        ("xce", bytes([0xFB])),
        ("rep #$10", bytes([0xC2, 0x10])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("ldx #$1fff", bytes([0xA2, 0xFF, 0x1F])),
        ("txs", bytes([0x9A])),
        # Force blank at full brightness.
        ("lda #$8f", bytes([0xA9, 0x8F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        # Seek the data ROM to offset 0 (four byte writes, LSB first).
        ("stz $2000", bytes([0x9C, 0x00, 0x20])),
        ("stz $2001", bytes([0x9C, 0x01, 0x20])),
        ("stz $2002", bytes([0x9C, 0x02, 0x20])),
        ("stz $2003", bytes([0x9C, 0x03, 0x20])),
        # CGRAM colour 0 = the first two data bytes (0x2211).
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("lda $2001", bytes([0xAD, 0x01, 0x20])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("lda $2001", bytes([0xAD, 0x01, 0x20])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        # Select track 1 at full volume and start it.
        ("lda #$01", bytes([0xA9, 0x01])),
        ("sta $2004", bytes([0x8D, 0x04, 0x20])),
        ("stz $2005", bytes([0x9C, 0x05, 0x20])),
        ("lda #$ff", bytes([0xA9, 0xFF])),
        ("sta $2006", bytes([0x8D, 0x06, 0x20])),
        ("lda #$03", bytes([0xA9, 0x03])),
        ("sta $2007", bytes([0x8D, 0x07, 0x20])),
        # Display on: the backdrop carries the data-read colour.
        ("lda #$0f", bytes([0xA9, 0x0F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
    )
    # Wait roughly ten frames (4 x 65536 decrements at ~5 clocks each).
    emit(("ldy #$0002", bytes([0xA0, 0x02, 0x00])))
    outer = label()
    emit(("ldx #$0000", bytes([0xA2, 0x00, 0x00])))
    inner = label()
    emit(("dex", bytes([0xCA])))
    emit(("bne inner", branch(0xD0, inner)))
    emit(("dey", bytes([0x88])))
    emit(("bne outer", branch(0xD0, outer)))
    # Phase 2: backdrop = MSU1 status (revision + playing).
    emit(
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("lda $2000", bytes([0xAD, 0x00, 0x20])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("stz $2122", bytes([0x9C, 0x22, 0x21])),
    )
    loop = label()
    emit(("bra loop", branch(0x80, loop)))

    rom = bytearray(0x8000)
    rom[0x7FC0:0x7FC0 + 8] = b"MSU1TEST"
    header = 0x7FC0
    rom[header + 0x15] = 0x20  # LoROM, slow
    rom[header + 0x16] = 0x02  # ROM + RAM + battery
    rom[header + 0x17] = 8     # 256 KiB
    rom[header + 0x18] = 0     # no SRAM
    rom[header + 0x19] = 1     # NTSC
    rom[header + 0x1A] = 1
    rom[header + 0x1C:header + 0x20] = b"\xff\xff\x00\x00"
    program = b"".join(code for _, code in CODE)
    rom[0:len(program)] = program
    rom[0x7FFC:0x7FFE] = struct.pack("<H", 0x8000)
    checksum_source = bytearray(rom)
    checksum_source[header + 0x1C:header + 0x20] = b"\x00\x00\x00\x00"
    checksum = sum(checksum_source) & 0xFFFF
    rom[header + 0x1C:header + 0x1E] = struct.pack("<H", (~checksum) & 0xFFFF)
    rom[header + 0x1E:header + 0x20] = struct.pack("<H", checksum)
    return bytes(rom)


def main() -> None:
    out = sys.argv[1] if len(sys.argv) > 1 else "msu1-test.sfc"
    data = bytes(range(0x11, 0x11 + 64))
    open("msu1.data.rom", "wb").write(data)
    pcm = bytearray(b"MSU1")
    pcm += struct.pack("<I", 0)  # loop sector 0 -> byte 8
    for _ in range(64):
        pcm += struct.pack("<hh", 1000, -1000)
    open("track-1.pcm", "wb").write(pcm)
    open(out, "wb").write(build())
    print(f"wrote {out}, msu1.data.rom, track-1.pcm")
    print(f"program bytes: {sum(len(c) for _, c in CODE)}")


if __name__ == "__main__":
    main()
