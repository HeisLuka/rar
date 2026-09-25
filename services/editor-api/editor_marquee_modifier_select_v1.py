#!/usr/bin/env python3
"""Release-time modifier composition for authored marquee selection V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from editor_marquee_select_v1 import (
    MarqueeFinishResultV1,
    MarqueeSelectTransactionV1,
    begin_marquee_select_v1,
    finish_marquee_select_v1,
)
from editor_multi_select_v1 import (
    AuthoredMultiSelectionStateV1,
    AuthoredSelectableNodeV1,
)
from object_selection_compose_v1 import compose_object_selection_v1
from object_selection_target_v1 import DirectNodeSelectionV1
from box_select_plan_v1 import PointEmu


ReleaseModifierV1 = Literal["none","shift","ctrl","cmd","alt"]


@dataclass(frozen=True)
class MarqueeModifierFinishResultV1:
    protocol_version: Literal["chaptera.marquee-modifier-finish-result.v1"]
    status: Literal["selected","cleared","no_change"]
    selection: AuthoredMultiSelectionStateV1
    candidate_node_ids: tuple[str,...]
    release_modifier: ReleaseModifierV1
    composition_mode: Literal["replace","toggle"]
    document_mutation_count: Literal[0]
    revision_created: Literal[False]
    reason: str | None


def begin_modifier_capable_marquee_v1(
    *,
    page_id: str,
    gesture_token: str,
    start_document_point: PointEmu,
    start_screen_x: float,
    start_screen_y: float,
    drag_threshold_px: float,
    pre_gesture_selection: AuthoredMultiSelectionStateV1,
    started_on_empty_canvas: bool,
):
    # Modifier state is intentionally not captured at start. V1 samples it once
    # at release; pointer-down Shift/Ctrl/Cmd/Alt does not alter geometry law.
    return begin_marquee_select_v1(
        page_id=page_id,
        gesture_token=gesture_token,
        start_document_point=start_document_point,
        start_screen_x=start_screen_x,
        start_screen_y=start_screen_y,
        drag_threshold_px=drag_threshold_px,
        pre_gesture_selection=pre_gesture_selection,
        started_on_empty_canvas=started_on_empty_canvas,
        modifier_state="none",
    )


def _compose_shift_toggle(
    *,
    pre: AuthoredMultiSelectionStateV1,
    candidates: tuple[str,...],
) -> AuthoredMultiSelectionStateV1:
    base_set=tuple(
        DirectNodeSelectionV1(pre.page_id,node_id)
        for node_id in pre.selected_node_ids
    )
    base_primary=(
        None
        if pre.primary_node_id is None
        else DirectNodeSelectionV1(pre.page_id,pre.primary_node_id)
    )
    candidate_set=tuple(
        DirectNodeSelectionV1(pre.page_id,node_id)
        for node_id in candidates
    )
    composed=compose_object_selection_v1(
        base_set=base_set,
        base_primary=base_primary,
        candidate_set=candidate_set,
        mode="toggle",
    )
    ids=tuple(target.node_id for target in composed.selected_set)
    primary=None if composed.primary is None else composed.primary.node_id
    return AuthoredMultiSelectionStateV1(
        protocol_version="chaptera.authored-multi-selection.v1",
        page_id=pre.page_id,
        selected_node_ids=ids,
        primary_node_id=primary,
    )


def finish_marquee_with_release_modifier_v1(
    *,
    transaction: MarqueeSelectTransactionV1,
    release_document_point: PointEmu,
    current_candidates: tuple[AuthoredSelectableNodeV1,...],
    release_modifier: ReleaseModifierV1,
) -> MarqueeModifierFinishResultV1:
    if release_modifier not in {"none","shift","ctrl","cmd","alt"}:
        raise ValueError("unsupported release modifier")

    base=finish_marquee_select_v1(
        transaction=transaction,
        release_document_point=release_document_point,
        current_candidates=current_candidates,
    )
    if base.status=="no_change":
        return MarqueeModifierFinishResultV1(
            protocol_version="chaptera.marquee-modifier-finish-result.v1",
            status="no_change",
            selection=base.selection,
            candidate_node_ids=(),
            release_modifier=release_modifier,
            composition_mode="toggle" if release_modifier=="shift" else "replace",
            document_mutation_count=0,
            revision_created=False,
            reason=base.reason,
        )

    candidates=base.selected_node_ids

    if release_modifier=="shift":
        selection=_compose_shift_toggle(
            pre=transaction.pre_gesture_selection,
            candidates=candidates,
        )
        status="selected" if selection.selected_node_ids else "cleared"
        return MarqueeModifierFinishResultV1(
            protocol_version="chaptera.marquee-modifier-finish-result.v1",
            status=status,
            selection=selection,
            candidate_node_ids=candidates,
            release_modifier=release_modifier,
            composition_mode="toggle",
            document_mutation_count=0,
            revision_created=False,
            reason=None,
        )

    # none/Ctrl/Cmd/Alt deliberately keep base Replace semantics in V1.
    return MarqueeModifierFinishResultV1(
        protocol_version="chaptera.marquee-modifier-finish-result.v1",
        status=base.status,
        selection=base.selection,
        candidate_node_ids=candidates,
        release_modifier=release_modifier,
        composition_mode="replace",
        document_mutation_count=0,
        revision_created=False,
        reason=(
            None
            if release_modifier=="none"
            else f"{release_modifier} marquee modifier is reserved/no-op in V1"
        ),
    )
