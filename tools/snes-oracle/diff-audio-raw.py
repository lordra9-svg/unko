#!/usr/bin/env python3
"""Bit-exact PCM comparison: our f32 stream vs ares' pre-resampler i16 stream.

Our harness writes `i16 / 32768.0` as f32, which represents every i16 exactly,
so the comparison is integer-exact after multiplying back. ares'
`ARES_RAW_AUDIO` writes interleaved i16 straight from `DSP::sample`.

The two DSP phases can differ by a few samples, so a small offset search picks
the alignment with the fewest differing samples, then the full overlap is
compared.

Usage: diff-audio-raw.py ours.f32 ares.i16 [--max-offset N] [--window N]
"""
from __future__ import annotations

import array
import argparse
from operator import ne


def load_f32_as_i16(path: str) -> array.array:
    samples = array.array("f")
    samples.frombytes(open(path, "rb").read())
    return array.array("h", (int(round(value * 32768.0)) for value in samples))


def load_i16(path: str) -> array.array:
    samples = array.array("h")
    samples.frombytes(open(path, "rb").read())
    return samples


def aligned(a: array.array, b: array.array, offset: int, window: int | None):
    if offset >= 0:
        left, right = a[offset:], b
    else:
        left, right = a, b[-offset:]
    n = min(len(left), len(right))
    if window is not None:
        n = min(n, window)
    return left, right, n


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("ours")
    parser.add_argument("ares")
    parser.add_argument("--max-offset", type=int, default=2048)
    parser.add_argument("--window", type=int, default=20000)
    args = parser.parse_args()

    ours = load_f32_as_i16(args.ours)
    ares = load_i16(args.ares)
    print(f"ours samples: {len(ours)} ({len(ours) // 2} stereo frames)")
    print(f"ares samples: {len(ares)} ({len(ares) // 2} stereo frames)")

    best = (1 << 62, 0)
    for offset in range(-args.max_offset, args.max_offset + 1):
        left, right, n = aligned(ares, ours, offset, args.window)
        if n < args.window:
            continue
        difference = sum(map(ne, left[:n], right[:n]))
        if difference < best[0]:
            best = (difference, offset)

    difference, offset = best
    if difference:
        print(f"no alignment is bit-exact; best offset {offset} "
              f"({difference} differing samples in the probe window)")
        return 1

    left, right, n = aligned(ares, ours, offset, None)
    total = sum(map(ne, left[:n], right[:n]))
    print(f"offset {offset}: {total} differing samples over {n} "
          f"({n // 2} stereo frames)")
    return 0 if total == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
