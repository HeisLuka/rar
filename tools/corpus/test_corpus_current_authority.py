#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RECEIPTS = ROOT / "tools" / "corpus" / "receipts"
CURRENT = RECEIPTS / "corpus-current-authority-2026-09-25.json"
PREDECESSOR = RECEIPTS / "corpus-current-authority-2026-09-25-v3.json"
PREDECESSOR_LIST = RECEIPTS / "current-rar-1515.sha256.txt"
SUCCESSOR = RECEIPTS / "current-rar-1521.sha256.txt"
INPUT = RECEIPTS / "publisher2002-sbs-06-input-2026-09-25.json"
SHA_RE = re.compile(r"^[0-9a-f]{64}$")


def canonical_digest(shas) -> str:
    payload = ("\n".join(sorted(shas)) + "\n").encode("ascii")
    return hashlib.sha256(payload).hexdigest()


def read_sha_list(path: Path) -> list[str]:
    return [
        line.strip()
        for line in path.read_text(encoding="ascii").splitlines()
        if line.strip()
    ]


def main() -> int:
    current = json.loads(CURRENT.read_text(encoding="utf-8"))
    predecessor_receipt = json.loads(PREDECESSOR.read_text(encoding="utf-8"))
    spec = json.loads(INPUT.read_text(encoding="utf-8"))

    assert current["schema"] == "rar-pub-corpus-current-authority-v4"
    assert predecessor_receipt["schema"] == "rar-pub-corpus-current-authority-v3"

    predecessor = read_sha_list(PREDECESSOR_LIST)
    assert predecessor == sorted(predecessor)
    assert len(predecessor) == len(set(predecessor)) == 1515
    assert all(SHA_RE.fullmatch(sha) for sha in predecessor)
    predecessor_set = set(predecessor)
    assert canonical_digest(predecessor_set) == (
        "3151155207344da1e611b690c0c7c80dfcb08fd3edf3191da59e2f14be45675c"
    )

    successor = read_sha_list(SUCCESSOR)
    assert successor == sorted(successor)
    assert len(successor) == len(set(successor)) == 1521
    assert all(SHA_RE.fullmatch(sha) for sha in successor)
    successor_set = set(successor)
    assert canonical_digest(successor_set) == (
        "ab9be4a8981ad8f18e9d0a29a4688407e82ffb6ea66d8364d9249228328e6669"
    )

    files = spec["files"]
    new_shas = {row["sha256"] for row in files}
    assert len(files) == len(new_shas) == 6
    assert all(SHA_RE.fullmatch(sha) for sha in new_shas)
    assert canonical_digest(new_shas) == (
        "74eccc943b9187a6a351b27ecdb3b666880953f8cbfeb9aa45cec8794afc0f15"
    )
    assert predecessor_set.isdisjoint(new_shas)
    assert successor_set == predecessor_set | new_shas

    pred = current["predecessor_authority"]
    assert pred["receipt"] == "tools/corpus/receipts/corpus-current-authority-2026-09-25-v3.json"
    assert pred["count"] == 1515
    assert pred["canonical_sorted_sha_lines_digest"] == (
        "sha256:3151155207344da1e611b690c0c7c80dfcb08fd3edf3191da59e2f14be45675c"
    )

    old_exact = predecessor_receipt["rar_exact_union"]
    assert old_exact["count"] == 1515
    assert old_exact["canonical_sorted_sha_lines_digest"] == (
        "sha256:3151155207344da1e611b690c0c7c80dfcb08fd3edf3191da59e2f14be45675c"
    )
    assert old_exact["retained_sha_list"] == "tools/corpus/receipts/current-rar-1515.sha256.txt"

    exact = current["rar_exact_union"]
    assert exact["count"] == 1521
    assert exact["canonical_sorted_sha_lines_digest"] == (
        "sha256:ab9be4a8981ad8f18e9d0a29a4688407e82ffb6ea66d8364d9249228328e6669"
    )
    assert exact["retained_sha_list"] == "tools/corpus/receipts/current-rar-1521.sha256.txt"

    tranches = exact["tranches"]
    expected_counts = {
        "baseline950": 950,
        "post_m1_delta129": 129,
        "container_net407": 407,
        "version_labelled_training29": 29,
        "publisher2002_sbs06": 6,
    }
    assert {k: tranches[k]["count"] for k in expected_counts} == expected_counts
    assert sum(expected_counts.values()) == 1521

    new = tranches["publisher2002_sbs06"]
    assert new["canonical_sorted_sha_lines_digest"] == (
        "sha256:74eccc943b9187a6a351b27ecdb3b666880953f8cbfeb9aa45cec8794afc0f15"
    )
    assert new["input_receipt"] == (
        "tools/corpus/receipts/publisher2002-sbs-06-input-2026-09-25.json"
    )
    assert new["upstream_pr"] == 646
    assert new["upstream_run_id"] == 36159842901
    assert new["upstream_artifact_id"] == 10874149235
    assert new["proof_pr"] == 693
    assert new["proof_run_id"] == 36172246859
    assert all(v == 0 for v in exact["pairwise_overlap"].values())

    hist = current["historical_authority"]
    bounds = current["global_complete_cfb_union"]
    assert hist["complete_count"] == 339
    assert hist["known_overlap_with_current_rar_minimum"] == 50
    assert bounds["lower_bound"] == 1521
    assert bounds["upper_bound"] == 339 + 1521 - 50 == 1810

    t721 = current["source_evidence"]["t721_registration"]
    assert t721["new_unique_sha_count"] == 6
    assert t721["predecessor_overlap"] == 0
    assert t721["proof_run_id"] == 36172246859
    assert t721["successor_count"] == 1521
    assert t721["successor_digest"] == (
        "sha256:ab9be4a8981ad8f18e9d0a29a4688407e82ffb6ea66d8364d9249228328e6669"
    )

    print(json.dumps({
        "schema": current["schema"],
        "predecessor_count": 1515,
        "new_tranche": 6,
        "rar_exact_union": 1521,
        "successor_digest": exact["canonical_sorted_sha_lines_digest"],
        "global_lower_bound": 1521,
        "global_upper_bound": 1810,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
