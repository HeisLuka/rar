#!/usr/bin/env python3
"""Deterministic transient viewport reveal for canonical text focus V1.

Product adapters own screen/ViewTransform normalization. This contract consumes
one authoritative canvas-EMU viewport receipt plus page placements and computes
only the minimum pan needed to reveal the exact canonical focus caret stop.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from resolved_text_caret_map_v1 import (
    CaretStopV1,
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    caret_map_hash_v1,
    resolve_story_position_v1,
)
from text_edit_session_v1 import TextEditSessionV1
from text_selection_state_v1 import TextSelectionStateV1


MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU

RevealReasonV1 = Literal[
    "accepted_edit",
    "ime_commit",
    "navigation",
    "undo_redo",
    "programmatic_jump",
    "authoritative_reflow",
]


class TextCaretRevealError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class PageCanvasPlacementV1:
    page_id: str
    canvas_origin_x_emu: int
    canvas_origin_y_emu: int


@dataclass(frozen=True)
class TextViewportReceiptV1:
    protocol_version: Literal["chaptera.text-viewport-receipt.v1"]
    view_revision_id: str
    viewport_x_emu: int
    viewport_y_emu: int
    viewport_width_emu: int
    viewport_height_emu: int
    emu_per_css_px: float
    safe_inset_left_emu: int
    safe_inset_right_emu: int
    safe_inset_top_emu: int
    safe_inset_bottom_emu: int
    page_placements: tuple[PageCanvasPlacementV1, ...]


@dataclass(frozen=True)
class TextCaretRevealResultV1:
    protocol_version: Literal["chaptera.text-caret-reveal-result.v1"]
    status: Literal["no_op", "pan", "reveal_unavailable", "reveal_unsupported", "reconcile_required"]
    reason: str | None
    reveal_reason: RevealReasonV1
    story_id: str
    focus_scalar: int
    target_stop_id: str | None
    target_page_id: str | None
    target_frame_id: str | None
    pan_delta_x_emu: int
    pan_delta_y_emu: int
    resulting_viewport_x_emu: int
    resulting_viewport_y_emu: int
    emu_per_css_px: float
    selection: TextSelectionStateV1
    selection_unchanged: Literal[True]
    typing_state_unchanged: Literal[True]
    preferred_inline_x_unchanged: Literal[True]
    document_mutation_count: Literal[0]
    undo_history_changed: Literal[False]


def _fail(code: str, message: str) -> None:
    raise TextCaretRevealError(code, message)


def _safe_emu(value: int, label: str) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < MIN_SAFE_EMU
        or value > MAX_SAFE_EMU
    ):
        _fail("invalid_viewport", f"{label} must be JavaScript-safe EMU")
    return value


def _validate_viewport(
    viewport: TextViewportReceiptV1,
    *,
    expected_view_revision_id: str,
) -> dict[str, PageCanvasPlacementV1]:
    if not isinstance(viewport, TextViewportReceiptV1):
        _fail("invalid_viewport", "TextViewportReceiptV1 is required")
    if viewport.protocol_version != "chaptera.text-viewport-receipt.v1":
        _fail("invalid_viewport", "viewport protocol mismatch")
    if (
        not isinstance(expected_view_revision_id, str)
        or not expected_view_revision_id
        or viewport.view_revision_id != expected_view_revision_id
    ):
        _fail("reconcile_required", "viewport/ViewTransform receipt is stale")
    for label, value in (
        ("viewport_x_emu", viewport.viewport_x_emu),
        ("viewport_y_emu", viewport.viewport_y_emu),
        ("viewport_width_emu", viewport.viewport_width_emu),
        ("viewport_height_emu", viewport.viewport_height_emu),
        ("safe_inset_left_emu", viewport.safe_inset_left_emu),
        ("safe_inset_right_emu", viewport.safe_inset_right_emu),
        ("safe_inset_top_emu", viewport.safe_inset_top_emu),
        ("safe_inset_bottom_emu", viewport.safe_inset_bottom_emu),
    ):
        _safe_emu(value, label)
    if viewport.viewport_width_emu <= 0 or viewport.viewport_height_emu <= 0:
        _fail("invalid_viewport", "viewport extent must be positive")
    if (
        viewport.safe_inset_left_emu < 0
        or viewport.safe_inset_right_emu < 0
        or viewport.safe_inset_top_emu < 0
        or viewport.safe_inset_bottom_emu < 0
        or viewport.safe_inset_left_emu + viewport.safe_inset_right_emu
        >= viewport.viewport_width_emu
        or viewport.safe_inset_top_emu + viewport.safe_inset_bottom_emu
        >= viewport.viewport_height_emu
    ):
        _fail("invalid_viewport", "safe visible inset leaves no positive viewport area")
    if (
        not isinstance(viewport.emu_per_css_px, (int, float))
        or isinstance(viewport.emu_per_css_px, bool)
        or viewport.emu_per_css_px <= 0
    ):
        _fail("invalid_viewport", "emu_per_css_px must be positive")
    if not isinstance(viewport.page_placements, tuple):
        _fail("invalid_viewport", "page_placements must be tuple")
    placements = {}
    for item in viewport.page_placements:
        if not isinstance(item, PageCanvasPlacementV1):
            _fail("invalid_viewport", "page placement is malformed")
        if not isinstance(item.page_id, str) or not item.page_id:
            _fail("invalid_viewport", "page placement page_id is required")
        if item.page_id in placements:
            _fail("invalid_viewport", "page placements must have unique page_id")
        _safe_emu(item.canvas_origin_x_emu, "page canvas origin x")
        _safe_emu(item.canvas_origin_y_emu, "page canvas origin y")
        placements[item.page_id] = item
    return placements


def _validate_session_layout(
    *,
    session: TextEditSessionV1,
    caret_map: ResolvedTextCaretMapV1,
) -> None:
    if not isinstance(session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 is required")
    if session.focus_owner != "story_text":
        _fail("reconcile_required", "text Story is not current focus owner")
    if not isinstance(caret_map, ResolvedTextCaretMapV1):
        _fail("invalid_caret_map", "ResolvedTextCaretMapV1 is required")
    if caret_map.story_id != session.story_id:
        _fail("reconcile_required", "caret map targets a different Story")
    if caret_map.layout_revision_id != session.layout_revision_id:
        _fail("reconcile_required", "session layout receipt is stale")
    if caret_map_hash_v1(caret_map) != session.caret_map_hash:
        _fail("reconcile_required", "session caret-map receipt differs from authoritative map")
    selection = session.selection
    if selection.story_id != session.story_id or selection.revision_id != session.revision_id:
        _fail("reconcile_required", "session selection belongs to a different Story/revision")


def _resolve_focus_stop(
    *,
    session: TextEditSessionV1,
    caret_map: ResolvedTextCaretMapV1,
) -> CaretStopV1:
    selection = session.selection
    stop_id = (
        selection.focus_visual_stop_id
        if selection.projection_state == "projected"
        and selection.layout_revision_id == caret_map.layout_revision_id
        else None
    )
    try:
        return resolve_story_position_v1(
            caret_map=caret_map,
            scalar_boundary=selection.focus_scalar,
            stop_id=stop_id,
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        if exc.code in {"unplaced_story_position"}:
            _fail("reveal_unavailable", "focus caret has no placed geometry")
        if exc.code in {
            "caret_affinity_required",
            "internal_cluster_unsupported",
            "invalid_caret_affinity",
        }:
            _fail("reveal_unsupported", str(exc))
        if exc.code in {"stale_layout_map"}:
            _fail("reconcile_required", str(exc))
        _fail(exc.code, str(exc))


def _unavailable_result(
    *,
    status: Literal["reveal_unavailable", "reveal_unsupported", "reconcile_required"],
    reason: str,
    reveal_reason: RevealReasonV1,
    session: TextEditSessionV1,
    viewport: TextViewportReceiptV1,
) -> TextCaretRevealResultV1:
    return TextCaretRevealResultV1(
        protocol_version="chaptera.text-caret-reveal-result.v1",
        status=status,
        reason=reason,
        reveal_reason=reveal_reason,
        story_id=session.story_id,
        focus_scalar=session.selection.focus_scalar,
        target_stop_id=None,
        target_page_id=None,
        target_frame_id=None,
        pan_delta_x_emu=0,
        pan_delta_y_emu=0,
        resulting_viewport_x_emu=viewport.viewport_x_emu,
        resulting_viewport_y_emu=viewport.viewport_y_emu,
        emu_per_css_px=float(viewport.emu_per_css_px),
        selection=session.selection,
        selection_unchanged=True,
        typing_state_unchanged=True,
        preferred_inline_x_unchanged=True,
        document_mutation_count=0,
        undo_history_changed=False,
    )


def plan_text_caret_reveal_v1(
    *,
    session: TextEditSessionV1,
    caret_map: ResolvedTextCaretMapV1,
    viewport: TextViewportReceiptV1,
    expected_view_revision_id: str,
    reveal_reason: RevealReasonV1,
) -> TextCaretRevealResultV1:
    if reveal_reason not in {
        "accepted_edit",
        "ime_commit",
        "navigation",
        "undo_redo",
        "programmatic_jump",
        "authoritative_reflow",
    }:
        _fail("unsupported_reveal_reason", "pointer/autoscroll/PageUp-style viewport motion is not caret reveal")

    try:
        placements = _validate_viewport(
            viewport,
            expected_view_revision_id=expected_view_revision_id,
        )
        _validate_session_layout(session=session, caret_map=caret_map)
    except TextCaretRevealError as exc:
        if exc.code == "reconcile_required":
            return _unavailable_result(
                status="reconcile_required",
                reason=str(exc),
                reveal_reason=reveal_reason,
                session=session,
                viewport=viewport,
            )
        raise

    try:
        stop = _resolve_focus_stop(session=session, caret_map=caret_map)
    except TextCaretRevealError as exc:
        if exc.code in {"reveal_unavailable", "reveal_unsupported", "reconcile_required"}:
            return _unavailable_result(
                status=exc.code,
                reason=str(exc),
                reveal_reason=reveal_reason,
                session=session,
                viewport=viewport,
            )
        raise

    placement = placements.get(stop.page_id)
    if placement is None:
        return _unavailable_result(
            status="reveal_unavailable",
            reason="authoritative target page has no canvas placement receipt",
            reveal_reason=reveal_reason,
            session=session,
            viewport=viewport,
        )

    caret_x = _safe_emu(
        placement.canvas_origin_x_emu + stop.page_x_emu,
        "target caret canvas x",
    )
    caret_top = _safe_emu(
        placement.canvas_origin_y_emu + stop.page_y_top_emu,
        "target caret canvas top",
    )
    caret_bottom = _safe_emu(
        placement.canvas_origin_y_emu + stop.page_y_bottom_emu,
        "target caret canvas bottom",
    )

    safe_left = viewport.viewport_x_emu + viewport.safe_inset_left_emu
    safe_right = (
        viewport.viewport_x_emu
        + viewport.viewport_width_emu
        - viewport.safe_inset_right_emu
    )
    safe_top = viewport.viewport_y_emu + viewport.safe_inset_top_emu
    safe_bottom = (
        viewport.viewport_y_emu
        + viewport.viewport_height_emu
        - viewport.safe_inset_bottom_emu
    )
    safe_height = safe_bottom - safe_top
    caret_height = caret_bottom - caret_top
    if caret_height > safe_height:
        return _unavailable_result(
            status="reveal_unsupported",
            reason="caret line is taller than configured safe visible area",
            reveal_reason=reveal_reason,
            session=session,
            viewport=viewport,
        )

    if caret_x < safe_left:
        dx = caret_x - safe_left
    elif caret_x > safe_right:
        dx = caret_x - safe_right
    else:
        dx = 0

    if caret_top < safe_top:
        dy = caret_top - safe_top
    elif caret_bottom > safe_bottom:
        dy = caret_bottom - safe_bottom
    else:
        dy = 0

    new_x = _safe_emu(viewport.viewport_x_emu + dx, "resulting viewport x")
    new_y = _safe_emu(viewport.viewport_y_emu + dy, "resulting viewport y")
    return TextCaretRevealResultV1(
        protocol_version="chaptera.text-caret-reveal-result.v1",
        status="no_op" if dx == 0 and dy == 0 else "pan",
        reason=None,
        reveal_reason=reveal_reason,
        story_id=session.story_id,
        focus_scalar=session.selection.focus_scalar,
        target_stop_id=stop.stop_id,
        target_page_id=stop.page_id,
        target_frame_id=stop.frame_id,
        pan_delta_x_emu=dx,
        pan_delta_y_emu=dy,
        resulting_viewport_x_emu=new_x,
        resulting_viewport_y_emu=new_y,
        emu_per_css_px=float(viewport.emu_per_css_px),
        selection=session.selection,
        selection_unchanged=True,
        typing_state_unchanged=True,
        preferred_inline_x_unchanged=True,
        document_mutation_count=0,
        undo_history_changed=False,
    )
