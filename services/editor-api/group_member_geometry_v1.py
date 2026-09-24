#!/usr/bin/env python3
"""Pure one-level authored Group member geometry planner V1.

Pointer and snapping intent is expressed in page EMU. Canonical child bounds
remain in the immediate Group local coordinate space. This module converts
between those spaces by reusing the exact one-boundary authored Group mapping
law already established by authored_group_geometry_v1.

No mutation, snapping policy, auto-fit, nested Group composition or native PUB
semantics live here.
"""

from __future__ import annotations

from dataclasses import dataclass

from authored_group_geometry_v1 import (
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_int,
    _round_ratio_nearest_emu,
    _validate_local_rect,
    _validate_rect,
    materialize_group_child_rect_v1,
)


class GroupMemberGeometryError(ValueError):
    pass


@dataclass(frozen=True)
class GroupMemberGeometryPlanV1:
    canonical_local_rect: RectEmu
    effective_page_rect: RectEmu


def _as_error(exc: Exception) -> GroupMemberGeometryError:
    return GroupMemberGeometryError(str(exc))


def _validate_spaces(
    *,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
) -> None:
    try:
        _validate_rect(group_bounds, "group_bounds")
        _validate_rect(local_coordinate_space, "local_coordinate_space")
    except AuthoredGroupGeometryError as exc:
        raise _as_error(exc) from exc
    if local_coordinate_space.x != 0 or local_coordinate_space.y != 0:
        raise GroupMemberGeometryError("V1 local coordinate space origin must be zero")


def _validate_local(
    *,
    local_coordinate_space: RectEmu,
    local_rect: RectEmu,
    label: str,
) -> None:
    try:
        _validate_local_rect(local_rect, local_coordinate_space, label)
    except AuthoredGroupGeometryError as exc:
        raise _as_error(exc) from exc


def _inverse_axis_edge(
    *,
    page_edge: int,
    group_origin: int,
    group_span: int,
    local_span: int,
    label: str,
) -> int:
    try:
        _checked_int(page_edge, f"{label}.page_edge")
        _checked_int(group_origin, f"{label}.group_origin")
        _checked_int(group_span, f"{label}.group_span")
        _checked_int(local_span, f"{label}.local_span")
    except AuthoredGroupGeometryError as exc:
        raise _as_error(exc) from exc
    if group_span <= 0 or local_span <= 0:
        raise GroupMemberGeometryError(f"{label} spans must be positive")
    relative = page_edge - group_origin
    if relative < 0 or relative > group_span:
        raise GroupMemberGeometryError(f"{label} page edge lies outside Group envelope")
    try:
        return _round_ratio_nearest_emu(relative * local_span, group_span)
    except AuthoredGroupGeometryError as exc:
        raise _as_error(exc) from exc


def project_group_member_rect_v1(
    *,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
    local_rect: RectEmu,
) -> GroupMemberGeometryPlanV1:
    """Project one canonical local child rect to its effective page rect."""
    _validate_spaces(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
    )
    _validate_local(
        local_coordinate_space=local_coordinate_space,
        local_rect=local_rect,
        label="local_rect",
    )
    try:
        page_rect = materialize_group_child_rect_v1(
            local_coordinate_space=local_coordinate_space,
            child_local_bounds=local_rect,
            current_group_bounds=group_bounds,
        )
    except AuthoredGroupGeometryError as exc:
        raise _as_error(exc) from exc
    return GroupMemberGeometryPlanV1(
        canonical_local_rect=local_rect,
        effective_page_rect=page_rect,
    )


def inverse_group_member_rect_v1(
    *,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
    desired_page_rect: RectEmu,
) -> GroupMemberGeometryPlanV1:
    """Map a desired page-space rect back to canonical local edges, then reproject."""
    _validate_spaces(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
    )
    try:
        _validate_rect(desired_page_rect, "desired_page_rect")
    except AuthoredGroupGeometryError as exc:
        raise _as_error(exc) from exc
    if (
        desired_page_rect.x < group_bounds.x
        or desired_page_rect.y < group_bounds.y
        or desired_page_rect.right > group_bounds.right
        or desired_page_rect.bottom > group_bounds.bottom
    ):
        raise GroupMemberGeometryError("desired page rect lies outside Group envelope")

    left = _inverse_axis_edge(
        page_edge=desired_page_rect.x,
        group_origin=group_bounds.x,
        group_span=group_bounds.width,
        local_span=local_coordinate_space.width,
        label="inverse_x_left",
    )
    right = _inverse_axis_edge(
        page_edge=desired_page_rect.right,
        group_origin=group_bounds.x,
        group_span=group_bounds.width,
        local_span=local_coordinate_space.width,
        label="inverse_x_right",
    )
    top = _inverse_axis_edge(
        page_edge=desired_page_rect.y,
        group_origin=group_bounds.y,
        group_span=group_bounds.height,
        local_span=local_coordinate_space.height,
        label="inverse_y_top",
    )
    bottom = _inverse_axis_edge(
        page_edge=desired_page_rect.bottom,
        group_origin=group_bounds.y,
        group_span=group_bounds.height,
        local_span=local_coordinate_space.height,
        label="inverse_y_bottom",
    )

    if right <= left or bottom <= top:
        raise GroupMemberGeometryError("inverse mapping collapsed to non-positive local rect")
    local_rect = RectEmu(left, top, right - left, bottom - top)
    _validate_local(
        local_coordinate_space=local_coordinate_space,
        local_rect=local_rect,
        label="inverse_local_rect",
    )
    return project_group_member_rect_v1(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
        local_rect=local_rect,
    )


