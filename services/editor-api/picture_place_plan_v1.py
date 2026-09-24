#!/usr/bin/env python3
"""Largest exact-aspect authored PictureFrame inside a drawn RectEMU envelope."""

from __future__ import annotations

from dataclasses import dataclass

from picture_resize_plan_v1 import (
    MAX_SAFE_EMU,
    PictureResizePlanError,
    RectEmu,
    _checked_add,
    _checked_int,
    _checked_mul,
    _ratio,
    _validate_rect,
)


class PicturePlacePlanError(ValueError):
    pass


@dataclass(frozen=True)
class PicturePlacePlanV1:
    envelope: RectEmu
    frame: RectEmu
    intrinsic_ratio: tuple[int, int]
    scale_k: int
    residual_emu: tuple[int, int, int, int]
    status: str = "planned"


def _translate_error(exc: Exception) -> PicturePlacePlanError:
    return PicturePlacePlanError(str(exc))


def plan_picture_place_v1(
    *,
    envelope: RectEmu,
    intrinsic_width_px: int,
    intrinsic_height_px: int,
) -> PicturePlacePlanV1:
    """Fit the largest positive exact intrinsic-ratio integer RectEMU in envelope."""
    try:
        _validate_rect(envelope, "envelope")
        ratio_w, ratio_h = _ratio(intrinsic_width_px, intrinsic_height_px)
        k = min(envelope.width // ratio_w, envelope.height // ratio_h)
        if k <= 0:
            raise PicturePlacePlanError(
                "envelope cannot represent one positive intrinsic-ratio unit"
            )

        width = _checked_mul(k, ratio_w, "frame.width")
        height = _checked_mul(k, ratio_h, "frame.height")

        residual_x = _checked_int(envelope.width - width, "residual_x")
        residual_y = _checked_int(envelope.height - height, "residual_y")
        left = residual_x // 2
        top = residual_y // 2
        right = residual_x - left
        bottom = residual_y - top

        x = _checked_add(envelope.x, left, "frame.x")
        y = _checked_add(envelope.y, top, "frame.y")
        frame = RectEmu(x, y, width, height)
        _validate_rect(frame, "frame")

        if frame.right > envelope.right or frame.bottom > envelope.bottom:
            raise PicturePlacePlanError("internal containment invariant failed")
        if frame.x < envelope.x or frame.y < envelope.y:
            raise PicturePlacePlanError("internal containment invariant failed")
        if frame.width * ratio_h != frame.height * ratio_w:
            raise PicturePlacePlanError("internal exact-ratio invariant failed")

        next_k = k + 1
        next_w_fits = next_k <= MAX_SAFE_EMU // ratio_w and next_k * ratio_w <= envelope.width
        next_h_fits = next_k <= MAX_SAFE_EMU // ratio_h and next_k * ratio_h <= envelope.height
        if next_w_fits and next_h_fits:
            raise PicturePlacePlanError("internal maximality invariant failed")

        return PicturePlacePlanV1(
            envelope=envelope,
            frame=frame,
            intrinsic_ratio=(ratio_w, ratio_h),
            scale_k=k,
            residual_emu=(left, top, right, bottom),
        )
    except PicturePlacePlanError:
        raise
    except PictureResizePlanError as exc:
        raise _translate_error(exc) from exc
