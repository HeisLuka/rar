#!/usr/bin/env python3
"""Pure deterministic exact-EMU align/distribute planner V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU

AlignDistributeModeV1 = Literal[
    "align_left",
    "align_right",
    "align_top",
    "align_bottom",
    "align_horizontal_center",
    "align_vertical_center",
    "distribute_horizontal",
    "distribute_vertical",
]


class AlignDistributePlanError(ValueError):
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
class GeometryMemberV1:
    node_id: str
    bounds: RectEmu


@dataclass(frozen=True)
class PlannedGeometryV1:
    node_id: str
    before: RectEmu
    after: RectEmu


@dataclass(frozen=True)
class AlignDistributePlanV1:
    mode: AlignDistributeModeV1
    members: tuple[PlannedGeometryV1, ...]
    status: str


def _checked_int(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise AlignDistributePlanError(f"{label} must be integer")
    if value < MIN_SAFE_EMU or value > MAX_SAFE_EMU:
        raise AlignDistributePlanError(f"{label} outside JavaScript-safe EMU range")
    return value


def _checked_add(left: int, right: int, label: str) -> int:
    return _checked_int(_checked_int(left, label) + _checked_int(right, label), label)


def _checked_sub(left: int, right: int, label: str) -> int:
    return _checked_int(_checked_int(left, label) - _checked_int(right, label), label)


def _validate_rect(rect: RectEmu, label: str) -> None:
    if not isinstance(rect, RectEmu):
        raise AlignDistributePlanError(f"{label} must be RectEmu")
    _checked_int(rect.x, f"{label}.x")
    _checked_int(rect.y, f"{label}.y")
    _checked_int(rect.width, f"{label}.width")
    _checked_int(rect.height, f"{label}.height")
    if rect.width <= 0 or rect.height <= 0:
        raise AlignDistributePlanError(f"{label} must have positive width/height")
    _ = rect.right
    _ = rect.bottom


def _round_half_away_from_zero(numerator: int) -> int:
    """Round numerator/2 to nearest integer; exact halves go away from zero."""
    if not isinstance(numerator, int) or isinstance(numerator, bool):
        raise AlignDistributePlanError("midpoint numerator must be integer")
    sign = -1 if numerator < 0 else 1
    absolute = abs(numerator)
    quotient, remainder = divmod(absolute, 2)
    if remainder:
        quotient += 1
    return _checked_int(sign * quotient, "rounded midpoint")


def _translated(rect: RectEmu, *, x: int | None = None, y: int | None = None) -> RectEmu:
    moved = RectEmu(
        rect.x if x is None else _checked_int(x, "after.x"),
        rect.y if y is None else _checked_int(y, "after.y"),
        rect.width,
        rect.height,
    )
    _validate_rect(moved, "after")
    return moved


def _canonical_members(
    members: tuple[GeometryMemberV1, ...],
) -> tuple[GeometryMemberV1, ...]:
    if not isinstance(members, tuple) or not members:
        raise AlignDistributePlanError("members must be a non-empty tuple")
    ids = []
    for index, member in enumerate(members):
        if not isinstance(member, GeometryMemberV1):
            raise AlignDistributePlanError(f"members[{index}] must be GeometryMemberV1")
        if not isinstance(member.node_id, str) or not member.node_id:
            raise AlignDistributePlanError(f"members[{index}].node_id is required")
        _validate_rect(member.bounds, f"members[{index}].bounds")
        ids.append(member.node_id)
    if len(set(ids)) != len(ids):
        raise AlignDistributePlanError("member NodeIds must be unique")
    return tuple(sorted(members, key=lambda member: member.node_id))


def _align(
    members: tuple[GeometryMemberV1, ...],
    mode: AlignDistributeModeV1,
) -> dict[str, RectEmu]:
    min_left = min(member.bounds.x for member in members)
    max_right = max(member.bounds.right for member in members)
    min_top = min(member.bounds.y for member in members)
    max_bottom = max(member.bounds.bottom for member in members)

    result: dict[str, RectEmu] = {}
    for member in members:
        rect = member.bounds
        if mode == "align_left":
            after = _translated(rect, x=min_left)
        elif mode == "align_right":
            after = _translated(rect, x=_checked_sub(max_right, rect.width, "align_right.x"))
        elif mode == "align_top":
            after = _translated(rect, y=min_top)
        elif mode == "align_bottom":
            after = _translated(rect, y=_checked_sub(max_bottom, rect.height, "align_bottom.y"))
        elif mode == "align_horizontal_center":
            # Exact ideal left = (min_left + max_right - width) / 2.
            numerator = min_left + max_right - rect.width
            after = _translated(rect, x=_round_half_away_from_zero(numerator))
        elif mode == "align_vertical_center":
            # Exact ideal top = (min_top + max_bottom - height) / 2.
            numerator = min_top + max_bottom - rect.height
            after = _translated(rect, y=_round_half_away_from_zero(numerator))
        else:
            raise AlignDistributePlanError("unsupported alignment mode")
        result[member.node_id] = after
    return result


def _axis_key(member: GeometryMemberV1, horizontal: bool) -> tuple[int, int, str]:
    rect = member.bounds
    return (
        rect.x if horizontal else rect.y,
        rect.right if horizontal else rect.bottom,
        member.node_id,
    )


def _distribute(
    members: tuple[GeometryMemberV1, ...],
    *,
    horizontal: bool,
) -> dict[str, RectEmu]:
    if len(members) < 3:
        raise AlignDistributePlanError("distribution requires at least three members")

    ordered = tuple(sorted(members, key=lambda member: _axis_key(member, horizontal)))
    first = ordered[0]
    last = ordered[-1]
    first_start = first.bounds.x if horizontal else first.bounds.y
    last_end = last.bounds.right if horizontal else last.bounds.bottom
    occupied_span = last_end - first_start
    _checked_int(occupied_span, "distribution occupied span")

    extents = [
        member.bounds.width if horizontal else member.bounds.height
        for member in ordered
    ]
    total_extent = sum(extents)
    _checked_int(total_extent, "distribution total extent")
    total_gap = occupied_span - total_extent
    _checked_int(total_gap, "distribution total gap")

    gap_count = len(ordered) - 1
    quotient, remainder = divmod(total_gap, gap_count)
    gaps = tuple(
        quotient + (1 if index < remainder else 0)
        for index in range(gap_count)
    )

    result: dict[str, RectEmu] = {first.node_id: first.bounds}
    cursor_end = first.bounds.right if horizontal else first.bounds.bottom

    for index, member in enumerate(ordered[1:], start=1):
        desired_start = cursor_end + gaps[index - 1]
        _checked_int(desired_start, "distribution desired start")

        if member is last:
            actual_start = member.bounds.x if horizontal else member.bounds.y
            if desired_start != actual_start:
                raise AlignDistributePlanError("distribution arithmetic failed to preserve outer member")
            after = member.bounds
        elif horizontal:
            after = _translated(member.bounds, x=desired_start)
        else:
            after = _translated(member.bounds, y=desired_start)

        result[member.node_id] = after
        cursor_end = after.right if horizontal else after.bottom

    return result


def plan_align_distribute_v1(
    *,
    members: tuple[GeometryMemberV1, ...],
    mode: AlignDistributeModeV1,
) -> AlignDistributePlanV1:
    canonical = _canonical_members(members)
    supported = {
        "align_left",
        "align_right",
        "align_top",
        "align_bottom",
        "align_horizontal_center",
        "align_vertical_center",
        "distribute_horizontal",
        "distribute_vertical",
    }
    if mode not in supported:
        raise AlignDistributePlanError("unsupported align/distribute mode")

    if mode == "distribute_horizontal":
        after_by_id = _distribute(canonical, horizontal=True)
    elif mode == "distribute_vertical":
        after_by_id = _distribute(canonical, horizontal=False)
    else:
        after_by_id = _align(canonical, mode)

    planned = tuple(
        PlannedGeometryV1(
            node_id=member.node_id,
            before=member.bounds,
            after=after_by_id[member.node_id],
        )
        for member in canonical
    )
    status = "no_change" if all(item.before == item.after for item in planned) else "planned"
    return AlignDistributePlanV1(mode=mode, members=planned, status=status)
