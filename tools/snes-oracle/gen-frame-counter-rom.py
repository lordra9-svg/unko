#!/usr/bin/env python3
"""Generate a minimal LoROM that displays its own NMI frame counter.

The point of this fixture is to settle a *methodology* question: when our
harness runs `FRAMES=N` and the ares capture harness runs `--frames N`, do the
two dumped frames have the same content, or are they one apart? Every game we
compare is either static or driven by an external oracle, so neither can answer
it; a ROM that changes on screen every single frame can.

Each NMI increments a 16-bit counter and writes its low 10 bits into CGRAM
colour 0 (blue = bits 0-4, green = bits 5-9, red = 0). The main screen is off,
so the whole visible frame is the backdrop, i.e. one flat colour that encodes
the frame index. Decoding it is a single pixel read.

Usage: gen-frame-counter-rom.py [out.sfc]
"""

from __future__ import annotations

import sys

# LoROM bank $00:$8000-$FFFF maps to file offset $0000-$7FFF.
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
        # Force blank while the PPU is configured.
        ("lda #$8f", bytes([0xA9, 0x8F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
        # BG mode 0, no layers: the backdrop alone fills the screen.
        ("stz $2105", bytes([0x9C, 0x05, 0x21])),
        ("stz $212c", bytes([0x9C, 0x2C, 0x21])),
        ("stz $212d", bytes([0x9C, 0x2D, 0x21])),
        ("stz $2130", bytes([0x9C, 0x30, 0x21])),
        ("stz $2131", bytes([0x9C, 0x31, 0x21])),
        # CGRAM colour 0 = black for now.
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("stz $2122", bytes([0x9C, 0x22, 0x21])),
        ("stz $2122", bytes([0x9C, 0x22, 0x21])),
        # NMIs off while the counter is cleared.
        ("stz $4200", bytes([0x9C, 0x00, 0x42])),
        ("rep #$20", bytes([0xC2, 0x20])),
        ("stz $0000", bytes([0x9C, 0x00, 0x00])),
        ("sep #$20", bytes([0xE2, 0x20])),
        # NMIs on, then display on at full brightness.
        ("lda #$80", bytes([0xA9, 0x80])),
        ("sta $4200", bytes([0x8D, 0x00, 0x42])),
        ("lda #$0f", bytes([0xA9, 0x0F])),
        ("sta $2100", bytes([0x8D, 0x00, 0x21])),
    )
    loop = label()
    emit(
        ("wai", bytes([0xCB])),
        ("bra loop", branch(0x80, loop)),
    )

    # NMI: bump the counter, then publish its low 10 bits as the backdrop.
    nmi = label()
    skip = label() + 5  # just past "inc $0001"
    emit(
        ("php", bytes([0x08])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("lda $4210", bytes([0xAD, 0x10, 0x42])),
        ("inc $0000", bytes([0xEE, 0x00, 0x00])),
        ("bne skip", branch(0xD0, skip)),
        ("inc $0001", bytes([0xEE, 0x01, 0x00])),
        ("rep #$20", bytes([0xC2, 0x20])),
        ("lda $0000", bytes([0xAD, 0x00, 0x00])),
        ("and #$03ff", bytes([0x29, 0xFF, 0x03])),
        ("sta $00", bytes([0x85, 0x00])),
        ("sep #$20", bytes([0xE2, 0x20])),
        ("stz $2121", bytes([0x9C, 0x21, 0x21])),
        ("lda $00", bytes([0xA5, 0x00])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("lda $01", bytes([0xA5, 0x01])),
        ("sta $2122", bytes([0x8D, 0x22, 0x21])),
        ("plp", bytes([0x28])),
        ("rti", bytes([0x40])),
    )

    rom = bytearray(0x8000)
    code = b"".join(b for _, b in CODE)
    rom[0x0000 : 0x0000 + len(code)] = code

    # Header at $7FC0 (LoROM, 32 KiB, no SRAM, USA/NTSC).
    title = b"FRAME COUNTER".ljust(21, b" ")
    rom[0x7FC0:0x7FD5] = title
    rom[0x7FD5] = 0x20  # LoROM
    rom[0x7FD6] = 0x00  # no coprocessor
    rom[0x7FD7] = 0x05  # 32 KiB
    rom[0x7FD8] = 0x00  # no SRAM
    rom[0x7FD9] = 0x01  # USA / Canada (NTSC)
    rom[0x7FDA] = 0x33
    rom[0x7FDB] = 0x00
    rom[0x7FDC:0x7FDE] = b"\xff\xff"  # no expansion
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
    vector(0x7FFE, 0x8000)  # emulation IRQ/BRK
    vector(0x7FEE, 0x8000)  # native IRQ
    return bytes(rom)


def main() -> int:
    out = sys.argv[1] if len(sys.argv) > 1 else "frame-counter.sfc"
    rom = build()
    with open(out, "wb") as fp:
        fp.write(rom)
    print(f"wrote {out} ({len(rom)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
