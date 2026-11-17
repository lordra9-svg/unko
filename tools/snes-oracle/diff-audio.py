#!/usr/bin/env python3
"""Compare two interleaved stereo f32 PCM streams at the S-DSP rate (32 kHz).

The two DSP phases can differ by a sample, so a small offset search picks the
alignment with the lowest mean absolute error. Reports the first sample above
the tolerance after alignment, plus RMS and peak levels.

Usage: diff-audio.py ours.f32 ares.f32 [--max-offset N] [--tolerance X] [--mono]
"""
from __future__ import annotations

import argparse
import struct
import sys


def load(path: str) -> list[float]:
    data = open(path, "rb").read()
    n = len(data) // 4
    return list(struct.unpack("<%df" % n, data[: 4 * n]))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("ours")
    ap.add_argument("ares")
    ap.add_argument("--max-offset", type=int, default=1024)
    ap.add_argument("--tolerance", type=float, default=1.0 / 32768.0)
    ap.add_argument("--mono", action="store_true")
    ap.add_argument("--window", type=int, default=200_000)
    args = ap.parse_args()

    ours = load(args.ours)
    ares = load(args.ares)
    if args.mono:
        ours = [(ours[i] + ours[i + 1]) * 0.5 for i in range(0, len(ours) - 1, 2)]
        ares = [(ares[i] + ares[i + 1]) * 0.5 for i in range(0, len(ares) - 1, 2)]

    print(f"ours samples: {len(ours) // (1 if args.mono else 2)}")
    print(f"ares samples: {len(ares) // (1 if args.mono else 2)}")

    def score(offset: int) -> tuple[float, int]:
        ai = max(0, offset)
        bi = max(0, -offset)
        n = min(len(ares) - ai, len(ours) - bi, args.window)
        if n <= 0:
            return (float("inf"), 0)
        total = 0.0
        for k in range(n):
            total += abs(ares[ai + k] - ours[bi + k])
        return (total / n, n)

    best = min(range(-args.max_offset, args.max_offset + 1), key=lambda o: score(o)[0])
    best_err, best_n = score(best)
    print(f"best offset: {best} (mean |d| {best_err:.3e} over {best_n})")

    ai = max(0, best)
    bi = max(0, -best)
    n = min(len(ares) - ai, len(ours) - bi)
    first = None
    worst = 0.0
    sq = 0.0
    for k in range(n):
        d = abs(ares[ai + k] - ours[bi + k])
        worst = max(worst, d)
        sq += d * d
        if first is None and d > args.tolerance:
            first = k
    print(f"first > tol: {first}")
    print(f"peak |d|:    {worst:.3e}")
    print(f"rms |d|:     {(sq / n) ** 0.5:.3e}" if n else "rms |d|: n/a")
    print(f"peak level:  ares {max((abs(v) for v in ares), default=0.0):.6f} "
          f"ours {max((abs(v) for v in ours), default=0.0):.6f}")

    return 0 if first is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
