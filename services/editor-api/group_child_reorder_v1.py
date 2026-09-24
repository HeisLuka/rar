#!/usr/bin/env python3
"""Stable set reorder over one authored Group.children lane."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

GroupChildReorderModeV1 = Literal[
    "bring_to_front",
    "send_to_back",
    "bring_forward_one",
    "send_backward_one",
]


class GroupChildReorderError(ValueError):
    pass


@dataclass(frozen=True)
class GroupChildSnapshotV1:
    node_id: str
    parent_group_id: str
    provenance: str = "author_created"


@dataclass(frozen=True)
class ReorderGroupChildrenSetPlanV1:
    parent_group_id: str
    before_children: tuple[str, ...]
    after_children: tuple[str, ...]
    selected_in_lane_order: tuple[str, ...]
    mode: GroupChildReorderModeV1
    status: str


def _validate_id(value: str, label: str) -> None:
    if not isinstance(value, str) or not value:
        raise GroupChildReorderError(f"{label} is required")


def _validate_lane(children: tuple[str, ...]) -> None:
    if not isinstance(children, tuple):
        raise GroupChildReorderError("Group.children must be tuple")
    for index, node_id in enumerate(children):
        _validate_id(node_id, f"children[{index}]")
    if len(set(children)) != len(children):
        raise GroupChildReorderError("Group.children contains duplicate NodeIds")


def _move_one_step(
    before: tuple[str, ...],
    selected: set[str],
    *,
    forward: bool,
) -> tuple[str, ...]:
    lane = list(before)
    if forward:
        # Stable block motion toward front/top (higher index): iterate from front
        # so selected members never leapfrog one another.
        for index in range(len(lane) - 2, -1, -1):
            if lane[index] in selected and lane[index + 1] not in selected:
                lane[index], lane[index + 1] = lane[index + 1], lane[index]
    else:
        # Stable block motion toward back/bottom (lower index): iterate from back.
        for index in range(1, len(lane)):
            if lane[index] in selected and lane[index - 1] not in selected:
                lane[index], lane[index - 1] = lane[index - 1], lane[index]
    return tuple(lane)


def plan_reorder_group_children_set_v1(
    *,
    parent_group_id: str,
    expected_children: tuple[str, ...],
    current_children: tuple[str, ...],
    child_snapshots: tuple[GroupChildSnapshotV1, ...],
    selected_node_ids: tuple[str, ...],
    mode: GroupChildReorderModeV1,
) -> ReorderGroupChildrenSetPlanV1:
    _validate_id(parent_group_id, "parent_group_id")
    _validate_lane(expected_children)
    _validate_lane(current_children)
    if current_children != expected_children:
        raise GroupChildReorderError("stale Group.children list")
    if mode not in {
        "bring_to_front",
        "send_to_back",
        "bring_forward_one",
        "send_backward_one",
    }:
        raise GroupChildReorderError("unsupported Group child reorder mode")
    if not isinstance(child_snapshots, tuple):
        raise GroupChildReorderError("child_snapshots must be tuple")
    by_id = {}
    for index, snapshot in enumerate(child_snapshots):
        if not isinstance(snapshot, GroupChildSnapshotV1):
            raise GroupChildReorderError(
                f"child_snapshots[{index}] must be GroupChildSnapshotV1"
            )
        _validate_id(snapshot.node_id, f"child_snapshots[{index}].node_id")
        _validate_id(snapshot.parent_group_id, f"child_snapshots[{index}].parent_group_id")
        if snapshot.node_id in by_id:
            raise GroupChildReorderError("duplicate child snapshot")
        if snapshot.parent_group_id != parent_group_id:
            raise GroupChildReorderError("mixed-parent Group child set")
        if snapshot.provenance != "author_created":
            raise GroupChildReorderError("source-backed/imported Group child is unsupported")
        by_id[snapshot.node_id] = snapshot

    if set(by_id) != set(current_children):
        raise GroupChildReorderError("snapshot membership disagrees with Group.children")

    if not isinstance(selected_node_ids, tuple) or not selected_node_ids:
        raise GroupChildReorderError("selected_node_ids must be non-empty tuple")
    if len(set(selected_node_ids)) != len(selected_node_ids):
        raise GroupChildReorderError("selected_node_ids contains duplicates")
    selected = set(selected_node_ids)
    if not selected.issubset(set(current_children)):
        raise GroupChildReorderError("selected child is not a direct member")

    normalized = tuple(node_id for node_id in current_children if node_id in selected)
    unselected = tuple(node_id for node_id in current_children if node_id not in selected)

    if mode == "bring_to_front":
        after = unselected + normalized
    elif mode == "send_to_back":
        after = normalized + unselected
    elif mode == "bring_forward_one":
        after = _move_one_step(current_children, selected, forward=True)
    else:
        after = _move_one_step(current_children, selected, forward=False)

    status = "no_change" if after == current_children else "planned"
    return ReorderGroupChildrenSetPlanV1(
        parent_group_id=parent_group_id,
        before_children=current_children,
        after_children=after,
        selected_in_lane_order=normalized,
        mode=mode,
        status=status,
    )
