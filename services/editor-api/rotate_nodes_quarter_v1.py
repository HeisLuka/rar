#!/usr/bin/env python3
"""Atomic multi-node quarter-turn mutation V1.

Composes the existing RotateNodeQuarterTurnV1 law over a normalized set of
Chaptera-authored direct page-owned rectangles. Every member receives the same
signed quarter-turn increment around its own authored-bounds center. The set is
not treated as a rigid aggregate object.
"""

from __future__ import annotations

import copy

from rotate_quarter_v1 import (
    RotateQuarterError,
    apply_rotate_node_quarter_v1,
    authored_bounds_center_v1,
    validate_affine_v1,
)


class RotateNodesQuarterError(RotateQuarterError):
    pass


def validate_rotate_nodes_quarter_intent_v1(command: object) -> None:
    allowed = {
        "kind",
        "page_id",
        "entries",
        "pivot_policy",
        "quarter_turns",
    }
    if (
        not isinstance(command, dict)
        or command.get("kind") != "rotate_nodes_quarter_turn"
        or set(command) != allowed
    ):
        raise RotateNodesQuarterError(
            "RotateNodesQuarterTurn contains non-intent/authoritative fields"
        )

    page_id = command.get("page_id")
    if not isinstance(page_id, str) or not page_id:
        raise RotateNodesQuarterError("RotateNodesQuarterTurn page_id is required")

    entries = command.get("entries")
    if (
        not isinstance(entries, list)
        or len(entries) < 2
        or len(entries) > 1024
    ):
        raise RotateNodesQuarterError(
            "RotateNodesQuarterTurn entries must contain 2..1024 members"
        )

    node_ids: list[str] = []
    for index, entry in enumerate(entries):
        if (
            not isinstance(entry, dict)
            or set(entry) != {"node_id", "expected_before"}
        ):
            raise RotateNodesQuarterError(
                f"RotateNodesQuarterTurn entry[{index}] is malformed"
            )
        node_id = entry.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise RotateNodesQuarterError(
                f"RotateNodesQuarterTurn entry[{index}].node_id is required"
            )
        node_ids.append(node_id)
        validate_affine_v1(
            entry.get("expected_before"),
            f"RotateNodesQuarterTurn entry[{index}].expected_before",
        )

    if len(set(node_ids)) != len(node_ids):
        raise RotateNodesQuarterError(
            "RotateNodesQuarterTurn NodeIds must be unique"
        )
    if node_ids != sorted(node_ids):
        raise RotateNodesQuarterError(
            "RotateNodesQuarterTurn entries must be normalized by NodeId"
        )

    if command.get("pivot_policy") != "per_node_authored_bounds_center":
        raise RotateNodesQuarterError(
            "RotateNodesQuarterTurn pivot_policy must be per_node_authored_bounds_center"
        )

    turns = command.get("quarter_turns")
    if not isinstance(turns, int) or isinstance(turns, bool):
        raise RotateNodesQuarterError(
            "RotateNodesQuarterTurn quarter_turns must be integer"
        )
    if turns % 4 == 0:
        raise RotateNodesQuarterError(
            "full-turn/no-op batch rotation is not a durable edit"
        )


