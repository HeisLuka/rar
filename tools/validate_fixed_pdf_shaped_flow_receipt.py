#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "fixed-output" / "v1" / "shaped-flow-producer-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("shaped-flow receipt schema validation failed\n" + detail)


def validate_semantics(receipt):
    lines = receipt["lines"]
    runs = receipt["runs"]
    if len(lines) != len(runs):
        raise AssertionError("visible shaped-flow line count differs from fixed run count")

    for i, (line, run) in enumerate(zip(lines, runs)):
        if line["line_index"] != i or run["run_index"] != i:
            raise AssertionError("line/run order is not canonical")
        for key in ("frame_node_id", "story_id", "glyph_count", "glyph_sequence_hash"):
            if line[key] != run[key]:
                raise AssertionError(f"line/run mismatch for {key}")
        if line["scalar_start"] != run["scalar_base"]:
            raise AssertionError("run scalar_base must equal Story-global line scalar_start")
        if line["scalar_end"] != run["scalar_end"]:
            raise AssertionError("run scalar_end must equal resolved line scalar_end")
        if line["scalar_end"] < line["scalar_start"]:
            raise AssertionError("invalid scalar range")
        if run["baseline_x"] != 0:
            raise AssertionError("bounded bridge baseline_x must remain zero")

    inv = receipt["invariants"]
    if inv["reshaping_calls"] != 0:
        raise AssertionError("fixed-output bridge must not reshape resolved lines")
    if inv["raw_text_emitted"]:
        raise AssertionError("public receipt must not contain raw text")
    if inv["ascii_gate_applied"]:
        raise AssertionError("bridge-level ASCII gate must be disabled")
    if inv["overset_tail_painted"]:
        raise AssertionError("overset tail must not be painted")
    if not inv["line_order_preserved"] or not inv["story_global_clusters_preserved"]:
        raise AssertionError("resolved line provenance was not preserved")

    return {
        "receipt_kind": receipt["receipt_version"],
        "source_hash": receipt["source_hash"],
        "flow_id": receipt["flow_id"],
        "visible_line_count": len(lines),
        "fixed_run_count": len(runs),
        "story_overset": receipt["story_overset"],
        "reshaping_calls": 0,
        "ascii_gate_applied": False,
        "raw_text_emitted": False,
        "line_order_preserved": True,
        "story_global_clusters_preserved": True,
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_fixed_pdf_shaped_flow_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_schema(receipt)
    print(json.dumps(validate_semantics(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
