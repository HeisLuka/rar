#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from pathlib import Path


REQUIRED_TOP_LEVEL = {
    "schema_version",
    "task",
    "producer",
    "scope",
    "environment",
    "case_count",
    "all_equivalent",
    "stale_text_rejected",
    "environment_mismatch_rejected",
    "cases",
}


def fail(message: str) -> None:
    raise SystemExit(f"invalid layout prepared story receipt: {message}")


def validate(path: Path) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    missing = REQUIRED_TOP_LEVEL.difference(data)
    if missing:
        fail(f"missing top-level keys: {sorted(missing)}")
    if data["schema_version"] != "layout-prepared-story-receipt-v1":
        fail("unexpected schema_version")
    if data["task"] != "LAYOUT-PREPARED-STORY-01":
        fail("unexpected task")
    if not isinstance(data["scope"], dict):
        fail("scope must be an object")
    if data["scope"].get("source_free_modelcheck") is not True:
        fail("receipt must explicitly be source-free")
    if data["scope"].get("canonical_engine_proof") is not False:
        fail("public modelcheck must not claim canonical engine proof")
    if data["scope"].get("publisher_fidelity_proof") is not False:
        fail("public modelcheck must not claim Publisher fidelity proof")
    cases = data["cases"]
    if not isinstance(cases, list) or not cases:
        fail("cases must be a non-empty list")
    if data["case_count"] != len(cases):
        fail("case_count does not match cases")
    if data["all_equivalent"] is not True:
        fail("full/prepared equivalence failed")
    if data["stale_text_rejected"] is not True:
        fail("stale text was not rejected")
    if data["environment_mismatch_rejected"] is not True:
        fail("environment mismatch was not rejected")

    for index, case in enumerate(cases):
        required = {
            "name",
            "frame_widths",
            "lines_per_frame",
            "equivalent",
            "full_hash",
            "incremental_hash",
            "full_shape_calls",
            "incremental_shape_calls",
            "prepared_reuse_count",
            "line_count",
            "overset_scalars",
            "next_scalar",
        }
        missing_case = required.difference(case)
        if missing_case:
            fail(f"case {index} missing keys: {sorted(missing_case)}")
        if case["equivalent"] is not True:
            fail(f"case {index} is not equivalent")
        if case["full_hash"] != case["incremental_hash"]:
            fail(f"case {index} hashes differ")
        if case["full_shape_calls"] < 1:
            fail(f"case {index} full path did not prepare")
        if case["incremental_shape_calls"] != 0:
            fail(f"case {index} incremental path unexpectedly prepared")
        if case["prepared_reuse_count"] < 1:
            fail(f"case {index} did not record prepared reuse")
        if not case["frame_widths"] or any(width <= 0 for width in case["frame_widths"]):
            fail(f"case {index} has invalid frame widths")
        if case["lines_per_frame"] <= 0:
            fail(f"case {index} has invalid lines_per_frame")

    return data


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("receipt", type=Path)
    args = parser.parse_args()
    data = validate(args.receipt)
    print(
        json.dumps(
            {
                "valid": True,
                "task": data["task"],
                "case_count": data["case_count"],
                "producer": data["producer"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
