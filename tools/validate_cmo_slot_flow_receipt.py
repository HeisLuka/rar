#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = (
    ROOT
    / "packages"
    / "protocol"
    / "pub-projection"
    / "v1"
    / "cmo-slot-flow-producer-receipt.schema.json"
)


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema).iter_errors(receipt),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError("Cmo slot-flow receipt schema validation failed\n" + detail)


def reject_raw_text(value, path="$"):
    if isinstance(value, dict):
        for key, child in value.items():
            lower = key.lower()
            if lower in {"text", "logical_text", "story_text", "carrier_text"}:
                raise AssertionError(f"raw text field is forbidden at {path}.{key}")
            reject_raw_text(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_raw_text(child, f"{path}[{index}]")


def validate_semantics(receipt):
    reject_raw_text(receipt)

    if receipt["story_marker_count"] != receipt["slot_count"]:
        raise AssertionError("U+FFFC marker count must equal admitted slot count")

    host = receipt["host"]
    slots = receipt["visible_slots"]
    last_source_order = -1
    last_scalar = -1
    prior_height = 0
    for visible_index, slot in enumerate(slots):
        if slot["slot_index"] != visible_index:
            raise AssertionError("visible slot indices must remain a canonical prefix")
        if slot["source_order"] <= last_source_order:
            raise AssertionError("Cmo source order was not preserved")
        if slot["scalar_index"] <= last_scalar:
            raise AssertionError("slot scalar order was not preserved")
        expected_before = prior_height + slot["preceding_text_height_emu"]
        if slot["used_height_before_emu"] != expected_before:
            raise AssertionError(
                "slot vertical cursor does not account exactly for intervening shaped-line height"
            )
        if (
            slot["used_height_after_emu"]
            != slot["used_height_before_emu"] + slot["intrinsic_height_emu"]
        ):
            raise AssertionError("slot vertical cursor does not consume intrinsic height")
        if slot["resolved_x_emu"] != 0:
            raise AssertionError("bounded block-slot x origin must remain zero")
        if slot["resolved_y_emu"] != slot["used_height_before_emu"]:
            raise AssertionError("resolved slot y must equal the block-flow cursor")
        if slot["resolved_width_emu"] != slot["intrinsic_width_emu"]:
            raise AssertionError("slot width was scaled")
        if slot["resolved_height_emu"] != slot["intrinsic_height_emu"]:
            raise AssertionError("slot height was scaled")
        if slot["resolved_width_emu"] > host["width_emu"]:
            raise AssertionError("visible slot exceeds host width")
        if slot["used_height_after_emu"] > host["height_emu"]:
            raise AssertionError("visible slot exceeds host height")
        prior_height = slot["used_height_after_emu"]
        last_source_order = slot["source_order"]
        last_scalar = slot["scalar_index"]

    overset = receipt["overset"]
    if overset["story_overset"]:
        if overset["remaining_item_count"] <= 0:
            raise AssertionError("overset requires a non-empty remaining item suffix")
        if overset["remaining_slot_count"] <= 0 and overset["first_nonfitting_kind"] == "object_slot":
            raise AssertionError("object-slot overset must retain at least the failing slot")
        if overset["first_nonfitting_kind"] is None:
            raise AssertionError("overset must identify the first non-fitting item kind")
        if overset["failure_reason"] is None:
            raise AssertionError("overset must identify a bounded failure reason")
    else:
        if any(
            overset[key] is not None
            for key in (
                "first_nonfitting_kind",
                "first_nonfitting_slot_index",
                "first_nonfitting_scalar_index",
                "failure_reason",
            )
        ):
            raise AssertionError("non-overset receipt must not carry failure identity")
        if overset["remaining_item_count"] or overset["remaining_slot_count"]:
            raise AssertionError("non-overset receipt cannot retain a hidden suffix")
        if len(slots) != receipt["slot_count"]:
            raise AssertionError("all admitted slots must be visible when there is no overset")

    inv = receipt["invariants"]
    if not inv["u_fffc_marker_count_preserved"]:
        raise AssertionError("U+FFFC marker provenance was not preserved")
    if not inv["slot_order_preserved"]:
        raise AssertionError("slot order was not preserved")
    if inv["carrier_reparent_count"] != 0:
        raise AssertionError("carrier source parentage must not change")
    if inv["scaling_applied"]:
        raise AssertionError("bounded Cmo slots must not be scaled")
    if inv["skip_to_fit"]:
        raise AssertionError("slot flow must stop at the first non-fitting item")
    if inv["raw_text_emitted"]:
        raise AssertionError("public receipt must not contain raw text")
    if inv["nested_cmo_admitted"]:
        raise AssertionError("nested Cmo flow is outside the V1 admission")

    return {
        "receipt_kind": receipt["receipt_version"],
        "source_hash": receipt["source_hash"],
        "slot_count": receipt["slot_count"],
        "visible_slot_count": len(slots),
        "story_overset": overset["story_overset"],
        "remaining_slot_count": overset["remaining_slot_count"],
        "scaling_applied": False,
        "skip_to_fit": False,
        "raw_text_emitted": False,
    }


def main():
    if len(sys.argv) != 2:
        print("usage: validate_cmo_slot_flow_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_schema(receipt)
    print(json.dumps(validate_semantics(receipt), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
