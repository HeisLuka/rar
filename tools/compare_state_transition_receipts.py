#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import pathlib
from typing import Any

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "packages" / "protocol" / "state-transition" / "v1" / "receipt.schema.json"


def canonical_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def load_schema() -> dict[str, Any]:
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return schema


def validate_receipt(receipt: dict[str, Any]) -> None:
    validator = Draft202012Validator(load_schema())
    errors = sorted(validator.iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError("state transition receipt schema validation failed\n" + detail)


def _known_value(field: dict[str, Any], label: str, reasons: list[dict[str, Any]]) -> Any:
    status = field["status"]
    if status != "known":
        reasons.append(
            {
                "kind": "missing_authority",
                "field": label,
                "status": status,
                "reason": field["reason"],
            }
        )
        return None
    return field["value"]


def compare_receipts(left: dict[str, Any], right: dict[str, Any]) -> dict[str, Any]:
    validate_receipt(left)
    validate_receipt(right)

    not_comparable: list[dict[str, Any]] = []
    divergences: list[dict[str, Any]] = []

    for label, lv, rv in [
        ("receipt_version", left["receipt_version"], right["receipt_version"]),
        ("contract.id", left["contract"]["id"], right["contract"]["id"]),
        ("contract.version", left["contract"]["version"], right["contract"]["version"]),
        ("operation.kind", left["operation"]["kind"], right["operation"]["kind"]),
        ("operation.inputs", left["operation"]["inputs"], right["operation"]["inputs"]),
        (
            "comparison.required_state_fields",
            left["comparison"]["required_state_fields"],
            right["comparison"]["required_state_fields"],
        ),
        (
            "comparison.required_invariants",
            left["comparison"]["required_invariants"],
            right["comparison"]["required_invariants"],
        ),
    ]:
        if canonical_json(lv) != canonical_json(rv):
            not_comparable.append(
                {
                    "kind": "incompatible_contract",
                    "field": label,
                    "left": lv,
                    "right": rv,
                }
            )

    if not_comparable:
        return {
            "comparison_version": "chaptera.state-transition-comparison.v1",
            "status": "not_comparable",
            "reasons": not_comparable,
        }

    required_state = left["comparison"]["required_state_fields"]
    required_invariants = left["comparison"]["required_invariants"]

    comparable_values: list[tuple[str, str, Any, Any]] = []
    for phase in ("before", "after"):
        for field_name in required_state:
            left_fields = left[phase]["fields"]
            right_fields = right[phase]["fields"]
            if field_name not in left_fields:
                not_comparable.append(
                    {
                        "kind": "missing_required_field",
                        "field": f"{phase}.{field_name}",
                        "side": "left",
                    }
                )
                continue
            if field_name not in right_fields:
                not_comparable.append(
                    {
                        "kind": "missing_required_field",
                        "field": f"{phase}.{field_name}",
                        "side": "right",
                    }
                )
                continue
            lv = _known_value(left_fields[field_name], f"{phase}.{field_name}.left", not_comparable)
            rv = _known_value(right_fields[field_name], f"{phase}.{field_name}.right", not_comparable)
            if left_fields[field_name]["status"] == "known" and right_fields[field_name]["status"] == "known":
                comparable_values.append((phase, field_name, lv, rv))

    for field_name in required_invariants:
        if field_name not in left["invariants"]:
            not_comparable.append(
                {
                    "kind": "missing_required_invariant",
                    "field": field_name,
                    "side": "left",
                }
            )
            continue
        if field_name not in right["invariants"]:
            not_comparable.append(
                {
                    "kind": "missing_required_invariant",
                    "field": field_name,
                    "side": "right",
                }
            )
            continue
        lv = _known_value(left["invariants"][field_name], f"invariant.{field_name}.left", not_comparable)
        rv = _known_value(right["invariants"][field_name], f"invariant.{field_name}.right", not_comparable)
        if left["invariants"][field_name]["status"] == "known" and right["invariants"][field_name]["status"] == "known":
            comparable_values.append(("invariant", field_name, lv, rv))

    if not_comparable:
        return {
            "comparison_version": "chaptera.state-transition-comparison.v1",
            "status": "not_comparable",
            "reasons": not_comparable,
        }

    for phase, field_name, lv, rv in comparable_values:
        if canonical_json(lv) != canonical_json(rv):
            divergences.append(
                {
                    "kind": "value_mismatch",
                    "phase": phase,
                    "field": field_name,
                    "left": lv,
                    "right": rv,
                }
            )

    if divergences:
        return {
            "comparison_version": "chaptera.state-transition-comparison.v1",
            "status": "divergent",
            "reasons": divergences,
        }

    return {
        "comparison_version": "chaptera.state-transition-comparison.v1",
        "status": "equivalent",
        "reasons": [],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("left", type=pathlib.Path)
    parser.add_argument("right", type=pathlib.Path)
    args = parser.parse_args()

    left = json.loads(args.left.read_text(encoding="utf-8"))
    right = json.loads(args.right.read_text(encoding="utf-8"))
    print(json.dumps(compare_receipts(left, right), ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
