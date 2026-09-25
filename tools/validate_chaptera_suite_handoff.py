#!/usr/bin/env python3
"""Validate Chaptera local suite handoff packets and source-free acceptances."""

from __future__ import annotations

import argparse
import json
import pathlib

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "product" / "desktop-suite" / "v1"
PACKET_SCHEMA = BASE / "handoff.schema.json"
ACCEPTANCE_SCHEMA = BASE / "handoff-acceptance.schema.json"


def _schema(path: pathlib.Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(value)
    return value


def _validate(value: dict, schema_path: pathlib.Path, label: str) -> None:
    errors = sorted(
        Draft202012Validator(_schema(schema_path)).iter_errors(value),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError(f"{label} schema validation failed\n{detail}")


def validate_packet(value: dict) -> dict:
    _validate(value, PACKET_SCHEMA, "suite handoff packet")
    target = value["target_product_id"]
    if target == "chaptera.editor":
        expected = ("edit_supported_pub", "reader_supported", "none_observed")
    elif target == "chaptera.rescue":
        expected = ("diagnose_or_recover", "reader_failure_recovery_eligible", "unknown")
    else:
        raise AssertionError(f"unexpected target: {target}")

    actual = (
        value["requested_job"],
        value["context"]["capability"],
        value["context"]["loss_state"],
    )
    if actual != expected:
        raise AssertionError(f"handoff route/context mismatch: expected={expected!r} actual={actual!r}")
    if value["provenance"]["mutable_document_state_included"]:
        raise AssertionError("handoff packet must not transport mutable document state")
    return {
        "kind": "packet",
        "sender": value["sender_product_id"],
        "target": target,
        "requested_job": value["requested_job"],
        "source_sha256": value["source"]["sha256"],
        "local_path_present": True,
        "mutable_document_state_included": False,
    }


def validate_acceptance(value: dict) -> dict:
    _validate(value, ACCEPTANCE_SCHEMA, "suite handoff acceptance")
    expected_job = {
        "chaptera.editor": "edit_supported_pub",
        "chaptera.rescue": "diagnose_or_recover",
    }[value["receiver_product_id"]]
    if value["requested_job"] != expected_job:
        raise AssertionError("receiver/requested_job mismatch")
    if value["source_path_serialized"]:
        raise AssertionError("durable acceptance must remain source-path-free")
    if value["mutable_document_state_received"]:
        raise AssertionError("receiver must not accept mutable document state")
    return {
        "kind": "acceptance",
        "sender": value["sender_product_id"],
        "receiver": value["receiver_product_id"],
        "requested_job": value["requested_job"],
        "source_sha256": value["source_sha256"],
        "source_unchanged": True,
        "source_free": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("kind", choices=("packet", "acceptance"))
    parser.add_argument("path", type=pathlib.Path)
    args = parser.parse_args()

    value = json.loads(args.path.read_text(encoding="utf-8"))
    summary = validate_packet(value) if args.kind == "packet" else validate_acceptance(value)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
