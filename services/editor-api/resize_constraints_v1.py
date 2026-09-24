#!/usr/bin/env python3
"""Deterministic RectEMU resize constraints V1.

Input is immutable base geometry, a handle, current raw target geometry, and a
semantic modifier mask. Toolkit key names do not enter this core planner.

V1 laws:
- uncentered resize keeps the opposite base edge/corner fixed;
- centered resize preserves doubled base center on every active axis;
- aspect lock applies only to corner handles;
- aspect controlling axis is the larger normalized absolute size change
  (exact cross multiplication, tie -> X);
- the derived aspect dimension uses nearest-EMU rational rounding;
- when centered, the derived dimension is the nearest integer with the parity
  required to preserve the doubled center exactly (tie -> larger).

No snapping, minimum-size UX, rotation, mutation, or Publisher micro-rounding
claim lives here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import (
    MAX_SAFE_EMU,
    MIN_SAFE_EMU,
    AuthoredGroupGeometryError,
    RectEmu,
    _checked_int,
    _round_ratio_nearest_emu,
    _validate_rect,
)


ResizeHandleV1 = Literal["n", "s", "e", "w", "ne", "nw", "se", "sw"]
AspectControlAxisV1 = Literal["x", "y"]


class ResizeConstraintError(ValueError):
    pass


@dataclass(frozen=True)
class ResizeModifierMaskV1:
    centered: bool = False
    aspect_lock: bool = False

    def __post_init__(self) -> None:
        if not isinstance(self.centered, bool):
            raise ResizeConstraintError("centered modifier must be boolean")
        if not isinstance(self.aspect_lock, bool):
            raise ResizeConstraintError("aspect_lock modifier must be boolean")


@dataclass(frozen=True)
class ResizeConstraintPlanV1:
    protocol_version: Literal["chaptera.resize-constraint.v1"]
    handle: ResizeHandleV1
    modifiers: ResizeModifierMaskV1
    base_rect: RectEmu
    raw_target_rect: RectEmu
    constrained_rect: RectEmu
    centered_applied: bool
    aspect_applied: bool
    aspect_control_axis: AspectControlAxisV1 | None
    changed: bool


def _fail(message: str) -> None:
    raise ResizeConstraintError(message)


def _safe(value: int, label: str) -> int:
    try:
        return _checked_int(value, label)
    except AuthoredGroupGeometryError as exc:
        raise ResizeConstraintError(str(exc)) from exc


def _rect(rect: RectEmu, label: str) -> None:
    try:
        _validate_rect(rect, label)
    except AuthoredGroupGeometryError as exc:
        raise ResizeConstraintError(str(exc)) from exc


def _axes(handle: ResizeHandleV1) -> tuple[bool, bool]:
    if handle not in {"n", "s", "e", "w", "ne", "nw", "se", "sw"}:
        _fail("unsupported resize handle")
    return ("e" in handle or "w" in handle, "n" in handle or "s" in handle)


def _center2_x(base: RectEmu) -> int:
    return _safe(base.x + base.right, "base.center2_x")


def _center2_y(base: RectEmu) -> int:
    return _safe(base.y + base.bottom, "base.center2_y")


def _desired_width(
    *,
    base: RectEmu,
    raw: RectEmu,
    handle: ResizeHandleV1,
    centered: bool,
) -> int:
    if "e" in handle:
        dragged = raw.right
        if centered:
            extent = 2 * dragged - _center2_x(base)
        else:
            extent = dragged - base.x
    elif "w" in handle:
        dragged = raw.x
        if centered:
            extent = _center2_x(base) - 2 * dragged
        else:
            extent = base.right - dragged
    else:
        return base.width
    extent = _safe(extent, "desired_width")
    if extent <= 0:
        _fail("horizontal resize crossed the fixed center/opposite edge")
    return extent


def _desired_height(
    *,
    base: RectEmu,
    raw: RectEmu,
    handle: ResizeHandleV1,
    centered: bool,
) -> int:
    if "s" in handle:
        dragged = raw.bottom
        if centered:
            extent = 2 * dragged - _center2_y(base)
        else:
            extent = dragged - base.y
    elif "n" in handle:
        dragged = raw.y
        if centered:
            extent = _center2_y(base) - 2 * dragged
        else:
            extent = base.bottom - dragged
    else:
        return base.height
    extent = _safe(extent, "desired_height")
    if extent <= 0:
        _fail("vertical resize crossed the fixed center/opposite edge")
    return extent


def _nearest_rational(
    numerator: int,
    denominator: int,
    label: str,
) -> int:
    if not isinstance(numerator, int) or numerator <= 0:
        _fail(f"{label} numerator must be positive integer")
    if not isinstance(denominator, int) or denominator <= 0:
        _fail(f"{label} denominator must be positive integer")
    try:
        result = _round_ratio_nearest_emu(numerator, denominator)
    except AuthoredGroupGeometryError as exc:
        raise ResizeConstraintError(str(exc)) from exc
    result = _safe(result, label)
    if result <= 0:
        _fail(f"{label} must remain positive")
    return result


def _nearest_rational_with_parity(
    *,
    numerator: int,
    denominator: int,
    required_parity: int,
    label: str,
) -> int:
    """Nearest positive integer of required parity; exact-distance tie -> larger."""

    if not isinstance(numerator, int) or numerator <= 0:
        _fail(f"{label} numerator must be positive integer")
    if not isinstance(denominator, int) or denominator <= 0:
        _fail(f"{label} denominator must be positive integer")
    if required_parity not in {0, 1}:
        _fail(f"{label} parity must be 0 or 1")

    floor = numerator // denominator
    candidates = set()
    for value in range(max(1, floor - 3), floor + 5):
        if value > 0 and value % 2 == required_parity:
            candidates.add(value)
    if not candidates:
        # Only possible near zero for even parity: 2 is the first positive.
        candidate = 1 if required_parity == 1 else 2
        candidates.add(candidate)

    best = min(
        candidates,
        key=lambda value: (
            abs(value * denominator - numerator),
            -value,
        ),
    )
    best = _safe(best, label)
    if best <= 0:
        _fail(f"{label} must remain positive")
    return best


def _aspect_control_axis(
    *,
    base: RectEmu,
    desired_width: int,
    desired_height: int,
) -> AspectControlAxisV1:
    dx = abs(desired_width - base.width)
    dy = abs(desired_height - base.height)
    # Compare |dw|/W against |dh|/H without division. Exact tie -> X.
    left = dx * base.height
    right = dy * base.width
    return "x" if left >= right else "y"


def _constrained_extents(
    *,
    base: RectEmu,
    desired_width: int,
    desired_height: int,
    handle: ResizeHandleV1,
    modifiers: ResizeModifierMaskV1,
) -> tuple[int, int, bool, AspectControlAxisV1 | None]:
    active_x, active_y = _axes(handle)
    aspect_applied = modifiers.aspect_lock and active_x and active_y

    if not aspect_applied:
        return desired_width, desired_height, False, None

    axis = _aspect_control_axis(
        base=base,
        desired_width=desired_width,
        desired_height=desired_height,
    )

    if axis == "x":
        width = desired_width
        numerator = width * base.height
        denominator = base.width
        if modifiers.centered:
            height = _nearest_rational_with_parity(
                numerator=numerator,
                denominator=denominator,
                required_parity=base.height % 2,
                label="aspect_height",
            )
        else:
            height = _nearest_rational(
                numerator,
                denominator,
                "aspect_height",
            )
    else:
        height = desired_height
        numerator = height * base.width
        denominator = base.height
        if modifiers.centered:
            width = _nearest_rational_with_parity(
                numerator=numerator,
                denominator=denominator,
                required_parity=base.width % 2,
                label="aspect_width",
            )
        else:
            width = _nearest_rational(
                numerator,
                denominator,
                "aspect_width",
            )

    return width, height, True, axis


def _axis_edges(
    *,
    base_start: int,
    base_end: int,
    extent: int,
    negative_handle: bool,
    positive_handle: bool,
    active: bool,
    centered: bool,
    center2: int,
    label: str,
) -> tuple[int, int]:
    if not active:
        return base_start, base_end

    if centered:
        if (center2 - extent) % 2 != 0:
            _fail(f"{label} extent parity cannot preserve doubled center")
        start = (center2 - extent) // 2
        end = (center2 + extent) // 2
    elif positive_handle:
        start = base_start
        end = base_start + extent
    elif negative_handle:
        end = base_end
        start = base_end - extent
    else:
        _fail(f"{label} active axis has no matching handle")

    start = _safe(start, f"{label}.start")
    end = _safe(end, f"{label}.end")
    if end <= start:
        _fail(f"{label} constrained extent must remain positive")
    return start, end


def plan_resize_constraint_v1(
    *,
    base_rect: RectEmu,
    handle: ResizeHandleV1,
    raw_target_rect: RectEmu,
    modifiers: ResizeModifierMaskV1,
) -> ResizeConstraintPlanV1:
    _rect(base_rect, "base_rect")
    _rect(raw_target_rect, "raw_target_rect")
    if not isinstance(modifiers, ResizeModifierMaskV1):
        _fail("modifiers must be ResizeModifierMaskV1")

    active_x, active_y = _axes(handle)

    desired_width = _desired_width(
        base=base_rect,
        raw=raw_target_rect,
        handle=handle,
        centered=modifiers.centered,
    )
    desired_height = _desired_height(
        base=base_rect,
        raw=raw_target_rect,
        handle=handle,
        centered=modifiers.centered,
    )

    width, height, aspect_applied, axis = _constrained_extents(
        base=base_rect,
        desired_width=desired_width,
        desired_height=desired_height,
        handle=handle,
        modifiers=modifiers,
    )
    width = _safe(width, "constrained.width")
    height = _safe(height, "constrained.height")
    if width <= 0 or height <= 0:
        _fail("constrained dimensions must remain positive")

    left, right = _axis_edges(
        base_start=base_rect.x,
        base_end=base_rect.right,
        extent=width,
        negative_handle="w" in handle,
        positive_handle="e" in handle,
        active=active_x,
        centered=modifiers.centered,
        center2=(
            _center2_x(base_rect)
            if modifiers.centered and active_x
            else 0
        ),
        label="x_axis",
    )
    top, bottom = _axis_edges(
        base_start=base_rect.y,
        base_end=base_rect.bottom,
        extent=height,
        negative_handle="n" in handle,
        positive_handle="s" in handle,
        active=active_y,
        centered=modifiers.centered,
        center2=(
            _center2_y(base_rect)
            if modifiers.centered and active_y
            else 0
        ),
        label="y_axis",
    )

    constrained = RectEmu(
        left,
        top,
        right - left,
        bottom - top,
    )
    _rect(constrained, "constrained_rect")

    # Explicit invariant checks rather than relying on construction alone.
    if active_x and modifiers.centered:
        if constrained.x + constrained.right != base_rect.x + base_rect.right:
            _fail("centered horizontal resize changed doubled base center")
    if active_y and modifiers.centered:
        if constrained.y + constrained.bottom != base_rect.y + base_rect.bottom:
            _fail("centered vertical resize changed doubled base center")

    if active_x and not modifiers.centered:
        if "e" in handle and constrained.x != base_rect.x:
            _fail("east resize changed fixed left edge")
        if "w" in handle and constrained.right != base_rect.right:
            _fail("west resize changed fixed right edge")
    if active_y and not modifiers.centered:
        if "s" in handle and constrained.y != base_rect.y:
            _fail("south resize changed fixed top edge")
        if "n" in handle and constrained.bottom != base_rect.bottom:
            _fail("north resize changed fixed bottom edge")

    if not active_x and (
        constrained.x != base_rect.x or constrained.width != base_rect.width
    ):
        _fail("inactive horizontal axis changed")
    if not active_y and (
        constrained.y != base_rect.y or constrained.height != base_rect.height
    ):
        _fail("inactive vertical axis changed")

    return ResizeConstraintPlanV1(
        protocol_version="chaptera.resize-constraint.v1",
        handle=handle,
        modifiers=modifiers,
        base_rect=base_rect,
        raw_target_rect=raw_target_rect,
        constrained_rect=constrained,
        centered_applied=modifiers.centered,
        aspect_applied=aspect_applied,
        aspect_control_axis=axis,
        changed=constrained != base_rect,
    )
