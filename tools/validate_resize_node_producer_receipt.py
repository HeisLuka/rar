#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "protocol" / "editor-resize" / "v1"
SCHEMA = BASE / "producer-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("ResizeNode producer receipt schema validation failed\n" + detail)


def validate_semantics(receipt):
    commit = receipt["commit"]
    if commit["operation_count_after"] != commit["operation_count_before"] + 1:
        raise AssertionError("ResizeNode commit must append exactly one durable operation")
    if commit["pure_move"] or not commit["size_changed"]:
        raise AssertionError("ResizeNode must change width or height and must not encode a pure move")
    if not (commit["after_width_positive"] and commit["after_height_positive"]):
        raise AssertionError("ResizeNode after-size must stay strictly positive")
    if not commit["bounds_commitments_distinct"]:
        raise AssertionError("ResizeNode before/after commitments must differ")

    target = receipt["target_gate"]
    if target != {
        "direct_page_owned": True,
        "identity_transform": True,
        "original_bounds_valid": True,
        "node_id_redacted": True,
    }:
        raise AssertionError("ResizeNode V1 target gate widened or leaked canonical identity")

    persistence = receipt["persistence"]
    expected_persistence = {
        "project_schema": "pub-editor-v0.5",
        "feature": "node.geometry.bounds",
        "property_path": "node.bounds",
        "format_representability": "lossless",
        "native_pub_writer_state": "writer_blocked",
    }
    if persistence != expected_persistence:
        raise AssertionError("ResizeNode persistence boundary changed")

    if receipt["undo_redo"] != {
        "undo_restores_exact_before": True,
        "redo_restores_exact_after": True,
    }:
        raise AssertionError("ResizeNode exact undo/redo proof is incomplete")

    if receipt["replay"] != {
        "fresh_session_reproduces_after": True,
        "operation_count_preserved": True,
        "legacy_v0_4_rejected": True,
        "stale_before_rejected_transactionally": True,
    }:
        raise AssertionError("ResizeNode replay/schema/stale-state proof is incomplete")

    if receipt["exports"] != {
        "idml_reflects_resized_bounds": True,
        "odg_reflects_resized_bounds": True,
        "source_pub_unchanged": True,
    }:
        raise AssertionError("ResizeNode editable-export/source-immutability proof is incomplete")

    if not all(receipt["negative_probes"].values()):
        raise AssertionError("ResizeNode negative probe set is incomplete")

    if any(receipt["privacy"].values()):
        raise AssertionError("source-free ResizeNode receipt contains or admits private values")

    return {
        "receipt_version": receipt["receipt_version"],
        "operation_contract": receipt["operation_contract"],
        "platform": receipt["build"]["platform"],
        "fixture_kind": receipt["fixture_kind"],
        "one_durable_operation": True,
        "size_change_required": True,
        "signed_origin_supported": commit["position_may_be_signed"],
        "positive_size_enforced": True,
        "undo_redo_exact": True,
        "transactional_replay": True,
        "idml_odg_geometry_persistence": True,
        "native_pub_writer_not_promoted": True,
        "source_free_receipt": True,
    }


def validate_receipt(receipt):
    validate_schema(receipt)
    return validate_semantics(receipt)


def main():
    if len(sys.argv) != 2:
        print("usage: validate_resize_node_producer_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    receipt = json.loads(path.read_text(encoding="utf-8"))
    print(json.dumps(validate_receipt(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
