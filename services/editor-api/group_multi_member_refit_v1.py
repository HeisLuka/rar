#!/usr/bin/env python3
"""Pure one-Group multi-member expand/rebase planner V1.

Several direct children may have proposed effective rectangles in the Group's
immediate-parent coordinate system. The planner computes one expand-only
envelope, rebases every direct child exactly once, preserves all unselected
siblings bit-for-bit, and returns the resulting Group rectangle as the single
candidate for an optional outer ancestor cascade.

No graph mutation, topology, z-order, shrink, independent per-member Group
expansion, or native Publisher semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import AuthoredGroupGeometryError, RectEmu, _validate_rect
from group_refit_plan_v1 import (
    GroupRefitChildResultV1,
    GroupRefitChildV1,
    GroupRefitPlanError,
    _ancestor_status,
    _materialize_children,
    _rebase_exact,
    _union_rects,
    _validate_group_state,
)
from group_transform_chain_v1 import (
    AUTHORED_GROUP_PROVENANCE_V1,
    AuthoredGroupEdgeV1,
)


GroupMultiMemberRefitStatusV1 = Literal[
    "planned",
    "no_envelope_change",
    "ancestor_refit_required",
]


class GroupMultiMemberRefitError(ValueError):
    pass


@dataclass(frozen=True)
class GroupMultiMemberProposalV1:
    node_id: str
    desired_effective_parent_rect: RectEmu


@dataclass(frozen=True)
class GroupMultiMemberRefitPlanV1:
    status: GroupMultiMemberRefitStatusV1
    group_id: str
    page_id: str
    old_group_bounds: RectEmu
    old_local_coordinate_space: RectEmu
    new_group_bounds: RectEmu
    new_local_coordinate_space: RectEmu
    children: tuple[GroupRefitChildResultV1, ...]
    proposed_node_ids: tuple[str, ...]
    ancestor_path_child_candidate: RectEmu


def _fail(message: str) -> None:
    raise GroupMultiMemberRefitError(message)


def _contains(outer: RectEmu, inner: RectEmu) -> bool:
    return (
        inner.x >= outer.x
        and inner.y >= outer.y
        and inner.right <= outer.right
        and inner.bottom <= outer.bottom
    )


def _validate_edge_and_snapshot(
    *,
    group: AuthoredGroupEdgeV1,
    children: tuple[GroupRefitChildV1, ...],
) -> None:
    if not isinstance(group, AuthoredGroupEdgeV1):
        _fail("group must be AuthoredGroupEdgeV1")
    if group.provenance != AUTHORED_GROUP_PROVENANCE_V1:
        _fail("unsupported authored Group provenance")
    if not isinstance(group.group_id, str) or not group.group_id:
        _fail("group_id is required")
    if not isinstance(group.page_id, str) or not group.page_id:
        _fail("page_id is required")

    try:
        _validate_group_state(
            group_bounds=group.bounds_in_parent,
            local_coordinate_space=group.local_coordinate_space,
            children=children,
        )
    except GroupRefitPlanError as exc:
        raise GroupMultiMemberRefitError(str(exc)) from exc

    child_order = tuple(child.node_id for child in children)
    if child_order != group.children:
        _fail("stale Group membership/order snapshot")


def plan_group_multi_member_refit_v1(
    *,
    group: AuthoredGroupEdgeV1,
    children: tuple[GroupRefitChildV1, ...],
    proposals: tuple[GroupMultiMemberProposalV1, ...],
    parent_local_coordinate_space: RectEmu | None = None,
) -> GroupMultiMemberRefitPlanV1:
    """Plan one deterministic expand-only Group rebase for several children."""

    _validate_edge_and_snapshot(group=group, children=children)

    if not isinstance(proposals, tuple) or not proposals:
        _fail("multi-member refit requires a non-empty proposal set")

    current_ids = set(group.children)
    by_id: dict[str, RectEmu] = {}
    for index, proposal in enumerate(proposals):
        if not isinstance(proposal, GroupMultiMemberProposalV1):
            _fail(f"proposals[{index}] must be GroupMultiMemberProposalV1")
        if not isinstance(proposal.node_id, str) or not proposal.node_id:
            _fail(f"proposals[{index}].node_id is required")
        if proposal.node_id in by_id:
            _fail("multi-member proposal NodeIds must be unique")
        if proposal.node_id not in current_ids:
            _fail("proposal target is not a current direct child")
        try:
            _validate_rect(
                proposal.desired_effective_parent_rect,
                f"proposals[{index}].desired_effective_parent_rect",
            )
        except AuthoredGroupGeometryError as exc:
            raise GroupMultiMemberRefitError(str(exc)) from exc
        by_id[proposal.node_id] = proposal.desired_effective_parent_rect

    proposed_node_ids = tuple(sorted(by_id))

    try:
        materialized = _materialize_children(
            group_bounds=group.bounds_in_parent,
            local_coordinate_space=group.local_coordinate_space,
            children=children,
        )
    except GroupRefitPlanError as exc:
        raise GroupMultiMemberRefitError(str(exc)) from exc

    all_fit = all(_contains(group.bounds_in_parent, by_id[node_id]) for node_id in proposed_node_ids)
    if all_fit:
        return GroupMultiMemberRefitPlanV1(
            status="no_envelope_change",
            group_id=group.group_id,
            page_id=group.page_id,
            old_group_bounds=group.bounds_in_parent,
            old_local_coordinate_space=group.local_coordinate_space,
            new_group_bounds=group.bounds_in_parent,
            new_local_coordinate_space=group.local_coordinate_space,
            children=materialized,
            proposed_node_ids=proposed_node_ids,
            ancestor_path_child_candidate=group.bounds_in_parent,
        )

    required: list[GroupRefitChildResultV1] = []
    for child in materialized:
        effective = by_id.get(child.node_id, child.effective_parent_rect)
        required.append(
            GroupRefitChildResultV1(
                node_id=child.node_id,
                local_rect=child.local_rect,
                effective_parent_rect=effective,
            )
        )

    try:
        envelope = _union_rects(
            (group.bounds_in_parent,)
            + tuple(by_id[node_id] for node_id in proposed_node_ids)
        )
    except GroupRefitPlanError as exc:
        raise GroupMultiMemberRefitError(str(exc)) from exc

    if (
        envelope.x > group.bounds_in_parent.x
        or envelope.y > group.bounds_in_parent.y
        or envelope.right < group.bounds_in_parent.right
        or envelope.bottom < group.bounds_in_parent.bottom
    ):
        _fail("multi-member refit must never shrink the Group envelope")

    try:
        rebased = _rebase_exact(
            envelope=envelope,
            required_effective=tuple(required),
        )
        status = _ancestor_status(
            envelope=envelope,
            parent_local_coordinate_space=parent_local_coordinate_space,
        )
    except GroupRefitPlanError as exc:
        raise GroupMultiMemberRefitError(
            "proposed multi-member geometry is not exactly representable: " + str(exc)
        ) from exc

    return GroupMultiMemberRefitPlanV1(
        status=status,
        group_id=group.group_id,
        page_id=group.page_id,
        old_group_bounds=group.bounds_in_parent,
        old_local_coordinate_space=group.local_coordinate_space,
        new_group_bounds=envelope,
        new_local_coordinate_space=RectEmu(0, 0, envelope.width, envelope.height),
        children=rebased,
        proposed_node_ids=proposed_node_ids,
        ancestor_path_child_candidate=envelope,
    )
