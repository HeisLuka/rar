#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RECEIPTS = ROOT / "tools" / "corpus" / "receipts"
CURRENT = RECEIPTS / "corpus-current-authority-2026-09-25.json"
PREDECESSOR = RECEIPTS / "corpus-current-authority-2026-09-24.json"
SUCCESSOR = RECEIPTS / "current-rar-1515.sha256.txt"
INPUT = RECEIPTS / "version-labelled-training-29-input-2026-09-25.json"
SHA_RE = re.compile(r"^[0-9a-f]{64}$")


def canonical_digest(shas) -> str:
    payload = ("\\n".join(sorted(shas)) + "\\n").encode("ascii")
    return hashlib.sha256(payload).hexdigest()


def main() -> int:
    current = json.loads(CURRENT.read_text(encoding="utf-8"))
    predecessor_receipt = json.loads(PREDECESSOR.read_text(encoding="utf-8"))
    spec = json.loads(INPUT.read_text(encoding="utf-8"))

    assert current["schema"] == "rar-pub-corpus-current-authority-v3"
    assert predecessor_receipt["schema"] == "rar-pub-corpus-current-authority-v2"

    successor = [
        line.strip()
        for line in SUCCESSOR.read_text(encoding="ascii").splitlines()
        if line.strip()
    ]
    assert successor == sorted(successor)
    assert len(successor) == len(set(successor)) == 1515
    assert all(SHA_RE.fullmatch(sha) for sha in successor)
    successor_set = set(successor)
    assert canonical_digest(successor_set) == "3151155207344da1e611b690c0c7c80dfcb08fd3edf3191da59e2f14be45675c"

    observations = [row for package in spec["packages"] for row in package["files"]]
    new_shas = {row["sha256"] for row in observations}
    assert len(observations) == 33
    assert len(new_shas) == 29
    assert canonical_digest(new_shas) == "eb2cd6ad8f21860f93d33763a8f630135c12c128e0160e0d1909c0c3ae228ae8"
    assert new_shas <= successor_set

    predecessor = successor_set - new_shas
    assert len(predecessor) == 1486
    assert canonical_digest(predecessor) == "885eb9dad74f72617f00d7f9c113c8d9f44c295bc14e5e53c525d0e01e91c683"
    assert predecessor_receipt["rar_exact_union"]["count"] == 1486
    assert predecessor_receipt["rar_exact_union"]["canonical_sorted_sha_lines_digest"] == (
        "sha256:885eb9dad74f72617f00d7f9c113c8d9f44c295bc14e5e53c525d0e01e91c683"
    )

    exact = current["rar_exact_union"]
    assert exact["count"] == 1515
    assert exact["canonical_sorted_sha_lines_digest"] == (
        "sha256:3151155207344da1e611b690c0c7c80dfcb08fd3edf3191da59e2f14be45675c"
    )
    assert exact["retained_sha_list"] == "tools/corpus/receipts/current-rar-1515.sha256.txt"
    tranches = exact["tranches"]
    assert sum(tranches[k]["count"] for k in (
        "baseline950", "post_m1_delta129", "container_net407", "version_labelled_training29"
    )) == 1515
    assert tranches["version_labelled_training29"]["count"] == 29
    assert tranches["version_labelled_training29"]["observation_count"] == 33
    assert tranches["version_labelled_training29"]["proof_run_id"] == 36153583127
    assert tranches["version_labelled_training29"]["proof_artifact_id"] == 10873945783
    assert all(v == 0 for v in exact["pairwise_overlap"].values())

    hist = current["historical_authority"]
    bounds = current["global_complete_cfb_union"]
    assert hist["complete_count"] == 339
    assert hist["known_overlap_with_current_rar_minimum"] == 50
    assert bounds["lower_bound"] == 1515
    assert bounds["upper_bound"] == 339 + 1515 - 50 == 1804

    print(json.dumps({
        "schema": current["schema"],
        "rar_exact_union": 1515,
        "successor_digest": exact["canonical_sorted_sha_lines_digest"],
        "new_tranche": 29,
        "global_lower_bound": 1515,
        "global_upper_bound": 1804,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
