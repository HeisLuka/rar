#!/usr/bin/env python3
"""Canonical FitGroupToContentsV1 over a bounded public EditorProject fixture.

The operation changes only one authored Group envelope/local parameterization
and its direct children's local rectangles. Effective immediate-parent geometry
of every direct child is invariant.
"""

from __future__ import annotations

import copy
from dataclasses import asdict
from typing import Literal

from authored_group_geometry_v1 import RectEmu, materialize_group_child_rect_v1
from group_refit_plan_v1 import GroupRefitChildV1, plan_group_refit_v1


PROJECT_SCHEMA_V1 = "chaptera.editor-project.group-fit.v1"
GROUP_PROVENANCE_V1 = "chaptera-authored-group-v1"


class FitGroupContentsError(ValueError):
    def __init__(self, message: str, code: str = "invalid_fit_group_contents") -> None:
        super().__init__(message)
        self.code = code


class FitGroupContentsNoOp(FitGroupContentsError):
    def __init__(self, group_id: str) -> None:
        super().__init__("Group already tightly fits current contents", "no_change")
        self.group_id = group_id


def _rect_from_dict(value: dict, label: str) -> RectEmu:
    if not isinstance(value, dict) or set(value) != {"x", "y", "width", "height"}:
        raise FitGroupContentsError(f"{label} must be exact RectEMU")
    rect = RectEmu(value["x"], value["y"], value["width"], value["height"])
    # Shared materializer/refit validators own integer/range validation later.
    if rect.width <= 0 or rect.height <= 0:
        raise FitGroupContentsError(f"{label} must be positive")
    return rect


