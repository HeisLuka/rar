#!/usr/bin/env python3
"""Current source-neutral DeleteNode V1 for one authored page-owned Shape.

This is the modern authoritative executor behind the existing
chaptera.delete-node-intent.v1 protocol. It operates on the current
EditorProject shapes registry + AuthoredStackV1 lane rather than the legacy
synthetic nodes/Page.children representation.
"""

from __future__ import annotations

import copy
import hashlib
import json
from typing import Any

from authored_stack_v1 import validate_authored_lane
from create_shape_v1 import validate_uuid7_node_id_v1


class DeleteNodeV1Error(ValueError):
    pass


def _hash_id(value: Any) -> str:
    payload = json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _fail(message: str) -> None:
    raise DeleteNodeV1Error(message)


def _validate_command(command: object) -> dict:
    allowed = {
        "kind",
        "node_id",
        "expected_state_id",
        "expected_parent_id",
        "expected_child_index",
    }
    if (
        not isinstance(command, dict)
        or command.get("kind") != "delete_node"
        or set(command) != allowed
    ):
        _fail("DeleteNode contains non-intent/authoritative fields")

    try:
        validate_uuid7_node_id_v1(command.get("node_id"))
    except ValueError as exc:
        _fail(str(exc))

    parent_id = command.get("expected_parent_id")
    if not isinstance(parent_id, str) or not parent_id:
        _fail("DeleteNode expected_parent_id is required")

    state_id = command.get("expected_state_id")
    if (
        not isinstance(state_id, str)
        or not state_id.startswith("sha256:")
        or len(state_id) != 71
        or any(ch not in "0123456789abcdef" for ch in state_id[7:])
    ):
        _fail("DeleteNode expected_state_id must be sha256:<lowercase hex>")

    index = command.get("expected_child_index")
    if isinstance(index, bool) or not isinstance(index, int) or index < 0:
        _fail("DeleteNode expected_child_index must be a non-negative integer")
    return command


def _require_current_page_owned_shape(
    base_project: dict,
    command: dict,
) -> tuple[dict, list[str], int]:
    node_id = command["node_id"]
    page_id = command["expected_parent_id"]

    pages = base_project.get("pages")
    if not isinstance(pages, dict) or not isinstance(pages.get(page_id), dict):
        _fail("delete_node_page_missing")
    page = pages[page_id]
    if page.get("authoring_enabled") is False:
        _fail("delete_node_page_not_authorable")

    shapes = base_project.get("shapes")
    if not isinstance(shapes, dict):
        _fail("delete_node_shapes_registry_missing")
    entity = shapes.get(node_id)
    if not isinstance(entity, dict):
        _fail("already_deleted_or_missing")

    if (
        entity.get("node_id") != node_id
        or entity.get("kind") != "shape"
        or entity.get("shape_kind") != "rectangle"
        or entity.get("page_id") != page_id
        or entity.get("parent_id") != page_id
        or entity.get("provenance") != {"kind": "author_created"}
        or entity.get("paint", {}).get("provenance") != {"kind": "author_created"}
    ):
        _fail("unsupported_delete_node_class")

    # V1 is deliberately one direct Page-owned authored leaf. Container-owned
    # shapes, TextFrames, picture frames, Groups and source-backed identities
    # remain outside this operation.
    for forbidden in ("story_id", "text_frame_id", "group_id", "source_ref"):
        if entity.get(forbidden) is not None:
            _fail("delete_node_has_unsupported_ownership")

    actual_state_id = _hash_id(entity)
    if actual_state_id != command["expected_state_id"]:
        _fail("stale_delete_node_state")

    authored = base_project.get("authored_stacks")
    if not isinstance(authored, dict):
        _fail("delete_node_authored_stacks_missing")
    lane = authored.get(page_id)
    try:
        validate_authored_lane(lane)
    except ValueError as exc:
        _fail(f"delete_node_authored_lane_invalid: {exc}")
    if lane.count(node_id) != 1:
        _fail("delete_node_target_not_exactly_once_in_authored_stack")
    actual_index = lane.index(node_id)
    if actual_index != command["expected_child_index"]:
        _fail("stale_delete_node_order")

    return entity, list(lane), actual_index


