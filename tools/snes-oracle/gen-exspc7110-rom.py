#!/usr/bin/env python3
"""Generate a first-party 7 MiB EXSPC7110 test ROM.

There is no 7 MiB EXSPC7110 image in the local test set, so this fixture makes
the ``$40-$4F:$0000-$FFFF`` expansion window directly observable: it reads a
handful of expansion addresses and copies them into work RAM at ``$7E:1000``,
where both cores can dump them.

The image is a 7 MiB `EXSPC7110-RAM-EPSONRTC` image (1 MiB program, 5 MiB data,
1 MiB expansion). SPC7110 boards expose the 1 MiB program as a linear window
(`$00-$0F:$8000-$FFFF` plus `$C0-$CF:$0000-$FFFF`), so the header and vectors
live at the end of the program, exactly like Momotarou Dentetsu Happy:

    file 0xFFD5 = 0x3A   map mode: SPC7110
    file 0xFFD6 = 0xF9   cartridge type: SPC7110 with Epson RTC
    file 0xFFD8 = 0x05   save RAM: 32 KiB
    file 0xFFDA = 0x33   extended header present
    file 0xFFFC = 0x8000 reset vector (program-relative)

Expansion byte at offset ``e`` is ``(((e >> 16) << 4) | (e & 0x0F)) ^ 0xA5``, so
the high nibble is the 64 KiB bank and the low nibble the offset within it.

Usage: gen-exspc7110-rom.py OUT.sfc
"""

import sys

IMAGE_SIZE = 0x700000
PROM_SIZE = 0x100000
DROM_SIZE = 0x500000
EXPANSION_BASE = PROM_SIZE + DROM_SIZE

# (name, 24-bit expansion address) in the order they are stored to $7E:1000.
PROBES = [
    ("$40:0000", 0x400000),
    ("$40:0001", 0x400001),
    ("$40:000F", 0x40000F),
    ("$41:0000", 0x410000),
    ("$42:1234", 0x421234),
    ("$45:8000", 0x458000),
    ("$48:0000", 0x480000),
    ("$4A:0100", 0x4A0100),
    ("$4D:ABCD", 0x4DABCD),
    ("$4E:0000", 0x4E0000),
    ("$4F:0000", 0x4F0000),
    ("$4F:FFFE", 0x4FFFFE),
    ("$4F:FFFF", 0x4FFFFF),
]


def expansion_byte(offset):
    e = offset - EXPANSION_BASE
    return ((((e >> 16) & 0xF) << 4) | (e & 0x0F)) ^ 0xA5


def build():
    rom = bytearray(IMAGE_SIZE)

    # Program: distinguishable from the data ROM, but never read by the test.
    for i in range(PROM_SIZE):
        rom[i] = (i * 7 + 0x11) & 0xFF
    # Data ROM: another pattern, also not read by the test.
    for i in range(PROM_SIZE, EXPANSION_BASE):
        rom[i] = (i * 13 + 0x5C) & 0xFF
    # Expansion ROM: bank in the high nibble, offset in the low nibble.
    for i in range(EXPANSION_BASE, IMAGE_SIZE):
        rom[i] = expansion_byte(i)

    code = bytearray()
    code += bytes([0x78])              # sei
    code += bytes([0x18])              # clc
    code += bytes([0xFB])              # xce
    code += bytes([0xE2, 0x30])        # sep #$30  (8-bit A/X/Y)
    code += bytes([0xA2, 0x00])        # ldx #$00
    for _, address in PROBES:
        code += bytes([0xAF]) + address.to_bytes(3, "little")  # lda.l addr
        code += bytes([0x9F, 0x00, 0x10, 0x7E])                # sta $7E1000,x
        code += bytes([0xE8])                                  # inx
    code += bytes([0x80, 0xFE])        # bra *

    # The SPC7110 MCU window is linear: `$00:8000` is program offset 0x8000.
    rom[0x8000:0x8000 + len(code)] = code

    # Header at the end of the program ROM (`headerAddress` 0xFFB0).
    title = b"EXSPC7110 EXPANSION".ljust(21, b" ")
    rom[0xFFC0:0xFFC0 + len(title)] = title
    rom[0xFFD5] = 0x3A   # map mode: SPC7110
    rom[0xFFD6] = 0xF9   # SPC7110 + Epson RTC
    rom[0xFFD7] = 0x0D   # nominal ROM size
    rom[0xFFD8] = 0x05   # 32 KiB save RAM
    rom[0xFFD9] = 0x01   # USA
    rom[0xFFDA] = 0x33   # extended header present
    rom[0xFFDB] = 0x00   # version
    rom[0xFFBF] = 0x00   # cartridge subtype

    # Vectors: RESET (and the rest) point at the entry point (program 0x8000).
    for vector in range(0xFFE4, 0x10000, 2):
        rom[vector] = 0x00
        rom[vector + 1] = 0x80
    rom[0xFFFC] = 0x00
    rom[0xFFFD] = 0x80

    # Checksum: canonical "complement + checksum == 0xffff" pair.
    rom[0xFFDC:0xFFE0] = b"\xff\xff\x00\x00"
    checksum = sum(rom) & 0xFFFF
    rom[0xFFDC] = (checksum ^ 0xFFFF) & 0xFF
    rom[0xFFDD] = (checksum ^ 0xFFFF) >> 8
    rom[0xFFDE] = checksum & 0xFF
    rom[0xFFDF] = checksum >> 8
    return bytes(rom)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: gen-exspc7110-rom.py OUT.sfc")
    rom = build()
    with open(sys.argv[1], "wb") as handle:
        handle.write(rom)
    print(f"wrote {sys.argv[1]} ({len(rom)} bytes, checksum ${sum(rom) & 0xFFFF:04X})")
    print(f"probes -> $7E:1000..${0x1000 + len(PROBES) - 1:04X}")
    for index, (name, address) in enumerate(PROBES):
        print(f"  {0x1000 + index:04X}  {name} = ${expansion_byte(address):02X}")


if __name__ == "__main__":
    main()
