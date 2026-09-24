#!/usr/bin/env python3
"""Deterministic document-space box selection planner V1."""

from __future__ import annotations

from dataclasses import dataclass

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


class BoxSelectPlanError(ValueError):
    pass


@dataclass(frozen=True)
class PointEmu:
    x: int
    y: int


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
class BoxSelectCandidateV1:
    node_id: str
    visual_bounds: RectEmu


@dataclass(frozen=True)
class BoxSelectPlanV1:
    selection_bounds: RectEmu | None
    selected_node_ids: tuple[str, ...]
    status: str


def _checked_int(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise BoxSelectPlanError(f"{label} must be integer")
    if value < MIN_SAFE_EMU or value > MAX_SAFE_EMU:
        raise BoxSelectPlanError(f"{label} outside JavaScript-safe EMU range")
    return value


def _checked_add(left: int, right: int, label: str) -> int:
    return _checked_int(_checked_int(left, label) + _checked_int(right, label), label)


def _validate_point(point: PointEmu, label: str) -> None:
    if not isinstance(point, PointEmu):
        raise BoxSelectPlanError(f"{label} must be PointEmu")
    _checked_int(point.x, f"{label}.x")
    _checked_int(point.y, f"{label}.y")


def _validate_rect(rect: RectEmu, label: str) -> None:
    if not isinstance(rect, RectEmu):
        raise BoxSelectPlanError(f"{label} must be RectEmu")
    _checked_int(rect.x, f"{label}.x")
    _checked_int(rect.y, f"{label}.y")
    _checked_int(rect.width, f"{label}.width")
    _checked_int(rect.height, f"{label}.height")
    if rect.width <= 0 or rect.height <= 0:
        raise BoxSelectPlanError(f"{label} must have positive width/height")
    _ = rect.right
    _ = rect.bottom


def _contains(outer: RectEmu, inner: RectEmu) -> bool:
    return (
        inner.x >= outer.x
        and inner.y >= outer.y
        and inner.right <= outer.right
        and inner.bottom <= outer.bottom
    )


def plan_box_select_v1(
    *,
    start: PointEmu,
    end: PointEmu,
    candidates: tuple[BoxSelectCandidateV1, ...],
) -> BoxSelectPlanV1:
    _validate_point(start, "start")
    _validate_point(end, "end")
    if not isinstance(candidates, tuple):
        raise BoxSelectPlanError("candidates must be tuple")

    ids = []
    for index, candidate in enumerate(candidates):
        if not isinstance(candidate, BoxSelectCandidateV1):
            raise BoxSelectPlanError(f"candidates[{index}] must be BoxSelectCandidateV1")
        if not isinstance(candidate.node_id, str) or not candidate.node_id:
            raise BoxSelectPlanError(f"candidates[{index}].node_id is required")
        _validate_rect(candidate.visual_bounds, f"candidates[{index}].visual_bounds")
        ids.append(candidate.node_id)
    if len(set(ids)) != len(ids):
        raise BoxSelectPlanError("candidate NodeIds must be unique")

    left = min(start.x, end.x)
    top = min(start.y, end.y)
    right = max(start.x, end.x)
    bottom = max(start.y, end.y)

    if left == right or top == bottom:
        return BoxSelectPlanV1(
            selection_bounds=None,
            selected_node_ids=(),
            status="no_change",
        )

    bounds = RectEmu(left, top, right - left, bottom - top)
    _validate_rect(bounds, "selection_bounds")

    selected = tuple(sorted(
        candidate.node_id
        for candidate in candidates
        if _contains(bounds, candidate.visual_bounds)
    ))
    return BoxSelectPlanV1(
        selection_bounds=bounds,
        selected_node_ids=selected,
        status="selected" if selected else "empty",
    )
