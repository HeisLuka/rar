#!/usr/bin/env python3
"""Atomic exact-bounds ResizeNodesV1 for authored page-owned rectangles."""

from __future__ import annotations

import copy
from typing import Literal

from create_shape_v1 import CreateShapeError, validate_rect_emu_v1


class ResizeNodesV1Error(ValueError):
    pass


def _rect(rect: dict, label: str) -> dict:
    try:
        validate_rect_emu_v1(rect, label)
    except CreateShapeError as exc:
        raise ResizeNodesV1Error(str(exc)) from exc
    return rect


def normalize_resize_nodes_request_v1(request: dict) -> dict:
    normalized = copy.deepcopy(request)
    command = normalized.get("command")
    if isinstance(command, dict) and isinstance(command.get("entries"), list):
        command["entries"] = sorted(
            command["entries"],
            key=lambda entry: entry.get("node_id", "")
            if isinstance(entry, dict)
            else "",
        )
    return normalized


def validate_resize_nodes_request_v1(request: dict) -> None:
    if request.get("protocol_version") != "chaptera.resize-nodes-intent.v1":
        raise ResizeNodesV1Error("ResizeNodesV1 protocol_version is required")
    command = request.get("command")
    if (
        not isinstance(command, dict)
        or set(command) != {"kind", "page_id", "entries"}
        or command.get("kind") != "resize_nodes"
    ):
        raise ResizeNodesV1Error("ResizeNodesV1 command is malformed")
    page_id = command.get("page_id")
    if not isinstance(page_id, str) or not page_id:
        raise ResizeNodesV1Error("ResizeNodesV1 page_id is required")
    entries = command.get("entries")
    if not isinstance(entries, list) or len(entries) < 2 or len(entries) > 1024:
        raise ResizeNodesV1Error("ResizeNodesV1 requires 2..1024 entries")

    ids = []
    has_size_change = False
    for index, entry in enumerate(entries):
        if (
            not isinstance(entry, dict)
            or set(entry) != {"node_id", "expected_before", "after"}
        ):
            raise ResizeNodesV1Error(f"ResizeNodesV1 entry[{index}] is malformed")
        node_id = entry.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise ResizeNodesV1Error(f"ResizeNodesV1 entry[{index}].node_id is required")
        ids.append(node_id)
        before = _rect(entry.get("expected_before"), f"entry[{index}].expected_before")
        after = _rect(entry.get("after"), f"entry[{index}].after")
        if before["width"] != after["width"] or before["height"] != after["height"]:
            has_size_change = True

    if len(set(ids)) != len(ids):
        raise ResizeNodesV1Error("ResizeNodesV1 NodeIds must be unique")
    if ids != sorted(ids):
        raise ResizeNodesV1Error("ResizeNodesV1 entries must be normalized by NodeId")
    if not has_size_change:
        raise ResizeNodesV1Error(
            "ResizeNodesV1 must include a genuine size change; translation-only batches belong to MoveNodes"
        )


def _validate_target(shape: dict, *, node_id: str, page_id: str) -> None:
    if not isinstance(shape, dict):
        raise ResizeNodesV1Error(f"ResizeNodesV1 target {node_id!r} is missing")
    if shape.get("kind") != "shape" or shape.get("shape_kind") != "rectangle":
        raise ResizeNodesV1Error("ResizeNodesV1 supports ordinary rectangles only")
    if shape.get("page_id") != page_id or shape.get("parent_id") != page_id:
        raise ResizeNodesV1Error("ResizeNodesV1 target must be direct page-owned")
    if shape.get("transform") != {"kind": "identity"}:
        raise ResizeNodesV1Error("ResizeNodesV1 requires identity-transform rectangles")
    if shape.get("provenance") != {"kind": "author_created"}:
        raise ResizeNodesV1Error("ResizeNodesV1 supports author-created rectangles only")
    _rect(shape.get("bounds"), f"{node_id}.bounds")


