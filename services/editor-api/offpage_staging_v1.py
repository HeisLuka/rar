#!/usr/bin/env python3
"""Source-neutral bounded off-page staging geometry for Chaptera Editor V1.

This module deliberately does not own desktop rendering, selection, pointer
events, or document mutation. It defines the shared geometry/admission law that
those runtime consumers must obey.
"""

from __future__ import annotations

from dataclasses import dataclass

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


class OffpageStagingError(ValueError):
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
class OffpageAdmission:
    node_id: str
    page_id: str
    bounds: RectEmu
    placement: str


def _checked_int(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise OffpageStagingError(f"{label} must be integer")
    if value < MIN_SAFE_EMU or value > MAX_SAFE_EMU:
        raise OffpageStagingError(f"{label} is outside JavaScript-safe EMU range")
    return value


def _checked_add(left: int, right: int, label: str) -> int:
    return _checked_int(_checked_int(left, label) + _checked_int(right, label), label)


def _checked_sub(left: int, right: int, label: str) -> int:
    return _checked_int(_checked_int(left, label) - _checked_int(right, label), label)


def _validate_rect(rect: RectEmu, label: str) -> None:
    if not isinstance(rect, RectEmu):
        raise OffpageStagingError(f"{label} must be RectEmu")
    _checked_int(rect.x, f"{label}.x")
    _checked_int(rect.y, f"{label}.y")
    _checked_int(rect.width, f"{label}.width")
    _checked_int(rect.height, f"{label}.height")
    if rect.width <= 0 or rect.height <= 0:
        raise OffpageStagingError(f"{label} must have positive width/height")
    _ = rect.right
    _ = rect.bottom


def work_area_bounds(page_bounds: RectEmu, margin_emu: int) -> RectEmu:
    """Return the bounded non-printing work area around one printable page."""
    _validate_rect(page_bounds, "page_bounds")
    margin_emu = _checked_int(margin_emu, "margin_emu")
    if margin_emu <= 0:
        raise OffpageStagingError("margin_emu must be positive")
    twice = _checked_add(margin_emu, margin_emu, "margin_emu.twice")
    return RectEmu(
        _checked_sub(page_bounds.x, margin_emu, "work_area.x"),
        _checked_sub(page_bounds.y, margin_emu, "work_area.y"),
        _checked_add(page_bounds.width, twice, "work_area.width"),
        _checked_add(page_bounds.height, twice, "work_area.height"),
    )


def _intersects(left: RectEmu, right: RectEmu) -> bool:
    _validate_rect(left, "left")
    _validate_rect(right, "right")
    return not (
        left.right <= right.x
        or right.right <= left.x
        or left.bottom <= right.y
        or right.bottom <= left.y
    )


def _contained(inner: RectEmu, outer: RectEmu) -> bool:
    _validate_rect(inner, "inner")
    _validate_rect(outer, "outer")
    return (
        inner.x >= outer.x
        and inner.y >= outer.y
        and inner.right <= outer.right
        and inner.bottom <= outer.bottom
    )


def classify_placement(bounds: RectEmu, page_bounds: RectEmu) -> str:
    """Classify geometry without changing page ownership."""
    _validate_rect(bounds, "bounds")
    _validate_rect(page_bounds, "page_bounds")
    if _contained(bounds, page_bounds):
        return "inside_page"
    if _intersects(bounds, page_bounds):
        return "partially_offpage"
    return "fully_offpage"


def admit_authored_offpage_node(
    *,
    node_id: str,
    page_id: str,
    parent_id: str,
    author_created: bool,
    source_backed: bool,
    transform_identity: bool,
    bounds: RectEmu,
    page_bounds: RectEmu,
    margin_emu: int,
) -> OffpageAdmission:
    """Admit only unambiguous direct page-owned Chaptera-created nodes.

    Objects may have signed coordinates and may be wholly outside the printable
    page. Visibility/selectability is admitted only while their bounds intersect
    the bounded work area. This function does not re-parent the node.
    """
    if not isinstance(node_id, str) or not node_id:
        raise OffpageStagingError("node_id is required")
    if not isinstance(page_id, str) or not page_id:
        raise OffpageStagingError("page_id is required")
    if parent_id != page_id:
        raise OffpageStagingError("node must remain direct page-owned")
    if author_created is not True or source_backed is not False:
        raise OffpageStagingError("V1 admits Chaptera-created non-source-backed nodes only")
    if transform_identity is not True:
        raise OffpageStagingError("V1 requires identity transform")

    _validate_rect(bounds, "bounds")
    area = work_area_bounds(page_bounds, margin_emu)
    if not _intersects(bounds, area):
        raise OffpageStagingError("node is outside bounded off-page work area")

    return OffpageAdmission(
        node_id=node_id,
        page_id=page_id,
        bounds=bounds,
        placement=classify_placement(bounds, page_bounds),
    )


def move_bounds_preserving_size(bounds: RectEmu, x_emu: int, y_emu: int) -> RectEmu:
    """Apply the MoveNode geometry law used by off-page staging."""
    _validate_rect(bounds, "bounds")
    moved = RectEmu(
        _checked_int(x_emu, "x_emu"),
        _checked_int(y_emu, "y_emu"),
        bounds.width,
        bounds.height,
    )
    _validate_rect(moved, "moved")
    if moved == bounds:
        raise OffpageStagingError("MoveNode no-op is not admitted")
    return moved


def printable_intersection(bounds: RectEmu, page_bounds: RectEmu) -> RectEmu | None:
    """Clip fixed/print output to the printable page; never expand the page."""
    _validate_rect(bounds, "bounds")
    _validate_rect(page_bounds, "page_bounds")
    left = max(bounds.x, page_bounds.x)
    top = max(bounds.y, page_bounds.y)
    right = min(bounds.right, page_bounds.right)
    bottom = min(bounds.bottom, page_bounds.bottom)
    if right <= left or bottom <= top:
        return None
    return RectEmu(left, top, right - left, bottom - top)
