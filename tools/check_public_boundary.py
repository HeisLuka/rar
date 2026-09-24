#!/usr/bin/env python3
import os
import pathlib
import subprocess
import sys

FROZEN_COMPONENTS = {"yab", "miy", "pub-rs"}
BASE = os.environ.get("PUBLIC_BOUNDARY_BASE", "origin/main")
HEAD = os.environ.get("PUBLIC_BOUNDARY_HEAD", "HEAD")

def changed_paths():
    out = subprocess.check_output(
        ["git", "diff", "--name-only", "--diff-filter=ACMR", f"{BASE}...{HEAD}"],
        text=True,
    )
    return [p.strip() for p in out.splitlines() if p.strip()]

def frozen_hits(path):
    hits = set()
    for raw in pathlib.PurePosixPath(path).parts:
        part = raw.lower()
        for frozen in FROZEN_COMPONENTS:
            if (
                part == frozen
                or part.startswith(frozen + "-")
                or part.endswith("-" + frozen)
                or ("-" + frozen + "-") in part
            ):
                hits.add(frozen)
    return sorted(hits)

bad = []
for path in changed_paths():
    hits = frozen_hits(path)
    if hits:
        bad.append((path, hits))

if bad:
    print("PUBLIC BOUNDARY VIOLATION: changed paths look like mirrors of frozen/private repositories.")
    print("Rar may consume minimum public-safe slices and source-free receipts, not repository mirrors.")
    for path, hits in bad:
        print(f"  {path}: forbidden path component(s) {', '.join(hits)}")
    sys.exit(1)

print("public-boundary path guard: PASS")
