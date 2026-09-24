#!/usr/bin/env python3
"""Exact RelativeToPage align/distribute geometry planner V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from align_distribute_plan_v1 import (
    AlignDistributePlanError,
    GeometryMemberV1,
    PlannedGeometryV1,
    RectEmu,
    _checked_int,
    _round_half_away_from_zero,
    _translated,
    _validate_rect,
)


PageAlignDistributeModeV1 = Literal[
    "align_left",
    "align_right",
    "align_top",
    "align_bottom",
    "align_horizontal_center",
    "align_vertical_center",
    "distribute_horizontal",
    "distribute_vertical",
]


class AlignDistributePagePlanError(ValueError):
    pass


@dataclass(frozen=True)
class AlignDistributePagePlanV1:
    mode: PageAlignDistributeModeV1
    page_bounds: RectEmu
    members: tuple[PlannedGeometryV1, ...]
    status: str


def _fail(message: str) -> None:
    raise AlignDistributePagePlanError(message)


def _validated_members(
    members: tuple[GeometryMemberV1, ...],
) -> tuple[GeometryMemberV1, ...]:
    if not isinstance(members, tuple) or not members:
        _fail("members must be a non-empty tuple")
    node_ids = []
    for index, member in enumerate(members):
        if not isinstance(member, GeometryMemberV1):
            _fail(f"members[{index}] must be GeometryMemberV1")
        if not isinstance(member.node_id, str) or not member.node_id:
            _fail(f"members[{index}].node_id is required")
        try:
            _validate_rect(member.bounds, f"members[{index}].bounds")
        except AlignDistributePlanError as exc:
            raise AlignDistributePagePlanError(str(exc)) from exc
        node_ids.append(member.node_id)
    if len(set(node_ids)) != len(node_ids):
        _fail("member NodeIds must be unique")
    return tuple(sorted(members, key=lambda member: member.node_id))


def _safe(value: int, label: str) -> int:
    try:
        return _checked_int(value, label)
    except AlignDistributePlanError as exc:
        raise AlignDistributePagePlanError(str(exc)) from exc


def _move(rect: RectEmu, *, x: int | None = None, y: int | None = None) -> RectEmu:
    try:
        return _translated(rect, x=x, y=y)
    except AlignDistributePlanError as exc:
        raise AlignDistributePagePlanError(str(exc)) from exc


def _align_to_page(
    *,
    page: RectEmu,
    members: tuple[GeometryMemberV1, ...],
    mode: PageAlignDistributeModeV1,
) -> dict[str, RectEmu]:
    result: dict[str, RectEmu] = {}
    for member in members:
        rect = member.bounds
        if mode == "align_left":
            after = _move(rect, x=page.x)
        elif mode == "align_right":
            after = _move(rect, x=_safe(page.right - rect.width, "align_right.x"))
        elif mode == "align_top":
            after = _move(rect, y=page.y)
        elif mode == "align_bottom":
            after = _move(rect, y=_safe(page.bottom - rect.height, "align_bottom.y"))
        elif mode == "align_horizontal_center":
            numerator = page.x + page.right - rect.width
            try:
                x = _round_half_away_from_zero(numerator)
            except AlignDistributePlanError as exc:
                raise AlignDistributePagePlanError(str(exc)) from exc
            after = _move(rect, x=x)
        elif mode == "align_vertical_center":
            numerator = page.y + page.bottom - rect.height
            try:
                y = _round_half_away_from_zero(numerator)
            except AlignDistributePlanError as exc:
                raise AlignDistributePagePlanError(str(exc)) from exc
            after = _move(rect, y=y)
        else:
            _fail("unsupported page alignment mode")
        result[member.node_id] = after
    return result


def _axis_key(member: GeometryMemberV1, horizontal: bool) -> tuple[int, int, str]:
    rect = member.bounds
    return (
        rect.x if horizontal else rect.y,
        rect.right if horizontal else rect.bottom,
        member.node_id,
    )


def _axis_extent(member: GeometryMemberV1, horizontal: bool) -> int:
    return member.bounds.width if horizontal else member.bounds.height


def _axis_start(rect: RectEmu, horizontal: bool) -> int:
    return rect.x if horizontal else rect.y


def _axis_end(rect: RectEmu, horizontal: bool) -> int:
    return rect.right if horizontal else rect.bottom


def _move_axis(rect: RectEmu, start: int, horizontal: bool) -> RectEmu:
    return _move(rect, x=start) if horizontal else _move(rect, y=start)


def _distributed_gaps(total_gap: int, gap_count: int) -> tuple[int, ...]:
    if gap_count <= 0:
        _fail("gap_count must be positive")
    quotient, remainder = divmod(total_gap, gap_count)
    return tuple(
        quotient + (1 if index < remainder else 0)
        for index in range(gap_count)
    )


def _distribute_to_page(
    *,
    page: RectEmu,
    members: tuple[GeometryMemberV1, ...],
    horizontal: bool,
) -> dict[str, RectEmu]:
    if len(members) < 2:
        _fail("page distribution requires at least two members")

    ordered = tuple(sorted(members, key=lambda member: _axis_key(member, horizontal)))
    page_start = page.x if horizontal else page.y
    page_end = page.right if horizontal else page.bottom
    page_span = page.width if horizontal else page.height

    total_extent = _safe(
        sum(_axis_extent(member, horizontal) for member in ordered),
        "distribution.total_extent",
    )
    result: dict[str, RectEmu] = {}

    if total_extent <= page_span:
        total_free = _safe(page_span - total_extent, "distribution.total_free")
        gaps = _distributed_gaps(total_free, len(ordered) + 1)
        cursor = _safe(page_start + gaps[0], "distribution.first_start")
        for index, member in enumerate(ordered):
            after = _move_axis(member.bounds, cursor, horizontal)
            result[member.node_id] = after
            cursor = _safe(
                _axis_end(after, horizontal) + gaps[index + 1],
                "distribution.cursor",
            )
        if cursor != page_end:
            _fail("free-space distribution failed page trailing-edge invariant")
        return result

    total_between_gap = _safe(page_span - total_extent, "distribution.total_overlap")
    gaps = _distributed_gaps(total_between_gap, len(ordered) - 1)

    first = ordered[0]
    first_after = _move_axis(first.bounds, page_start, horizontal)
    result[first.node_id] = first_after
    cursor_end = _axis_end(first_after, horizontal)

    for index, member in enumerate(ordered[1:], start=1):
        desired_start = _safe(
            cursor_end + gaps[index - 1],
            "distribution.overlap_start",
        )
        if index == len(ordered) - 1:
            pinned_start = _safe(
                page_end - _axis_extent(member, horizontal),
                "distribution.last_pinned_start",
            )
            if desired_start != pinned_start:
                _fail("overlap distribution failed pinned outer-edge invariant")
            after = _move_axis(member.bounds, pinned_start, horizontal)
        else:
            after = _move_axis(member.bounds, desired_start, horizontal)
        result[member.node_id] = after
        cursor_end = _axis_end(after, horizontal)

    if cursor_end != page_end:
        _fail("overlap distribution failed trailing page-edge invariant")
    return result


def plan_align_distribute_page_v1(
    *,
    page_bounds: RectEmu,
    members: tuple[GeometryMemberV1, ...],
    mode: PageAlignDistributeModeV1,
) -> AlignDistributePagePlanV1:
    try:
        _validate_rect(page_bounds, "page_bounds")
    except AlignDistributePlanError as exc:
        raise AlignDistributePagePlanError(str(exc)) from exc
    canonical = _validated_members(members)

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
        _fail("unsupported RelativeToPage mode")

    if mode == "distribute_horizontal":
        after_by_id = _distribute_to_page(
            page=page_bounds,
            members=canonical,
            horizontal=True,
        )
    elif mode == "distribute_vertical":
        after_by_id = _distribute_to_page(
            page=page_bounds,
            members=canonical,
            horizontal=False,
        )
    else:
        after_by_id = _align_to_page(page=page_bounds, members=canonical, mode=mode)

    planned = tuple(
        PlannedGeometryV1(
            node_id=member.node_id,
            before=member.bounds,
            after=after_by_id[member.node_id],
        )
        for member in canonical
    )
    return AlignDistributePagePlanV1(
        mode=mode,
        page_bounds=page_bounds,
        members=planned,
        status="no_change" if all(item.before == item.after for item in planned) else "planned",
    )
