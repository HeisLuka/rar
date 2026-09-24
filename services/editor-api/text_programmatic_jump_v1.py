#!/usr/bin/env python3
"""Semantic Story/range jump into the active text editor session V1."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from resolved_text_caret_map_v1 import (
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_caret_reveal_v1 import (
    TextCaretRevealResultV1,
    TextViewportReceiptV1,
    plan_text_caret_reveal_v1,
)
from text_edit_session_v1 import (
    CompositionResolutionV1,
    TextEditSessionError,
    TextEditSessionV1,
    TextEntryCandidateV1,
    TextInitialPositionV1,
    TextSessionTransitionV1,
    enter_text_edit_session_v1,
    switch_text_edit_session_v1,
)
from text_format_overlay_v1 import TextFormatOverlayStateV1
from text_selection_state_v1 import (
    TextSelectionStateError,
    TextSelectionStateV1,
    build_text_selection_state_v1,
    project_selection_state_v1,
    validate_selection_state_v1,
)
from text_typing_format_state_v1 import (
    clear_typing_state_for_context_change_v1,
)


class TextProgrammaticJumpError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


SelectionModeV1 = Literal["collapsed_caret", "exact_range"]
GeometryStateV1 = Literal["projected", "semantic_geometry_pending"]


@dataclass(frozen=True)
class TextProgrammaticJumpRequestV1:
    protocol_version: Literal["chaptera.text-programmatic-jump.v1"]
    document_id: str
    revision_id: str
    story_id: str
    start_scalar: int
    end_scalar: int
    selection_mode: SelectionModeV1
    reason: str


@dataclass(frozen=True)
class TextProgrammaticJumpResultV1:
    protocol_version: Literal["chaptera.text-programmatic-jump-result.v1"]
    request: TextProgrammaticJumpRequestV1
    transition_kind: Literal["enter", "reuse", "story_switch"]
    session: TextEditSessionV1
    geometry_state: GeometryStateV1
    reveal: TextCaretRevealResultV1
    focus_context_discontinuity: bool
    undo_group_boundary: bool
    authoring_mutation_count: Literal[0]
    undo_history_entry_count: Literal[0]


def _fail(code: str, message: str) -> None:
    raise TextProgrammaticJumpError(code, message)


def _validate_request(request: TextProgrammaticJumpRequestV1) -> None:
    if not isinstance(request, TextProgrammaticJumpRequestV1):
        _fail("invalid_jump", "TextProgrammaticJumpRequestV1 is required")
    if request.protocol_version != "chaptera.text-programmatic-jump.v1":
        _fail("invalid_jump", "jump protocol mismatch")
    for label, value in (
        ("document_id", request.document_id),
        ("revision_id", request.revision_id),
        ("story_id", request.story_id),
        ("reason", request.reason),
    ):
        if not isinstance(value, str) or not value:
            _fail("invalid_jump", f"{label} is required")
    if (
        not isinstance(request.start_scalar, int)
        or isinstance(request.start_scalar, bool)
        or not isinstance(request.end_scalar, int)
        or isinstance(request.end_scalar, bool)
        or request.start_scalar < 0
        or request.end_scalar < request.start_scalar
    ):
        _fail("invalid_jump", "canonical scalar target is invalid")
    if request.selection_mode == "collapsed_caret":
        if request.start_scalar != request.end_scalar:
            _fail("invalid_jump", "collapsed_caret requires start == end")
    elif request.selection_mode == "exact_range":
        if request.end_scalar <= request.start_scalar:
            _fail("invalid_jump", "exact_range must be non-empty")
    else:
        _fail("invalid_jump", "selection_mode is unsupported")


def _validate_candidate(
    candidate: TextEntryCandidateV1,
    *,
    story_id: str,
) -> None:
    if not isinstance(candidate, TextEntryCandidateV1):
        _fail("invalid_target_capability", "TextEntryCandidateV1 is required")
    if candidate.story_id != story_id:
        _fail("jump_story_mismatch", "target capability belongs to a different Story")
    if candidate.capability == "read_only":
        _fail("read_only_story", candidate.reason or "target Story is read-only")
    if candidate.capability != "editable":
        _fail(
            "story_editing_unsupported",
            candidate.reason or "target Story is not editable",
        )


def _build_target_selection(
    *,
    request: TextProgrammaticJumpRequestV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
) -> tuple[TextSelectionStateV1, GeometryStateV1]:
    if domain.story_id != request.story_id:
        _fail("jump_story_mismatch", "target edit domain belongs to a different Story")
    if domain.status != "known":
        _fail("edit_domain_unknown", "target Story ordinary edit domain is unknown")

    try:
        semantic = build_text_selection_state_v1(
            domain=domain,
            revision_id=request.revision_id,
            anchor_scalar=request.start_scalar,
            focus_scalar=request.end_scalar,
            preferred_inline_x_emu=None,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))

    try:
        projection = project_selection_state_v1(
            state=semantic,
            domain=domain,
            caret_map=caret_map,
        )
        return projection.state, "projected"
    except TextSelectionStateError as exc:
        code = exc.code
        if code in {"caret_affinity_required", "invalid_caret_affinity"}:
            _fail("caret_affinity_required", str(exc))
        if request.selection_mode == "collapsed_caret":
            if code in {"unplaced_story_position", "internal_cluster_unsupported"}:
                _fail(
                    "collapsed_target_unplaced",
                    "collapsed programmatic caret requires an admitted physical stop",
                )
            _fail(code, str(exc))
        if code in {
            "unplaced_story_position",
            "internal_cluster_unsupported",
        }:
            # Non-empty semantic selection is valid independent of current
            # placement. INTERACTION-TEXT-UNPLACED-01 may add richer affordances;
            # V1 keeps the canonical range and surfaces geometry pending.
            return semantic, "semantic_geometry_pending"
        _fail(code, str(exc))


def _clear_transient_typing(
    *,
    selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    format_state: TextFormatOverlayStateV1,
):
    try:
        return clear_typing_state_for_context_change_v1(
            selection=selection,
            domain=domain,
            format_state=format_state,
        )
    except ValueError as exc:
        _fail(getattr(exc, "code", "typing_context_mismatch"), str(exc))


def _prepare_same_story_session(
    *,
    active_session: TextEditSessionV1,
    request: TextProgrammaticJumpRequestV1,
    candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    composition_resolution: CompositionResolutionV1 | None,
) -> tuple[TextEditSessionV1, bool, bool]:
    if active_session.document_id != request.document_id:
        _fail("cross_document_jump_unsupported", "active text session belongs to another document")
    if active_session.revision_id != request.revision_id:
        _fail("jump_stale", "active text session revision differs from jump revision")
    if active_session.story_id != request.story_id:
        _fail("jump_story_mismatch", "same-Story reuse called for different Story")
    if active_session.composition_session is not None:
        if composition_resolution not in {"cancelled", "committed", "reconciled"}:
            _fail(
                "composition_transition_required",
                "active composition must commit/cancel/reconcile before programmatic jump",
            )
    elif composition_resolution is not None:
        _fail("invalid_composition_resolution", "no active composition requires resolution")

    _validate_candidate(candidate, story_id=request.story_id)
    if domain.story_id != request.story_id or caret_map.story_id != request.story_id:
        _fail("jump_story_mismatch", "target authority context belongs to a different Story")
    if caret_map.layout_revision_id != active_session.layout_revision_id:
        _fail("reconcile_required", "session must rebind to current layout before same-Story jump")
    return replace(active_session, composition_session=None), False, False


def _prepare_enter_or_switch(
    *,
    active_session: TextEditSessionV1 | None,
    request: TextProgrammaticJumpRequestV1,
    candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
    composition_resolution: CompositionResolutionV1 | None,
) -> tuple[TextEditSessionV1, Literal["enter", "story_switch"], bool, bool]:
    _validate_candidate(candidate, story_id=request.story_id)
    if active_session is None:
        try:
            entered = enter_text_edit_session_v1(
                session_id=f"jump:{request.document_id}:{request.story_id}",
                incarnation=0,
                document_id=request.document_id,
                entry_candidates=(candidate,),
                revision_id=request.revision_id,
                domain=domain,
                caret_map=caret_map,
                format_state=format_state,
                expected_layout_revision_id=expected_layout_revision_id,
                initial_position=TextInitialPositionV1(request.start_scalar),
                pending_interaction_metadata=(("entry_reason", request.reason),),
            )
        except TextEditSessionError as exc:
            if (
                request.selection_mode == "collapsed_caret"
                and exc.code in {"unplaced_story_position", "internal_cluster_unsupported"}
            ):
                _fail(
                    "collapsed_target_unplaced",
                    "collapsed programmatic caret requires an admitted physical stop",
                )
            _fail(exc.code, str(exc))
        return entered.session, "enter", False, False

    if active_session.document_id != request.document_id:
        _fail("cross_document_jump_unsupported", "active text session belongs to another document")
    try:
        switched = switch_text_edit_session_v1(
            active_session,
            entry_candidates=(candidate,),
            revision_id=request.revision_id,
            domain=domain,
            caret_map=caret_map,
            format_state=format_state,
            expected_layout_revision_id=expected_layout_revision_id,
            initial_position=TextInitialPositionV1(request.start_scalar),
            composition_resolution=composition_resolution,
            pending_interaction_metadata=(("entry_reason", request.reason),),
        )
    except TextEditSessionError as exc:
        if (
            request.selection_mode == "collapsed_caret"
            and exc.code in {"unplaced_story_position", "internal_cluster_unsupported"}
        ):
            _fail(
                "collapsed_target_unplaced",
                "collapsed programmatic caret requires an admitted physical stop",
            )
        _fail(exc.code, str(exc))
    return (
        switched.session,
        "story_switch",
        switched.focus_context_discontinuity,
        switched.undo_group_boundary,
    )


def execute_text_programmatic_jump_v1(
    *,
    request: TextProgrammaticJumpRequestV1,
    target_candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
    viewport: TextViewportReceiptV1,
    expected_view_revision_id: str,
    active_session: TextEditSessionV1 | None = None,
    composition_resolution: CompositionResolutionV1 | None = None,
) -> TextProgrammaticJumpResultV1:
    _validate_request(request)
    if caret_map.layout_revision_id != expected_layout_revision_id:
        _fail("reconcile_required", "target caret map layout receipt is stale")
    if caret_map.story_id != request.story_id:
        _fail("jump_story_mismatch", "caret map targets a different Story")
    if caret_map.story_scalar_len != domain.raw_scalar_len:
        _fail("reconcile_required", "target caret map Story extent differs from edit domain")
    if format_state.story_id != request.story_id:
        _fail("jump_story_mismatch", "format state targets a different Story")
    if format_state.story_scalar_len != domain.raw_scalar_len:
        _fail("typing_context_mismatch", "format state extent differs from edit domain")

    if active_session is not None and active_session.story_id == request.story_id:
        session, discontinuity, undo_boundary = _prepare_same_story_session(
            active_session=active_session,
            request=request,
            candidate=target_candidate,
            domain=domain,
            caret_map=caret_map,
            format_state=format_state,
            composition_resolution=composition_resolution,
        )
        transition_kind: Literal["enter", "reuse", "story_switch"] = "reuse"
    else:
        session, kind, discontinuity, undo_boundary = _prepare_enter_or_switch(
            active_session=active_session,
            request=request,
            candidate=target_candidate,
            domain=domain,
            caret_map=caret_map,
            format_state=format_state,
            expected_layout_revision_id=expected_layout_revision_id,
            composition_resolution=composition_resolution,
        )
        transition_kind = kind

    selection, geometry_state = _build_target_selection(
        request=request,
        domain=domain,
        caret_map=caret_map,
    )
    typing_state = _clear_transient_typing(
        selection=selection,
        domain=domain,
        format_state=format_state,
    )
    session = replace(
        session,
        story_id=request.story_id,
        revision_id=request.revision_id,
        selection=selection,
        typing_state=typing_state,
        composition_session=None,
        current_frame_id=(
            selection.focus_visual_stop_id and session.current_frame_id
        ) or session.current_frame_id,
    )

    reveal = plan_text_caret_reveal_v1(
        session=session,
        caret_map=caret_map,
        viewport=viewport,
        expected_view_revision_id=expected_view_revision_id,
        reveal_reason="programmatic_jump",
    )
    if reveal.status == "reveal_unsupported" and reveal.reason and "multiple physical" in reveal.reason:
        _fail("caret_affinity_required", reveal.reason)

    if reveal.target_frame_id is not None:
        session = replace(session, current_frame_id=reveal.target_frame_id)

    return TextProgrammaticJumpResultV1(
        protocol_version="chaptera.text-programmatic-jump-result.v1",
        request=request,
        transition_kind=transition_kind,
        session=session,
        geometry_state=geometry_state,
        reveal=reveal,
        focus_context_discontinuity=discontinuity,
        undo_group_boundary=undo_boundary,
        authoring_mutation_count=0,
        undo_history_entry_count=0,
    )
