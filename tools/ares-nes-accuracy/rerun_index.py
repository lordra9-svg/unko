#!/usr/bin/env python3
"""Re-measure NES games with the palette-index metric.

Compares Unko and ares at the PPU's native 9-bit colour indices (emphasis and
greyscale included) instead of their RGB palettes, so visually identical
frames no longer score near zero when the two emulators use different RGB
tables.

Usage:
    rerun_index.py A            # every A entry in docs/nes-accuracy-not-100.csv
    rerun_index.py A --all      # every A entry in the ares ledger
    rerun_index.py --titles "Akuma-kun" "Bayou Billy"

The primary score is the mean per-pixel index agreement over frames 30..119,
using the best +/-3 frame alignment. The final frame and the fraction of
frames with byte-identical index hashes are kept as diagnostics.
"""

import csv
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile
from array import array
from operator import eq

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
UNKO = os.path.join(ROOT, 'target/release/unko')
CAPTURE = os.path.join(ROOT, 'tools/snes-oracle/ares-capture.sh')
NES_DIR = os.environ.get(
    'UNKO_NES_DIR',
    '/var/home/Lordra/ai/refs/Minerva_Myrient/No-Intro/Nintendo - Nintendo Entertainment System (Headered)',
)
LEDGER = os.path.join(ROOT, 'compatibility/accuracy-ares.jsonl')
SHORTFALL = os.path.join(ROOT, 'docs/nes-accuracy-not-100.csv')
FRAMES = int(os.environ.get('UNKO_ACC_FRAMES', '120'))
WIDTH, HEIGHT = 256, 240
PIXELS = WIDTH * HEIGHT
# ares' RP2C07 (PAL) raster only draws PPU x=2..253 and y=1..239; the rest of
# its canvas is the fixed black backdrop. Mask those columns/rows so the
# comparison covers the area both cores actually render.
PAL_MASK = [y * WIDTH + x for y in range(1, HEIGHT) for x in range(2, 254)]
BAD = re.compile(r'(\((?:Unl|Pirate|Beta|Bios|Aftermarket|Proto|Demo)[^)]*\)|\[(?:BIOS|Bios)\])', re.I)


def strip_article(title):
    title = title.strip()
    for article in ('The ', 'A ', 'An '):
        if title.startswith(article):
            return title[len(article):]
    return title


def load_seq(path):
    data = array('H')
    with open(path, 'rb') as handle:
        data.frombytes(handle.read())
    return data


def agreement(ours, ref, shift, start=30, end=FRAMES, mask=None):
    same = total = 0
    available = len(ref) // PIXELS
    for index in range(start, end):
        other = index + shift
        if not 0 <= other < available:
            continue
        x = ours[index * PIXELS:(index + 1) * PIXELS]
        y = ref[other * PIXELS:(other + 1) * PIXELS]
        if mask is None:
            same += sum(map(eq, x, y))
            total += PIXELS
        else:
            same += sum(1 for offset in mask if x[offset] == y[offset])
            total += len(mask)
    return same, total


def index_metrics(ours, ref, mask=None):
    best_rate, best_shift = -1.0, 0
    for shift in range(-3, 4):
        same, total = agreement(ours, ref, shift, mask=mask)
        if total and same / total > best_rate:
            best_rate, best_shift = same / total, shift
    same, total = agreement(ours, ref, best_shift, mask=mask)
    final_same, final_total = agreement(ours, ref, best_shift, FRAMES - 1, FRAMES, mask=mask)
    return {
        'video_pct': round(same / max(1, total) * 100, 4),
        'final_video_pct': round(final_same / max(1, final_total) * 100, 4),
        'index_shift': best_shift,
    }


def parse_trace(path):
    frames = []
    for line in open(path):
        parts = line.split()
        if parts and parts[0] == 'frame':
            frames.append(tuple(int(value, 16) for value in parts[2:5]))
    return frames


