#!/usr/bin/env python3
"""Exact document-space nudge delta policy V1.

This is pure command policy:
    direction + semantic modifier state -> exact signed EMU vector.

Evidence boundary:
- BASE_NUDGE_EMU = 118872 is exactly 0.13 inch, matching the documented
  ribbon-era Microsoft Publisher default nudge distance.
- COARSE_NUDGE_EMU = 10 * BASE_NUDGE_EMU is an explicit Chaptera V1 product
  policy. It is NOT claimed to be a Publisher modifier/default law.

Platform key adapters may map their chosen keyboard gesture to the semantic
modifier state below. This module does not know zoom, DPI, pixels, focus,
selection, key repeat, snapping, bounds or commit semantics.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal


EMU_PER_INCH = 914_400
BASE_NUDGE_EMU = 118_872  # 13/100 inch exactly.
COARSE_NUDGE_EMU = 1_188_720  # Chaptera V1: exactly 10x base.

NudgeDirectionV1 = Literal["left", "right", "up", "down"]
NudgeModifierStateV1 = Literal["none", "coarse"]


class NudgePlanError(ValueError):
    pass


@dataclass(frozen=True)
class NudgePlanV1:
    protocol_version: Literal["chaptera.nudge-plan.v1"]
    direction: NudgeDirectionV1
    modifier_state: NudgeModifierStateV1
    step_emu: int
    dx_emu: int
    dy_emu: int


def _fail(message: str) -> None:
    raise NudgePlanError(message)


def _validate_constants() -> None:
    if BASE_NUDGE_EMU * 100 != EMU_PER_INCH * 13:
        _fail("BASE_NUDGE_EMU must remain exactly 0.13 inch")
    if COARSE_NUDGE_EMU != BASE_NUDGE_EMU * 10:
        _fail("COARSE_NUDGE_EMU must remain exactly 10x base")
    if BASE_NUDGE_EMU <= 0 or COARSE_NUDGE_EMU <= 0:
        _fail("nudge constants must remain positive")


def plan_nudge_v1(
    *,
    direction: NudgeDirectionV1,
    modifier_state: NudgeModifierStateV1,
) -> NudgePlanV1:
    """Resolve one semantic arrow command to an exact document-space vector."""

    _validate_constants()

    if direction not in {"left", "right", "up", "down"}:
        _fail("unsupported nudge direction")
    if modifier_state not in {"none", "coarse"}:
        _fail("unsupported nudge modifier state")

    step = (
        BASE_NUDGE_EMU
        if modifier_state == "none"
        else COARSE_NUDGE_EMU
    )

    if direction == "left":
        dx, dy = -step, 0
    elif direction == "right":
        dx, dy = step, 0
    elif direction == "up":
        dx, dy = 0, -step
    else:
        dx, dy = 0, step

    return NudgePlanV1(
        protocol_version="chaptera.nudge-plan.v1",
        direction=direction,
        modifier_state=modifier_state,
        step_emu=step,
        dx_emu=dx,
        dy_emu=dy,
    )
