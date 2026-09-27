#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

SCHEMA = "chaptera.autonomus-id-list-synth-receipt.v1"
PROOF_CASES = [
    "ordered_repeated_handle_u32",
    "same_id_different_wire_not_promoted",
    "explicit_count_mismatch_fails_closed",
    "truncated_entry_preserves_cursor",
    "unsupported_wire_preserves_cursor",
    "nested_container_respects_declared_span",
    "declared_length_underflow_rejected",
    "declared_length_out_of_bounds_rejected",
]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(test_source: Path, commit_sha: str) -> dict:
    if len(commit_sha) != 40 or any(ch not in "0123456789abcdef" for ch in commit_sha.lower()):
        raise ValueError("commit SHA must be 40 hex characters")
    return {
        "schema": SCHEMA,
        "task": "AUTONOMUS-GH-IDLIST-SYNTH-HARNESS-01",
        "parent_task": "PUB-T-746",
        "git_commit": commit_sha.lower(),
        "parser_surface": "vendor/producer-a/crates/pub-contents::parse_confirmed_block",
        "test_source": test_source.as_posix(),
        "test_source_sha256": sha256(test_source),
        "proof_cases": PROOF_CASES,
        "proof_case_count": len(PROOF_CASES),
        "source_free": True,
        "private_pub_bytes_used": False,
        "semantic_claim_promotion": False,
        "limitations": [
            "Synthetic structural contract only.",
            "Does not classify the 176 private T733 residual rows.",
            "Does not assign semantic meaning to generic id_list output.",
        ],
    }


def validate(receipt: dict) -> None:
    if receipt.get("schema") != SCHEMA:
        raise ValueError("wrong schema")
    if receipt.get("parent_task") != "PUB-T-746":
        raise ValueError("wrong parent task")
    if receipt.get("proof_case_count") != len(PROOF_CASES):
        raise ValueError("wrong proof-case count")
    if receipt.get("proof_cases") != PROOF_CASES:
        raise ValueError("proof-case list drift")
    if receipt.get("source_free") is not True:
        raise ValueError("source_free must be true")
    if receipt.get("private_pub_bytes_used") is not False:
        raise ValueError("private_pub_bytes_used must be false")
    if receipt.get("semantic_claim_promotion") is not False:
        raise ValueError("semantic_claim_promotion must be false")
    digest = receipt.get("test_source_sha256")
    if not isinstance(digest, str) or len(digest) != 64:
        raise ValueError("test_source_sha256 must be SHA-256")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--test-source", required=True, type=Path)
    parser.add_argument("--commit-sha", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    receipt = build(args.test_source, args.commit_sha)
    validate(receipt)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
