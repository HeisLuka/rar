#!/usr/bin/env python3
"""Bottom-up authored Group ancestor refit cascade planner V1.

The input is an immutable valid authored Group path ordered outermost ->
immediate parent Group plus one desired target rectangle in page coordinates.
The planner:
1. converts the desired page rectangle into the deepest Group's immediate-
   parent coordinate system using the shared exact rational edge law;
2. runs the existing expand-only GroupRefitPlanV1 at the deepest Group;
3. when that envelope escapes its parent local space, propagates one changed
   path-child rectangle bottom-up, refitting only the necessary ancestors;
4. returns patches in deepest -> outer order.

No graph mutation, topology change, shrink, rotation, source-backed admission
or native Publisher semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_add,
    _checked_int,
    _round_ratio_nearest_emu,
    _validate_rect,
)
from group_refit_plan_v1 import (
    GroupRefitChildV1,
    GroupRefitPlanError,
    GroupRefitPlanV1,
    plan_group_refit_v1,
)
from group_transform_chain_v1 import (
    AUTHORED_GROUP_PROVENANCE_V1,
    AuthoredGroupEdgeV1,
    GroupTransformChainError,
    validate_group_transform_chain_v1,
)


CascadeStatusV1 = Literal["planned", "no_change", "rejected"]
CascadeReasonV1 = Literal[
    "no_change",
    "not_exactly_representable",
    "stale_path",
    "unsupported",
]


@dataclass(frozen=True)
class GroupCascadeSnapshotV1:
    edge: AuthoredGroupEdgeV1
    children: tuple[GroupRefitChildV1, ...]


@dataclass(frozen=True)
class GroupAncestorRefitPatchV1:
    group_id: str
    path_index: int
    refit: GroupRefitPlanV1


@dataclass(frozen=True)
class GroupAncestorRefitCascadePlanV1:
    status: CascadeStatusV1
    target_id: str
    page_id: str
    desired_page_rect: RectEmu
    patches: tuple[GroupAncestorRefitPatchV1, ...]
    reason: CascadeReasonV1 | None = None


def _rejected(
    *,
    target_id: str,
    page_id: str,
    desired_page_rect: RectEmu,
    reason: CascadeReasonV1,
) -> GroupAncestorRefitCascadePlanV1:
    return GroupAncestorRefitCascadePlanV1(
        status="rejected",
        target_id=target_id,
        page_id=page_id,
        desired_page_rect=desired_page_rect,
        patches=(),
        reason=reason,
    )


def _validate_rect_or_unsupported(rect: RectEmu) -> bool:
    try:
        _validate_rect(rect, "desired_page_rect")
    except AuthoredGroupGeometryError:
        return False
    return True


def _snapshot_path_is_current(
    *,
    target_id: str,
    page_id: str,
    snapshots: tuple[GroupCascadeSnapshotV1, ...],
) -> tuple[bool, CascadeReasonV1 | None]:
    if not isinstance(target_id, str) or not target_id:
        return False, "unsupported"
    if not isinstance(page_id, str) or not page_id:
        return False, "unsupported"
    if not isinstance(snapshots, tuple) or not snapshots:
        return False, "unsupported"

    edges: list[AuthoredGroupEdgeV1] = []
    for index, snapshot in enumerate(snapshots):
        if not isinstance(snapshot, GroupCascadeSnapshotV1):
            return False, "unsupported"
        edge = snapshot.edge
        if (
            not isinstance(edge, AuthoredGroupEdgeV1)
            or edge.provenance != AUTHORED_GROUP_PROVENANCE_V1
        ):
            return False, "unsupported"
        if not isinstance(snapshot.children, tuple) or not snapshot.children:
            return False, "unsupported"
        child_ids = tuple(
            child.node_id
            for child in snapshot.children
            if isinstance(child, GroupRefitChildV1)
        )
        if len(child_ids) != len(snapshot.children):
            return False, "unsupported"
        if child_ids != edge.children:
            return False, "stale_path"
        if len(set(child_ids)) != len(child_ids):
            return False, "stale_path"
        edges.append(edge)

        expected_path_child = (
            target_id
            if index == len(snapshots) - 1
            else snapshots[index + 1].edge.group_id
        )
        by_id = {child.node_id: child for child in snapshot.children}
        if expected_path_child not in by_id:
            return False, "stale_path"
        if index < len(snapshots) - 1:
            next_edge = snapshots[index + 1].edge
            if by_id[expected_path_child].local_rect != next_edge.bounds_in_parent:
                return False, "stale_path"

    try:
        validate_group_transform_chain_v1(
            target_id=target_id,
            target_page_id=page_id,
            ancestry=tuple(edges),
        )
    except GroupTransformChainError as exc:
        text = str(exc)
        if (
            "provenance" in text
            or "depth" in text
            or "local coordinate" in text
            or "positive" in text
        ):
            return False, "unsupported"
        return False, "stale_path"
    return True, None


def _map_edge_forward_unbounded(
    *,
    local_coordinate_space: RectEmu,
    local_rect: RectEmu,
    group_bounds: RectEmu,
) -> RectEmu:
    """Use the shared exact edge rounding law but permit expansion candidates."""
    try:
        _validate_rect(local_coordinate_space, "local_coordinate_space")
        _validate_rect(local_rect, "local_rect")
        _validate_rect(group_bounds, "group_bounds")
    except AuthoredGroupGeometryError as exc:
        raise ValueError(str(exc)) from exc
    if local_coordinate_space.x != 0 or local_coordinate_space.y != 0:
        raise ValueError("local coordinate space origin must be zero")

    def map_x(edge: int) -> int:
        scaled = _round_ratio_nearest_emu(
            edge * group_bounds.width,
            local_coordinate_space.width,
        )
        return _checked_add(group_bounds.x, scaled, "cascade.forward.x")

    def map_y(edge: int) -> int:
        scaled = _round_ratio_nearest_emu(
            edge * group_bounds.height,
            local_coordinate_space.height,
        )
        return _checked_add(group_bounds.y, scaled, "cascade.forward.y")

    left = map_x(local_rect.x)
    right = map_x(local_rect.right)
    top = map_y(local_rect.y)
    bottom = map_y(local_rect.bottom)
    width = right - left
    height = bottom - top
    if width <= 0 or height <= 0:
        raise ValueError("forward edge mapping collapsed")
    return RectEmu(left, top, width, height)


def _map_edge_inverse_unbounded(
    *,
    parent_rect: RectEmu,
    edge: AuthoredGroupEdgeV1,
) -> RectEmu:
    """Inverse of the shared exact edge law, permitting outside-envelope intent."""
    try:
        _validate_rect(parent_rect, "parent_rect")
        _validate_rect(edge.bounds_in_parent, "group_bounds")
        _validate_rect(edge.local_coordinate_space, "local_coordinate_space")
    except AuthoredGroupGeometryError as exc:
        raise ValueError(str(exc)) from exc
    if (
        edge.local_coordinate_space.x != 0
        or edge.local_coordinate_space.y != 0
    ):
        raise ValueError("local coordinate space origin must be zero")

    def map_x(page_edge: int) -> int:
        relative = page_edge - edge.bounds_in_parent.x
        numerator = _checked_int(
            relative * edge.local_coordinate_space.width,
            "cascade.inverse.x_numerator",
        )
        sign = -1 if numerator < 0 else 1
        value = _round_ratio_nearest_emu(
            abs(numerator),
            edge.bounds_in_parent.width,
        )
        return sign * value

    def map_y(page_edge: int) -> int:
        relative = page_edge - edge.bounds_in_parent.y
        numerator = _checked_int(
            relative * edge.local_coordinate_space.height,
            "cascade.inverse.y_numerator",
        )
        sign = -1 if numerator < 0 else 1
        value = _round_ratio_nearest_emu(
            abs(numerator),
            edge.bounds_in_parent.height,
        )
        return sign * value

    left = map_x(parent_rect.x)
    right = map_x(parent_rect.right)
    top = map_y(parent_rect.y)
    bottom = map_y(parent_rect.bottom)
    if right <= left or bottom <= top:
        raise ValueError("inverse edge mapping collapsed")
    return RectEmu(left, top, right - left, bottom - top)


def _inverse_prefix_exact(
    *,
    desired_page_rect: RectEmu,
    prefix: tuple[AuthoredGroupEdgeV1, ...],
) -> RectEmu | None:
    current = desired_page_rect
    for edge in prefix:
        try:
            current = _map_edge_inverse_unbounded(
                parent_rect=current,
                edge=edge,
            )
        except (ValueError, AuthoredGroupGeometryError):
            return None

    # Exact representability fence: candidate local edges must round back to the
    # requested page rectangle through the same boundaries.
    check = current
    for edge in reversed(prefix):
        try:
            check = _map_edge_forward_unbounded(
                local_coordinate_space=edge.local_coordinate_space,
                local_rect=check,
                group_bounds=edge.bounds_in_parent,
            )
        except (ValueError, AuthoredGroupGeometryError):
            return None
    if check != desired_page_rect:
        return None
    return current


def plan_group_ancestor_refit_cascade_v1(
    *,
    target_id: str,
    page_id: str,
    snapshots: tuple[GroupCascadeSnapshotV1, ...],
    desired_page_rect: RectEmu,
) -> GroupAncestorRefitCascadePlanV1:
    if not _validate_rect_or_unsupported(desired_page_rect):
        return _rejected(
            target_id=target_id,
            page_id=page_id,
            desired_page_rect=desired_page_rect,
            reason="unsupported",
        )

    valid, reason = _snapshot_path_is_current(
        target_id=target_id,
        page_id=page_id,
        snapshots=snapshots,
    )
    if not valid:
        return _rejected(
            target_id=target_id,
            page_id=page_id,
            desired_page_rect=desired_page_rect,
            reason=reason or "stale_path",
        )

    edges = tuple(snapshot.edge for snapshot in snapshots)
    deepest_index = len(snapshots) - 1
    desired_in_deepest_parent = _inverse_prefix_exact(
        desired_page_rect=desired_page_rect,
        prefix=edges[:deepest_index],
    )
    if desired_in_deepest_parent is None:
        return _rejected(
            target_id=target_id,
            page_id=page_id,
            desired_page_rect=desired_page_rect,
            reason="not_exactly_representable",
        )

    deepest = snapshots[deepest_index]
    parent_local = (
        snapshots[deepest_index - 1].edge.local_coordinate_space
        if deepest_index > 0
        else None
    )
    try:
        refit = plan_group_refit_v1(
            mode="expand_only_with_proposed_child_rect",
            group_bounds=deepest.edge.bounds_in_parent,
            local_coordinate_space=deepest.edge.local_coordinate_space,
            children=deepest.children,
            target_node_id=target_id,
            desired_target_parent_rect=desired_in_deepest_parent,
            parent_local_coordinate_space=parent_local,
        )
    except GroupRefitPlanError:
        return _rejected(
            target_id=target_id,
            page_id=page_id,
            desired_page_rect=desired_page_rect,
            reason="not_exactly_representable",
        )

    if refit.status == "no_envelope_change":
        return GroupAncestorRefitCascadePlanV1(
            status="no_change",
            target_id=target_id,
            page_id=page_id,
            desired_page_rect=desired_page_rect,
            patches=(),
            reason="no_change",
        )

    patches: list[GroupAncestorRefitPatchV1] = [
        GroupAncestorRefitPatchV1(
            group_id=deepest.edge.group_id,
            path_index=deepest_index,
            refit=refit,
        )
    ]
    if refit.status == "planned":
        return GroupAncestorRefitCascadePlanV1(
            status="planned",
            target_id=target_id,
            page_id=page_id,
            desired_page_rect=desired_page_rect,
            patches=tuple(patches),
        )

    changed_child_id = deepest.edge.group_id
    changed_child_local_rect = refit.new_group_bounds

    for index in range(deepest_index - 1, -1, -1):
        parent = snapshots[index]
        try:
            desired_child_parent_rect = _map_edge_forward_unbounded(
                local_coordinate_space=parent.edge.local_coordinate_space,
                local_rect=changed_child_local_rect,
                group_bounds=parent.edge.bounds_in_parent,
            )
        except (ValueError, AuthoredGroupGeometryError):
            return _rejected(
                target_id=target_id,
                page_id=page_id,
                desired_page_rect=desired_page_rect,
                reason="not_exactly_representable",
            )

        outer_local = (
            snapshots[index - 1].edge.local_coordinate_space
            if index > 0
            else None
        )
        try:
            parent_refit = plan_group_refit_v1(
                mode="expand_only_with_proposed_child_rect",
                group_bounds=parent.edge.bounds_in_parent,
                local_coordinate_space=parent.edge.local_coordinate_space,
                children=parent.children,
                target_node_id=changed_child_id,
                desired_target_parent_rect=desired_child_parent_rect,
                parent_local_coordinate_space=outer_local,
            )
        except GroupRefitPlanError:
            return _rejected(
                target_id=target_id,
                page_id=page_id,
                desired_page_rect=desired_page_rect,
                reason="not_exactly_representable",
            )

        if parent_refit.status == "no_envelope_change":
            # A deeper step reported ancestor_refit_required, so a contained
            # parent result would contradict the immutable path snapshot.
            return _rejected(
                target_id=target_id,
                page_id=page_id,
                desired_page_rect=desired_page_rect,
                reason="stale_path",
            )

        patches.append(
            GroupAncestorRefitPatchV1(
                group_id=parent.edge.group_id,
                path_index=index,
                refit=parent_refit,
            )
        )
        if parent_refit.status == "planned":
            return GroupAncestorRefitCascadePlanV1(
                status="planned",
                target_id=target_id,
                page_id=page_id,
                desired_page_rect=desired_page_rect,
                patches=tuple(patches),
            )

        changed_child_id = parent.edge.group_id
        changed_child_local_rect = parent_refit.new_group_bounds

    return _rejected(
        target_id=target_id,
        page_id=page_id,
        desired_page_rect=desired_page_rect,
        reason="unsupported",
    )
