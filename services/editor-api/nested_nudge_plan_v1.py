#!/usr/bin/env python3
"""Exact nested authored-Group nudge adapter V1.

Nudge distance is document/page-space policy. Group scale must not silently
change it. This adapter calls NudgePlanV1 exactly once, derives one candidate
integer translation in the current Group-local coordinate system from an exact
selected-member page anchor, applies that same local vector to every selected
direct sibling, and proves by forward projection that every effective page
rectangle moved by exactly the requested document-space vector.

If one integer local vector cannot satisfy all selected rectangles, the result
is explicitly not_exactly_representable. No member-specific rounding fallback,
mutation, snapping, repeat grouping, refit, reparenting or z-order lives here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_int,
    _validate_local_rect,
)
from group_transform_chain_v1 import (
    AuthoredGroupEdgeV1,
    GroupPointEmu,
    GroupTransformChainError,
    inverse_group_transform_point_v1,
)
from nested_group_selection_v1 import (
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionScopeV1,
)
from nested_multi_geometry_v1 import (
    NestedMultiGeometryError,
    NestedMultiMemberV1,
    _project_rect,
    _validate_inputs,
)
import nudge_plan_v1


NestedNudgeStatusV1 = Literal["planned", "not_exactly_representable"]


class NestedNudgePlanError(ValueError):
    pass


@dataclass(frozen=True)
class NestedNudgeMemberResultV1:
    node_id: str
    before_local: RectEmu
    after_local: RectEmu
    before_page: RectEmu
    after_page: RectEmu


@dataclass(frozen=True)
class NestedNudgePlanV1:
    status: NestedNudgeStatusV1
    page_id: str
    container_path: tuple[str, ...]
    direction: str
    modifier_state: str
    requested_page_delta: tuple[int, int]
    local_translation: tuple[int, int] | None
    members: tuple[NestedNudgeMemberResultV1, ...]
    reason: str | None = None


def _not_exact(
    *,
    scope: NestedGroupSelectionScopeV1,
    direction: str,
    modifier_state: str,
    page_delta: tuple[int, int],
    reason: str,
) -> NestedNudgePlanV1:
    return NestedNudgePlanV1(
        status="not_exactly_representable",
        page_id=scope.page_id,
        container_path=scope.container_path,
        direction=direction,
        modifier_state=modifier_state,
        requested_page_delta=page_delta,
        local_translation=None,
        members=(),
        reason=reason,
    )


def _translated(rect: RectEmu, dx: int, dy: int) -> RectEmu:
    try:
        x = _checked_int(rect.x + dx, "translated.x")
        y = _checked_int(rect.y + dy, "translated.y")
    except AuthoredGroupGeometryError as exc:
        raise NestedNudgePlanError(str(exc)) from exc
    return RectEmu(x, y, rect.width, rect.height)


def plan_nested_nudge_v1(
    *,
    scope: NestedGroupSelectionScopeV1,
    selection_snapshot: NestedGroupPathSnapshotV1,
    ancestry: tuple[AuthoredGroupEdgeV1, ...],
    members: tuple[NestedMultiMemberV1, ...],
    direction: str,
    modifier_state: str,
) -> NestedNudgePlanV1:
    """Resolve one document-space nudge to one exact current-container vector."""

    try:
        normalized = _validate_inputs(
            scope=scope,
            selection_snapshot=selection_snapshot,
            ancestry=ancestry,
            members=members,
        )
    except NestedMultiGeometryError as exc:
        raise NestedNudgePlanError(str(exc)) from exc

    # Acceptance invariant: exactly one NudgePlanV1 call for one command.
    try:
        nudge = nudge_plan_v1.plan_nudge_v1(
            direction=direction,
            modifier_state=modifier_state,
        )
    except nudge_plan_v1.NudgePlanError as exc:
        raise NestedNudgePlanError(str(exc)) from exc

    page_delta = (nudge.dx_emu, nudge.dy_emu)
    authority = normalized[0]
    before_anchor_page_rect = _project_rect(
        node_id=authority.node_id,
        page_id=scope.page_id,
        ancestry=ancestry,
        local_rect=authority.local_rect,
    )
    before_anchor = GroupPointEmu(
        before_anchor_page_rect.x,
        before_anchor_page_rect.y,
    )
    desired_anchor = GroupPointEmu(
        before_anchor.x + nudge.dx_emu,
        before_anchor.y + nudge.dy_emu,
    )

    try:
        before_local_anchor = inverse_group_transform_point_v1(
            target_id=authority.node_id,
            target_page_id=scope.page_id,
            ancestry=ancestry,
            page_point=before_anchor,
        ).canonical_local_point
        after_local_anchor = inverse_group_transform_point_v1(
            target_id=authority.node_id,
            target_page_id=scope.page_id,
            ancestry=ancestry,
            page_point=desired_anchor,
        ).canonical_local_point
    except GroupTransformChainError:
        return _not_exact(
            scope=scope,
            direction=direction,
            modifier_state=modifier_state,
            page_delta=page_delta,
            reason="anchor_translation_not_exactly_representable",
        )

    try:
        local_dx = _checked_int(
            after_local_anchor.x - before_local_anchor.x,
            "nested_nudge.local_dx",
        )
        local_dy = _checked_int(
            after_local_anchor.y - before_local_anchor.y,
            "nested_nudge.local_dy",
        )
    except AuthoredGroupGeometryError as exc:
        raise NestedNudgePlanError(str(exc)) from exc

    if local_dx == 0 and local_dy == 0:
        return _not_exact(
            scope=scope,
            direction=direction,
            modifier_state=modifier_state,
            page_delta=page_delta,
            reason="document_delta_collapses_in_local_space",
        )

    results: list[NestedNudgeMemberResultV1] = []
    for member in normalized:
        before_local = member.local_rect
        after_local = RectEmu(
            before_local.x + local_dx,
            before_local.y + local_dy,
            before_local.width,
            before_local.height,
        )
        try:
            _validate_local_rect(
                after_local,
                ancestry[-1].local_coordinate_space,
                f"{member.node_id}.after_local",
            )
        except AuthoredGroupGeometryError:
            return _not_exact(
                scope=scope,
                direction=direction,
                modifier_state=modifier_state,
                page_delta=page_delta,
                reason="translated_member_exits_current_container",
            )

        before_page = _project_rect(
            node_id=member.node_id,
            page_id=scope.page_id,
            ancestry=ancestry,
            local_rect=before_local,
        )
        after_page = _project_rect(
            node_id=member.node_id,
            page_id=scope.page_id,
            ancestry=ancestry,
            local_rect=after_local,
        )

        expected_page = _translated(
            before_page,
            nudge.dx_emu,
            nudge.dy_emu,
        )
        if after_page != expected_page:
            return _not_exact(
                scope=scope,
                direction=direction,
                modifier_state=modifier_state,
                page_delta=page_delta,
                reason="single_local_vector_does_not_translate_every_member_exactly",
            )

        results.append(
            NestedNudgeMemberResultV1(
                node_id=member.node_id,
                before_local=before_local,
                after_local=after_local,
                before_page=before_page,
                after_page=after_page,
            )
        )

    return NestedNudgePlanV1(
        status="planned",
        page_id=scope.page_id,
        container_path=scope.container_path,
        direction=nudge.direction,
        modifier_state=nudge.modifier_state,
        requested_page_delta=page_delta,
        local_translation=(local_dx, local_dy),
        members=tuple(results),
    )
