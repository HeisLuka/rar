#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any

try:
    import jsonschema
except ImportError as error:
    raise SystemExit("jsonschema==4.23.0 is required") from error

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "packages" / "protocol" / "movenode-diagnostic" / "v1.schema.json"
RECEIPT_VERSION = "chaptera.movenode-diagnostic-receipt.v1"
BLAST_VERSION = "chaptera.operation-blast-radius.v1"
EMU_PER_POINT = Decimal(12700)


class ValidationError(RuntimeError):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_object(path: Path) -> tuple[dict[str, Any], bytes]:
    raw = path.read_bytes()
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValidationError(f"{path} must contain a JSON object")
    return value, raw


def decimal(value: Any, label: str) -> Decimal:
    if not isinstance(value, str):
        raise ValidationError(f"{label} must be a decimal string")
    try:
        return Decimal(value)
    except InvalidOperation as error:
        raise ValidationError(f"{label} invalid decimal") from error


def rect_delta(before: dict[str, Any], after: dict[str, Any]) -> tuple[int, int]:
    if before["width"] != after["width"] or before["height"] != after["height"]:
        raise ValidationError("Chaptera MoveNode changed width/height")
    return after["x"] - before["x"], after["y"] - before["y"]


def native_delta(before: dict[str, Any], after: dict[str, Any]) -> tuple[Decimal, Decimal]:
    if decimal(before["width"], "native.before.width") != decimal(after["width"], "native.after.width"):
        raise ValidationError("native mutation changed width")
    if decimal(before["height"], "native.before.height") != decimal(after["height"], "native.after.height"):
        raise ValidationError("native mutation changed height")
    return (
        decimal(after["left"], "native.after.left") - decimal(before["left"], "native.before.left"),
        decimal(after["top"], "native.after.top") - decimal(before["top"], "native.before.top"),
    )


def geometry_equal(left: dict[str, Any], right: dict[str, Any], label: str) -> None:
    for field in ("left", "top", "width", "height"):
        if decimal(left[field], f"{label}.left.{field}") != decimal(
            right[field], f"{label}.right.{field}"
        ):
            raise ValidationError(f"{label} differs at {field}")


def within_tolerance(actual: int, native_points: Decimal, tolerance_emu: int, axis: str) -> None:
    predicted = native_points * EMU_PER_POINT
    error = abs(Decimal(actual) - predicted)
    if error > Decimal(tolerance_emu):
        raise ValidationError(
            f"{axis} delta mismatch: chaptera={actual} emu native={native_points} pt "
            f"predicted={predicted} emu tolerance={tolerance_emu}"
        )