def frame_sequence_pct(ours_trace, ref_trace, shift):
    ours = parse_trace(ours_trace)
    ref = parse_trace(ref_trace)
    count = min(len(ours), len(ref))
    if count <= 30:
        return 0.0
    matches = sum(
        1
        for index in range(30, count)
        if 0 <= index + shift < len(ref) and ours[index][2] == ref[index + shift][2]
    )
    return round(matches / (count - 30) * 100, 4)


def mapper_of(data):
    if len(data) < 16 or data[:4] != b'NES\x1a':
        return None
    mapper = (data[6] >> 4) | (data[7] & 0xF0)
    if data[7] & 0x08:
        mapper |= (data[8] & 0x0F) << 8
    return mapper


def detect_region(data):
    if len(data) < 16 or data[:4] != b'NES\x1a':
        return 'NTSC'
    if data[7] & 0x08:  # NES 2.0
        timing = data[12] & 0x03
        if timing == 1:
            return 'PAL'
        if timing == 3:
            return 'DENDY'
        return 'NTSC'
    return 'PAL' if data[9] & 0x01 else 'NTSC'


def process(title, rom, sha, region, tmp):
    ours_seq = os.path.join(tmp, 'ours.idxseq')
    ours_trace = os.path.join(tmp, 'ours.trace')
    subprocess.run(
        [UNKO, 'trace', rom, '--frames', str(FRAMES), '--out', ours_trace,
         '--indices-seq', ours_seq, '--no-state'],
        check=True, capture_output=True,
    )
    ref_dir = os.path.join(tmp, 'ares')
    shutil.rmtree(ref_dir, ignore_errors=True)
    env = os.environ.copy()
    env.update({'ARES_SYSTEM': 'Famicom', 'ARES_AUDIO_RATE': '44100', 'TMPDIR': tmp})
    if region == 'PAL':
        env['ARES_REGION'] = 'PAL'
    subprocess.run([CAPTURE, rom, str(FRAMES), ref_dir], check=True,
                   capture_output=True, env=env)
    ours = load_seq(ours_seq)
    ref = load_seq(os.path.join(ref_dir, 'frame.raw.idxseq'))
    expected = FRAMES * PIXELS
    if len(ours) != expected or len(ref) != expected:
        raise RuntimeError(
            f'index sequences are incomplete ({len(ours)} / {len(ref)} of {expected})'
        )
    metrics = index_metrics(ours, ref, PAL_MASK if region == 'PAL' else None)
    last = ref[(FRAMES - 1) * PIXELS:FRAMES * PIXELS]
    ours_last = ours[(FRAMES - 1) * PIXELS:FRAMES * PIXELS]
    if len(set(last)) == 1 and len(set(ours_last)) > 1:
        metrics.update({
            'status': 'error',
            'error': 'ares capture is a uniform blank frame (board unsupported by the oracle)',
        })
    if region != 'PAL':
        # PAL hashes still contain ares' clipped border pixels, so the strict
        # frame-sequence diagnostic is only meaningful for NTSC.
        metrics['frame_seq_pct'] = frame_sequence_pct(
            ours_trace, os.path.join(ref_dir, 'trace.txt'), metrics['index_shift']
        )
    metrics.setdefault('status', 'ok')
    metrics.update({'tested': '2026-11-27', 'metric': 'nes-index'})
    if region != 'NTSC':
        metrics['region'] = region
    return metrics