def _rect_dict(rect: RectEmu) -> dict:
    return {"x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height}


def _contains(outer: RectEmu, inner: RectEmu) -> bool:
    return (
        inner.x >= outer.x
        and inner.y >= outer.y
        and inner.right <= outer.right
        and inner.bottom <= outer.bottom
    )


def _require_project(project: dict) -> None:
    if not isinstance(project, dict):
        raise FitGroupContentsError("EditorProject must be an object")
    if project.get("schema_version") != PROJECT_SCHEMA_V1:
        raise FitGroupContentsError(
            "unsupported EditorProject schema for FitGroupToContentsV1",
            "unsupported_project_schema",
        )
    if not isinstance(project.get("source_hash"), str):
        raise FitGroupContentsError("EditorProject source_hash is required")
    if not isinstance(project.get("operations"), list):
        raise FitGroupContentsError("EditorProject operations must be a list")
    if not isinstance(project.get("groups"), dict) or not isinstance(project.get("nodes"), dict):
        raise FitGroupContentsError("EditorProject groups/nodes maps are required")


def snapshot_group_v1(project: dict, group_id: str) -> dict:
    _require_project(project)
    group = project["groups"].get(group_id)
    if not isinstance(group, dict):
        raise FitGroupContentsError("target Group is missing", "missing_group")
    if group.get("provenance") != GROUP_PROVENANCE_V1:
        raise FitGroupContentsError("target Group provenance is unsupported", "unsupported_group")
    children = group.get("children")
    if not isinstance(children, list) or not children:
        raise FitGroupContentsError("target Group must have direct children", "invalid_group")
    rows = []
    for node_id in children:
        if not isinstance(node_id, str) or not node_id:
            raise FitGroupContentsError("Group child identity is invalid", "invalid_group")
        node = project["nodes"].get(node_id)
        if not isinstance(node, dict):
            raise FitGroupContentsError("Group child is missing", "stale_group")
        if node.get("parent_group_id") != group_id:
            raise FitGroupContentsError("Group child parent link is stale", "stale_group")
        rows.append({"node_id": node_id, "local_rect": copy.deepcopy(node.get("bounds"))})
    return {
        "group_id": group_id,
        "page_id": group.get("page_id"),
        "parent_group_id": group.get("parent_group_id"),
        "provenance": group.get("provenance"),
        "bounds": copy.deepcopy(group.get("bounds")),
        "local_coordinate_space": copy.deepcopy(group.get("local_coordinate_space")),
        "children": rows,
    }


def validate_fit_group_request_v1(request: dict) -> None:
    if request.get("protocol_version") != "chaptera.fit-group-contents-intent.v1":
        raise FitGroupContentsError("FitGroupToContentsV1 protocol_version is required")
    command = request.get("command")
    if (
        not isinstance(command, dict)
        or set(command) != {"kind", "group_id", "expected_group"}
        or command.get("kind") != "fit_group_to_contents"
    ):
        raise FitGroupContentsError("FitGroupToContentsV1 command is malformed")
    if not isinstance(command.get("group_id"), str) or not command["group_id"]:
        raise FitGroupContentsError("FitGroupToContentsV1 group_id is required")
    expected = command.get("expected_group")
    if not isinstance(expected, dict):
        raise FitGroupContentsError("FitGroupToContentsV1 expected_group is required")
    if expected.get("group_id") != command["group_id"]:
        raise FitGroupContentsError("expected_group identity differs from command")


def validate_fit_group_operation_v1(command: dict, operation: dict) -> None:
    if not isinstance(operation, dict) or operation.get("kind") != "fit_group_to_contents":
        raise FitGroupContentsError("canonical FitGroupToContentsV1 operation is malformed")
    if operation.get("group_id") != command.get("group_id"):
        raise FitGroupContentsError("canonical FitGroupToContentsV1 targets a different Group")
    if operation.get("before") != command.get("expected_group"):
        raise FitGroupContentsError("canonical FitGroupToContentsV1 before-state differs from precondition")
    if not isinstance(operation.get("after"), dict):
        raise FitGroupContentsError("canonical FitGroupToContentsV1 after-state is required")


def execute_fit_group_to_contents_v1(
    base_project: dict,
    command: dict,
) -> tuple[dict, dict, list]:
    _require_project(base_project)
    if command.get("kind") != "fit_group_to_contents":
        raise FitGroupContentsError("unsupported fit-group command")

    group_id = command.get("group_id")
    current = snapshot_group_v1(base_project, group_id)
    if current != command.get("expected_group"):
        raise FitGroupContentsError("stale FitGroupToContentsV1 expected state", "stale_group")

    group = base_project["groups"][group_id]
    old_bounds = _rect_from_dict(group.get("bounds"), "group.bounds")
    old_local = _rect_from_dict(group.get("local_coordinate_space"), "group.local_coordinate_space")
    if old_local.x != 0 or old_local.y != 0:
        raise FitGroupContentsError("Group local coordinate origin must be zero", "invalid_group")

    children = tuple(
        GroupRefitChildV1(
            node_id=row["node_id"],
            local_rect=_rect_from_dict(row["local_rect"], f"child[{row['node_id']}].local_rect"),
        )
        for row in current["children"]
    )
    before_effective = {
        child.node_id: materialize_group_child_rect_v1(
            local_coordinate_space=old_local,
            child_local_bounds=child.local_rect,
            current_group_bounds=old_bounds,
        )
        for child in children
    }

    plan = plan_group_refit_v1(
        mode="tight_fit_current_contents",
        group_bounds=old_bounds,
        local_coordinate_space=old_local,
        children=children,
    )

    after_children = {row.node_id: row for row in plan.children}
    for node_id, effective in before_effective.items():
        if after_children[node_id].effective_parent_rect != effective:
            raise FitGroupContentsError("tight fit changed direct-child effective geometry")

    if group.get("parent_group_id") is not None:
        parent = base_project["groups"].get(group["parent_group_id"])
        if not isinstance(parent, dict):
            raise FitGroupContentsError("parent Group is missing", "stale_group")
        parent_local = _rect_from_dict(
            parent.get("local_coordinate_space"),
            "parent.local_coordinate_space",
        )
        if not _contains(parent_local, plan.new_group_bounds):
            raise FitGroupContentsError(
                "tight-fit Group escapes immediate parent local space",
                "invalid_group",
            )

    after_snapshot = copy.deepcopy(current)
    after_snapshot["bounds"] = _rect_dict(plan.new_group_bounds)
    after_snapshot["local_coordinate_space"] = _rect_dict(plan.new_local_coordinate_space)
    after_snapshot["children"] = [
        {"node_id": node_id, "local_rect": _rect_dict(after_children[node_id].local_rect)}
        for node_id in group["children"]
    ]

    if after_snapshot == current:
        raise FitGroupContentsNoOp(group_id)

    project = copy.deepcopy(base_project)
    target_group = project["groups"][group_id]
    target_group["bounds"] = copy.deepcopy(after_snapshot["bounds"])
    target_group["local_coordinate_space"] = copy.deepcopy(after_snapshot["local_coordinate_space"])
    for row in after_snapshot["children"]:
        project["nodes"][row["node_id"]]["bounds"] = copy.deepcopy(row["local_rect"])

    operation = {
        "kind": "fit_group_to_contents",
        "group_id": group_id,
        "before": current,
        "after": after_snapshot,
    }
    project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
    return operation, project, [
        {
            "key": "group.geometry.fit_to_contents",
            "state": "supported",
            "note": "effective child geometry preserved exactly",
        }
    ]


def apply_fit_group_operation_state_v1(
    project: dict,
    operation: dict,
    *,
    state: Literal["before", "after"],
    append_operation: bool = False,
) -> dict:
    """Apply exact operation snapshot for deterministic undo/redo/replay tests."""
    _require_project(project)
    if operation.get("kind") != "fit_group_to_contents" or state not in {"before", "after"}:
        raise FitGroupContentsError("invalid FitGroupToContentsV1 replay operation")
    snap = operation[state]
    group_id = operation["group_id"]
    current = snapshot_group_v1(project, group_id)
    opposite = operation["after" if state == "before" else "before"]
    if current != opposite:
        raise FitGroupContentsError("FitGroupToContentsV1 replay pre-state mismatch", "stale_group")

    out = copy.deepcopy(project)
    group = out["groups"][group_id]
    group["bounds"] = copy.deepcopy(snap["bounds"])
    group["local_coordinate_space"] = copy.deepcopy(snap["local_coordinate_space"])
    for row in snap["children"]:
        out["nodes"][row["node_id"]]["bounds"] = copy.deepcopy(row["local_rect"])
    if append_operation:
        out["operations"] = list(out["operations"]) + [copy.deepcopy(operation)]
    return out
