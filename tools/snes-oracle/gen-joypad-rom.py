#!/usr/bin/env python3
"""Generate a LoROM that displays the auto-polled joypad state.

Useful for verifying controller scripting in a capture harness: the frame's
backdrop colour is CGRAM colour 0 whose low byte is `$4218` and high byte is
`$4219` (the auto-polled controller 1 state), refreshed in NMI. Holding a button
therefore changes the frame in a way that can be decoded from a single pixel.

The ROM enables auto-poll ($4200 bit 0) and NMI ($4200 bit 7) and nothing else.

Usage: gen-joypad-rom.py [out.sfc]
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


def build() -> bytes:
    emit(
        ("sei", bytes([0x78])),
        ("clc", bytes([0x18])),
        ("xce", bytes([0xFB])),
        ("rep #$10", bytes([0xC2, 0x10])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("ldx #$1fff", bytes([0xA2, 0xFF, 0x1F])),
        ("txs", bytes([0x9A])),
        ("lda #$8f", bytes([0xA9, 0x8F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        ("stz $2105", bytes([0x9C, 0x05, 0x21])),
        ("stz $212c", bytes([0x9C, 0x2C, 0x21])),
        ("stz $212d", bytes([0x9C, 0x2D, 0x21])),
        ("stz $2130", bytes([0x9C, 0x30, 0x21])),
        ("stz $2131", bytes([0x9C, 0x31, 0x21])),
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("stz $2122", bytes([0x9C, 0x22, 0x21])),
        ("stz $2122", bytes([0x9C, 0x22, 0x21])),
        # Auto-poll + NMI.
        ("lda #$81", bytes([0xA9, 0x81])),
        ("sta $4200", bytes([0x8D, 0x00, 0x42])),
        ("lda #$0f", bytes([0xA9, 0x0F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
    )
    # The main loop samples the auto-poll registers; $4218/$4219 are cleared
    # when polling starts, so they are only valid after it completes.
    # Sample once per frame from the visible period, after the auto-poll has
    # finished: $4218 is cleared when polling starts and holds partial data
    # until it completes.
    wait_visible = label()
    emit(
        ("lda $4212", bytes([0xAD, 0x12, 0x42])),
        ("and #$80", bytes([0x29, 0x80])),
    )
    emit(("bne wait_visible", branch(0xD0, wait_visible)))
    wait_ready = label()
    emit(
        ("lda $4212", bytes([0xAD, 0x12, 0x42])),
        ("and #$01", bytes([0x29, 0x01])),
    )
    emit(("bne wait_ready", branch(0xD0, wait_ready)))
    emit(
        ("lda $4218", bytes([0xAD, 0x18, 0x42])),
        ("sta $10", bytes([0x85, 0x10])),
        ("lda $4219", bytes([0xAD, 0x19, 0x42])),
        ("sta $11", bytes([0x85, 0x11])),
    )
    emit(("bra wait_visible", branch(0x80, wait_visible)))

    # NMI: publish $4218/$4219 as CGRAM colour 0.
    nmi = label()
    emit(
        ("php", bytes([0x08])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("lda $4210", bytes([0xAD, 0x10, 0x42])),
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("lda $10", bytes([0xA5, 0x10])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("lda $11", bytes([0xA5, 0x11])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("plp", bytes([0x28])),
        ("rti", bytes([0x40])),
    )
    irq = label()
    emit(("rti", bytes([0x40])))

    rom = bytearray(0x8000)
    code = b"".join(b for _, b in CODE)
    rom[0x0000 : 0x0000 + len(code)] = code

    rom[0x7FC0:0x7FD5] = b" " * 21
    rom[0x7FC0:0x7FCB] = b"JOYPAD TEST"
    rom[0x7FD5] = 0x20
    rom[0x7FD6] = 0x00
    rom[0x7FD7] = 0x05
    rom[0x7FD8] = 0x00
    rom[0x7FD9] = 0x01
    rom[0x7FDA] = 0x33
    rom[0x7FDB] = 0x00
    rom[0x7FDC:0x7FDE] = b"\x00\x00"
    rom[0x7FDE:0x7FE0] = b"\xff\xff"

    def vector(offset: int, value: int) -> None:
        rom[offset] = value & 0xFF
        rom[offset + 1] = (value >> 8) & 0xFF

    vector(0x7FEA, 0x8000 + nmi)
    vector(0x7FFA, 0x8000 + nmi)
    vector(0x7FFC, 0x8000)
    vector(0x7FFE, 0x8000 + irq)
    vector(0x7FEE, 0x8000 + irq)
    return bytes(rom)


def main() -> int:
    out = sys.argv[1] if len(sys.argv) > 1 else "joypad.sfc"
    with open(out, "wb") as fp:
        fp.write(build())
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