def validate_rotate_nodes_quarter_operation_v1(
    command: dict,
    operation: object,
) -> None:
    validate_rotate_nodes_quarter_intent_v1(command)
    expected_keys = {
        "kind",
        "page_id",
        "entries",
        "pivot_policy",
        "quarter_turns",
    }
    if (
        not isinstance(operation, dict)
        or set(operation) != expected_keys
        or operation.get("kind") != "rotate_nodes_quarter_turn"
    ):
        raise RotateNodesQuarterError(
            "authoritative executor returned malformed RotateNodesQuarterTurn operation"
        )
    if operation.get("page_id") != command.get("page_id"):
        raise RotateNodesQuarterError(
            "canonical RotateNodesQuarterTurn page differs from accepted intent"
        )
    if operation.get("pivot_policy") != command.get("pivot_policy"):
        raise RotateNodesQuarterError(
            "canonical RotateNodesQuarterTurn pivot policy differs from intent"
        )
    if operation.get("quarter_turns") != command["quarter_turns"] % 4:
        raise RotateNodesQuarterError(
            "canonical RotateNodesQuarterTurn quarter_turns differ from intent"
        )

    requested = command["entries"]
    actual = operation.get("entries")
    if not isinstance(actual, list) or len(actual) != len(requested):
        raise RotateNodesQuarterError(
            "canonical RotateNodesQuarterTurn entry count differs from intent"
        )

    actual_ids: list[str] = []
    for index, (expected, member) in enumerate(zip(requested, actual)):
        if (
            not isinstance(member, dict)
            or set(member)
            != {"node_id", "before", "after", "pivot"}
        ):
            raise RotateNodesQuarterError(
                f"canonical RotateNodesQuarterTurn entry[{index}] is malformed"
            )
        if member.get("node_id") != expected["node_id"]:
            raise RotateNodesQuarterError(
                "canonical RotateNodesQuarterTurn NodeId order differs from normalized intent"
            )
        actual_ids.append(member["node_id"])
        before = validate_affine_v1(
            member.get("before"),
            f"canonical RotateNodesQuarterTurn entry[{index}].before",
        )
        after = validate_affine_v1(
            member.get("after"),
            f"canonical RotateNodesQuarterTurn entry[{index}].after",
        )
        expected_before = validate_affine_v1(
            expected["expected_before"],
            f"RotateNodesQuarterTurn entry[{index}].expected_before",
        )
        if before != expected_before:
            raise RotateNodesQuarterError(
                "canonical RotateNodesQuarterTurn before-state differs from precondition"
            )
        if before == after:
            raise RotateNodesQuarterError(
                "canonical RotateNodesQuarterTurn member must change transform"
            )
        pivot = member.get("pivot")
        if (
            not isinstance(pivot, dict)
            or set(pivot) != {"x", "y"}
            or not all(isinstance(pivot.get(axis), str) for axis in ("x", "y"))
        ):
            raise RotateNodesQuarterError(
                "canonical RotateNodesQuarterTurn member pivot is malformed"
            )

    if actual_ids != sorted(actual_ids) or len(set(actual_ids)) != len(actual_ids):
        raise RotateNodesQuarterError(
            "canonical RotateNodesQuarterTurn entries are not uniquely normalized"
        )


def apply_rotate_nodes_quarter_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    validate_rotate_nodes_quarter_intent_v1(command)

    shapes = base_project.get("shapes")
    if not isinstance(shapes, dict):
        raise RotateNodesQuarterError("canonical shapes registry is required")

    page_id = command["page_id"]
    canonical_turns = command["quarter_turns"] % 4
    members: list[dict] = []

    # Preflight every member against the same immutable base project.
    for entry in command["entries"]:
        node_id = entry["node_id"]
        entity = shapes.get(node_id)
        if not isinstance(entity, dict):
            raise RotateNodesQuarterError(
                f"rotate batch target {node_id} is missing"
            )
        if entity.get("page_id") != page_id:
            raise RotateNodesQuarterError(
                "RotateNodesQuarterTurn members must share command.page_id"
            )

        single_command = {
            "kind": "rotate_node_quarter_turn",
            "node_id": node_id,
            "expected_before": copy.deepcopy(entry["expected_before"]),
            "pivot_policy": "authored_bounds_center",
            "quarter_turns": command["quarter_turns"],
        }
        single_operation, _, _ = apply_rotate_node_quarter_v1(
            base_project,
            single_command,
        )
        expected_pivot = authored_bounds_center_v1(entity.get("bounds"))
        if single_operation["pivot"] != expected_pivot:
            raise RotateNodesQuarterError(
                "single-object quarter-turn returned unexpected pivot"
            )
        members.append(
            {
                "node_id": node_id,
                "before": copy.deepcopy(single_operation["before"]),
                "after": copy.deepcopy(single_operation["after"]),
                "pivot": copy.deepcopy(single_operation["pivot"]),
            }
        )

    resulting = copy.deepcopy(base_project)
    for member in members:
        resulting["shapes"][member["node_id"]]["transform"] = {
            "kind": "affine",
            **copy.deepcopy(member["after"]),
        }

    operation = {
        "kind": "rotate_nodes_quarter_turn",
        "page_id": page_id,
        "entries": members,
        "pivot_policy": "per_node_authored_bounds_center",
        "quarter_turns": canonical_turns,
    }
    resulting.setdefault("operations", []).append(copy.deepcopy(operation))

    consequences = [
        {
            "key": "editable_output.affine_transform",
            "state": "supported",
            "note": "all authored member quarter-turn affines are materialized atomically",
        },
        {
            "key": "native_pub_write",
            "state": "unsupported",
            "note": "source PUB remains immutable in V1",
        },
    ]
    return operation, resulting, consequences
