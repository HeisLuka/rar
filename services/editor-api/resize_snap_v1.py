#!/usr/bin/env python3
"""Resize-handle snapping over the shared explicit SnapIndex V1.

The ordinary target RectEmu is already produced by the base resize law.
This adapter uses only the moving edge on each active handle axis as the source
snap anchor. It never snaps the resizing rectangle center/midline.

No semantic authoring mutation, modifier coupling, rotation/grid/baseline snap,
peer enumeration, or native PUB persistence occurs here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import RectEmu
from snap_index_v1 import (
    MAX_SAFE_EMU,
    MIN_SAFE_EMU,
    SnapFeedbackV1,
    SnapIndexError,
    SnapIndexV1,
)


ResizeHandleV1 = Literal["n", "ne", "e", "se", "s", "sw", "w", "nw"]
_HANDLES = {"n", "ne", "e", "se", "s", "sw", "w", "nw"}


class ResizeSnapError(ValueError):
    pass


@dataclass(frozen=True)
class ResizeSnapPlanV1:
    handle: ResizeHandleV1
    ordinary_target: RectEmu
    corrected_target: RectEmu
    x_feedback: SnapFeedbackV1 | None
    y_feedback: SnapFeedbackV1 | None


def _fail(message: str) -> None:
    raise ResizeSnapError(message)


def _checked_emu(value: int, label: str) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < MIN_SAFE_EMU
        or value > MAX_SAFE_EMU
    ):
        _fail(f"{label} must be a JavaScript-safe EMU integer")
    return value


def _validate_rect(rect: RectEmu, label: str) -> None:
    if not isinstance(rect, RectEmu):
        _fail(f"{label} must be RectEmu")
    for field in ("x", "y", "width", "height"):
        _checked_emu(getattr(rect, field), f"{label}.{field}")
    if rect.width <= 0 or rect.height <= 0:
        _fail(f"{label} must have positive size")
    _checked_emu(rect.x + rect.width, f"{label}.right")
    _checked_emu(rect.y + rect.height, f"{label}.bottom")


def _x_anchor(handle: str, rect: RectEmu) -> tuple[str, int] | None:
    if "w" in handle:
        return ("min", rect.x)
    if "e" in handle:
        return ("max", rect.x + rect.width)
    return None


def _y_anchor(handle: str, rect: RectEmu) -> tuple[str, int] | None:
    if "n" in handle:
        return ("min", rect.y)
    if "s" in handle:
        return ("max", rect.y + rect.height)
    return None


def plan_resize_snap_v1(
    *,
    handle: ResizeHandleV1,
    ordinary_target: RectEmu,
    snap_index: SnapIndexV1,
    tolerance_emu: int,
    excluded_node_ids: tuple[str, ...] = (),
) -> ResizeSnapPlanV1:
    if handle not in _HANDLES:
        _fail("unsupported resize handle")
    _validate_rect(ordinary_target, "ordinary_target")
    if not isinstance(snap_index, SnapIndexV1):
        _fail("snap_index must be SnapIndexV1")

    x_match = None
    y_match = None
    try:
        x_anchor = _x_anchor(handle, ordinary_target)
        if x_anchor is not None:
            x_match = snap_index.best_axis_match_v1(
                axis="x",
                moving_anchors=(x_anchor,),
                tolerance_emu=tolerance_emu,
                excluded_node_ids=excluded_node_ids,
            )

        y_anchor = _y_anchor(handle, ordinary_target)
        if y_anchor is not None:
            y_match = snap_index.best_axis_match_v1(
                axis="y",
                moving_anchors=(y_anchor,),
                tolerance_emu=tolerance_emu,
                excluded_node_ids=excluded_node_ids,
            )
    except SnapIndexError as exc:
        raise ResizeSnapError(str(exc)) from exc

    x = ordinary_target.x
    y = ordinary_target.y
    width = ordinary_target.width
    height = ordinary_target.height
    right = ordinary_target.x + ordinary_target.width
    bottom = ordinary_target.y + ordinary_target.height

    if x_match is not None:
        dx = x_match.correction_emu
        if "w" in handle:
            x = _checked_emu(x + dx, "corrected_target.x")
            width = _checked_emu(right - x, "corrected_target.width")
        else:
            width = _checked_emu(width + dx, "corrected_target.width")

    if y_match is not None:
        dy = y_match.correction_emu
        if "n" in handle:
            y = _checked_emu(y + dy, "corrected_target.y")
            height = _checked_emu(bottom - y, "corrected_target.height")
        else:
            height = _checked_emu(height + dy, "corrected_target.height")

    corrected = RectEmu(x=x, y=y, width=width, height=height)
    _validate_rect(corrected, "corrected_target")

    if "w" in handle and corrected.x + corrected.width != right:
        _fail("west resize snap moved the fixed east edge")
    if "e" in handle and corrected.x != ordinary_target.x:
        _fail("east resize snap moved the fixed west edge")
    if "n" in handle and corrected.y + corrected.height != bottom:
        _fail("north resize snap moved the fixed south edge")
    if "s" in handle and corrected.y != ordinary_target.y:
        _fail("south resize snap moved the fixed north edge")

    return ResizeSnapPlanV1(
        handle=handle,
        ordinary_target=ordinary_target,
        corrected_target=corrected,
        x_feedback=None if x_match is None else x_match.feedback,
        y_feedback=None if y_match is None else y_match.feedback,
    )
