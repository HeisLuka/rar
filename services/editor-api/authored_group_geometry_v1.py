#!/usr/bin/env python3
"""Exact source-neutral geometry plan for first Chaptera-authored Group V1.

This module deliberately owns geometry only. It does not mutate a document
graph, authored stack, selection state, layout state, or native PUB carriers.

V1 law:
- aggregate group bounds are the exact page-space bbox of 2+ admitted members;
- local space is [0, 0, group.width, group.height];
- child local bounds are exact page bounds translated by (-group.left,-group.top);
- later materialization maps every local edge independently through exact
  integer rational scaling with nearest-EMU, ties away from zero.
"""

from __future__ import annotations

from dataclasses import dataclass

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


class AuthoredGroupGeometryError(ValueError):
    pass


@dataclass(frozen=True)
class RectEmu:
    x: int
    y: int
    width: int
    height: int

    @property
    def right(self) -> int:
        return _checked_add(self.x, self.width, "rect.right")

    @property
    def bottom(self) -> int:
        return _checked_add(self.y, self.height, "rect.bottom")


@dataclass(frozen=True)
class GroupMemberInput:
    node_id: str
    page_id: str
    bounds: RectEmu


@dataclass(frozen=True)
class GroupChildLocal:
    node_id: str
    bounds: RectEmu


@dataclass(frozen=True)
class AuthoredGroupGeometryPlanV1:
    group_id: str
    page_id: str
    group_bounds: RectEmu
    local_coordinate_space: RectEmu
    children: tuple[GroupChildLocal, ...]
    transform: str = "identity"


