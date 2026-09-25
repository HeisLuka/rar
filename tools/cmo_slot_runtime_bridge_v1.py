#!/usr/bin/env python3
"""Bridge typed Cmo projection context into the native Rust slot-flow authority.

The bridge consumes the current resolved graph plus already-resolved shaped-line
metadata. It derives U+FFFC scalar positions locally, pairs them with typed
PlcCmob/Cmo relations in source order, resolves intrinsic carrier extents from
the canonical graph, invokes pub-cmo-slot-flow, and materializes only the
visible projected carrier instances.

Raw Story text is used only for local marker/range validation and is never
emitted in the public-safe result.
"""

from __future__ import annotations

import argparse
import copy
import json
import pathlib
import subprocess
import sys
from collections import defaultdict
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from resolved_graph_scene_bridge_v1 import (  # noqa: E402
    ResolvedGraphSceneError,
    _projection_context,
    require_rect,
    require_uuid,
    source_hash_from_graph,
)

U_FFFC = "\uFFFC"
INPUT_VERSION = "chaptera.cmo-slot-flow-input.v1"
RUNTIME_VERSION = "chaptera.cmo-slot-runtime-bridge.v1"
I64_MIN = -(1 << 63)
I64_MAX = (1 << 63) - 1


class CmoSlotRuntimeError(ValueError):
    pass