def execute_resize_nodes_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    pseudo_request = {
        "protocol_version": "chaptera.resize-nodes-intent.v1",
        "command": copy.deepcopy(command),
    }
    validate_resize_nodes_request_v1(pseudo_request)

    shapes = base_project.get("shapes")
    if not isinstance(shapes, dict):
        raise ResizeNodesV1Error("canonical shapes registry must be an object")
    operations = base_project.get("operations")
    if not isinstance(operations, list):
        raise ResizeNodesV1Error("canonical project operations must be a list")
    page_id = command["page_id"]

    # Full-set preflight. No candidate mutation happens before every member passes.
    canonical_entries = []
    for requested in command["entries"]:
        node_id = requested["node_id"]
        shape = shapes.get(node_id)
        _validate_target(shape, node_id=node_id, page_id=page_id)
        before = copy.deepcopy(shape["bounds"])
        if before != requested["expected_before"]:
            raise ResizeNodesV1Error("stale ResizeNodesV1 before-state")
        canonical_entries.append(
            {
                "node_id": node_id,
                "before": before,
                "after": copy.deepcopy(requested["after"]),
            }
        )

    project = copy.deepcopy(base_project)
    for entry in canonical_entries:
        project["shapes"][entry["node_id"]]["bounds"] = copy.deepcopy(entry["after"])

    operation = {
        "kind": "resize_nodes",
        "page_id": page_id,
        "entries": canonical_entries,
    }
    project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
    return operation, project, [
        {
            "key": "node.geometry.bounds.batch",
            "state": "supported",
            "note": "atomic ResizeNodesV1",
        },
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]


def validate_resize_nodes_operation_v1(command: dict, operation: dict) -> None:
    if (
        not isinstance(operation, dict)
        or set(operation) != {"kind", "page_id", "entries"}
        or operation.get("kind") != "resize_nodes"
        or operation.get("page_id") != command.get("page_id")
    ):
        raise ResizeNodesV1Error("canonical ResizeNodesV1 operation is malformed")
    expected = command.get("entries")
    actual = operation.get("entries")
    if not isinstance(actual, list) or len(actual) != len(expected):
        raise ResizeNodesV1Error("canonical ResizeNodesV1 entry count differs")
    has_size_change = False
    for index, (requested, canonical) in enumerate(zip(expected, actual)):
        if (
            not isinstance(canonical, dict)
            or set(canonical) != {"node_id", "before", "after"}
            or canonical.get("node_id") != requested.get("node_id")
        ):
            raise ResizeNodesV1Error(f"canonical ResizeNodesV1 entry[{index}] is malformed")
        _rect(canonical.get("before"), f"canonical[{index}].before")
        _rect(canonical.get("after"), f"canonical[{index}].after")
        if canonical["before"] != requested["expected_before"]:
            raise ResizeNodesV1Error("canonical ResizeNodesV1 before-state differs from precondition")
        if canonical["after"] != requested["after"]:
            raise ResizeNodesV1Error("canonical ResizeNodesV1 after-state differs from accepted intent")
        if (
            canonical["before"]["width"] != canonical["after"]["width"]
            or canonical["before"]["height"] != canonical["after"]["height"]
        ):
            has_size_change = True
    if not has_size_change:
        raise ResizeNodesV1Error("canonical ResizeNodesV1 is translation-only")


def apply_resize_nodes_operation_state_v1(
    project: dict,
    operation: dict,
    *,
    state: Literal["before", "after"],
) -> dict:
    if state not in {"before", "after"}:
        raise ResizeNodesV1Error("ResizeNodesV1 replay state must be before/after")
    if not isinstance(operation, dict) or operation.get("kind") != "resize_nodes":
        raise ResizeNodesV1Error("ResizeNodesV1 replay operation is malformed")
    out = copy.deepcopy(project)
    shapes = out.get("shapes")
    if not isinstance(shapes, dict):
        raise ResizeNodesV1Error("canonical shapes registry must be an object")
    opposite = "after" if state == "before" else "before"

    # Replay/undo is also all-or-nothing: preflight all exact opposite states first.
    for entry in operation.get("entries", []):
        shape = shapes.get(entry["node_id"])
        _validate_target(shape, node_id=entry["node_id"], page_id=operation["page_id"])
        if shape["bounds"] != entry[opposite]:
            raise ResizeNodesV1Error("ResizeNodesV1 replay pre-state mismatch")
    for entry in operation["entries"]:
        shapes[entry["node_id"]]["bounds"] = copy.deepcopy(entry[state])
    return out