def validate(receipt: dict[str, Any], blast: dict[str, Any], blast_raw: bytes) -> dict[str, Any]:
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    jsonschema.Draft202012Validator(schema).validate(receipt)

    if receipt["receipt_version"] != RECEIPT_VERSION:
        raise ValidationError("wrong receipt_version")
    if blast.get("schema_version") != BLAST_VERSION:
        raise ValidationError("wrong blast-radius schema_version")

    source_hash = receipt["source_sha256"]
    chaptera = receipt["chaptera"]
    native = receipt["native_experiment"]
    control = native["control"]
    mutation = native["mutation"]
    binding = receipt["blast_radius"]

    if binding["receipt_sha256"] != sha256_bytes(blast_raw):
        raise ValidationError("blast-radius receipt SHA-256 mismatch")
    if binding["schema_version"] != BLAST_VERSION:
        raise ValidationError("blast-radius binding schema mismatch")

    for label, value in (
        ("blast binding source", binding["source_sha256"]),
        ("blast artifact source", blast["artifacts"]["source"]["sha256"]),
        ("control baseline", control["baseline_source_sha256"]),
        ("mutation baseline", mutation["baseline_source_sha256"]),
    ):
        if value != source_hash:
            raise ValidationError(f"{label} does not match immutable source")

    if binding["control_sha256"] != blast["artifacts"]["control"]["sha256"]:
        raise ValidationError("control artifact hash does not match blast radius")
    if binding["mutation_sha256"] != blast["artifacts"]["mutation"]["sha256"]:
        raise ValidationError("mutation artifact hash does not match blast radius")
    if control["first_save_sha256"] != binding["control_sha256"]:
        raise ValidationError("native control first Save hash mismatch")
    if mutation["first_save_sha256"] != binding["mutation_sha256"]:
        raise ValidationError("native mutation first Save hash mismatch")

    blast_operation = blast.get("operation")
    if not isinstance(blast_operation, dict) or blast_operation.get("kind") != "MoveNode":
        raise ValidationError("blast-radius operation is not MoveNode")
    if blast_operation.get("node_id") != chaptera["node_id"]:
        raise ValidationError("blast-radius NodeId does not match Chaptera intent")

    geometry_equal(control["before"], control["after"], "no-op control")
    geometry_equal(control["before"], mutation["before"], "matched baseline geometry")

    dx_emu, dy_emu = rect_delta(chaptera["before"], chaptera["after"])
    native_dx_pt, native_dy_pt = native_delta(mutation["before"], mutation["after"])
    axis = native["axis"]
    tolerance = native["tolerance_emu"]

    if native["emu_per_point"] != 12700:
        raise ValidationError("unexpected EMU/point conversion constant")

    if axis == "x":
        if dx_emu == 0 or dy_emu != 0:
            raise ValidationError("x experiment must be an X-only Chaptera MoveNode")
        if native_dx_pt == 0 or native_dy_pt != 0:
            raise ValidationError("x experiment must be an X-only native mutation")
        within_tolerance(dx_emu, native_dx_pt, tolerance, "x")
    elif axis == "y":
        if dy_emu == 0 or dx_emu != 0:
            raise ValidationError("y experiment must be a Y-only Chaptera MoveNode")
        if native_dy_pt == 0 or native_dx_pt != 0:
            raise ValidationError("y experiment must be a Y-only native mutation")
        within_tolerance(dy_emu, native_dy_pt, tolerance, "y")
    else:
        raise ValidationError("unsupported mutation axis")

    for label, arm in (("control", control), ("mutation", mutation)):
        if arm["parser_accepted"] is not True:
            raise ValidationError(f"{label} artifact rejected by Chaptera parser")
        if arm["publisher_reopen_accepted"] is not True:
            raise ValidationError(f"{label} artifact rejected on Publisher reopen")

    second = blast["second_save_convergence"]
    bound_second = binding.get("second_save_sha256")
    mutation_second = mutation.get("second_save_sha256")
    if second["status"] == "unavailable":
        if bound_second is not None or mutation_second is not None:
            raise ValidationError("second-save hash present but blast convergence unavailable")
    else:
        artifact = second.get("artifact")
        if not isinstance(artifact, dict) or not isinstance(artifact.get("sha256"), str):
            raise ValidationError("blast second-save artifact missing")
        blast_second_hash = artifact["sha256"]
        if bound_second != blast_second_hash or mutation_second != blast_second_hash:
            raise ValidationError("mutation second-save hash mismatch")
        if second["status"] == "converged" and blast_second_hash != mutation["first_save_sha256"]:
            raise ValidationError("converged second Save does not equal first mutation Save")
        if second["status"] == "changed" and blast_second_hash == mutation["first_save_sha256"]:
            raise ValidationError("changed second Save unexpectedly equals first mutation Save")

    invariants = blast.get("invariants", {})
    if invariants.get("matched_noop_control_used") is not True:
        raise ValidationError("blast radius did not use matched no-op control")
    if invariants.get("unexplained_collateral_preserved") is not True:
        raise ValidationError("blast radius did not preserve unexplained collateral")
    if invariants.get("native_pub_writer_capability_granted") is not False:
        raise ValidationError("blast radius improperly grants native writer capability")
    if receipt["invariants"]["native_pub_writer_capability_granted"] is not False:
        raise ValidationError("diagnostic receipt improperly grants native writer capability")

    return {
        "receipt_version": RECEIPT_VERSION,
        "source_sha256": source_hash,
        "axis": axis,
        "chaptera_delta_emu": {"x": dx_emu, "y": dy_emu},
        "native_delta_points": {"x": str(native_dx_pt), "y": str(native_dy_pt)},
        "tolerance_emu": tolerance,
        "blast_radius_sha256": binding["receipt_sha256"],
        "unexplained_collateral_count": blast["classification_counts"][
            "unexplained_collateral"
        ],
        "second_save_convergence": second["status"],
        "valid": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--blast-radius", required=True, type=Path)
    args = parser.parse_args()
    try:
        receipt, _ = load_object(args.receipt)
        blast, blast_raw = load_object(args.blast_radius)
        result = validate(receipt, blast, blast_raw)
    except (
        OSError,
        json.JSONDecodeError,
        jsonschema.ValidationError,
        ValidationError,
        KeyError,
        TypeError,
    ) as error:
        print(f"movenode-diagnostic: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
