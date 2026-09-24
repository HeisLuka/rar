#!/usr/bin/env python3
from __future__ import annotations

import csv
import hashlib
import json
import re
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TSV = ROOT / "tools" / "corpus" / "receipts" / "corpus-post-m1-delta-2026-09-24.tsv"
META = ROOT / "tools" / "corpus" / "receipts" / "corpus-post-m1-delta-2026-09-24.json"
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
EXPECTED = Counter({"class_templates": 52, "training": 15, "mvp_matrix": 62})


def main() -> int:
    meta = json.loads(META.read_text(encoding="utf-8"))
    with TSV.open("r", encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle, delimiter="\t"))

    if len(rows) != 129:
        raise SystemExit(f"expected 129 rows, got {len(rows)}")

    shas = [row["sha256"] for row in rows]
    if shas != sorted(shas):
        raise SystemExit("receipt SHA rows must be sorted")
    if len(set(shas)) != 129:
        raise SystemExit("receipt contains duplicate SHA")
    if any(not SHA_RE.fullmatch(sha) for sha in shas):
        raise SystemExit("receipt contains malformed SHA-256")

    lanes = Counter(row["lane"] for row in rows)
    if lanes != EXPECTED:
        raise SystemExit(f"lane counts mismatch: {dict(lanes)}")

    if meta["delta"]["exact_sha_count"] != 129:
        raise SystemExit("metadata delta count mismatch")
    if Counter(meta["delta"]["lane_counts"]) != EXPECTED:
        raise SystemExit("metadata lane counts mismatch")
    if any(meta["delta"]["cross_lane_overlap_counts"].values()):
        raise SystemExit("metadata claims cross-lane overlap")
    if meta["comparison_baseline"]["exact_sha_count"] != 950:
        raise SystemExit("baseline count mismatch")
    if meta["rar_union_after_delta_sha_count"] != 1079:
        raise SystemExit("post-delta Rar union mismatch")
    if meta["authority_boundary"]["global_authority_recomputed"]:
        raise SystemExit("receipt must not overclaim global corpus authority")

    digest = hashlib.sha256(TSV.read_bytes()).hexdigest()
    print(json.dumps({
        "schema": meta["schema"],
        "rows": len(rows),
        "lane_counts": dict(sorted(lanes.items())),
        "tsv_sha256": digest,
        "rar_union_after_delta_sha_count": 1079,
        "global_authority_recomputed": False,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
