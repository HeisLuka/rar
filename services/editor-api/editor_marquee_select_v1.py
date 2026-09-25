#!/usr/bin/env python3
"""Empty-canvas marquee selection transaction V1."""

from __future__ import annotations

from dataclasses import dataclass, replace
from math import hypot
from typing import Literal

from box_select_plan_v1 import (
    BoxSelectCandidateV1,
    BoxSelectPlanError,
    PointEmu,
    RectEmu,
    plan_box_select_v1,
)
from editor_multi_select_v1 import (
    AuthoredMultiSelectionStateV1,
    AuthoredSelectableNodeV1,
)


class EditorMarqueeSelectError(ValueError):
    pass


@dataclass(frozen=True)
class MarqueeSelectTransactionV1:
    protocol_version: Literal["chaptera.marquee-select-transaction.v1"]
    page_id: str
    gesture_token: str
    start_document_point: PointEmu
    start_screen_x: float
    start_screen_y: float
    drag_threshold_px: float
    pre_gesture_selection: AuthoredMultiSelectionStateV1
    stage: Literal["armed", "dragging"]
    current_document_point: PointEmu
    overlay_bounds: RectEmu | None


@dataclass(frozen=True)
class MarqueeStartResultV1:
    status: Literal["started", "declined_nonempty_hit", "suppressed_modifier"]
    transaction: MarqueeSelectTransactionV1 | None
    document_mutation_count: Literal[0]
    reason: str | None


@dataclass(frozen=True)
class MarqueeUpdateResultV1:
    transaction: MarqueeSelectTransactionV1
    canonical_selection: AuthoredMultiSelectionStateV1
    threshold_crossed: bool
    document_mutation_count: Literal[0]


@dataclass(frozen=True)
class MarqueeFinishResultV1:
    status: Literal["selected", "cleared", "no_change", "cancelled"]
    selection: AuthoredMultiSelectionStateV1
    selected_node_ids: tuple[str, ...]
    primary_node_id: str | None
    selection_bounds: RectEmu | None
    document_mutation_count: Literal[0]
    revision_created: Literal[False]
    reason: str | None


def _validate_float(value: float, label: str) -> float:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise EditorMarqueeSelectError(f"{label} must be numeric")
    return float(value)


def _overlay_bounds(start: PointEmu, end: PointEmu) -> RectEmu | None:
    left=min(start.x,end.x)
    top=min(start.y,end.y)
    right=max(start.x,end.x)
    bottom=max(start.y,end.y)
    if left==right or top==bottom:
        return None
    return RectEmu(left,top,right-left,bottom-top)


def begin_marquee_select_v1(
    *,
    page_id: str,
    gesture_token: str,
    start_document_point: PointEmu,
    start_screen_x: float,
    start_screen_y: float,
    drag_threshold_px: float,
    pre_gesture_selection: AuthoredMultiSelectionStateV1,
    started_on_empty_canvas: bool,
    modifier_state: Literal["none","shift","ctrl","cmd","alt"]="none",
) -> MarqueeStartResultV1:
    if not isinstance(page_id,str) or not page_id:
        raise EditorMarqueeSelectError("page_id is required")
    if not isinstance(gesture_token,str) or not gesture_token:
        raise EditorMarqueeSelectError("gesture_token is required")
    if pre_gesture_selection.page_id != page_id:
        raise EditorMarqueeSelectError("pre-gesture selection belongs to another page")
    sx=_validate_float(start_screen_x,"start_screen_x")
    sy=_validate_float(start_screen_y,"start_screen_y")
    threshold=_validate_float(drag_threshold_px,"drag_threshold_px")
    if threshold <= 0:
        raise EditorMarqueeSelectError("drag_threshold_px must be positive")
    if modifier_state != "none":
        return MarqueeStartResultV1(
            status="suppressed_modifier",
            transaction=None,
            document_mutation_count=0,
            reason="modifier-composed marquee is outside V1",
        )
    if not started_on_empty_canvas:
        return MarqueeStartResultV1(
            status="declined_nonempty_hit",
            transaction=None,
            document_mutation_count=0,
            reason="object/text/handle route owns pointer-down",
        )
    tx=MarqueeSelectTransactionV1(
        protocol_version="chaptera.marquee-select-transaction.v1",
        page_id=page_id,
        gesture_token=gesture_token,
        start_document_point=start_document_point,
        start_screen_x=sx,
        start_screen_y=sy,
        drag_threshold_px=threshold,
        pre_gesture_selection=pre_gesture_selection,
        stage="armed",
        current_document_point=start_document_point,
        overlay_bounds=None,
    )
    return MarqueeStartResultV1(
        status="started",
        transaction=tx,
        document_mutation_count=0,
        reason=None,
    )


