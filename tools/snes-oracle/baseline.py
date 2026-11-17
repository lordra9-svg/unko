#!/usr/bin/env python3
"""Run every SNES test ROM through unko-snes and record a regression manifest.

Each row: rom path, sha256, frame sha256, audio sha256, frame count, status.
Run once before a change and once after; diff the two CSVs.

Usage: baseline.py [--frames N] [--out path] [--limit N] [--filter substr]
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import os
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
IPL = os.environ.get(
    "IPL", "/var/home/Lordra/ai/refs/ares/ares/System/Super Famicom/ipl.rom"
)
EXTS = (".sfc", ".smc", ".fig", ".swc")


def sha(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--frames", type=int, default=600)
    ap.add_argument("--out", default="/tmp/snes-baseline.csv")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--filter", default="")
    ap.add_argument("--roms", default=os.path.join(ROOT, "test-roms", "snes"))
    args = ap.parse_args()

    roms = []
    for base, _dirs, files in os.walk(args.roms):
        for name in sorted(files):
            if name.lower().endswith(EXTS):
                path = os.path.join(base, name)
                if args.filter and args.filter not in path:
                    continue
                roms.append(path)
    roms.sort()
    if args.limit:
        roms = roms[: args.limit]

    rows = []
    for i, rom in enumerate(roms, 1):
        with tempfile.TemporaryDirectory() as tmp:
            frame = os.path.join(tmp, "frame.raw")
            audio = os.path.join(tmp, "audio.f32")
            env = dict(os.environ)
            env.update(FRAMES=str(args.frames), FRAME_OUT=frame, AUDIO_OUT=audio)
            binary = os.path.join(ROOT, "target", "release", "examples", "snes-info")
            cmd = (
                [binary, rom, IPL]
                if os.path.exists(binary)
                else ["cargo", "run", "-q", "--release", "-p", "unko-snes",
                      "--example", "snes-info", "--", rom, IPL]
            )
            proc = subprocess.run(
                cmd,
                cwd=ROOT,
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True,
            )
            status = "ok" if proc.returncode == 0 else "error"
            frame_hash = sha(frame) if os.path.exists(frame) and os.path.getsize(frame) else ""
            audio_hash = sha(audio) if os.path.exists(audio) and os.path.getsize(audio) else ""
        rows.append([os.path.relpath(rom, ROOT), sha(rom), frame_hash, audio_hash, args.frames, status])
        print(f"[{i}/{len(roms)}] {rows[-1][0]} {status}", file=sys.stderr)

    with open(args.out, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["rom", "rom_sha256", "frame_sha256", "audio_sha256", "frames", "status"])
        w.writerows(rows)
    print(f"wrote {args.out} ({len(rows)} rows)")
    return 0 if all(r[-1] == "ok" for r in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
