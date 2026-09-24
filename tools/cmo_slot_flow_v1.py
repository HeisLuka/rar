#!/usr/bin/env python3
"""Source-free bounded Cmo/U+FFFC object-slot flow reference producer.

This module is deliberately downstream of the typed PlcCmob projection context
and upstream of native rendering. It models only the admitted single-frame
block-slot law required by CMO-SLOT-FLOW-01:

* object slots preserve their Story-global U+FFFC scalar index;
* carrier identity/extent remain independent from the target Story;
* text-line and object-slot heights share one checked vertical EMU cursor;
* carrier width must fit the host frame without scaling;
* the first non-fitting item establishes overset and later items are never
  searched for something smaller that happens to fit;
* carriers are projected visually, never reparented semantically.

No raw Story text is accepted or emitted.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
from typing import Any

INPUT_VERSION = "chaptera.cmo-slot-flow-input.v1"
RECEIPT_VERSION = "chaptera.cmo-slot-flow-receipt.v1"
UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
TOKEN_RE = re.compile(r"^[A-Za-z0-9._:-]+$")
I64_MAX = (1 << 63) - 1


class CmoSlotFlowError(ValueError):
    pass


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def hash_id(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise CmoSlotFlowError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise CmoSlotFlowError(
            f"{label} fields mismatch: missing={sorted(expected-actual)} extra={sorted(actual-expected)}"
        )
    return value


def require_int(value: Any, label: str, *, minimum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise CmoSlotFlowError(f"{label} must be an integer")
    if minimum is not None and value < minimum:
        raise CmoSlotFlowError(f"{label} must be >= {minimum}")
    if value > I64_MAX:
        raise CmoSlotFlowError(f"{label} exceeds signed 64-bit range")
    return value


def require_uuid(value: Any, label: str) -> str:
    if not isinstance(value, str) or not UUID_RE.fullmatch(value):
        raise CmoSlotFlowError(f"{label} must be canonical lowercase UUID")
    return value


def require_source_hash(value: Any) -> str:
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        raise CmoSlotFlowError("source_hash must be lowercase SHA-256")
    return value


def require_token(value: Any, label: str, *, max_len: int) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value) > max_len
        or not TOKEN_RE.fullmatch(value)
    ):
        raise CmoSlotFlowError(f"{label} is not a bounded identifier")
    return value


def checked_add(left: int, right: int, label: str) -> int:
    value = left + right
    if value > I64_MAX:
        raise CmoSlotFlowError(f"{label} overflow")
    return value


def validate_producer(value: Any) -> dict[str, Any]:
    value = require_exact_keys(
        value,
        {"implementation", "commit_or_build", "core_integration"},
        "producer",
    )
    if value["core_integration"] is not True:
        raise CmoSlotFlowError("producer.core_integration must be true")
    return {
        "implementation": require_token(
            value["implementation"], "producer.implementation", max_len=128
        ),
        "commit_or_build": require_token(
            value["commit_or_build"], "producer.commit_or_build", max_len=160
        ),
        "core_integration": True,
    }


def validate_input(value: Any) -> dict[str, Any]:
    value = require_exact_keys(
        value,
        {
            "schema_version",
            "producer",
            "source_hash",
            "target_story_id",
            "target_frame_node_id",
            "target_frame_count",
            "story_marker_count",
            "host",
            "items",
        },
        "input",
    )
    if value["schema_version"] != INPUT_VERSION:
        raise CmoSlotFlowError("input schema_version mismatch")

    target_frame_count = require_int(
        value["target_frame_count"], "target_frame_count", minimum=1
    )
    if target_frame_count != 1:
        raise CmoSlotFlowError(
            "bounded Cmo slot-flow V1 admits exactly one target frame"
        )

    host = require_exact_keys(value["host"], {"width_emu", "height_emu"}, "host")
    host_width = require_int(host["width_emu"], "host.width_emu", minimum=1)
    host_height = require_int(host["height_emu"], "host.height_emu", minimum=1)

    items = value["items"]
    if not isinstance(items, list):
        raise CmoSlotFlowError("items must be an array")

    normalized_items: list[dict[str, Any]] = []
    slot_count = 0
    last_slot_scalar = -1
    last_source_order = -1
    for index, raw in enumerate(items):
        label = f"items[{index}]"
        if not isinstance(raw, dict):
            raise CmoSlotFlowError(f"{label} must be an object")
        kind = raw.get("kind")
        if kind == "shaped_line":
            raw = require_exact_keys(
                raw,
                {
                    "kind",
                    "scalar_start",
                    "scalar_end",
                    "consumed_scalar_end",
                    "height_emu",
                },
                label,
            )
            start = require_int(raw["scalar_start"], f"{label}.scalar_start", minimum=0)
            end = require_int(raw["scalar_end"], f"{label}.scalar_end", minimum=0)
            consumed = require_int(
                raw["consumed_scalar_end"],
                f"{label}.consumed_scalar_end",
                minimum=0,
            )
            if end < start or consumed < end:
                raise CmoSlotFlowError(f"{label} has invalid scalar range")
            normalized_items.append(
                {
                    "kind": "shaped_line",
                    "scalar_start": start,
                    "scalar_end": end,
                    "consumed_scalar_end": consumed,
                    "height_emu": require_int(
                        raw["height_emu"], f"{label}.height_emu", minimum=1
                    ),
                }
            )
            continue

        if kind != "object_slot":
            raise CmoSlotFlowError(f"{label}.kind is unsupported")
        raw = require_exact_keys(
            raw,
            {
                "kind",
                "slot_index",
                "scalar_index",
                "source_order",
                "cmo_id",
                "carrier_node_id",
                "carrier_story_id",
                "intrinsic_width_emu",
                "intrinsic_height_emu",
            },
            label,
        )
        slot_index = require_int(raw["slot_index"], f"{label}.slot_index", minimum=0)
        if slot_index != slot_count:
            raise CmoSlotFlowError(
                f"{label}.slot_index {slot_index} is not canonical ordinal {slot_count}"
            )
        scalar_index = require_int(
            raw["scalar_index"], f"{label}.scalar_index", minimum=0
        )
        source_order = require_int(
            raw["source_order"], f"{label}.source_order", minimum=0
        )
        if scalar_index <= last_slot_scalar:
            raise CmoSlotFlowError("object-slot scalar indices must be strictly increasing")
        if source_order <= last_source_order:
            raise CmoSlotFlowError("Cmo source_order must be strictly increasing per target")
        last_slot_scalar = scalar_index
        last_source_order = source_order

        carrier_story_id = raw["carrier_story_id"]
        if carrier_story_id is not None:
            carrier_story_id = require_uuid(
                carrier_story_id, f"{label}.carrier_story_id"
            )

        normalized_items.append(
            {
                "kind": "object_slot",
                "slot_index": slot_index,
                "scalar_index": scalar_index,
                "source_order": source_order,
                "cmo_id": require_int(raw["cmo_id"], f"{label}.cmo_id", minimum=1),
                "carrier_node_id": require_uuid(
                    raw["carrier_node_id"], f"{label}.carrier_node_id"
                ),
                "carrier_story_id": carrier_story_id,
                "intrinsic_width_emu": require_int(
                    raw["intrinsic_width_emu"],
                    f"{label}.intrinsic_width_emu",
                    minimum=1,
                ),
                "intrinsic_height_emu": require_int(
                    raw["intrinsic_height_emu"],
                    f"{label}.intrinsic_height_emu",
                    minimum=1,
                ),
            }
        )
        slot_count += 1

    marker_count = require_int(
        value["story_marker_count"], "story_marker_count", minimum=0
    )
    if marker_count != slot_count:
        raise CmoSlotFlowError(
            f"story_marker_count {marker_count} != object slot count {slot_count}"
        )

    return {
        "schema_version": INPUT_VERSION,
        "producer": validate_producer(value["producer"]),
        "source_hash": require_source_hash(value["source_hash"]),
        "target_story_id": require_uuid(value["target_story_id"], "target_story_id"),
        "target_frame_node_id": require_uuid(
            value["target_frame_node_id"], "target_frame_node_id"
        ),
        "target_frame_count": 1,
        "story_marker_count": marker_count,
        "host": {"width_emu": host_width, "height_emu": host_height},
        "items": normalized_items,
    }


def build_receipt(value: Any) -> dict[str, Any]:
    input_value = validate_input(value)
    host_width = input_value["host"]["width_emu"]
    host_height = input_value["host"]["height_emu"]

    used_height = 0
    visible_slots: list[dict[str, Any]] = []
    first_nonfitting_index: int | None = None
    first_nonfitting_kind: str | None = None
    first_nonfitting_slot_index: int | None = None
    first_nonfitting_scalar_index: int | None = None
    failure_reason: str | None = None

    for item_index, item in enumerate(input_value["items"]):
        if item["kind"] == "shaped_line":
            next_height = checked_add(
                used_height, item["height_emu"], "shaped-line vertical cursor"
            )
            if next_height > host_height:
                first_nonfitting_index = item_index
                first_nonfitting_kind = "shaped_line"
                first_nonfitting_scalar_index = item["scalar_start"]
                failure_reason = "height"
                break
            used_height = next_height
            continue

        width_fits = item["intrinsic_width_emu"] <= host_width
        next_height = checked_add(
            used_height, item["intrinsic_height_emu"], "object-slot vertical cursor"
        )
        height_fits = next_height <= host_height
        if not width_fits or not height_fits:
            first_nonfitting_index = item_index
            first_nonfitting_kind = "object_slot"
            first_nonfitting_slot_index = item["slot_index"]
            first_nonfitting_scalar_index = item["scalar_index"]
            if not width_fits and not height_fits:
                failure_reason = "width_and_height"
            elif not width_fits:
                failure_reason = "width"
            else:
                failure_reason = "height"
            break

        instance_id = hash_id(
            {
                "projection_kind": "cmo_story_slot",
                "target_story_id": input_value["target_story_id"],
                "target_frame_node_id": input_value["target_frame_node_id"],
                "scalar_index": item["scalar_index"],
                "carrier_node_id": item["carrier_node_id"],
                "source_order": item["source_order"],
            }
        )
        visible_slots.append(
            {
                "slot_index": item["slot_index"],
                "scalar_index": item["scalar_index"],
                "source_order": item["source_order"],
                "cmo_id": item["cmo_id"],
                "carrier_node_id": item["carrier_node_id"],
                "carrier_story_id": item["carrier_story_id"],
                "instance_id": instance_id,
                "used_height_before_emu": used_height,
                "used_height_after_emu": next_height,
                "intrinsic_width_emu": item["intrinsic_width_emu"],
                "intrinsic_height_emu": item["intrinsic_height_emu"],
                "resolved_x_emu": 0,
                "resolved_y_emu": used_height,
                "resolved_width_emu": item["intrinsic_width_emu"],
                "resolved_height_emu": item["intrinsic_height_emu"],
            }
        )
        used_height = next_height

    items = input_value["items"]
    if first_nonfitting_index is None:
        remaining_item_count = 0
        remaining_slot_count = 0
    else:
        tail = items[first_nonfitting_index:]
        remaining_item_count = len(tail)
        remaining_slot_count = sum(item["kind"] == "object_slot" for item in tail)

    story_overset = first_nonfitting_index is not None
    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "producer": input_value["producer"],
        "source_hash": input_value["source_hash"],
        "target_story_id": input_value["target_story_id"],
        "target_frame_node_id": input_value["target_frame_node_id"],
        "host": input_value["host"],
        "story_marker_count": input_value["story_marker_count"],
        "slot_count": input_value["story_marker_count"],
        "visible_slots": visible_slots,
        "overset": {
            "story_overset": story_overset,
            "first_nonfitting_kind": first_nonfitting_kind,
            "first_nonfitting_slot_index": first_nonfitting_slot_index,
            "first_nonfitting_scalar_index": first_nonfitting_scalar_index,
            "failure_reason": failure_reason,
            "remaining_item_count": remaining_item_count,
            "remaining_slot_count": remaining_slot_count,
        },
        "invariants": {
            "u_fffc_marker_count_preserved": True,
            "slot_order_preserved": True,
            "carrier_reparent_count": 0,
            "scaling_applied": False,
            "skip_to_fit": False,
            "raw_text_emitted": False,
            "nested_cmo_admitted": False,
        },
    }
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=pathlib.Path)
    args = parser.parse_args()
    try:
        value = json.loads(args.input.read_text(encoding="utf-8"))
        print(json.dumps(build_receipt(value), indent=2, sort_keys=True))
        return 0
    except (OSError, json.JSONDecodeError, CmoSlotFlowError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