def target_titles(letter, all_games, explicit, mappers=None, zero=False):
    ledger = [json.loads(line) for line in open(LEDGER)]
    if '--index' in sys.argv[1:]:
        return [entry['title'] for entry in ledger
                if entry.get('metric') == 'nes-index' and not BAD.search(entry['title'])]
    if '--errors' in sys.argv[1:]:
        return [entry['title'] for entry in ledger
                if entry.get('metric') == 'nes-index'
                and entry.get('status') == 'error'
                and not BAD.search(entry['title'])]
    if zero:
        return [entry['title'] for entry in ledger
                if entry.get('metric') == 'nes-index'
                and entry.get('status') == 'ok'
                and entry.get('video_pct') == 0.0]
    if mappers:
        wanted = []
        for entry in ledger:
            if entry.get('system') != 'nes' or BAD.search(entry['title']):
                continue
            path = os.path.join(NES_DIR, entry['title'] + '.zip')
            if not os.path.exists(path):
                continue
            try:
                with zipfile.ZipFile(path) as archive:
                    member = [name for name in archive.namelist()
                              if name.lower().endswith(('.nes', '.unh'))][0]
                    header = archive.read(member)[:16]
            except Exception:
                continue
            if mapper_of(header) in mappers:
                wanted.append(entry['title'])
        return wanted
    if explicit:
        return [entry['title'] for entry in ledger if any(x.lower() in entry['title'].lower() for x in explicit)]
    if all_games:
        return [entry['title'] for entry in ledger
                if (entry.get('system') == 'nes' and not BAD.search(entry['title'])
                    and strip_article(entry['title'])[:1].upper() == letter.upper())]
    rows = list(csv.DictReader(open(SHORTFALL)))
    return [row['title'] for row in rows
            if not BAD.search(row['title'])
            and strip_article(row['title'])[:1].upper() == letter.upper()]


def main():
    args = [arg for arg in sys.argv[1:] if not arg.startswith('--')]
    flags = {arg for arg in sys.argv[1:] if arg.startswith('--')}
    explicit = None if args and len(args[0]) == 1 else args
    letter = args[0] if args and len(args[0]) == 1 else ''
    mapper_flag = next((arg for arg in sys.argv[1:] if arg.startswith('--mappers=')), None)
    mappers = None
    if mapper_flag:
        mappers = {int(value, 0) for value in mapper_flag.split('=', 1)[1].split(',') if value}
    titles = target_titles(letter, '--all' in flags, explicit, mappers, '--zero' in flags)

    rows = [json.loads(line) for line in open(LEDGER)]
    by_sha = {row.get('sha256'): row for row in rows}
    tmp = tempfile.mkdtemp(prefix='rerun-', dir=os.environ.get('UNKO_ACC_TMP', '/tmp/opencode'))

    for title in sorted(titles):
        path = os.path.join(NES_DIR, title + '.zip')
        if not os.path.exists(path):
            print('MISSING', title, flush=True)
            continue
        try:
            with zipfile.ZipFile(path) as archive:
                member = [name for name in archive.namelist()
                          if name.lower().endswith(('.nes', '.unh'))][0]
                data = archive.read(member)
            rom = os.path.join(tmp, 'game.nes')
            open(rom, 'wb').write(data)
            sha = hashlib.sha256(data).hexdigest()
            record = by_sha.get(sha)
            region = detect_region(data)
            if record and record.get('region') == 'PAL':
                region = 'PAL'
            metrics = process(title, rom, sha, region, tmp)
            targets = [row for row in rows if row.get('sha256') == sha]
            if not targets:
                record = {'system': 'nes', 'format': 'Headered', 'title': title,
                          'oracle': 'ares', 'frames': FRAMES, 'sha256': sha}
                rows.append(record)
                by_sha[sha] = record
                targets = [record]
            for record in targets:
                for key in ('geometry_pct', 'best_geometry_pct', 'best_shift',
                            'steady_geometry_pct', 'palette_pct'):
                    record.pop(key, None)
                record.update(metrics)
            print(f"{metrics['video_pct']:8.3f} steady "
                  f"{metrics['final_video_pct']:8.3f} final "
                  f"{metrics.get('frame_seq_pct', 0):7.2f} seq "
                  f"shift {metrics['index_shift']:+d}  {title}", flush=True)
        except Exception as error:  # noqa: BLE001 - report and continue the batch
            print('ERROR', title, str(error)[:200], flush=True)
        with open(LEDGER, 'w') as handle:
            for row in rows:
                handle.write(json.dumps(row, ensure_ascii=False) + '\n')
    shutil.rmtree(tmp, ignore_errors=True)


if __name__ == '__main__':
    main()
