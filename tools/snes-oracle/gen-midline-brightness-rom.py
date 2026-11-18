#!/usr/bin/env python3
"""Generate a LoROM whose brightness changes mid-frame at scanline 100.

The screen is a solid backdrop (CGRAM colour 0 = white). The main loop polls
OPVCT (`$213E`) until the raster reaches line 100, then writes `$2100 = $08`
(dim); the NMI handler restores `$2100 = $0F` (bright) every vblank. The frame
therefore shows a *bright/dim boundary* at the scanline where the write lands.

That boundary is a direct probe of the renderer's sampling point: if two
emulators sample the register at different lines the boundary row differs, and
if they sample identically the row is the same. It cannot be answered by a
frame counter that only updates in vblank.

Usage: gen-midline-brightness-rom.py [out.sfc] [line]
"""

from __future__ import annotations

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


def build(line: int) -> bytes:
    # Entry: native mode, 16-bit index, 8-bit accumulator.
    emit(
        ("sei", bytes([0x78])),
        ("clc", bytes([0x18])),
        ("xce", bytes([0xFB])),
        ("rep #$10", bytes([0xC2, 0x10])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("ldx #$1fff", bytes([0xA2, 0xFF, 0x1F])),
        ("txs", bytes([0x9A])),
        # Force blank while configuring.
        ("lda #$8f", bytes([0xA9, 0x8F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        # Mode 0, no layers: the backdrop fills the screen.
        ("stz $2105", bytes([0x9C, 0x05, 0x21])),
        ("stz $212c", bytes([0x9C, 0x2C, 0x21])),
        ("stz $212d", bytes([0x9C, 0x2D, 0x21])),
        ("stz $2130", bytes([0x9C, 0x30, 0x21])),
        ("stz $2131", bytes([0x9C, 0x31, 0x21])),
        # CGRAM colour 0 = white.
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("lda #$ff", bytes([0xA9, 0xFF])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        # Display on at full brightness, then NMIs on.
        ("stz $4200", bytes([0x9C, 0x00, 0x42])),
        ("lda #$0f", bytes([0xA9, 0x0F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        # Vertical IRQ at the chosen line, plus NMI.
        ("lda #line", bytes([0xA9, line & 0xFF])),
        ("sta $4209", bytes([0x8D, 0x09, 0x42])),
        ("lda #line_hi", bytes([0xA9, (line >> 8) & 0xFF])),
        ("sta $420a", bytes([0x8D, 0x0A, 0x42])),
        ("lda #$A0", bytes([0xA9, 0xA0])),
        ("sta $4200", bytes([0x8D, 0x00, 0x42])),
        ("cli", bytes([0x58])),
    )
    # Main loop does nothing; the IRQ handler dims the screen mid-frame.
    main = label()
    emit(("bra main", branch(0x80, main)))

    # IRQ: acknowledge and dim for the rest of the visible area.
    irq = label()
    emit(
        ("php", bytes([0x08])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("lda $4211", bytes([0xAD, 0x11, 0x42])),
        ("lda #$08", bytes([0xA9, 0x08])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        ("plp", bytes([0x28])),
        ("rti", bytes([0x40])),
    )

    # NMI: restore full brightness every vblank.
    nmi = label()
    emit(
        ("php", bytes([0x08])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("lda $4210", bytes([0xAD, 0x10, 0x42])),
        ("lda #$0f", bytes([0xA9, 0x0F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        ("plp", bytes([0x28])),
        ("rti", bytes([0x40])),
    )

    rom = bytearray(0x8000)
    code = b"".join(b for _, b in CODE)
    rom[0x0000 : 0x0000 + len(code)] = code

    title = b"MIDLINE BRIGHT".ljust(21, b" ")
    rom[0x7FC0:0x7FD5] = title
    rom[0x7FD5] = 0x20  # LoROM
    rom[0x7FD6] = 0x00
    rom[0x7FD7] = 0x05  # 32 KiB
    rom[0x7FD8] = 0x00
    rom[0x7FD9] = 0x01  # NTSC
    rom[0x7FDA] = 0x33
    rom[0x7FDB] = 0x00
    rom[0x7FDC:0x7FDE] = b"\xff\xff"
    checksum = 0
    rom[0x7FDE:0x7FE0] = b"\x00\x00"
    for byte in rom:
        checksum = (checksum + byte) & 0xFFFF
    rom[0x7FDE:0x7FE0] = bytes([checksum & 0xFF, checksum >> 8])

    def vector(offset: int, value: int) -> None:
        rom[offset] = value & 0xFF
        rom[offset + 1] = (value >> 8) & 0xFF

    vector(0x7FEA, 0x8000 + nmi)  # native NMI
    vector(0x7FFA, 0x8000 + nmi)  # emulation NMI
    vector(0x7FFC, 0x8000)  # reset
    vector(0x7FFE, 0x8000 + irq)  # emulation IRQ/BRK
    vector(0x7FEE, 0x8000 + irq)  # native IRQ
    return bytes(rom)


def main() -> int:
    out = sys.argv[1] if len(sys.argv) > 1 else "midline-brightness.sfc"
    line = int(sys.argv[2], 0) if len(sys.argv) > 2 else 100
    rom = build(line)
    with open(out, "wb") as fp:
        fp.write(rom)
    print(f"wrote {out} ({len(rom)} bytes, boundary at line {line})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
