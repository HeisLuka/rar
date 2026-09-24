#!/usr/bin/env python3
"""Deterministic composition of resize modifiers with snapping V1.

This solver never post-corrects a constrained rectangle. Every snap proposal is
converted into a raw active-edge intent and re-solved from the immutable base by
ResizeConstraintV1. Therefore centered and aspect invariants remain authority.

Proposal law:
- Ctrl/center-only: active axes snap independently, then mirror through the base center.
- Shift/aspect corner: generate X-driven and Y-driven common-scale proposals.
- A driven proposal may count as two-axis only when its derived other edge is
  also within tolerance of an independently admitted SnapIndex target.
- Ctrl+Shift uses the same proposal generation through centered constraint math.
- score: snapped axis count descending, total active-edge correction from the
  unconstrained pointer intent ascending, then shared SnapIndex stable tie-break.
- identical final RectEMU values are deduplicated.
- no valid proposal returns the unsnapped constrained rectangle with no feedback.

No mutation, minimum-size UX, rotation, grid/baseline snap, or native PUB write.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from authored_group_geometry_v1 import RectEmu
from resize_constraints_v1 import (
    ResizeConstraintError,
    ResizeModifierMaskV1,
    plan_resize_constraint_v1,
)
from resize_snap_v1 import ResizeSnapError, plan_resize_snap_v1
from snap_index_v1 import (
    SnapAxisMatchV1,
    SnapFeedbackV1,
    SnapIndexError,
    SnapIndexV1,
    snap_feedback_stable_key_v1,
)


ComposeSourceV1 = Literal[
    "fallback",
    "ordinary",
    "centered",
    "x_driven",
    "y_driven",
]


class ResizeConstraintSnapComposeError(ValueError):
    pass


@dataclass(frozen=True)
class ResizeConstraintSnapComposeV1:
    protocol_version: Literal["chaptera.resize-constraint-snap-compose.v1"]
    handle: str
    modifiers: ResizeModifierMaskV1
    base_rect: RectEmu
    raw_target_rect: RectEmu
    unsnapped_constrained_rect: RectEmu
    final_rect: RectEmu
    x_feedback: SnapFeedbackV1 | None
    y_feedback: SnapFeedbackV1 | None
    snapped_axis_count: int
    source: ComposeSourceV1


@dataclass(frozen=True)
class _Proposal:
    rect: RectEmu
    x_feedback: SnapFeedbackV1 | None
    y_feedback: SnapFeedbackV1 | None
    source: ComposeSourceV1


def _fail(message: str) -> None:
    raise ResizeConstraintSnapComposeError(message)


def _active_x(handle: str) -> bool:
    return "e" in handle or "w" in handle


def _active_y(handle: str) -> bool:
    return "n" in handle or "s" in handle


def _edge(rect: RectEmu, handle: str, axis: str) -> int:
    if axis == "x":
        if "w" in handle:
            return rect.x
        if "e" in handle:
            return rect.right
    elif axis == "y":
        if "n" in handle:
            return rect.y
        if "s" in handle:
            return rect.bottom
    _fail(f"handle has no active {axis} edge")


def _anchor(handle: str, axis: str) -> str:
    if axis == "x":
        if "w" in handle:
            return "min"
        if "e" in handle:
            return "max"
    elif axis == "y":
        if "n" in handle:
            return "min"
        if "s" in handle:
            return "max"
    _fail(f"handle has no active {axis} anchor")


def _raw_with_active_edges(
    *,
    original_raw: RectEmu,
    handle: str,
    x_edge: int | None,
    y_edge: int | None,
) -> RectEmu:
    """Build any positive raw rectangle carrying the requested dragged edges.

    ResizeConstraintV1 reads only the active dragged edge from raw geometry and
    derives authoritative extents from immutable base geometry.
    """
    x = original_raw.x
    y = original_raw.y
    width = original_raw.width
    height = original_raw.height

    if x_edge is not None:
        if "w" in handle:
            x = x_edge
            width = 1
        elif "e" in handle:
            x = x_edge - 1
            width = 1
        else:
            _fail("x_edge supplied for inactive horizontal handle")

    if y_edge is not None:
        if "n" in handle:
            y = y_edge
            height = 1
        elif "s" in handle:
            y = y_edge - 1
            height = 1
        else:
            _fail("y_edge supplied for inactive vertical handle")

    return RectEmu(x=x, y=y, width=width, height=height)


def _axis_matches(
    *,
    snap_index: SnapIndexV1,
    axis: str,
    handle: str,
    raw_target_rect: RectEmu,
    tolerance_emu: int,
    excluded_node_ids: tuple[str, ...],
) -> tuple[SnapAxisMatchV1, ...]:
    if axis == "x" and not _active_x(handle):
        return ()
    if axis == "y" and not _active_y(handle):
        return ()
    return snap_index.axis_matches_v1(
        axis=axis,
        moving_anchors=((_anchor(handle, axis), _edge(raw_target_rect, handle, axis)),),
        tolerance_emu=tolerance_emu,
        excluded_node_ids=excluded_node_ids,
    )


def _feedback_score(feedback: SnapFeedbackV1) -> tuple:
    return (
        0 if feedback.axis == "x" else 1,
        *snap_feedback_stable_key_v1(feedback),
    )


def _proposal_score(
    proposal: _Proposal,
    *,
    raw_target_rect: RectEmu,
    handle: str,
) -> tuple:
    feedbacks = tuple(
        feedback
        for feedback in (proposal.x_feedback, proposal.y_feedback)
        if feedback is not None
    )
    axis_count = len(feedbacks)
    total_correction = 0
    if _active_x(handle):
        total_correction += abs(
            _edge(proposal.rect, handle, "x")
            - _edge(raw_target_rect, handle, "x")
        )
    if _active_y(handle):
        total_correction += abs(
            _edge(proposal.rect, handle, "y")
            - _edge(raw_target_rect, handle, "y")
        )
    feedback_key = tuple(sorted(_feedback_score(value) for value in feedbacks))
    source_rank = {
        "centered": 0,
        "x_driven": 1,
        "y_driven": 2,
        "ordinary": 3,
        "fallback": 4,
    }[proposal.source]
    return (-axis_count, total_correction, feedback_key, source_rank)


def _best_other_feedback(
    *,
    matches: tuple[SnapAxisMatchV1, ...],
    final_edge: int,
    tolerance_emu: int,
) -> SnapFeedbackV1 | None:
    admitted = [
        match.feedback
        for match in matches
        if abs(match.feedback.position_emu - final_edge) <= tolerance_emu
    ]
    if not admitted:
        return None
    return min(
        admitted,
        key=lambda feedback: (
            abs(feedback.position_emu - final_edge),
            snap_feedback_stable_key_v1(feedback),
        ),
    )


def _solve_from_edges(
    *,
    base_rect: RectEmu,
    handle: str,
    raw_target_rect: RectEmu,
    modifiers: ResizeModifierMaskV1,
    x_edge: int | None,
    y_edge: int | None,
) -> RectEmu | None:
    try:
        proposal_raw = _raw_with_active_edges(
            original_raw=raw_target_rect,
            handle=handle,
            x_edge=x_edge,
            y_edge=y_edge,
        )
        return plan_resize_constraint_v1(
            base_rect=base_rect,
            handle=handle,
            raw_target_rect=proposal_raw,
            modifiers=modifiers,
        ).constrained_rect
    except (ResizeConstraintError, ResizeConstraintSnapComposeError):
        return None


def _centered_proposals(
    *,
    base_rect: RectEmu,
    handle: str,
    raw_target_rect: RectEmu,
    modifiers: ResizeModifierMaskV1,
    x_matches: tuple[SnapAxisMatchV1, ...],
    y_matches: tuple[SnapAxisMatchV1, ...],
) -> list[_Proposal]:
    # No aspect coupling: the independently best candidate on each axis is the
    # globally best candidate for the score law.
    x = x_matches[0] if x_matches else None
    y = y_matches[0] if y_matches else None
    if x is None and y is None:
        return []

    rect = _solve_from_edges(
        base_rect=base_rect,
        handle=handle,
        raw_target_rect=raw_target_rect,
        modifiers=modifiers,
        x_edge=None if x is None else x.feedback.position_emu,
        y_edge=None if y is None else y.feedback.position_emu,
    )
    if rect is None:
        return []
    return [
        _Proposal(
            rect=rect,
            x_feedback=None if x is None else x.feedback,
            y_feedback=None if y is None else y.feedback,
            source="centered",
        )
    ]


def _aspect_proposals(
    *,
    base_rect: RectEmu,
    handle: str,
    raw_target_rect: RectEmu,
    modifiers: ResizeModifierMaskV1,
    x_matches: tuple[SnapAxisMatchV1, ...],
    y_matches: tuple[SnapAxisMatchV1, ...],
    tolerance_emu: int,
) -> list[_Proposal]:
    proposals: list[_Proposal] = []
    base_x_edge = _edge(base_rect, handle, "x")
    base_y_edge = _edge(base_rect, handle, "y")

    for match in x_matches:
        rect = _solve_from_edges(
            base_rect=base_rect,
            handle=handle,
            raw_target_rect=raw_target_rect,
            modifiers=modifiers,
            x_edge=match.feedback.position_emu,
            y_edge=base_y_edge,
        )
        if rect is None:
            continue
        # X-driven must actually land on the chosen X snap line.
        if _edge(rect, handle, "x") != match.feedback.position_emu:
            continue
        y_feedback = _best_other_feedback(
            matches=y_matches,
            final_edge=_edge(rect, handle, "y"),
            tolerance_emu=tolerance_emu,
        )
        proposals.append(
            _Proposal(
                rect=rect,
                x_feedback=match.feedback,
                y_feedback=y_feedback,
                source="x_driven",
            )
        )

    for match in y_matches:
        rect = _solve_from_edges(
            base_rect=base_rect,
            handle=handle,
            raw_target_rect=raw_target_rect,
            modifiers=modifiers,
            x_edge=base_x_edge,
            y_edge=match.feedback.position_emu,
        )
        if rect is None:
            continue
        if _edge(rect, handle, "y") != match.feedback.position_emu:
            continue
        x_feedback = _best_other_feedback(
            matches=x_matches,
            final_edge=_edge(rect, handle, "x"),
            tolerance_emu=tolerance_emu,
        )
        proposals.append(
            _Proposal(
                rect=rect,
                x_feedback=x_feedback,
                y_feedback=match.feedback,
                source="y_driven",
            )
        )
    return proposals


def _dedupe_and_choose(
    proposals: list[_Proposal],
    *,
    raw_target_rect: RectEmu,
    handle: str,
) -> _Proposal | None:
    by_rect: dict[tuple[int, int, int, int], _Proposal] = {}
    for proposal in proposals:
        key = (
            proposal.rect.x,
            proposal.rect.y,
            proposal.rect.width,
            proposal.rect.height,
        )
        prior = by_rect.get(key)
        if prior is None or _proposal_score(
            proposal,
            raw_target_rect=raw_target_rect,
            handle=handle,
        ) < _proposal_score(
            prior,
            raw_target_rect=raw_target_rect,
            handle=handle,
        ):
            by_rect[key] = proposal
    if not by_rect:
        return None
    return min(
        by_rect.values(),
        key=lambda proposal: _proposal_score(
            proposal,
            raw_target_rect=raw_target_rect,
            handle=handle,
        ),
    )


def plan_resize_constraint_snap_compose_v1(
    *,
    base_rect: RectEmu,
    handle: str,
    raw_target_rect: RectEmu,
    modifiers: ResizeModifierMaskV1,
    snap_index: SnapIndexV1,
    tolerance_emu: int,
    excluded_node_ids: tuple[str, ...] = (),
) -> ResizeConstraintSnapComposeV1:
    if not isinstance(modifiers, ResizeModifierMaskV1):
        _fail("modifiers must be ResizeModifierMaskV1")
    if not modifiers.centered and not modifiers.aspect_lock:
        _fail("compositor requires a non-empty centered/aspect modifier mask")
    if not isinstance(snap_index, SnapIndexV1):
        _fail("snap_index must be SnapIndexV1")

    try:
        constrained_plan = plan_resize_constraint_v1(
            base_rect=base_rect,
            handle=handle,
            raw_target_rect=raw_target_rect,
            modifiers=modifiers,
        )
        x_matches = _axis_matches(
            snap_index=snap_index,
            axis="x",
            handle=handle,
            raw_target_rect=raw_target_rect,
            tolerance_emu=tolerance_emu,
            excluded_node_ids=excluded_node_ids,
        )
        y_matches = _axis_matches(
            snap_index=snap_index,
            axis="y",
            handle=handle,
            raw_target_rect=raw_target_rect,
            tolerance_emu=tolerance_emu,
            excluded_node_ids=excluded_node_ids,
        )
    except (ResizeConstraintError, SnapIndexError) as exc:
        raise ResizeConstraintSnapComposeError(str(exc)) from exc

    # Shift on an edge is deliberately inert in ResizeConstraintV1. With no
    # centered constraint there is therefore no coupled invariant to preserve;
    # reuse the ordinary resize snap plan directly.
    if not modifiers.centered and not constrained_plan.aspect_applied:
        try:
            ordinary = plan_resize_snap_v1(
                handle=handle,
                ordinary_target=constrained_plan.constrained_rect,
                snap_index=snap_index,
                tolerance_emu=tolerance_emu,
                excluded_node_ids=excluded_node_ids,
            )
        except ResizeSnapError as exc:
            raise ResizeConstraintSnapComposeError(str(exc)) from exc
        axis_count = int(ordinary.x_feedback is not None) + int(
            ordinary.y_feedback is not None
        )
        return ResizeConstraintSnapComposeV1(
            protocol_version="chaptera.resize-constraint-snap-compose.v1",
            handle=handle,
            modifiers=modifiers,
            base_rect=base_rect,
            raw_target_rect=raw_target_rect,
            unsnapped_constrained_rect=constrained_plan.constrained_rect,
            final_rect=ordinary.corrected_target,
            x_feedback=ordinary.x_feedback,
            y_feedback=ordinary.y_feedback,
            snapped_axis_count=axis_count,
            source="ordinary" if axis_count else "fallback",
        )

    if constrained_plan.aspect_applied:
        proposals = _aspect_proposals(
            base_rect=base_rect,
            handle=handle,
            raw_target_rect=raw_target_rect,
            modifiers=modifiers,
            x_matches=x_matches,
            y_matches=y_matches,
            tolerance_emu=tolerance_emu,
        )
    else:
        proposals = _centered_proposals(
            base_rect=base_rect,
            handle=handle,
            raw_target_rect=raw_target_rect,
            modifiers=modifiers,
            x_matches=x_matches,
            y_matches=y_matches,
        )

    winner = _dedupe_and_choose(
        proposals,
        raw_target_rect=raw_target_rect,
        handle=handle,
    )
    if winner is None:
        winner = _Proposal(
            rect=constrained_plan.constrained_rect,
            x_feedback=None,
            y_feedback=None,
            source="fallback",
        )

    axis_count = int(winner.x_feedback is not None) + int(
        winner.y_feedback is not None
    )
    return ResizeConstraintSnapComposeV1(
        protocol_version="chaptera.resize-constraint-snap-compose.v1",
        handle=handle,
        modifiers=modifiers,
        base_rect=base_rect,
        raw_target_rect=raw_target_rect,
        unsnapped_constrained_rect=constrained_plan.constrained_rect,
        final_rect=winner.rect,
        x_feedback=winner.x_feedback,
        y_feedback=winner.y_feedback,
        snapped_axis_count=axis_count,
        source=winner.source,
    )
