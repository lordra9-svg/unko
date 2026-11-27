#!/usr/bin/env python3
"""Rebuild the NES below-100 list from the ares ledger."""

import csv
import json
import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LEDGER = os.path.join(ROOT, 'compatibility/accuracy-ares.jsonl')
OUT_MD = os.path.join(ROOT, 'docs/nes-accuracy-not-100.md')
OUT_CSV = os.path.join(ROOT, 'docs/nes-accuracy-not-100.csv')
# Campaign policy: unlicensed/pirate/beta/bios/aftermarket/prototype dumps are
# not part of the walk-through.
IGNORED = re.compile(r'(\((?:Unl|Pirate|Beta|Bios|Aftermarket|Proto)[^)]*\)|\[(?:BIOS|Bios)\])', re.I)


def score(record):
    if record.get('metric') == 'nes-index':
        return record.get('video_pct')
    if record.get('steady_geometry_pct') is not None:
        return record.get('steady_geometry_pct')
    if record.get('video_pct') is not None:
        return record.get('video_pct')
    return record.get('best_geometry_pct')


def main():
    rows = [json.loads(line) for line in open(LEDGER)]
    measured = [row for row in rows if row.get('status') == 'ok']
    errors = [row for row in rows if row.get('status') != 'ok']
    below_all = [row for row in measured if score(row) is not None and score(row) < 100]
    below = [row for row in below_all if not IGNORED.search(row['title'])]
    ignored = [row for row in below_all if IGNORED.search(row['title'])]
    errors_all = [row for row in errors if not IGNORED.search(row['title'])]
    ignored_errors = [row for row in errors if IGNORED.search(row['title'])]
    errors = errors_all
    below.sort(key=lambda row: (score(row), row['title']))
    errors.sort(key=lambda row: row['title'])

    with open(OUT_CSV, 'w', newline='') as handle:
        writer = csv.writer(handle)
        writer.writerow([
            'title', 'status', 'accuracy_pct', 'metric', 'steady_geometry_pct',
            'video_pct', 'final_video_pct', 'frame_seq_pct', 'palette_pct',
            'best_geometry_pct', 'same_index_geometry_pct', 'frames', 'error', 'sha256',
        ])
        for row in below + errors:
            index_metric = row.get('metric') == 'nes-index'
            metric = ('index' if index_metric else
                      ('steady geometry' if row.get('steady_geometry_pct') is not None
                       else 'final-frame pixel'))
            writer.writerow([
                row.get('title', ''), row.get('status', ''),
                score(row) if score(row) is not None else '', metric,
                row.get('steady_geometry_pct', ''), row.get('video_pct', ''),
                row.get('final_video_pct', ''), row.get('frame_seq_pct', ''),
                row.get('palette_pct', ''), row.get('best_geometry_pct', ''),
                row.get('geometry_pct', ''), row.get('frames', ''),
                row.get('error', ''), row.get('sha256', ''),
            ])

    buckets = [('0-50%', 0, 50), ('50-90%', 50, 90), ('90-99%', 90, 99), ('99-100%', 99, 100)]
    index_measured = [row for row in measured if row.get('metric') == 'nes-index']
    lines = [
        '# NES games not at 100% accuracy (ares primary)',
        '',
        'Generated from `compatibility/accuracy-ares.jsonl` - **ares** primary, 120 frames. RustyNES remains secondary.',
        '',
        '**Metric:** newly re-run letters use the NES palette-index agreement over frames 30+ (`video_pct`); records not yet revisited keep their historical steady-geometry or final-frame value.',
        '',
        f'- NES archives: **{len(rows)}**',
        f'- Measured: **{len(measured)}**',
        f'- Measured with the palette-index metric: **{len(index_measured)}**',
        f'- Below 100% by the relevant ares metric: **{len(below)}**',
        f'- Ignored tagged dumps below 100%: **{len(ignored)}**',
        f'- No measurement / oracle error: **{len(errors)}**',
        f'- Ignored tagged dumps with errors: **{len(ignored_errors)}**',
        f'- Total needing attention: **{len(below) + len(errors)}**',
        '',
        '| Band | Games |',
        '|---|---:|',
    ]
    for label, low, high in buckets:
        count = sum(1 for row in below if low <= score(row) < high)
        lines.append(f'| {label} | {count} |')
    lines += [
        '',
        'Full list: [`nes-accuracy-not-100.csv`](nes-accuracy-not-100.csv)',
        '',
        '## Worst 50 measured games',
        '',
        '| Accuracy | Metric | Game |',
        '|---:|---|---|',
    ]
    for row in below[:50]:
        if row.get('metric') == 'nes-index':
            metric = 'index'
        elif row.get('steady_geometry_pct') is not None:
            metric = 'steady'
        elif row.get('region') == 'PAL':
            metric = 'PAL frame'
        else:
            metric = 'frame'
        lines.append(f"| {score(row):.2f}% | {metric} | {row['title'].replace('|', '\\|')} |")
    lines += ['', '## No measurement / oracle errors', '', '| Game | Error |', '|---|---|']
    for row in errors[:50]:
        lines.append(f"| {row['title'].replace('|', '\\|')} | {(row.get('error') or '').replace('|', '\\|')[:180]} |")
    if len(errors) > 50:
        lines.append(f'| ... | {len(errors) - 50} more in the CSV |')
    lines.append('')
    open(OUT_MD, 'w').write('\n'.join(lines) + '\n')
    print(f'below {len(below)} errors {len(errors)} measured {len(measured)} index {len(index_measured)}')


if __name__ == '__main__':
    main()