def _checked_int(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise AuthoredGroupGeometryError(f"{label} must be an integer")
    if value < MIN_SAFE_EMU or value > MAX_SAFE_EMU:
        raise AuthoredGroupGeometryError(f"{label} is outside JavaScript-safe EMU range")
    return value


def _checked_add(left: int, right: int, label: str) -> int:
    _checked_int(left, f"{label}.left")
    _checked_int(right, f"{label}.right")
    value = left + right
    return _checked_int(value, label)


def _checked_sub(left: int, right: int, label: str) -> int:
    _checked_int(left, f"{label}.left")
    _checked_int(right, f"{label}.right")
    value = left - right
    return _checked_int(value, label)


def _validate_rect(rect: RectEmu, label: str) -> None:
    if not isinstance(rect, RectEmu):
        raise AuthoredGroupGeometryError(f"{label} must be RectEmu")
    _checked_int(rect.x, f"{label}.x")
    _checked_int(rect.y, f"{label}.y")
    _checked_int(rect.width, f"{label}.width")
    _checked_int(rect.height, f"{label}.height")
    if rect.width <= 0 or rect.height <= 0:
        raise AuthoredGroupGeometryError(f"{label} must have positive width/height")
    _ = rect.right
    _ = rect.bottom


def _round_ratio_nearest_emu(numerator: int, denominator: int) -> int:
    """Round non-negative rational to nearest integer; exact half rounds upward.

    For V1 local/group scaling both numerator factors are non-negative and the
    denominator is positive. "upward" therefore equals ties away from zero.
    """
    if not isinstance(numerator, int) or numerator < 0:
        raise AuthoredGroupGeometryError("scale numerator must be non-negative integer")
    if not isinstance(denominator, int) or denominator <= 0:
        raise AuthoredGroupGeometryError("scale denominator must be positive integer")
    quotient, remainder = divmod(numerator, denominator)
    if remainder * 2 >= denominator:
        quotient += 1
    return _checked_int(quotient, "scaled edge")


def plan_authored_group_geometry_v1(
    *,
    group_id: str,
    members: list[GroupMemberInput],
) -> AuthoredGroupGeometryPlanV1:
    if not isinstance(group_id, str) or not group_id:
        raise AuthoredGroupGeometryError("group_id is required")
    if not isinstance(members, list) or len(members) < 2:
        raise AuthoredGroupGeometryError("V1 Group requires at least two members")

    node_ids = []
    page_ids = []
    for index, member in enumerate(members):
        if not isinstance(member, GroupMemberInput):
            raise AuthoredGroupGeometryError(f"member[{index}] must be GroupMemberInput")
        if not isinstance(member.node_id, str) or not member.node_id:
            raise AuthoredGroupGeometryError(f"member[{index}].node_id is required")
        if not isinstance(member.page_id, str) or not member.page_id:
            raise AuthoredGroupGeometryError(f"member[{index}].page_id is required")
        _validate_rect(member.bounds, f"member[{index}].bounds")
        node_ids.append(member.node_id)
        page_ids.append(member.page_id)

    if len(set(node_ids)) != len(node_ids):
        raise AuthoredGroupGeometryError("duplicate Group member NodeId")
    if group_id in set(node_ids):
        raise AuthoredGroupGeometryError("group_id cannot equal a member NodeId")
    if len(set(page_ids)) != 1:
        raise AuthoredGroupGeometryError("all Group members must belong to one page")

    left = min(member.bounds.x for member in members)
    top = min(member.bounds.y for member in members)
    right = max(member.bounds.right for member in members)
    bottom = max(member.bounds.bottom for member in members)
    width = _checked_sub(right, left, "group.width")
    height = _checked_sub(bottom, top, "group.height")
    if width <= 0 or height <= 0:
        raise AuthoredGroupGeometryError("aggregate Group extent must be positive")

    group_bounds = RectEmu(left, top, width, height)
    local_space = RectEmu(0, 0, width, height)

    children = []
    for member in members:
        local = RectEmu(
            _checked_sub(member.bounds.x, left, "child.local.x"),
            _checked_sub(member.bounds.y, top, "child.local.y"),
            member.bounds.width,
            member.bounds.height,
        )
        _validate_local_rect(local, local_space, f"child[{member.node_id}]")
        children.append(GroupChildLocal(node_id=member.node_id, bounds=local))

    return AuthoredGroupGeometryPlanV1(
        group_id=group_id,
        page_id=page_ids[0],
        group_bounds=group_bounds,
        local_coordinate_space=local_space,
        children=tuple(children),
    )


def _validate_local_rect(rect: RectEmu, local_space: RectEmu, label: str) -> None:
    _validate_rect(local_space, "local_coordinate_space")
    _validate_rect(rect, label)
    if local_space.x != 0 or local_space.y != 0:
        raise AuthoredGroupGeometryError("V1 local coordinate space origin must be zero")
    if rect.x < 0 or rect.y < 0:
        raise AuthoredGroupGeometryError(f"{label} starts outside local coordinate space")
    if rect.right > local_space.width or rect.bottom > local_space.height:
        raise AuthoredGroupGeometryError(f"{label} exceeds local coordinate space")


def materialize_group_child_rect_v1(
    *,
    local_coordinate_space: RectEmu,
    child_local_bounds: RectEmu,
    current_group_bounds: RectEmu,
) -> RectEmu:
    _validate_rect(current_group_bounds, "current_group_bounds")
    _validate_local_rect(
        child_local_bounds,
        local_coordinate_space,
        "child_local_bounds",
    )

    def map_x(local_edge: int) -> int:
        scaled = _round_ratio_nearest_emu(
            local_edge * current_group_bounds.width,
            local_coordinate_space.width,
        )
        return _checked_add(current_group_bounds.x, scaled, "materialized.x_edge")

    def map_y(local_edge: int) -> int:
        scaled = _round_ratio_nearest_emu(
            local_edge * current_group_bounds.height,
            local_coordinate_space.height,
        )
        return _checked_add(current_group_bounds.y, scaled, "materialized.y_edge")

    left = map_x(child_local_bounds.x)
    right = map_x(child_local_bounds.right)
    top = map_y(child_local_bounds.y)
    bottom = map_y(child_local_bounds.bottom)

    width = _checked_sub(right, left, "materialized.width")
    height = _checked_sub(bottom, top, "materialized.height")
    if width <= 0 or height <= 0:
        raise AuthoredGroupGeometryError("materialized child rect must remain positive")

    return RectEmu(left, top, width, height)