def execute_delete_node_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    command = _validate_command(command)
    if not isinstance(base_project, dict):
        _fail("base project must be object")

    entity, lane_before, index = _require_current_page_owned_shape(
        base_project,
        command,
    )
    node_id = command["node_id"]
    page_id = command["expected_parent_id"]
    lane_after = lane_before[:index] + lane_before[index + 1 :]

    project = copy.deepcopy(base_project)
    project["shapes"] = copy.deepcopy(base_project["shapes"])
    del project["shapes"][node_id]
    project["authored_stacks"] = copy.deepcopy(base_project["authored_stacks"])
    project["authored_stacks"][page_id] = lane_after

    operations = project.get("operations")
    if not isinstance(operations, list):
        _fail("canonical project operations must be list")

    before_entity = copy.deepcopy(entity)
    before_state_id = _hash_id(before_entity)
    operation = {
        "kind": "delete_node",
        "node_id": node_id,
        "before_entity": before_entity,
        "before_state_id": before_state_id,
        "parent_id": page_id,
        "child_index": index,
        "authored_lane_before": lane_before,
        "authored_lane_after": lane_after,
        "provenance": {"kind": "author_created"},
    }
    operations.append(copy.deepcopy(operation))

    return operation, project, [
        {
            "key": "node.delete",
            "state": "supported",
            "note": "intentional_effective_deletion",
        },
        {"key": "container.order", "state": "supported", "note": None},
        {"key": "layout.scene", "state": "invalidated", "note": None},
        {"key": "editable_export", "state": "invalidated", "note": None},
    ]


def validate_delete_node_operation_v1(command: dict, operation: object) -> None:
    _validate_command(command)
    expected = {
        "kind",
        "node_id",
        "before_entity",
        "before_state_id",
        "parent_id",
        "child_index",
        "authored_lane_before",
        "authored_lane_after",
        "provenance",
    }
    if not isinstance(operation, dict) or set(operation) != expected:
        _fail("authoritative executor returned malformed DeleteNode operation")
    if operation.get("kind") != "delete_node":
        _fail("authoritative executor returned non-DeleteNode operation")
    if operation.get("node_id") != command["node_id"]:
        _fail("canonical DeleteNode targets a different node")
    if operation.get("parent_id") != command["expected_parent_id"]:
        _fail("canonical DeleteNode parent differs from expected precondition")
    if operation.get("child_index") != command["expected_child_index"]:
        _fail("canonical DeleteNode child index differs from expected precondition")

    before_entity = operation.get("before_entity")
    if not isinstance(before_entity, dict):
        _fail("canonical DeleteNode before_entity is required")
    before_state_id = _hash_id(before_entity)
    if before_state_id != command["expected_state_id"]:
        _fail("canonical DeleteNode entity state differs from expected precondition")
    if operation.get("before_state_id") != before_state_id:
        _fail("canonical DeleteNode before_state_id is not bound to before_entity")
    if operation.get("provenance") != {"kind": "author_created"}:
        _fail("canonical DeleteNode provenance must be author_created")

    before = operation.get("authored_lane_before")
    after = operation.get("authored_lane_after")
    try:
        validate_authored_lane(before)
        validate_authored_lane(after)
    except ValueError as exc:
        _fail(f"canonical DeleteNode authored lane invalid: {exc}")
    node_id = command["node_id"]
    index = command["expected_child_index"]
    if index >= len(before) or before[index] != node_id or before.count(node_id) != 1:
        _fail("canonical DeleteNode authored lane before violates precondition")
    expected_after = before[:index] + before[index + 1 :]
    if after != expected_after:
        _fail("canonical DeleteNode authored lane after is invalid")
