#!/usr/bin/env python3
"""Aspect-preserving corner resize planner for authored PictureFrames V1."""

from __future__ import annotations

from dataclasses import dataclass
from math import gcd
from typing import Literal

MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU

CornerHandleV1 = Literal["nw", "ne", "se", "sw"]


class PictureResizePlanError(ValueError):
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
class PictureResizePlanV1:
    handle: CornerHandleV1
    before: RectEmu
    after: RectEmu
    intrinsic_ratio: tuple[int, int]
    scale_k: int
    status: str


def _checked_int(value: int, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise PictureResizePlanError(f"{label} must be integer")
    if value < MIN_SAFE_EMU or value > MAX_SAFE_EMU:
        raise PictureResizePlanError(f"{label} outside JavaScript-safe EMU range")
    return value


def _checked_add(left: int, right: int, label: str) -> int:
    return _checked_int(_checked_int(left, label) + _checked_int(right, label), label)


def _checked_mul(left: int, right: int, label: str) -> int:
    return _checked_int(_checked_int(left, label) * _checked_int(right, label), label)


def _validate_rect(rect: RectEmu, label: str) -> None:
    if not isinstance(rect, RectEmu):
        raise PictureResizePlanError(f"{label} must be RectEmu")
    _checked_int(rect.x, f"{label}.x")
    _checked_int(rect.y, f"{label}.y")
    _checked_int(rect.width, f"{label}.width")
    _checked_int(rect.height, f"{label}.height")
    if rect.width <= 0 or rect.height <= 0:
        raise PictureResizePlanError(f"{label} must have positive width/height")
    _ = rect.right
    _ = rect.bottom


def _validate_point(point: PointEmu) -> None:
    if not isinstance(point, PointEmu):
        raise PictureResizePlanError("pointer_target must be PointEmu")
    _checked_int(point.x, "pointer_target.x")
    _checked_int(point.y, "pointer_target.y")


def _ratio(width_px: int, height_px: int) -> tuple[int, int]:
    width_px = _checked_int(width_px, "intrinsic_width_px")
    height_px = _checked_int(height_px, "intrinsic_height_px")
    if width_px <= 0 or height_px <= 0:
        raise PictureResizePlanError("intrinsic dimensions must be positive")
    divisor = gcd(width_px, height_px)
    return width_px // divisor, height_px // divisor


def _fixed_corner(before: RectEmu, handle: CornerHandleV1) -> PointEmu:
    if handle == "nw":
        return PointEmu(before.right, before.bottom)
    if handle == "ne":
        return PointEmu(before.x, before.bottom)
    if handle == "se":
        return PointEmu(before.x, before.y)
    if handle == "sw":
        return PointEmu(before.right, before.y)
    raise PictureResizePlanError("unsupported resize handle")


def _envelope(
    *,
    fixed: PointEmu,
    pointer: PointEmu,
    handle: CornerHandleV1,
) -> tuple[int, int]:
    if handle == "nw":
        valid = pointer.x < fixed.x and pointer.y < fixed.y
    elif handle == "ne":
        valid = pointer.x > fixed.x and pointer.y < fixed.y
    elif handle == "se":
        valid = pointer.x > fixed.x and pointer.y > fixed.y
    elif handle == "sw":
        valid = pointer.x < fixed.x and pointer.y > fixed.y
    else:
        raise PictureResizePlanError("unsupported resize handle")
    if not valid:
        raise PictureResizePlanError("pointer crossed fixed corner or left expected quadrant")

    width = abs(pointer.x - fixed.x)
    height = abs(pointer.y - fixed.y)
    _checked_int(width, "envelope.width")
    _checked_int(height, "envelope.height")
    return width, height


def plan_picture_resize_v1(
    *,
    before: RectEmu,
    handle: CornerHandleV1,
    pointer_target: PointEmu,
    intrinsic_width_px: int,
    intrinsic_height_px: int,
) -> PictureResizePlanV1:
    _validate_rect(before, "before")
    _validate_point(pointer_target)
    if handle not in {"nw", "ne", "se", "sw"}:
        raise PictureResizePlanError("unsupported resize handle")

    ratio_w, ratio_h = _ratio(intrinsic_width_px, intrinsic_height_px)
    fixed = _fixed_corner(before, handle)
    envelope_w, envelope_h = _envelope(
        fixed=fixed,
        pointer=pointer_target,
        handle=handle,
    )

    k = min(envelope_w // ratio_w, envelope_h // ratio_h)
    if k <= 0:
        raise PictureResizePlanError("pointer envelope cannot represent one positive intrinsic-ratio unit")

    width = _checked_mul(k, ratio_w, "after.width")
    height = _checked_mul(k, ratio_h, "after.height")

    if handle == "nw":
        x = fixed.x - width
        y = fixed.y - height
    elif handle == "ne":
        x = fixed.x
        y = fixed.y - height
    elif handle == "se":
        x = fixed.x
        y = fixed.y
    else:  # sw
        x = fixed.x - width
        y = fixed.y

    after = RectEmu(
        _checked_int(x, "after.x"),
        _checked_int(y, "after.y"),
        width,
        height,
    )
    _validate_rect(after, "after")
    if after.width * ratio_h != after.height * ratio_w:
        raise PictureResizePlanError("internal exact-ratio invariant failed")

    return PictureResizePlanV1(
        handle=handle,
        before=before,
        after=after,
        intrinsic_ratio=(ratio_w, ratio_h),
        scale_k=k,
        status="no_change" if after == before else "planned",
    )
