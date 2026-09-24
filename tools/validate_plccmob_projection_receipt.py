#!/usr/bin/env python3
import json
import pathlib
import sys
from collections import Counter

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "pub-projection" / "v1" / "plccmob-producer-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("PlcCmob receipt schema validation failed\n" + detail)


def validate_semantics(receipt):
    plc = receipt["plc_cmob"]
    relations = receipt["relations"]
    targets = receipt["targets"]

    if plc["declared_count"] != plc["row_count"]:
        raise AssertionError("declared PlcCmob count differs from row_count")
    if plc["row_count"] != len(relations):
        raise AssertionError("row_count differs from emitted relation count")
    if plc["raw_size"] != 16 + 24 * plc["row_count"]:
        raise AssertionError("raw PlcCmob size does not satisfy bounded 16 + 24*N law")

    expected_order = list(range(len(relations)))
    actual_order = [row["source_order"] for row in relations]
    if actual_order != expected_order:
        raise AssertionError("relations are not emitted in exact source order")

    for row in relations:
        if row["cmo_id"] != row["carrier_cmo_id"]:
            raise AssertionError("carrier CmoID does not match PlcCmob row CmoId")

    relation_counts = Counter(row["target_qsid"] for row in relations)
    seen_targets = set()
    for target in targets:
        qsid = target["target_qsid"]
        if qsid in seen_targets:
            raise AssertionError("duplicate target_qsid summary")
        seen_targets.add(qsid)
        if target["relation_count"] != relation_counts.get(qsid, 0):
            raise AssertionError("target relation_count does not match emitted relations")
        if target["object_marker_count"] != target["relation_count"]:
            raise AssertionError("target U+FFFC marker count does not match relation count")

    if set(relation_counts) != seen_targets:
        raise AssertionError("target summaries do not cover every projected target")

    invariants = receipt["invariants"]
    if not invariants["source_parentage_preserved"]:
        raise AssertionError("source parentage must remain preserved")
    if invariants["carrier_reparent_count"] != 0:
        raise AssertionError("carrier nodes must not be reparented")
    if invariants["raw_text_emitted"]:
        raise AssertionError("public receipt must not emit raw source/customer text")
    if not invariants["ordered_relation"]:
        raise AssertionError("ordered relation invariant must be asserted")

    return {
        "receipt_kind": receipt["receipt_version"],
        "producer": receipt["producer"],
        "source_hash": receipt["source_hash"],
        "projection_context_version": receipt["projection_context_version"],
        "declared_count": plc["declared_count"],
        "row_count": plc["row_count"],
        "raw_size": plc["raw_size"],
        "target_count": len(targets),
        "source_order_preserved": True,
        "carrier_cmo_id_matches": True,
        "marker_cardinality_matches": True,
        "source_parentage_preserved": True,
        "raw_text_emitted": False,
        "fail_closed_probe_count": len(receipt["fail_closed_probes"]),
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_plccmob_projection_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    receipt = json.loads(path.read_text(encoding="utf-8"))
    validate_schema(receipt)
    summary = validate_semantics(receipt)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