def update_marquee_select_v1(
    *,
    transaction: MarqueeSelectTransactionV1,
    current_document_point: PointEmu,
    current_screen_x: float,
    current_screen_y: float,
) -> MarqueeUpdateResultV1:
    if not isinstance(transaction,MarqueeSelectTransactionV1):
        raise EditorMarqueeSelectError("MarqueeSelectTransactionV1 is required")
    dx=_validate_float(current_screen_x,"current_screen_x")-transaction.start_screen_x
    dy=_validate_float(current_screen_y,"current_screen_y")-transaction.start_screen_y
    crossed=hypot(dx,dy) >= transaction.drag_threshold_px
    stage="dragging" if transaction.stage=="dragging" or crossed else "armed"
    overlay=(
        _overlay_bounds(transaction.start_document_point,current_document_point)
        if stage=="dragging"
        else None
    )
    next_tx=replace(
        transaction,
        stage=stage,
        current_document_point=current_document_point,
        overlay_bounds=overlay,
    )
    return MarqueeUpdateResultV1(
        transaction=next_tx,
        canonical_selection=transaction.pre_gesture_selection,
        threshold_crossed=(stage=="dragging"),
        document_mutation_count=0,
    )


def _current_page_authored_candidates(
    *,
    page_id: str,
    candidates: tuple[AuthoredSelectableNodeV1,...],
) -> tuple[BoxSelectCandidateV1,...]:
    out=[]
    seen=set()
    for candidate in candidates:
        if not isinstance(candidate,AuthoredSelectableNodeV1):
            raise EditorMarqueeSelectError("candidates must be AuthoredSelectableNodeV1 values")
        if candidate.page_id != page_id or not candidate.authored_direct:
            continue
        if candidate.node_id in seen:
            raise EditorMarqueeSelectError("duplicate authored candidate NodeId")
        seen.add(candidate.node_id)
        if candidate.width_emu <= 0 or candidate.height_emu <= 0:
            continue
        out.append(
            BoxSelectCandidateV1(
                node_id=candidate.node_id,
                visual_bounds=RectEmu(
                    candidate.x_emu,
                    candidate.y_emu,
                    candidate.width_emu,
                    candidate.height_emu,
                ),
            )
        )
    return tuple(out)


def finish_marquee_select_v1(
    *,
    transaction: MarqueeSelectTransactionV1,
    release_document_point: PointEmu,
    current_candidates: tuple[AuthoredSelectableNodeV1,...],
) -> MarqueeFinishResultV1:
    if not isinstance(transaction,MarqueeSelectTransactionV1):
        raise EditorMarqueeSelectError("MarqueeSelectTransactionV1 is required")
    if transaction.stage!="dragging":
        return MarqueeFinishResultV1(
            status="no_change",
            selection=transaction.pre_gesture_selection,
            selected_node_ids=transaction.pre_gesture_selection.selected_node_ids,
            primary_node_id=transaction.pre_gesture_selection.primary_node_id,
            selection_bounds=None,
            document_mutation_count=0,
            revision_created=False,
            reason="drag threshold was not crossed",
        )

    try:
        plan=plan_box_select_v1(
            start=transaction.start_document_point,
            end=release_document_point,
            candidates=_current_page_authored_candidates(
                page_id=transaction.page_id,
                candidates=current_candidates,
            ),
        )
    except BoxSelectPlanError as exc:
        raise EditorMarqueeSelectError(str(exc)) from exc

    ids=plan.selected_node_ids
    primary=ids[0] if len(ids)==1 else None
    selection=AuthoredMultiSelectionStateV1(
        protocol_version="chaptera.authored-multi-selection.v1",
        page_id=transaction.page_id,
        selected_node_ids=ids,
        primary_node_id=primary,
    )
    return MarqueeFinishResultV1(
        status="selected" if ids else "cleared",
        selection=selection,
        selected_node_ids=ids,
        primary_node_id=primary,
        selection_bounds=plan.selection_bounds,
        document_mutation_count=0,
        revision_created=False,
        reason=None,
    )


def cancel_marquee_select_v1(
    transaction: MarqueeSelectTransactionV1,
) -> MarqueeFinishResultV1:
    if not isinstance(transaction,MarqueeSelectTransactionV1):
        raise EditorMarqueeSelectError("MarqueeSelectTransactionV1 is required")
    s=transaction.pre_gesture_selection
    return MarqueeFinishResultV1(
        status="cancelled",
        selection=s,
        selected_node_ids=s.selected_node_ids,
        primary_node_id=s.primary_node_id,
        selection_bounds=None,
        document_mutation_count=0,
        revision_created=False,
        reason="pre-gesture selection restored",
    )
