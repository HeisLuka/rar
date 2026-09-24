#!/usr/bin/env python3
"""Reusable atomic Group.children batch lifecycle planning V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

InsertPolicyV1 = Literal["append_block_at_front", "insert_block_after_anchor"]


class GroupChildBatchLifecycleError(ValueError):
    pass


@dataclass(frozen=True)
class GroupChildCandidateV1:
    node_id: str
    parent_group_id: str
    provenance: str = "author_created"


@dataclass(frozen=True)
class GroupChildBatchPlanV1:
    parent_group_id: str
    before_children: tuple[str, ...]
    after_children: tuple[str, ...]
    inserted_node_ids: tuple[str, ...] = ()
    removed_node_ids: tuple[str, ...] = ()
    removed_indices: tuple[int, ...] = ()
    policy: str | None = None
    anchor_node_id: str | None = None
    status: str = "planned"


def _id(value: str, label: str) -> None:
    if not isinstance(value, str) or not value:
        raise GroupChildBatchLifecycleError(f"{label} is required")


def _lane(value: tuple[str, ...], label: str) -> None:
    if not isinstance(value, tuple):
        raise GroupChildBatchLifecycleError(f"{label} must be tuple")
    for index, node_id in enumerate(value):
        _id(node_id, f"{label}[{index}]")
    if len(set(value)) != len(value):
        raise GroupChildBatchLifecycleError(f"{label} contains duplicate NodeIds")


def _validate_candidates(
    candidates: tuple[GroupChildCandidateV1, ...],
    *,
    parent_group_id: str,
) -> tuple[str, ...]:
    if not isinstance(candidates, tuple) or not candidates:
        raise GroupChildBatchLifecycleError("candidates must be a non-empty ordered tuple")
    ids = []
    for index, candidate in enumerate(candidates):
        if not isinstance(candidate, GroupChildCandidateV1):
            raise GroupChildBatchLifecycleError(
                f"candidates[{index}] must be GroupChildCandidateV1"
            )
        _id(candidate.node_id, f"candidates[{index}].node_id")
        _id(candidate.parent_group_id, f"candidates[{index}].parent_group_id")
        if candidate.parent_group_id != parent_group_id:
            raise GroupChildBatchLifecycleError("candidate has wrong parent Group")
        if candidate.provenance != "author_created":
            raise GroupChildBatchLifecycleError("source-backed/imported candidate is unsupported")
        ids.append(candidate.node_id)
    if len(set(ids)) != len(ids):
        raise GroupChildBatchLifecycleError("candidate NodeIds must be unique")
    return tuple(ids)


def plan_group_child_batch_insert_v1(
    *,
    parent_group_id: str,
    expected_children: tuple[str, ...],
    current_children: tuple[str, ...],
    candidates: tuple[GroupChildCandidateV1, ...],
    policy: InsertPolicyV1,
    anchor_node_id: str | None = None,
) -> GroupChildBatchPlanV1:
    _id(parent_group_id, "parent_group_id")
    _lane(expected_children, "expected_children")
    _lane(current_children, "current_children")
    if expected_children != current_children:
        raise GroupChildBatchLifecycleError("stale Group.children list")

    candidate_ids = _validate_candidates(candidates, parent_group_id=parent_group_id)
    if set(candidate_ids) & set(current_children):
        raise GroupChildBatchLifecycleError("candidate already belongs to Group.children")

    if policy == "append_block_at_front":
        if anchor_node_id is not None:
            raise GroupChildBatchLifecycleError("AppendBlockAtFront does not accept anchor")
        after = current_children + candidate_ids
    elif policy == "insert_block_after_anchor":
        _id(anchor_node_id, "anchor_node_id")
        if anchor_node_id not in current_children:
            raise GroupChildBatchLifecycleError("anchor is not a current direct child")
        insert_at = current_children.index(anchor_node_id) + 1
        after = current_children[:insert_at] + candidate_ids + current_children[insert_at:]
    else:
        raise GroupChildBatchLifecycleError("unsupported Group child insert policy")

    return GroupChildBatchPlanV1(
        parent_group_id=parent_group_id,
        before_children=current_children,
        after_children=after,
        inserted_node_ids=candidate_ids,
        policy=policy,
        anchor_node_id=anchor_node_id,
    )


def plan_group_child_batch_remove_v1(
    *,
    parent_group_id: str,
    expected_children: tuple[str, ...],
    current_children: tuple[str, ...],
    candidates: tuple[GroupChildCandidateV1, ...],
) -> GroupChildBatchPlanV1:
    _id(parent_group_id, "parent_group_id")
    _lane(expected_children, "expected_children")
    _lane(current_children, "current_children")
    if expected_children != current_children:
        raise GroupChildBatchLifecycleError("stale Group.children list")

    candidate_ids = _validate_candidates(candidates, parent_group_id=parent_group_id)
    missing = [node_id for node_id in candidate_ids if node_id not in current_children]
    if missing:
        raise GroupChildBatchLifecycleError("remove candidate is not a current direct child")

    selected = set(candidate_ids)
    indices = tuple(index for index, node_id in enumerate(current_children) if node_id in selected)
    removed_in_lane_order = tuple(current_children[index] for index in indices)
    after = tuple(node_id for node_id in current_children if node_id not in selected)

    return GroupChildBatchPlanV1(
        parent_group_id=parent_group_id,
        before_children=current_children,
        after_children=after,
        removed_node_ids=removed_in_lane_order,
        removed_indices=indices,
    )


def restore_group_child_batch_removal_v1(plan: GroupChildBatchPlanV1) -> tuple[str, ...]:
    if not isinstance(plan, GroupChildBatchPlanV1):
        raise GroupChildBatchLifecycleError("plan must be GroupChildBatchPlanV1")
    if not plan.removed_node_ids:
        raise GroupChildBatchLifecycleError("plan has no removal to restore")
    if len(plan.removed_node_ids) != len(plan.removed_indices):
        raise GroupChildBatchLifecycleError("removal plan indices mismatch")

    lane = list(plan.after_children)
    for node_id, index in zip(plan.removed_node_ids, plan.removed_indices):
        if index < 0 or index > len(lane):
            raise GroupChildBatchLifecycleError("invalid removal restore index")
        lane.insert(index, node_id)
    restored = tuple(lane)
    if restored != plan.before_children:
        raise GroupChildBatchLifecycleError("removal inverse does not restore exact prior lane")
    return restored
