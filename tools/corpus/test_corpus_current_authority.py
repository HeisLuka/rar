#!/usr/bin/env python3
from __future__ import annotations

import csv
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RECEIPTS = ROOT / "tools" / "corpus" / "receipts"
CURRENT = RECEIPTS / "corpus-current-authority-2026-09-24.json"
OLD = RECEIPTS / "corpus-authority-lower-bound-2026-09-24.json"
DELTA_TSV = RECEIPTS / "corpus-post-m1-delta-2026-09-24.tsv"
SHA_RE = re.compile(r"^[0-9a-f]{64}$")


def canonical_digest(shas: list[str]) -> str:
    payload = ("\n".join(sorted(shas)) + "\n").encode("ascii")
    return hashlib.sha256(payload).hexdigest()


def main() -> int:
    current = json.loads(CURRENT.read_text(encoding="utf-8"))
    old = json.loads(OLD.read_text(encoding="utf-8"))

    assert current["schema"] == "rar-pub-corpus-current-authority-v2"
    hist = current["historical_authority"]
    assert hist["complete_count"] == old["historical_complete_authority_count"] == 339
    assert hist["exact_identities_rematerialized"] == old["historical_exact_identities_rematerialized"] == 316
    assert hist["unrematerialized_complete_identities"] == old["historical_unrematerialized_complete_identities"] == 23
    assert hist["known_overlap_with_current_rar_minimum"] == old["rar_overlap_with_rematerialized_baseline"] == 50

    tranches = current["rar_exact_union"]["tranches"]
    assert tranches["baseline950"]["count"] == old["rar_exact_union_sha_list"]["count"] == 950
    assert tranches["baseline950"]["canonical_sorted_sha_lines_digest"] == "sha256:" + old["rar_exact_union_sha_list"]["sha256"]

    with DELTA_TSV.open("r", encoding="utf-8", newline="") as fh:
        delta_rows = list(csv.DictReader(fh, delimiter="\t"))
    delta = [row["sha256"] for row in delta_rows]
    assert len(delta) == len(set(delta)) == tranches["post_m1_delta129"]["count"] == 129
    assert all(SHA_RE.fullmatch(sha) for sha in delta)
    assert canonical_digest(delta) == tranches["post_m1_delta129"]["canonical_sorted_sha_lines_digest"].removeprefix("sha256:")

    container = []
    for rel in tranches["container_net407"]["retained_parts"]:
        p = ROOT / rel
        rows = [line.strip() for line in p.read_text(encoding="utf-8").splitlines() if line.strip()]
        assert rows == sorted(rows)
        assert all(SHA_RE.fullmatch(sha) for sha in rows)
        container.extend(rows)
    assert len(container) == len(set(container)) == tranches["container_net407"]["count"] == 407
    assert container == sorted(container)
    assert canonical_digest(container) == tranches["container_net407"]["canonical_sorted_sha_lines_digest"].removeprefix("sha256:")
    assert set(delta).isdisjoint(container)

    pairwise = current["rar_exact_union"]["pairwise_overlap"]
    assert pairwise == {
        "baseline950__post_m1_delta129": 0,
        "baseline950__container_net407": 0,
        "post_m1_delta129__container_net407": 0,
    }
    assert current["rar_exact_union"]["count"] == 950 + 129 + 407 == 1486

    bounds = current["global_complete_cfb_union"]
    assert bounds["lower_bound"] == 1486
    assert bounds["upper_bound"] == 339 + 1486 - 50 == 1775
    assert bounds["lower_bound"] <= bounds["upper_bound"]

    assert current["rar_exact_union"]["canonical_sorted_sha_lines_digest"] == "sha256:885eb9dad74f72617f00d7f9c113c8d9f44c295bc14e5e53c525d0e01e91c683"
    print(json.dumps({
        "schema": current["schema"],
        "rar_exact_union": 1486,
        "container_net_new": 407,
        "global_lower_bound": 1486,
        "global_upper_bound": 1775,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