def _checked_i64(value: int, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise CmoSlotRuntimeError(f"{label} must be an integer")
    if value < I64_MIN or value > I64_MAX:
        raise CmoSlotRuntimeError(f"{label} exceeds signed 64-bit range")
    return value


def _checked_add(left: int, right: int, label: str) -> int:
    return _checked_i64(left + right, label)


def _node(graph: dict[str, Any], node_id: str, label: str) -> tuple[dict[str, Any], dict[str, Any]]:
    nodes = graph.get("nodes")
    if not isinstance(nodes, dict):
        raise CmoSlotRuntimeError("resolved graph nodes must be an object")
    node = nodes.get(node_id)
    if not isinstance(node, dict):
        raise CmoSlotRuntimeError(f"{label} node missing from current graph")
    header = node.get("header")
    if not isinstance(header, dict):
        raise CmoSlotRuntimeError(f"{label} node header missing")
    if require_uuid(header.get("id"), f"{label}.header.id") != node_id:
        raise CmoSlotRuntimeError(f"{label} node key/id mismatch")
    return node, header


def _story(graph: dict[str, Any], story_id: str, label: str) -> dict[str, Any]:
    stories = graph.get("stories")
    if not isinstance(stories, dict):
        raise CmoSlotRuntimeError("resolved graph stories must be an object")
    story = stories.get(story_id)
    if not isinstance(story, dict):
        raise CmoSlotRuntimeError(f"{label} Story missing from current graph")
    if require_uuid(story.get("id"), f"{label}.id") != story_id:
        raise CmoSlotRuntimeError(f"{label} Story key/id mismatch")
    text = story.get("text")
    if not isinstance(text, str):
        raise CmoSlotRuntimeError(f"{label} Story text missing")
    if any(0xD800 <= ord(ch) <= 0xDFFF for ch in text):
        raise CmoSlotRuntimeError(f"{label} Story contains surrogate code points")
    return story


def _story_frame_nodes(graph: dict[str, Any], story_id: str) -> list[str]:
    nodes = graph.get("nodes")
    if not isinstance(nodes, dict):
        raise CmoSlotRuntimeError("resolved graph nodes must be an object")
    result: list[str] = []
    for node_id, node in nodes.items():
        if not isinstance(node, dict):
            continue
        payload = node.get("payload")
        if not isinstance(payload, dict):
            continue
        frame = payload.get("story_frame")
        if isinstance(frame, dict) and frame.get("story_id") == story_id:
            require_uuid(node_id, "story frame node id")
            result.append(node_id)
    return sorted(result)


def _validate_shaped_flow(shaped_flow: Any, source_hash: str) -> tuple[int, list[dict[str, Any]]]:
    if not isinstance(shaped_flow, dict):
        raise CmoSlotRuntimeError("shaped_flow must be an object")
    expected = {
        "schema_version",
        "source_hash",
        "flow_id",
        "environment",
        "lines",
        "diagnostics",
    }
    if set(shaped_flow) != expected:
        raise CmoSlotRuntimeError("shaped_flow fields mismatch")
    if shaped_flow["schema_version"] != "chaptera.shaped-flow-bridge-input.v1":
        raise CmoSlotRuntimeError("unsupported shaped_flow schema_version")
    if shaped_flow["source_hash"] != source_hash:
        raise CmoSlotRuntimeError("shaped_flow source identity mismatch")
    environment = shaped_flow["environment"]
    if not isinstance(environment, dict) or set(environment) != {
        "font_size_emu",
        "line_height_emu",
    }:
        raise CmoSlotRuntimeError("shaped_flow.environment fields mismatch")
    line_height = _checked_i64(
        environment["line_height_emu"],
        "shaped_flow.environment.line_height_emu",
    )
    if line_height <= 0:
        raise CmoSlotRuntimeError("shaped_flow line height must be positive")
    lines = shaped_flow["lines"]
    if not isinstance(lines, list):
        raise CmoSlotRuntimeError("shaped_flow.lines must be an array")
    if not isinstance(shaped_flow["diagnostics"], list):
        raise CmoSlotRuntimeError("shaped_flow.diagnostics must be an array")
    return line_height, lines


def _target_items(
    *,
    graph: dict[str, Any],
    target_story_id: str,
    target_frame_node_id: str,
    relations: list[dict[str, Any]],
    shaped_lines: list[dict[str, Any]],
    line_height_emu: int,
) -> tuple[list[dict[str, Any]], int, dict[str, int], str]:
    story = _story(graph, target_story_id, "target")
    text = story["text"]
    marker_positions = [index for index, ch in enumerate(text) if ch == U_FFFC]

    ordered_relations = sorted(relations, key=lambda relation: relation["source_order"])
    if len(marker_positions) != len(ordered_relations):
        raise CmoSlotRuntimeError(
            f"target Story marker count {len(marker_positions)} != Cmo relation count {len(ordered_relations)}"
        )
    if not marker_positions:
        raise CmoSlotRuntimeError("Cmo target Story has no U+FFFC markers")

    frame_nodes = _story_frame_nodes(graph, target_story_id)
    if frame_nodes != [target_frame_node_id]:
        raise CmoSlotRuntimeError(
            "bounded Cmo slot-flow V1 requires exactly one target Story frame"
        )

    target_node, target_header = _node(
        graph,
        target_frame_node_id,
        "target frame",
    )
    payload = target_node.get("payload")
    frame = payload.get("story_frame") if isinstance(payload, dict) else None
    if not isinstance(frame, dict) or frame.get("story_id") != target_story_id:
        raise CmoSlotRuntimeError("target frame does not own target Story")
    host = require_rect(target_header.get("bounds"), "target frame bounds")
    target_page_id = require_uuid(
        target_header.get("parent_id"),
        "target frame parent_id",
    )

    covered: set[int] = set()
    line_items: list[tuple[int, dict[str, Any]]] = []
    for index, line in enumerate(shaped_lines):
        if not isinstance(line, dict):
            raise CmoSlotRuntimeError(f"shaped_flow.lines[{index}] must be an object")
        if line.get("story_id") != target_story_id:
            continue
        if line.get("frame_node_id") != target_frame_node_id:
            raise CmoSlotRuntimeError(
                f"shaped_flow.lines[{index}] Cmo target uses a different frame"
            )
        start = line.get("scalar_start")
        end = line.get("scalar_end")
        consumed = line.get("consumed_scalar_end")
        if (
            isinstance(start, bool)
            or not isinstance(start, int)
            or isinstance(end, bool)
            or not isinstance(end, int)
            or isinstance(consumed, bool)
            or not isinstance(consumed, int)
            or start < 0
            or end < start
            or consumed < end
            or consumed > len(text)
        ):
            raise CmoSlotRuntimeError(
                f"shaped_flow.lines[{index}] scalar range invalid"
            )
        if any(position in marker_positions for position in range(start, consumed)):
            raise CmoSlotRuntimeError(
                f"shaped_flow.lines[{index}] crosses a U+FFFC object-slot marker"
            )
        visible = line.get("text")
        if not isinstance(visible, str) or text[start:end] != visible:
            raise CmoSlotRuntimeError(
                f"shaped_flow.lines[{index}] text is stale relative to current Story"
            )
        for scalar_index in range(start, consumed):
            if scalar_index in covered:
                raise CmoSlotRuntimeError("Cmo target shaped-line scalar coverage overlaps")
            covered.add(scalar_index)
        line_items.append(
            (
                start,
                {
                    "kind": "shaped_line",
                    "scalar_start": start,
                    "scalar_end": end,
                    "consumed_scalar_end": consumed,
                    "height_emu": line_height_emu,
                },
            )
        )

    expected_covered = {
        index for index, ch in enumerate(text) if ch != U_FFFC
    }
    if covered != expected_covered:
        missing = sorted(expected_covered - covered)
        extra = sorted(covered - expected_covered)
        raise CmoSlotRuntimeError(
            f"Cmo target shaped-line coverage incomplete: missing={missing} extra={extra}"
        )

    slot_items: list[tuple[int, dict[str, Any]]] = []
    seen_orders: set[int] = set()
    for slot_index, (scalar_index, relation) in enumerate(
        zip(marker_positions, ordered_relations)
    ):
        source_order = relation["source_order"]
        if source_order in seen_orders:
            raise CmoSlotRuntimeError("duplicate Cmo source_order for target Story")
        seen_orders.add(source_order)
        carrier_id = relation["carrier_node_id"]
        _, carrier_header = _node(graph, carrier_id, f"Cmo carrier[{slot_index}]")
        carrier_bounds = require_rect(
            carrier_header.get("bounds"),
            f"Cmo carrier[{slot_index}] bounds",
        )
        carrier_story_id = relation["carrier_story_id"]
        if carrier_story_id is not None:
            _story(graph, carrier_story_id, f"Cmo carrier[{slot_index}]")
        slot_items.append(
            (
                scalar_index,
                {
                    "kind": "object_slot",
                    "slot_index": slot_index,
                    "scalar_index": scalar_index,
                    "source_order": source_order,
                    "cmo_id": relation["cmo_id"],
                    "carrier_node_id": carrier_id,
                    "carrier_story_id": carrier_story_id,
                    "intrinsic_width_emu": carrier_bounds["width"],
                    "intrinsic_height_emu": carrier_bounds["height"],
                },
            )
        )

    ordered = line_items + slot_items
    ordered.sort(key=lambda pair: (pair[0], 0 if pair[1]["kind"] == "shaped_line" else 1))
    return (
        [item for _, item in ordered],
        len(marker_positions),
        {"width_emu": host["width"], "height_emu": host["height"]},
        target_page_id,
    )


def _invoke_native_slot_flow(value: dict[str, Any]) -> dict[str, Any]:
    command = [
        "cargo",
        "run",
        "-q",
        "-p",
        "pub-cmo-slot-flow",
        "--bin",
        "cmo-slot-flow-packet",
    ]
    completed = subprocess.run(
        command,
        cwd=ROOT,
        input=json.dumps(value, ensure_ascii=False, separators=(",", ":")),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip()
        raise CmoSlotRuntimeError(
            "native Cmo slot-flow rejected current projection"
            + (f": {detail}" if detail else "")
        )
    try:
        receipt = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise CmoSlotRuntimeError("native Cmo slot-flow returned invalid JSON") from error
    if not isinstance(receipt, dict):
        raise CmoSlotRuntimeError("native Cmo slot-flow receipt must be an object")
    return receipt


def _materialize_instance(
    *,
    graph: dict[str, Any],
    receipt: dict[str, Any],
    slot: dict[str, Any],
    target_page_id: str,
) -> dict[str, Any]:
    target_frame_id = receipt["target_frame_node_id"]
    _, target_header = _node(graph, target_frame_id, "target frame")
    host = require_rect(target_header.get("bounds"), "target frame bounds")
    _, carrier_header = _node(
        graph,
        slot["carrier_node_id"],
        "Cmo carrier",
    )
    source_parent = require_uuid(
        carrier_header.get("parent_id"),
        "Cmo carrier source parent",
    )
    x = _checked_add(host["x"], slot["resolved_x_emu"], "Cmo projected x")
    y = _checked_add(host["y"], slot["resolved_y_emu"], "Cmo projected y")
    return {
        "origin": slot["carrier_node_id"],
        "parent_origin": target_page_id,
        "bounds": {
            "x": x,
            "y": y,
            "width": slot["resolved_width_emu"],
            "height": slot["resolved_height_emu"],
        },
        "transform": {
            "a": "1",
            "b": "0",
            "c": "0",
            "d": "1",
            "tx": 0,
            "ty": 0,
        },
        "instance_id": slot["instance_id"],
        "projection_kind": "cmo_story_slot",
        "source_parent_origin": source_parent,
        "target_story_origin": receipt["target_story_id"],
        "target_frame_origin": target_frame_id,
        "scalar_index": slot["scalar_index"],
        "source_order": slot["source_order"],
        "cmo_id": slot["cmo_id"],
        "carrier_story_origin": slot["carrier_story_id"],
    }


def build_cmo_slot_runtime_v1(
    *,
    resolved_graph: dict[str, Any],
    projection_context: dict[str, Any] | None,
    shaped_flow: dict[str, Any],
    implementation: str,
    commit_or_build: str,
) -> dict[str, Any]:
    source_hash = source_hash_from_graph(resolved_graph)
    try:
        context = _projection_context(projection_context)
    except ResolvedGraphSceneError as error:
        raise CmoSlotRuntimeError(str(error)) from error

    relations = context["cmo_relations"]
    if not relations:
        return {
            "runtime_version": RUNTIME_VERSION,
            "source_hash": source_hash,
            "receipts": [],
            "scene_instances": [],
            "story_overset": False,
        }

    line_height, shaped_lines = _validate_shaped_flow(shaped_flow, source_hash)
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for relation in relations:
        frame_id = relation["target_frame_node_id"]
        if frame_id is None:
            raise CmoSlotRuntimeError(
                "Cmo relation target_frame_node_id is unresolved"
            )
        grouped[(relation["target_story_id"], frame_id)].append(relation)

    receipts: list[dict[str, Any]] = []
    instances: list[dict[str, Any]] = []
    for (story_id, frame_id), target_relations in sorted(grouped.items()):
        target_qsids = {relation["target_qsid"] for relation in target_relations}
        if len(target_qsids) != 1:
            raise CmoSlotRuntimeError(
                "one target Story maps to multiple target_qsid values"
            )
        items, marker_count, host, target_page_id = _target_items(
            graph=resolved_graph,
            target_story_id=story_id,
            target_frame_node_id=frame_id,
            relations=target_relations,
            shaped_lines=shaped_lines,
            line_height_emu=line_height,
        )
        native_input = {
            "schema_version": INPUT_VERSION,
            "producer": {
                "implementation": implementation,
                "commit_or_build": commit_or_build,
                "core_integration": True,
            },
            "source_hash": source_hash,
            "target_story_id": story_id,
            "target_frame_node_id": frame_id,
            "target_frame_count": 1,
            "story_marker_count": marker_count,
            "host": host,
            "items": items,
        }
        receipt = _invoke_native_slot_flow(native_input)
        receipts.append(receipt)
        for slot in receipt.get("visible_slots", []):
            if not isinstance(slot, dict):
                raise CmoSlotRuntimeError("native visible slot must be an object")
            instances.append(
                _materialize_instance(
                    graph=resolved_graph,
                    receipt=receipt,
                    slot=slot,
                    target_page_id=target_page_id,
                )
            )

    instances.sort(
        key=lambda item: (
            item["parent_origin"],
            item["target_story_origin"],
            item["scalar_index"],
            item["instance_id"],
        )
    )
    receipts.sort(
        key=lambda item: (
            item["target_story_id"],
            item["target_frame_node_id"],
        )
    )
    return {
        "runtime_version": RUNTIME_VERSION,
        "source_hash": source_hash,
        "receipts": receipts,
        "scene_instances": instances,
        "story_overset": any(
            receipt.get("overset", {}).get("story_overset") is True
            for receipt in receipts
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument(
        "--implementation",
        default="rar-cmo-slot-runtime-bridge-v1",
    )
    parser.add_argument("--commit-or-build", required=True)
    args = parser.parse_args()
    try:
        value = json.loads(args.input.read_text(encoding="utf-8"))
        if not isinstance(value, dict) or set(value) != {
            "resolved_graph",
            "projection_context",
            "shaped_flow",
        }:
            raise CmoSlotRuntimeError("runtime input fields mismatch")
        result = build_cmo_slot_runtime_v1(
            resolved_graph=value["resolved_graph"],
            projection_context=value["projection_context"],
            shaped_flow=value["shaped_flow"],
            implementation=args.implementation,
            commit_or_build=args.commit_or_build,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(
            json.dumps(
                {
                    "status": "valid",
                    "receipt_count": len(result["receipts"]),
                    "visible_slot_count": len(result["scene_instances"]),
                    "story_overset": result["story_overset"],
                },
                sort_keys=True,
            )
        )
        return 0
    except (
        OSError,
        json.JSONDecodeError,
        CmoSlotRuntimeError,
        ResolvedGraphSceneError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
