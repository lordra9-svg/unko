#!/usr/bin/env python3
"""Query OSV.dev for advisories affecting every crates.io package in Cargo.lock.

Fails when an advisory ID is not in scripts/osv-baseline.txt, so new
vulnerabilities are visible in CI without hiding known unmaintained notices.
"""

from __future__ import annotations

import json
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "scripts" / "osv-baseline.txt"


def baseline_ids() -> set[str]:
    ids: set[str] = set()
    for line in BASELINE.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            ids.add(line)
    return ids


def main() -> int:
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    queries = [
        {
            "package": {"name": package["name"], "ecosystem": "crates.io"},
            "version": package["version"],
        }
        for package in lock["package"]
        if "source" in package and "crates.io" in package["source"]
    ]
    body = json.dumps({"queries": queries}).encode()
    request = urllib.request.Request(
        "https://api.osv.dev/v1/querybatch",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            result = json.load(response)
    except (urllib.error.URLError, TimeoutError, ValueError) as err:
        print(f"OSV audit failed: {err}", file=sys.stderr)
        return 2

    findings: list[tuple[str, str, str, str]] = []
    for query, entry in zip(queries, result.get("results", [])):
        for advisory in entry.get("vulns", []):
            findings.append(
                (
                    advisory.get("id", "unknown"),
                    query["package"]["name"],
                    query["version"],
                    advisory.get("summary", ""),
                )
            )

    accepted = baseline_ids()
    new = [finding for finding in findings if finding[0] not in accepted]
    print(
        f"OSV audit: {len(queries)} crates.io packages, "
        f"{len(findings)} advisory match(es), {len(new)} new"
    )
    for advisory, name, version, summary in findings:
        marker = "NEW" if advisory not in accepted else "known"
        print(f"  {marker:5} {advisory}  {name} {version}  {summary}")
    if new:
        print("new advisories require a code/dependency fix or a baseline review", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
