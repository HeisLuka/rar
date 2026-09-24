#!/usr/bin/env python3
"""UI-framework-neutral exact-EMU box-draw transaction V1."""

from __future__ import annotations

from dataclasses import dataclass

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


class BoxDrawError(ValueError):
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


@dataclass(frozen=True)
class BoxDrawTransactionV1:
    page_id: str
    anchor: PointEmu
    current: PointEmu
    cancelled: bool = False


@dataclass(frozen=True)
class BoxDrawPreviewV1:
    status: str
    bounds: RectEmu | None


@dataclass(frozen=True)
class BoxDrawCommitV1:
    status: str
    page_id: str
    bounds: RectEmu | None


def _checked_int(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise BoxDrawError(f"{label} must be integer")
    if value < MIN_SAFE_EMU or value > MAX_SAFE_EMU:
        raise BoxDrawError(f"{label} outside JavaScript-safe EMU range")
    return value


def _point(point: PointEmu, label: str) -> PointEmu:
    if not isinstance(point, PointEmu):
        raise BoxDrawError(f"{label} must be PointEmu")
    _checked_int(point.x, f"{label}.x")
    _checked_int(point.y, f"{label}.y")
    return point


def _page_id(value: str) -> str:
    if not isinstance(value, str) or not value:
        raise BoxDrawError("page_id is required")
    return value


def _normalized_rect(start: PointEmu, end: PointEmu) -> RectEmu | None:
    start = _point(start, "start")
    end = _point(end, "end")
    left = min(start.x, end.x)
    top = min(start.y, end.y)
    right = max(start.x, end.x)
    bottom = max(start.y, end.y)

    width = right - left
    height = bottom - top
    _checked_int(width, "bounds.width")
    _checked_int(height, "bounds.height")
    if width == 0 or height == 0:
        return None
    if width < 0 or height < 0:
        raise BoxDrawError("normalized bounds must be non-negative")
    return RectEmu(left, top, width, height)


def start_box_draw_v1(*, page_id: str, anchor: PointEmu) -> BoxDrawTransactionV1:
    return BoxDrawTransactionV1(
        page_id=_page_id(page_id),
        anchor=_point(anchor, "anchor"),
        current=anchor,
        cancelled=False,
    )


def update_box_draw_v1(
    transaction: BoxDrawTransactionV1,
    *,
    current: PointEmu,
) -> BoxDrawTransactionV1:
    if not isinstance(transaction, BoxDrawTransactionV1):
        raise BoxDrawError("transaction must be BoxDrawTransactionV1")
    if transaction.cancelled:
        raise BoxDrawError("cancelled transaction cannot be updated")
    _page_id(transaction.page_id)
    _point(transaction.anchor, "transaction.anchor")
    return BoxDrawTransactionV1(
        page_id=transaction.page_id,
        anchor=transaction.anchor,
        current=_point(current, "current"),
        cancelled=False,
    )


def preview_box_draw_v1(transaction: BoxDrawTransactionV1) -> BoxDrawPreviewV1:
    if not isinstance(transaction, BoxDrawTransactionV1):
        raise BoxDrawError("transaction must be BoxDrawTransactionV1")
    if transaction.cancelled:
        return BoxDrawPreviewV1(status="cancelled", bounds=None)
    bounds = _normalized_rect(transaction.anchor, transaction.current)
    if bounds is None:
        return BoxDrawPreviewV1(status="no_change", bounds=None)
    return BoxDrawPreviewV1(status="preview", bounds=bounds)


def cancel_box_draw_v1(transaction: BoxDrawTransactionV1) -> BoxDrawTransactionV1:
    if not isinstance(transaction, BoxDrawTransactionV1):
        raise BoxDrawError("transaction must be BoxDrawTransactionV1")
    return BoxDrawTransactionV1(
        page_id=_page_id(transaction.page_id),
        anchor=_point(transaction.anchor, "transaction.anchor"),
        current=_point(transaction.current, "transaction.current"),
        cancelled=True,
    )


def commit_box_draw_v1(transaction: BoxDrawTransactionV1) -> BoxDrawCommitV1:
    if not isinstance(transaction, BoxDrawTransactionV1):
        raise BoxDrawError("transaction must be BoxDrawTransactionV1")
    page_id = _page_id(transaction.page_id)
    if transaction.cancelled:
        return BoxDrawCommitV1(status="cancelled", page_id=page_id, bounds=None)

    bounds = _normalized_rect(transaction.anchor, transaction.current)
    if bounds is None:
        return BoxDrawCommitV1(status="no_change", page_id=page_id, bounds=None)
    return BoxDrawCommitV1(status="commit", page_id=page_id, bounds=bounds)
