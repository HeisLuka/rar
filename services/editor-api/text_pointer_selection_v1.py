#!/usr/bin/env python3
"""Canonical pointer click/Shift-click/drag selection over resolved Story geometry V1.

Product adapters own screen->page EMU conversion and focused-frame routing.
This module starts at authoritative page-space EMU and never accepts DOM/widget
selection as semantic state.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from resolved_text_caret_map_v1 import (
    CaretStopV1,
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    hit_test_story_position_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_composition_session_v1 import TextCompositionSessionV1
from text_format_overlay_v1 import TextFormatOverlayStateV1
from text_selection_state_v1 import (
    TextSelectionStateError,
    TextSelectionStateV1,
    build_text_selection_state_v1,
    edit_domain_id_v1,
    project_selection_state_v1,
    validate_selection_state_v1,
)
from text_typing_format_state_v1 import (
    TextTypingFormatStateV1,
    clear_typing_state_for_context_change_v1,
)


class TextPointerSelectionError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextPointerSelectionResultV1:
    protocol_version: Literal["chaptera.text-pointer-selection-result.v1"]
    selection: TextSelectionStateV1
    typing_state: TextTypingFormatStateV1 | None
    hit_stop: CaretStopV1 | None
    autoscroll_needed: bool
    document_mutation_count: Literal[0]


@dataclass(frozen=True)
class TextPointerDragSessionV1:
    protocol_version: Literal["chaptera.text-pointer-drag-session.v1"]
    story_id: str
    revision_id: str
    edit_domain_id: str
    layout_revision_id: str
    anchor_scalar: int
    anchor_stop_id: str
    current_selection: TextSelectionStateV1


@dataclass(frozen=True)
class TextPointerDragUpdateV1:
    protocol_version: Literal["chaptera.text-pointer-drag-update.v1"]
    drag_session: TextPointerDragSessionV1
    result: TextPointerSelectionResultV1


def _fail(code: str, message: str) -> None:
    raise TextPointerSelectionError(code, message)


def _require_composition_resolution(
    *,
    composition_session: TextCompositionSessionV1 | None,
    composition_resolution_acknowledged: bool,
) -> None:
    if composition_session is None:
        return
    if not isinstance(composition_session, TextCompositionSessionV1):
        _fail("invalid_composition", "composition_session must be TextCompositionSessionV1")
    if not composition_resolution_acknowledged:
        _fail(
            "composition_transition_required",
            "active composition must be cancelled/committed/reconciled before pointer selection",
        )


def _validate_context(
    *,
    domain: StoryEditDomainV1,
    revision_id: str,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
) -> None:
    if not isinstance(domain, StoryEditDomainV1):
        _fail("invalid_edit_domain", "StoryEditDomainV1 is required")
    if domain.status != "known":
        _fail("edit_domain_unknown", "ordinary pointer selection domain is unknown")
    if not isinstance(revision_id, str) or not revision_id:
        _fail("invalid_revision", "revision_id is required")
    if not isinstance(expected_layout_revision_id, str) or not expected_layout_revision_id:
        _fail("invalid_layout_revision", "expected_layout_revision_id is required")
    if not isinstance(caret_map, ResolvedTextCaretMapV1):
        _fail("invalid_caret_map", "ResolvedTextCaretMapV1 is required")
    if caret_map.layout_revision_id != expected_layout_revision_id:
        _fail("stale_layout_map", "caret map belongs to a different layout revision")
    if caret_map.story_id != domain.story_id:
        _fail("selection_story_mismatch", "caret map and edit domain target different Stories")
    if caret_map.story_scalar_len != domain.raw_scalar_len:
        _fail("stale_layout_map", "caret map Story extent differs from current edit domain")
    if not isinstance(format_state, TextFormatOverlayStateV1):
        _fail("invalid_format_state", "TextFormatOverlayStateV1 is required")
    if format_state.story_id != domain.story_id:
        _fail("selection_story_mismatch", "format state targets a different Story")
    if format_state.story_scalar_len != domain.raw_scalar_len:
        _fail("typing_context_mismatch", "format state Story extent differs from edit domain")


def _admitted_caret_map(
    *,
    caret_map: ResolvedTextCaretMapV1,
    domain: StoryEditDomainV1,
) -> ResolvedTextCaretMapV1:
    if domain.caret_start_boundary is None or domain.caret_end_boundary is None:
        _fail("edit_domain_unknown", "ordinary caret boundaries are unavailable")
    start = domain.caret_start_boundary
    end = domain.caret_end_boundary
    admitted = tuple(
        stop
        for stop in caret_map.caret_stops
        if start <= stop.scalar_boundary <= end
    )
    if not admitted:
        _fail("unplaced_hit_test", "no ordinary editable caret stop is materialized")

    admitted_lines = {stop.line_id for stop in admitted}
    lines = tuple(line for line in caret_map.lines if line.line_id in admitted_lines)
    return replace(caret_map, lines=lines, caret_stops=admitted)


def _hit(
    *,
    caret_map: ResolvedTextCaretMapV1,
    domain: StoryEditDomainV1,
    page_id: str,
    page_x_emu: int,
    page_y_emu: int,
    expected_layout_revision_id: str,
) -> CaretStopV1:
    filtered = _admitted_caret_map(caret_map=caret_map, domain=domain)
    try:
        stop = hit_test_story_position_v1(
            caret_map=filtered,
            page_id=page_id,
            page_x_emu=page_x_emu,
            page_y_emu=page_y_emu,
            expected_layout_revision_id=expected_layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        _fail(exc.code, str(exc))
    if domain.caret_start_boundary is None or domain.caret_end_boundary is None:
        _fail("edit_domain_unknown", "ordinary caret boundaries are unavailable")
    if not domain.caret_start_boundary <= stop.scalar_boundary <= domain.caret_end_boundary:
        _fail("protected_story_structure", "pointer hit resolved outside ordinary edit domain")
    return stop


def _project(
    *,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    anchor_stop_id: str | None,
    focus_stop_id: str | None,
) -> TextSelectionStateV1:
    try:
        return project_selection_state_v1(
            state=selection,
            domain=domain,
            caret_map=caret_map,
            anchor_stop_id=anchor_stop_id,
            focus_stop_id=focus_stop_id,
        ).state
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))


def _typing_after_pointer_relocation(
    *,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    format_state: TextFormatOverlayStateV1,
) -> TextTypingFormatStateV1 | None:
    try:
        return clear_typing_state_for_context_change_v1(
            selection=selection,
            domain=domain,
            format_state=format_state,
        )
    except ValueError as exc:
        _fail(getattr(exc, "code", "typing_context_mismatch"), str(exc))


def pointer_click_v1(
    *,
    domain: StoryEditDomainV1,
    revision_id: str,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    page_id: str,
    page_x_emu: int,
    page_y_emu: int,
    expected_layout_revision_id: str,
    existing_selection: TextSelectionStateV1 | None = None,
    shift: bool = False,
    composition_session: TextCompositionSessionV1 | None = None,
    composition_resolution_acknowledged: bool = False,
) -> TextPointerSelectionResultV1:
    _require_composition_resolution(
        composition_session=composition_session,
        composition_resolution_acknowledged=composition_resolution_acknowledged,
    )
    _validate_context(
        domain=domain,
        revision_id=revision_id,
        caret_map=caret_map,
        format_state=format_state,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    if not isinstance(shift, bool):
        _fail("invalid_pointer_gesture", "shift must be boolean")

    hit = _hit(
        caret_map=caret_map,
        domain=domain,
        page_id=page_id,
        page_x_emu=page_x_emu,
        page_y_emu=page_y_emu,
        expected_layout_revision_id=expected_layout_revision_id,
    )

    if shift:
        if not isinstance(existing_selection, TextSelectionStateV1):
            _fail("shift_anchor_required", "Shift-click requires an active canonical selection")
        try:
            validate_selection_state_v1(
                state=existing_selection,
                domain=domain,
                expected_revision_id=revision_id,
            )
        except TextSelectionStateError as exc:
            _fail(exc.code, str(exc))
        if existing_selection.story_id != hit.story_id:
            _fail(
                "cross_story_selection_unsupported",
                "Shift-click cannot synthesize a multi-Story text selection",
            )
        selection = build_text_selection_state_v1(
            domain=domain,
            revision_id=revision_id,
            anchor_scalar=existing_selection.anchor_scalar,
            focus_scalar=hit.scalar_boundary,
            preferred_inline_x_emu=None,
        )
        anchor_stop_id = (
            existing_selection.anchor_visual_stop_id
            if existing_selection.projection_state == "projected"
            and existing_selection.layout_revision_id == caret_map.layout_revision_id
            else None
        )
        selection = _project(
            selection=selection,
            domain=domain,
            caret_map=caret_map,
            anchor_stop_id=anchor_stop_id,
            focus_stop_id=hit.stop_id,
        )
    else:
        selection = build_text_selection_state_v1(
            domain=domain,
            revision_id=revision_id,
            anchor_scalar=hit.scalar_boundary,
            focus_scalar=hit.scalar_boundary,
            preferred_inline_x_emu=None,
        )
        selection = _project(
            selection=selection,
            domain=domain,
            caret_map=caret_map,
            anchor_stop_id=hit.stop_id,
            focus_stop_id=hit.stop_id,
        )

    typing = _typing_after_pointer_relocation(
        selection=selection,
        domain=domain,
        format_state=format_state,
    )
    return TextPointerSelectionResultV1(
        protocol_version="chaptera.text-pointer-selection-result.v1",
        selection=selection,
        typing_state=typing,
        hit_stop=hit,
        autoscroll_needed=False,
        document_mutation_count=0,
    )


def begin_pointer_drag_v1(
    *,
    domain: StoryEditDomainV1,
    revision_id: str,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    page_id: str,
    page_x_emu: int,
    page_y_emu: int,
    expected_layout_revision_id: str,
    composition_session: TextCompositionSessionV1 | None = None,
    composition_resolution_acknowledged: bool = False,
) -> TextPointerDragUpdateV1:
    result = pointer_click_v1(
        domain=domain,
        revision_id=revision_id,
        caret_map=caret_map,
        format_state=format_state,
        page_id=page_id,
        page_x_emu=page_x_emu,
        page_y_emu=page_y_emu,
        expected_layout_revision_id=expected_layout_revision_id,
        shift=False,
        composition_session=composition_session,
        composition_resolution_acknowledged=composition_resolution_acknowledged,
    )
    hit = result.hit_stop
    assert hit is not None
    drag = TextPointerDragSessionV1(
        protocol_version="chaptera.text-pointer-drag-session.v1",
        story_id=domain.story_id,
        revision_id=revision_id,
        edit_domain_id=edit_domain_id_v1(domain),
        layout_revision_id=caret_map.layout_revision_id,
        anchor_scalar=hit.scalar_boundary,
        anchor_stop_id=hit.stop_id,
        current_selection=result.selection,
    )
    return TextPointerDragUpdateV1(
        protocol_version="chaptera.text-pointer-drag-update.v1",
        drag_session=drag,
        result=result,
    )


def update_pointer_drag_v1(
    drag_session: TextPointerDragSessionV1,
    *,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    page_id: str,
    page_x_emu: int,
    page_y_emu: int,
    expected_layout_revision_id: str,
    pointer_in_hittable_view: bool = True,
    composition_session: TextCompositionSessionV1 | None = None,
    composition_resolution_acknowledged: bool = False,
) -> TextPointerDragUpdateV1:
    if not isinstance(drag_session, TextPointerDragSessionV1):
        _fail("invalid_drag_session", "TextPointerDragSessionV1 is required")
    if drag_session.protocol_version != "chaptera.text-pointer-drag-session.v1":
        _fail("invalid_drag_session", "pointer drag protocol mismatch")
    _require_composition_resolution(
        composition_session=composition_session,
        composition_resolution_acknowledged=composition_resolution_acknowledged,
    )
    if domain.story_id != drag_session.story_id or caret_map.story_id != drag_session.story_id:
        _fail(
            "cross_story_selection_unsupported",
            "pointer drag cannot cross canonical Story ownership",
        )
    if edit_domain_id_v1(domain) != drag_session.edit_domain_id:
        _fail("selection_reconcile_required", "drag edit-domain fence is stale")
    if drag_session.revision_id != drag_session.current_selection.revision_id:
        _fail("stale_selection_revision", "drag selection revision changed")
    _validate_context(
        domain=domain,
        revision_id=drag_session.revision_id,
        caret_map=caret_map,
        format_state=format_state,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    if not isinstance(pointer_in_hittable_view, bool):
        _fail("invalid_pointer_gesture", "pointer_in_hittable_view must be boolean")

    if not pointer_in_hittable_view:
        return TextPointerDragUpdateV1(
            protocol_version="chaptera.text-pointer-drag-update.v1",
            drag_session=drag_session,
            result=TextPointerSelectionResultV1(
                protocol_version="chaptera.text-pointer-selection-result.v1",
                selection=drag_session.current_selection,
                typing_state=_typing_after_pointer_relocation(
                    selection=drag_session.current_selection,
                    domain=domain,
                    format_state=format_state,
                ),
                hit_stop=None,
                autoscroll_needed=True,
                document_mutation_count=0,
            ),
        )

    hit = _hit(
        caret_map=caret_map,
        domain=domain,
        page_id=page_id,
        page_x_emu=page_x_emu,
        page_y_emu=page_y_emu,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    selection = build_text_selection_state_v1(
        domain=domain,
        revision_id=drag_session.revision_id,
        anchor_scalar=drag_session.anchor_scalar,
        focus_scalar=hit.scalar_boundary,
        preferred_inline_x_emu=None,
    )
    selection = _project(
        selection=selection,
        domain=domain,
        caret_map=caret_map,
        anchor_stop_id=drag_session.anchor_stop_id,
        focus_stop_id=hit.stop_id,
    )
    typing = _typing_after_pointer_relocation(
        selection=selection,
        domain=domain,
        format_state=format_state,
    )
    updated = replace(drag_session, current_selection=selection)
    return TextPointerDragUpdateV1(
        protocol_version="chaptera.text-pointer-drag-update.v1",
        drag_session=updated,
        result=TextPointerSelectionResultV1(
            protocol_version="chaptera.text-pointer-selection-result.v1",
            selection=selection,
            typing_state=typing,
            hit_stop=hit,
            autoscroll_needed=False,
            document_mutation_count=0,
        ),
    )


def end_pointer_drag_v1(
    drag_session: TextPointerDragSessionV1,
    *,
    domain: StoryEditDomainV1,
) -> TextSelectionStateV1:
    if not isinstance(drag_session, TextPointerDragSessionV1):
        _fail("invalid_drag_session", "TextPointerDragSessionV1 is required")
    if domain.story_id != drag_session.story_id:
        _fail("cross_story_selection_unsupported", "drag ended under a different Story")
    if edit_domain_id_v1(domain) != drag_session.edit_domain_id:
        _fail("selection_reconcile_required", "drag edit-domain fence changed before pointer-up")
    try:
        validate_selection_state_v1(
            state=drag_session.current_selection,
            domain=domain,
            expected_revision_id=drag_session.revision_id,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))
    return drag_session.current_selection
