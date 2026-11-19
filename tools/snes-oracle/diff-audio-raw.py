#!/usr/bin/env python3
"""Bit-exact PCM comparison: our f32 stream vs ares' pre-resampler i16 stream.

Our harness writes `i16 / 32768.0` as f32, which represents every i16 exactly,
so the comparison is integer-exact after multiplying back. ares'
`ARES_RAW_AUDIO` writes interleaved i16 straight from `DSP::sample`.

The two DSP phases can differ by a few samples. Alignment is searched on a loud
probe window in the middle of the stream (the opening seconds are silent and
would match at every offset), then the whole overlap is compared.

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


def aligned(a: array.array, b: array.array, offset: int, start: int, window: int):
    """Compare a[start + offset:] against b[start:], limited to `window`."""
    if offset >= 0:
        left, right = a[start + offset:], b[start:]
    else:
        left, right = a[start:], b[start - offset:]
    n = min(len(left), len(right), window)
    return left, right, n


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("ours")
    parser.add_argument("ares")
    parser.add_argument("--max-offset", type=int, default=256)
    parser.add_argument(
        "--window",
        type=int,
        default=0,
        help="samples to compare (0 = the whole overlap); alignment uses a "
        "40,000-sample probe in the middle of that window",
    )
    args = parser.parse_args()

    ours = load_f32_as_i16(args.ours)
    ares = load_i16(args.ares)
    print(f"ours samples: {len(ours)} ({len(ours) // 2} stereo frames)")
    print(f"ares samples: {len(ares)} ({len(ares) // 2} stereo frames)")

    total_window = args.window or min(len(ours), len(ares))
    probe = min(40_000, total_window // 2, len(ours), len(ares))
    # Pick the loudest probe window: the opening seconds of most games are
    # silent, and silence matches at every offset.
    best_energy, probe_start = -1, 0
    for start in range(0, max(1, total_window - probe), probe):
        energy = sum(abs(value) for value in ares[start:start + probe:64])
        if energy > best_energy:
            best_energy, probe_start = energy, start

    best = (1 << 62, 0)
    for offset in range(-args.max_offset, args.max_offset + 1):
        left, right, n = aligned(ares, ours, offset, probe_start, probe)
        if n < probe // 2:
            continue
        difference = sum(map(ne, left[:n], right[:n]))
        if difference < best[0]:
            best = (difference, offset)

    _, offset = best
    left, right, n = aligned(ares, ours, offset, 0, total_window if args.window else 1 << 62)
    total = sum(map(ne, left[:n], right[:n]))
    print(
        f"offset {offset}: {total} differing samples over {n} "
        f"({n // 2} stereo frames, {100.0 * (n - total) / n:.4f}% identical)"
    )
    return 0 if total == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
