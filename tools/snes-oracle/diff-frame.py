#!/usr/bin/env python3
"""Compare an unko-snes RGBA frame with an ares RGBA frame.

Default geometry is NTSC non-hires: ares renders 564x242 and contains the
256x224 visible frame at (26, 9) with horizontal scale 2 (each of our pixels is
two ares columns). Override with --x0/--y0/--sx if a demo uses another mode.

Exit status: 0 if every compared pixel is identical, 1 otherwise.
"""
from __future__ import annotations

import argparse
import sys


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("ours")
    ap.add_argument("ares")
    ap.add_argument("--ours-w", type=int, default=256)
    ap.add_argument("--ours-h", type=int, default=224)
    ap.add_argument("--ares-w", type=int, default=564)
    ap.add_argument("--ares-h", type=int, default=242)
    ap.add_argument("--x0", type=int, default=26)
    ap.add_argument("--y0", type=int, default=9)
    ap.add_argument("--sx", type=int, default=2)
    ap.add_argument("--diff", help="write a PPM diff image here")
    args = ap.parse_args()

    ours = open(args.ours, "rb").read()
    ares = open(args.ares, "rb").read()
    need_o = args.ours_w * args.ours_h * 4
    need_a = args.ares_w * args.ares_h * 4
    if len(ours) < need_o:
        sys.exit(f"{args.ours}: {len(ours)} bytes, need {need_o}")
    if len(ares) < need_a:
        sys.exit(f"{args.ares}: {len(ares)} bytes, need {need_a}")

    exact = 0
    total = 0
    error = 0
    worst = 0
    diff = bytearray(args.ours_w * args.ours_h * 3)
    for y in range(args.ours_h):
        for x in range(args.ours_w):
            oi = (y * args.ours_w + x) * 4
            ax = args.x0 + x * args.sx
            ay = args.y0 + y
            ai = (ay * args.ares_w + ax) * 4
            d = abs(ours[oi] - ares[ai]) + abs(ours[oi + 1] - ares[ai + 1]) + abs(
                ours[oi + 2] - ares[ai + 2]
            )
            total += 1
            error += d
            worst = max(worst, d)
            if d == 0:
                exact += 1
            di = (y * args.ours_w + x) * 3
            diff[di] = min(255, d)
            diff[di + 1] = min(255, d)
            diff[di + 2] = min(255, d)

    print(f"pixels:      {total}")
    print(f"exact:       {exact} ({100.0 * exact / total:.3f}%)")
    print(f"mean |d|:    {error / total:.4f} (sum of R,G,B abs deltas)")
    print(f"worst |d|:   {worst}")

    if args.diff:
        with open(args.diff, "wb") as f:
            f.write(b"P6\n%d %d\n255\n" % (args.ours_w, args.ours_h))
            f.write(diff)
        print(f"diff image:  {args.diff}")

    return 0 if exact == total else 1


if __name__ == "__main__":
    raise SystemExit(main())
