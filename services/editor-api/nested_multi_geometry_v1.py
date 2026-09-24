#!/usr/bin/env python3
"""Parent-local aggregate move/resize planner for one nested Group scope V1.

Canvas gesture input is page-space. Planning authority is the current authored
Group's local RectEMU space. Move converts base/current page points through the
shared GroupTransformChainV1 exactly once and applies one integer local delta
to every selected direct sibling. Resize converts one page-space target
aggregate into local space and delegates scaling to MultiResizePlanV1.

Preview page rectangles are authoritative forward projections of planned local
rectangles through GroupTransformChainV1. No mutation, envelope refit, snapping,
modifiers, reparenting, z-order or native Publisher semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_int,
    _validate_local_rect,
    _validate_rect,
)
from group_transform_chain_v1 import (
    AuthoredGroupEdgeV1,
    GroupPointEmu,
    GroupTransformChainError,
    inverse_group_transform_chain_v1,
    inverse_group_transform_point_v1,
    project_group_transform_chain_v1,
    validate_group_transform_chain_v1,
)
from multi_resize_plan_v1 import (
    MultiResizeMemberV1,
    MultiResizePlanError,
    ResizeHandleV1,
    plan_multi_resize_v1,
)
from nested_group_selection_v1 import (
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionScopeV1,
    NestedGroupSelectionTargetV1,
    NestedGroupSelectionError,
    validate_nested_path_snapshot_v1,
)


NestedMultiKindV1 = Literal["move", "resize"]


class NestedMultiGeometryError(ValueError):
    pass


@dataclass(frozen=True)
class NestedMultiMemberV1:
    node_id: str
    local_rect: RectEmu
    provenance: str = "chaptera-authored"
    transform: str = "identity"


@dataclass(frozen=True)
class NestedMultiMemberResultV1:
    node_id: str
    before_local: RectEmu
    after_local: RectEmu
    before_page: RectEmu
    after_page: RectEmu


@dataclass(frozen=True)
class NestedMultiGeometryPlanV1:
    kind: NestedMultiKindV1
    page_id: str
    container_path: tuple[str, ...]
    base_local_aggregate: RectEmu
    target_local_aggregate: RectEmu
    base_page_aggregate: RectEmu
    target_page_aggregate: RectEmu
    members: tuple[NestedMultiMemberResultV1, ...]
    local_translation: tuple[int, int] | None = None
    resize_handle: ResizeHandleV1 | None = None


def _fail(message: str) -> None:
    raise NestedMultiGeometryError(message)


def _union_rects(rects: tuple[RectEmu, ...], label: str) -> RectEmu:
    if not rects:
        _fail(f"{label} requires at least one rectangle")
    for index, rect in enumerate(rects):
        try:
            _validate_rect(rect, f"{label}[{index}]")
        except AuthoredGroupGeometryError as exc:
            raise NestedMultiGeometryError(str(exc)) from exc
    left = min(rect.x for rect in rects)
    top = min(rect.y for rect in rects)
    right = max(rect.right for rect in rects)
    bottom = max(rect.bottom for rect in rects)
    if right <= left or bottom <= top:
        _fail(f"{label} union must remain positive")
    result = RectEmu(left, top, right - left, bottom - top)
    try:
        _validate_rect(result, f"{label}.union")
    except AuthoredGroupGeometryError as exc:
        raise NestedMultiGeometryError(str(exc)) from exc
    return result


def _validate_inputs(
    *,
    scope: NestedGroupSelectionScopeV1,
    selection_snapshot: NestedGroupPathSnapshotV1,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    members: tuple[NestedMultiMemberV1, ...],
) -> tuple[NestedMultiMemberV1, ...]:
    if not isinstance(scope, NestedGroupSelectionScopeV1):
        _fail("NestedGroupSelectionScopeV1 is required")
    if len(scope.selected) < 2:
        _fail("NestedMultiGeometryPlanV1 requires at least two selected siblings")

    try:
        validate_nested_path_snapshot_v1(
            page_id=scope.page_id,
            group_path=scope.container_path,
            snapshot=selection_snapshot,
        )
    except NestedGroupSelectionError as exc:
        raise NestedMultiGeometryError(str(exc)) from exc

    direct_children = set(selection_snapshot.edges[-1].children)
    selected_ids = tuple(target.node_id for target in scope.selected)
    if len(set(selected_ids)) != len(selected_ids):
        _fail("nested selected NodeIds must be unique")
    if any(target.group_path != scope.container_path for target in scope.selected):
        _fail("selected targets must share exact current container path")
    if any(target.page_id != scope.page_id for target in scope.selected):
        _fail("selected targets must share current page")
    if any(node_id not in direct_children for node_id in selected_ids):
        _fail("selected target is not a current direct sibling")

    if not isinstance(ancestry, tuple):
        _fail("ancestry must be a tuple")
    if tuple(edge.group_id for edge in ancestry) != scope.container_path:
        _fail("transform ancestry differs from current selection container path")
    if not ancestry:
        _fail("nested multi geometry requires a Group ancestry path")

    if not isinstance(members, tuple) or len(members) < 2:
        _fail("members must contain at least two local geometry snapshots")

    by_id: dict[str, NestedMultiMemberV1] = {}
    for index, member in enumerate(members):
        if not isinstance(member, NestedMultiMemberV1):
            _fail(f"members[{index}] must be NestedMultiMemberV1")
        if not isinstance(member.node_id, str) or not member.node_id:
            _fail(f"members[{index}].node_id is required")
        if member.node_id in by_id:
            _fail("member NodeIds must be unique")
        if member.provenance != "chaptera-authored":
            _fail(f"member {member.node_id!r} has unsupported provenance")
        if member.transform != "identity":
            _fail(f"member {member.node_id!r} has unsupported transform")
        try:
            _validate_local_rect(
                member.local_rect,
                ancestry[-1].local_coordinate_space,
                f"members[{index}].local_rect",
            )
        except AuthoredGroupGeometryError as exc:
            raise NestedMultiGeometryError(str(exc)) from exc
        by_id[member.node_id] = member

    if set(by_id) != set(selected_ids):
        _fail("member geometry snapshot must exactly match selected sibling identities")

    normalized = tuple(by_id[node_id] for node_id in sorted(by_id))
    for member in normalized:
        try:
            validate_group_transform_chain_v1(
                target_id=member.node_id,
                target_page_id=scope.page_id,
                ancestry=ancestry,
            )
        except GroupTransformChainError as exc:
            raise NestedMultiGeometryError(str(exc)) from exc

    return normalized


def _project_rect(
    *,
    node_id: str,
    page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    local_rect: RectEmu,
) -> RectEmu:
    try:
        return project_group_transform_chain_v1(
            target_id=node_id,
            target_page_id=page_id,
            ancestry=ancestry,
            target_local_rect=local_rect,
        ).effective_page_rect
    except GroupTransformChainError as exc:
        raise NestedMultiGeometryError(str(exc)) from exc


def _aggregate_page_rect(
    *,
    authority_node_id: str,
    page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    local_rect: RectEmu,
) -> RectEmu:
    return _project_rect(
        node_id=authority_node_id,
        page_id=page_id,
        ancestry=ancestry,
        local_rect=local_rect,
    )


def _results(
    *,
    page_id: str,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    before_by_id: dict[str, RectEmu],
    after_by_id: dict[str, RectEmu],
) -> tuple[NestedMultiMemberResultV1, ...]:
    out: list[NestedMultiMemberResultV1] = []
    for node_id in sorted(before_by_id):
        before = before_by_id[node_id]
        after = after_by_id[node_id]
        out.append(
            NestedMultiMemberResultV1(
                node_id=node_id,
                before_local=before,
                after_local=after,
                before_page=_project_rect(
                    node_id=node_id,
                    page_id=page_id,
                    ancestry=ancestry,
                    local_rect=before,
                ),
                after_page=_project_rect(
                    node_id=node_id,
                    page_id=page_id,
                    ancestry=ancestry,
                    local_rect=after,
                ),
            )
        )
    return tuple(out)


def plan_nested_multi_move_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    selection_snapshot: NestedGroupPathSnapshotV1,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    members: tuple[NestedMultiMemberV1, ...],
    gesture_base_page_point: GroupPointEmu,
    gesture_current_page_point: GroupPointEmu,
) -> NestedMultiGeometryPlanV1:
    normalized = _validate_inputs(
        scope=scope,
        selection_snapshot=selection_snapshot,
        ancestry=ancestry,
        members=members,
    )
    authority_id = normalized[0].node_id

    try:
        base_point = inverse_group_transform_point_v1(
            target_id=authority_id,
            target_page_id=scope.page_id,
            ancestry=ancestry,
            page_point=gesture_base_page_point,
        ).canonical_local_point
        current_point = inverse_group_transform_point_v1(
            target_id=authority_id,
            target_page_id=scope.page_id,
            ancestry=ancestry,
            page_point=gesture_current_page_point,
        ).canonical_local_point
    except GroupTransformChainError as exc:
        raise NestedMultiGeometryError(str(exc)) from exc

    try:
        dx = _checked_int(current_point.x - base_point.x, "local_translation.dx")
        dy = _checked_int(current_point.y - base_point.y, "local_translation.dy")
    except AuthoredGroupGeometryError as exc:
        raise NestedMultiGeometryError(str(exc)) from exc
    if dx == 0 and dy == 0:
        _fail("nested multi move must not be a no-op")

    before_by_id = {member.node_id: member.local_rect for member in normalized}
    after_by_id: dict[str, RectEmu] = {}
    for member in normalized:
        before = member.local_rect
        after = RectEmu(
            before.x + dx,
            before.y + dy,
            before.width,
            before.height,
        )
        try:
            _validate_local_rect(
                after,
                ancestry[-1].local_coordinate_space,
                f"{member.node_id}.after_local",
            )
        except AuthoredGroupGeometryError as exc:
            raise NestedMultiGeometryError(str(exc)) from exc
        after_by_id[member.node_id] = after

    base_local = _union_rects(tuple(before_by_id.values()), "base_local")
    target_local = RectEmu(
        base_local.x + dx,
        base_local.y + dy,
        base_local.width,
        base_local.height,
    )
    authority_id = normalized[0].node_id
    return NestedMultiGeometryPlanV1(
        kind="move",
        page_id=scope.page_id,
        container_path=scope.container_path,
        base_local_aggregate=base_local,
        target_local_aggregate=target_local,
        base_page_aggregate=_aggregate_page_rect(
            authority_node_id=authority_id,
            page_id=scope.page_id,
            ancestry=ancestry,
            local_rect=base_local,
        ),
        target_page_aggregate=_aggregate_page_rect(
            authority_node_id=authority_id,
            page_id=scope.page_id,
            ancestry=ancestry,
            local_rect=target_local,
        ),
        members=_results(
            page_id=scope.page_id,
            ancestry=ancestry,
            before_by_id=before_by_id,
            after_by_id=after_by_id,
        ),
        local_translation=(dx, dy),
    )


def plan_nested_multi_resize_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    selection_snapshot: NestedGroupPathSnapshotV1,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    members: tuple[NestedMultiMemberV1, ...],
    handle: ResizeHandleV1,
    target_aggregate_page_rect: RectEmu,
) -> NestedMultiGeometryPlanV1:
    normalized = _validate_inputs(
        scope=scope,
        selection_snapshot=selection_snapshot,
        ancestry=ancestry,
        members=members,
    )
    authority_id = normalized[0].node_id
    before_by_id = {member.node_id: member.local_rect for member in normalized}
    base_local = _union_rects(tuple(before_by_id.values()), "base_local")

    try:
        inverse = inverse_group_transform_chain_v1(
            target_id=authority_id,
            target_page_id=scope.page_id,
            ancestry=ancestry,
            desired_page_rect=target_aggregate_page_rect,
        )
    except GroupTransformChainError as exc:
        raise NestedMultiGeometryError(str(exc)) from exc
    if inverse.effective_page_rect != target_aggregate_page_rect:
        _fail("target page aggregate is not exactly representable in current Group local space")

    target_local = inverse.canonical_local_rect

    try:
        resize = plan_multi_resize_v1(
            members=tuple(
                MultiResizeMemberV1(
                    node_id=member.node_id,
                    rect=member.local_rect,
                    provenance=member.provenance,
                    transform=member.transform,
                )
                for member in normalized
            ),
            base_aggregate=base_local,
            handle=handle,
            target_aggregate=target_local,
        )
    except MultiResizePlanError as exc:
        raise NestedMultiGeometryError(str(exc)) from exc

    after_by_id = {result.node_id: result.after for result in resize.members}
    for node_id, after in after_by_id.items():
        try:
            _validate_local_rect(
                after,
                ancestry[-1].local_coordinate_space,
                f"{node_id}.after_local",
            )
        except AuthoredGroupGeometryError as exc:
            raise NestedMultiGeometryError(str(exc)) from exc

    projected_target = _aggregate_page_rect(
        authority_node_id=authority_id,
        page_id=scope.page_id,
        ancestry=ancestry,
        local_rect=resize.target_aggregate,
    )
    if projected_target != target_aggregate_page_rect:
        _fail("planned local resize does not reproject to requested page aggregate")

    return NestedMultiGeometryPlanV1(
        kind="resize",
        page_id=scope.page_id,
        container_path=scope.container_path,
        base_local_aggregate=resize.base_aggregate,
        target_local_aggregate=resize.target_aggregate,
        base_page_aggregate=_aggregate_page_rect(
            authority_node_id=authority_id,
            page_id=scope.page_id,
            ancestry=ancestry,
            local_rect=resize.base_aggregate,
        ),
        target_page_aggregate=projected_target,
        members=_results(
            page_id=scope.page_id,
            ancestry=ancestry,
            before_by_id=before_by_id,
            after_by_id=after_by_id,
        ),
        resize_handle=handle,
    )