def plan_group_member_move_v1(
    *,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
    base_local_rect: RectEmu,
    desired_page_x: int,
    desired_page_y: int,
) -> GroupMemberGeometryPlanV1:
    """Move from immutable base local size using desired page-space top-left intent."""
    _validate_spaces(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
    )
    _validate_local(
        local_coordinate_space=local_coordinate_space,
        local_rect=base_local_rect,
        label="base_local_rect",
    )
    local_x = _inverse_axis_edge(
        page_edge=desired_page_x,
        group_origin=group_bounds.x,
        group_span=group_bounds.width,
        local_span=local_coordinate_space.width,
        label="move_x",
    )
    local_y = _inverse_axis_edge(
        page_edge=desired_page_y,
        group_origin=group_bounds.y,
        group_span=group_bounds.height,
        local_span=local_coordinate_space.height,
        label="move_y",
    )
    candidate = RectEmu(
        local_x,
        local_y,
        base_local_rect.width,
        base_local_rect.height,
    )
    _validate_local(
        local_coordinate_space=local_coordinate_space,
        local_rect=candidate,
        label="move_candidate",
    )
    return project_group_member_rect_v1(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
        local_rect=candidate,
    )


def plan_group_member_resize_v1(
    *,
    group_bounds: RectEmu,
    local_coordinate_space: RectEmu,
    base_local_rect: RectEmu,
    desired_page_left: int | None = None,
    desired_page_right: int | None = None,
    desired_page_top: int | None = None,
    desired_page_bottom: int | None = None,
) -> GroupMemberGeometryPlanV1:
    """Resize active page-space edges while fixed opposite edges remain base-local."""
    _validate_spaces(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
    )
    _validate_local(
        local_coordinate_space=local_coordinate_space,
        local_rect=base_local_rect,
        label="base_local_rect",
    )
    if desired_page_left is not None and desired_page_right is not None:
        raise GroupMemberGeometryError("V1 resize may activate only one horizontal edge")
    if desired_page_top is not None and desired_page_bottom is not None:
        raise GroupMemberGeometryError("V1 resize may activate only one vertical edge")
    if all(
        edge is None
        for edge in (
            desired_page_left,
            desired_page_right,
            desired_page_top,
            desired_page_bottom,
        )
    ):
        raise GroupMemberGeometryError("V1 resize requires at least one active edge")

    left = base_local_rect.x
    right = base_local_rect.right
    top = base_local_rect.y
    bottom = base_local_rect.bottom

    if desired_page_left is not None:
        left = _inverse_axis_edge(
            page_edge=desired_page_left,
            group_origin=group_bounds.x,
            group_span=group_bounds.width,
            local_span=local_coordinate_space.width,
            label="resize_left",
        )
    elif desired_page_right is not None:
        right = _inverse_axis_edge(
            page_edge=desired_page_right,
            group_origin=group_bounds.x,
            group_span=group_bounds.width,
            local_span=local_coordinate_space.width,
            label="resize_right",
        )

    if desired_page_top is not None:
        top = _inverse_axis_edge(
            page_edge=desired_page_top,
            group_origin=group_bounds.y,
            group_span=group_bounds.height,
            local_span=local_coordinate_space.height,
            label="resize_top",
        )
    elif desired_page_bottom is not None:
        bottom = _inverse_axis_edge(
            page_edge=desired_page_bottom,
            group_origin=group_bounds.y,
            group_span=group_bounds.height,
            local_span=local_coordinate_space.height,
            label="resize_bottom",
        )

    if right <= left or bottom <= top:
        raise GroupMemberGeometryError("resize candidate must remain positive")
    candidate = RectEmu(left, top, right - left, bottom - top)
    _validate_local(
        local_coordinate_space=local_coordinate_space,
        local_rect=candidate,
        label="resize_candidate",
    )
    return project_group_member_rect_v1(
        group_bounds=group_bounds,
        local_coordinate_space=local_coordinate_space,
        local_rect=candidate,
    )
