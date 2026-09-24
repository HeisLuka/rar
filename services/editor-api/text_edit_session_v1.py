#!/usr/bin/env python3
"""Source-neutral transient Story text editing session lifecycle V1.

This module owns focus/session lifecycle only. It never commits authoring
revisions and never persists caret, typing, composition, visual frame provenance
or focus state into EditorProject.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from resolved_text_caret_map_v1 import (
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    caret_map_hash_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_composition_session_v1 import TextCompositionSessionV1
from text_format_overlay_v1 import TextFormatOverlayStateV1
from text_pointer_selection_v1 import (
    TextPointerSelectionError,
    pointer_click_v1,
)
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
    validate_typing_format_state_v1,
)


class TextEditSessionError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


EntryCapabilityV1 = Literal["editable", "read_only", "unsupported"]
CompositionResolutionV1 = Literal["cancelled", "committed", "reconciled"]


@dataclass(frozen=True)
class TextEntryCandidateV1:
    target_id: str
    story_id: str
    frame_id: str | None
    capability: EntryCapabilityV1
    reason: str | None = None


@dataclass(frozen=True)
class TextPointerEntryContextV1:
    page_id: str
    page_x_emu: int
    page_y_emu: int


@dataclass(frozen=True)
class TextInitialPositionV1:
    scalar_boundary: int
    visual_stop_id: str | None = None


@dataclass(frozen=True)
class TextEditSessionV1:
    protocol_version: Literal["chaptera.text-edit-session.v1"]
    session_id: str
    incarnation: int
    document_id: str
    story_id: str
    revision_id: str
    edit_domain_id: str
    layout_revision_id: str
    caret_map_hash: str
    entry_frame_id: str | None
    current_frame_id: str | None
    focus_owner: Literal["story_text"]
    selection: TextSelectionStateV1
    typing_state: TextTypingFormatStateV1 | None
    composition_session: TextCompositionSessionV1 | None
    pending_interaction_metadata: tuple[tuple[str, str], ...]


@dataclass(frozen=True)
class TextSessionTransitionV1:
    protocol_version: Literal["chaptera.text-session-transition.v1"]
    kind: Literal["enter", "same_story_handoff", "story_switch", "authority_rebind"]
    previous_story_id: str | None
    current_story_id: str
    session: TextEditSessionV1
    focus_context_discontinuity: bool
    undo_group_boundary: bool
    lifecycle_document_mutation_count: Literal[0]


@dataclass(frozen=True)
class TextSessionExitV1:
    protocol_version: Literal["chaptera.text-session-exit.v1"]
    session_id: str
    incarnation: int
    closed_story_id: str
    reason: str
    composition_resolution: CompositionResolutionV1 | None
    focus_context_discontinuity: Literal[True]
    undo_group_boundary: Literal[True]
    lifecycle_document_mutation_count: Literal[0]
    still_pending_operation_ids: tuple[str, ...]


def _fail(code: str, message: str) -> None:
    raise TextEditSessionError(code, message)


def _required_string(value: str, label: str) -> str:
    if not isinstance(value, str) or not value:
        _fail("invalid_session", f"{label} is required")
    return value


def _canonical_metadata(
    metadata: tuple[tuple[str, str], ...],
) -> tuple[tuple[str, str], ...]:
    if not isinstance(metadata, tuple):
        _fail("invalid_session", "pending_interaction_metadata must be tuple")
    seen = set()
    out = []
    for item in metadata:
        if (
            not isinstance(item, tuple)
            or len(item) != 2
            or not isinstance(item[0], str)
            or not item[0]
            or not isinstance(item[1], str)
        ):
            _fail("invalid_session", "interaction metadata entries must be string pairs")
        if item[0] in seen:
            _fail("invalid_session", "interaction metadata keys must be unique")
        seen.add(item[0])
        out.append(item)
    out.sort(key=lambda pair: pair[0])
    return tuple(out)


def _resolve_entry_candidate(
    candidates: tuple[TextEntryCandidateV1, ...],
) -> TextEntryCandidateV1:
    if not isinstance(candidates, tuple) or not candidates:
        _fail("entry_owner_missing", "entry requires one resolved frame/Story candidate")
    if len(candidates) != 1:
        _fail(
            "entry_owner_ambiguous",
            "frame/Story entry mapping must resolve to exactly one candidate",
        )
    candidate = candidates[0]
    if not isinstance(candidate, TextEntryCandidateV1):
        _fail("entry_owner_invalid", "entry candidate is malformed")
    _required_string(candidate.target_id, "entry target_id")
    _required_string(candidate.story_id, "entry story_id")
    if candidate.frame_id is not None:
        _required_string(candidate.frame_id, "entry frame_id")
    if candidate.capability == "read_only":
        _fail(
            "read_only_story",
            candidate.reason or "resolved Story is read-only",
        )
    if candidate.capability == "unsupported":
        _fail(
            "story_editing_unsupported",
            candidate.reason or "resolved Story is not editable by this capability",
        )
    if candidate.capability != "editable":
        _fail("entry_owner_invalid", "entry capability is invalid")
    return candidate


def _validate_authoritative_context(
    *,
    story_id: str,
    revision_id: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
) -> None:
    _required_string(revision_id, "revision_id")
    _required_string(expected_layout_revision_id, "expected_layout_revision_id")
    if not isinstance(domain, StoryEditDomainV1):
        _fail("invalid_edit_domain", "StoryEditDomainV1 is required")
    if domain.story_id != story_id:
        _fail("entry_story_mismatch", "entry owner and edit domain target different Stories")
    if domain.status != "known":
        _fail("edit_domain_unknown", "Story ordinary edit domain is unavailable")
    if not isinstance(caret_map, ResolvedTextCaretMapV1):
        _fail("missing_caret_map", "authoritative ResolvedTextCaretMapV1 is required")
    if caret_map.story_id != story_id:
        _fail("entry_story_mismatch", "caret map targets a different Story")
    if caret_map.story_scalar_len != domain.raw_scalar_len:
        _fail("stale_layout_map", "caret map Story extent differs from edit domain")
    if caret_map.layout_revision_id != expected_layout_revision_id:
        _fail("stale_layout_map", "caret map belongs to a different layout revision")
    if not isinstance(format_state, TextFormatOverlayStateV1):
        _fail("invalid_format_state", "TextFormatOverlayStateV1 is required")
    if format_state.story_id != story_id:
        _fail("entry_story_mismatch", "format state targets a different Story")
    if format_state.story_scalar_len != domain.raw_scalar_len:
        _fail("typing_context_mismatch", "format state Story extent differs from edit domain")


def _typing_for_selection(
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


def _selection_from_initial_position(
    *,
    domain: StoryEditDomainV1,
    revision_id: str,
    caret_map: ResolvedTextCaretMapV1,
    initial: TextInitialPositionV1,
) -> TextSelectionStateV1:
    if not isinstance(initial, TextInitialPositionV1):
        _fail("invalid_initial_position", "TextInitialPositionV1 is required")
    try:
        selection = build_text_selection_state_v1(
            domain=domain,
            revision_id=revision_id,
            anchor_scalar=initial.scalar_boundary,
            focus_scalar=initial.scalar_boundary,
            preferred_inline_x_emu=None,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))

    # New empty Stories may have an authoritative empty caret map before the
    # first glyph/line is materialized. Boundary 0 remains an admitted semantic
    # insertion position and is intentionally layout-pending.
    if (
        domain.raw_scalar_len == 0
        and initial.scalar_boundary == 0
        and not caret_map.caret_stops
    ):
        if initial.visual_stop_id is not None:
            _fail(
                "invalid_caret_affinity",
                "empty Story has no materialized visual caret stop",
            )
        return selection

    try:
        projection = project_selection_state_v1(
            state=selection,
            domain=domain,
            caret_map=caret_map,
            anchor_stop_id=initial.visual_stop_id,
            focus_stop_id=initial.visual_stop_id,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))
    return projection.state


def _build_session(
    *,
    session_id: str,
    incarnation: int,
    document_id: str,
    candidate: TextEntryCandidateV1,
    revision_id: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    selection: TextSelectionStateV1,
    typing_state: TextTypingFormatStateV1 | None,
    metadata: tuple[tuple[str, str], ...],
) -> TextEditSessionV1:
    _required_string(session_id, "session_id")
    _required_string(document_id, "document_id")
    if (
        not isinstance(incarnation, int)
        or isinstance(incarnation, bool)
        or incarnation < 0
    ):
        _fail("invalid_session", "session incarnation must be non-negative integer")
    return TextEditSessionV1(
        protocol_version="chaptera.text-edit-session.v1",
        session_id=session_id,
        incarnation=incarnation,
        document_id=document_id,
        story_id=candidate.story_id,
        revision_id=revision_id,
        edit_domain_id=edit_domain_id_v1(domain),
        layout_revision_id=caret_map.layout_revision_id,
        caret_map_hash=caret_map_hash_v1(caret_map),
        entry_frame_id=candidate.frame_id,
        current_frame_id=candidate.frame_id,
        focus_owner="story_text",
        selection=selection,
        typing_state=typing_state,
        composition_session=None,
        pending_interaction_metadata=_canonical_metadata(metadata),
    )


def enter_text_edit_session_v1(
    *,
    session_id: str,
    incarnation: int,
    document_id: str,
    entry_candidates: tuple[TextEntryCandidateV1, ...],
    revision_id: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
    pointer_context: TextPointerEntryContextV1 | None = None,
    initial_position: TextInitialPositionV1 | None = None,
    pending_interaction_metadata: tuple[tuple[str, str], ...] = (),
) -> TextSessionTransitionV1:
    candidate = _resolve_entry_candidate(entry_candidates)
    _validate_authoritative_context(
        story_id=candidate.story_id,
        revision_id=revision_id,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    if (pointer_context is None) == (initial_position is None):
        _fail(
            "entry_position_required",
            "entry requires exactly one pointer context or explicit initial position",
        )

    if pointer_context is not None:
        if not isinstance(pointer_context, TextPointerEntryContextV1):
            _fail("invalid_pointer_context", "TextPointerEntryContextV1 is required")
        try:
            pointer = pointer_click_v1(
                domain=domain,
                revision_id=revision_id,
                caret_map=caret_map,
                format_state=format_state,
                page_id=pointer_context.page_id,
                page_x_emu=pointer_context.page_x_emu,
                page_y_emu=pointer_context.page_y_emu,
                expected_layout_revision_id=expected_layout_revision_id,
            )
        except TextPointerSelectionError as exc:
            _fail(exc.code, str(exc))
        selection = pointer.selection
        typing_state = pointer.typing_state
    else:
        assert initial_position is not None
        selection = _selection_from_initial_position(
            domain=domain,
            revision_id=revision_id,
            caret_map=caret_map,
            initial=initial_position,
        )
        typing_state = _typing_for_selection(
            selection=selection,
            domain=domain,
            format_state=format_state,
        )

    session = _build_session(
        session_id=session_id,
        incarnation=incarnation,
        document_id=document_id,
        candidate=candidate,
        revision_id=revision_id,
        domain=domain,
        caret_map=caret_map,
        selection=selection,
        typing_state=typing_state,
        metadata=pending_interaction_metadata,
    )
    return TextSessionTransitionV1(
        protocol_version="chaptera.text-session-transition.v1",
        kind="enter",
        previous_story_id=None,
        current_story_id=session.story_id,
        session=session,
        focus_context_discontinuity=False,
        undo_group_boundary=False,
        lifecycle_document_mutation_count=0,
    )


def attach_text_composition_v1(
    session: TextEditSessionV1,
    composition: TextCompositionSessionV1,
) -> TextEditSessionV1:
    if not isinstance(session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 is required")
    if session.composition_session is not None:
        _fail("composition_already_active", "text session already owns an active composition")
    if not isinstance(composition, TextCompositionSessionV1):
        _fail("invalid_composition", "TextCompositionSessionV1 is required")
    if (
        composition.story_id != session.story_id
        or composition.base_revision_id != session.revision_id
        or composition.edit_domain_id != session.edit_domain_id
    ):
        _fail("reconcile_required", "composition belongs to a different session authority context")
    return replace(session, composition_session=composition)


def _require_composition_resolved(
    session: TextEditSessionV1,
    resolution: CompositionResolutionV1 | None,
) -> None:
    if session.composition_session is None:
        if resolution is not None:
            _fail("invalid_composition_resolution", "no active composition requires resolution")
        return
    if resolution not in {"cancelled", "committed", "reconciled"}:
        _fail(
            "composition_transition_required",
            "active composition must commit/cancel/reconcile before focus context changes",
        )


def handoff_same_story_frame_v1(
    session: TextEditSessionV1,
    *,
    entry_candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
    pointer_context: TextPointerEntryContextV1 | None = None,
    composition_resolution: CompositionResolutionV1 | None = None,
) -> TextSessionTransitionV1:
    if not isinstance(session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 is required")
    candidate = _resolve_entry_candidate((entry_candidate,))
    if candidate.story_id != session.story_id:
        _fail(
            "story_switch_required",
            "different Story ownership requires explicit session switch",
        )
    _validate_authoritative_context(
        story_id=session.story_id,
        revision_id=session.revision_id,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    if edit_domain_id_v1(domain) != session.edit_domain_id:
        _fail("selection_reconcile_required", "session edit-domain receipt is stale")

    selection = session.selection
    typing_state = session.typing_state
    composition = session.composition_session

    if pointer_context is not None:
        _require_composition_resolved(session, composition_resolution)
        if not isinstance(pointer_context, TextPointerEntryContextV1):
            _fail("invalid_pointer_context", "TextPointerEntryContextV1 is required")
        try:
            pointer = pointer_click_v1(
                domain=domain,
                revision_id=session.revision_id,
                caret_map=caret_map,
                format_state=format_state,
                page_id=pointer_context.page_id,
                page_x_emu=pointer_context.page_x_emu,
                page_y_emu=pointer_context.page_y_emu,
                expected_layout_revision_id=expected_layout_revision_id,
                composition_session=composition,
                composition_resolution_acknowledged=composition is not None,
            )
        except TextPointerSelectionError as exc:
            _fail(exc.code, str(exc))
        selection = pointer.selection
        typing_state = pointer.typing_state
        composition = None
    elif caret_map.layout_revision_id != session.layout_revision_id:
        # Visual provenance changed without a pointer relocation. Reproject the
        # same canonical selection through the new authoritative layout.
        try:
            projection = project_selection_state_v1(
                state=build_text_selection_state_v1(
                    domain=domain,
                    revision_id=session.revision_id,
                    anchor_scalar=selection.anchor_scalar,
                    focus_scalar=selection.focus_scalar,
                    preferred_inline_x_emu=None,
                ),
                domain=domain,
                caret_map=caret_map,
            )
        except TextSelectionStateError as exc:
            _fail(exc.code, str(exc))
        selection = projection.state
        typing_state = _typing_for_selection(
            selection=selection,
            domain=domain,
            format_state=format_state,
        )

    updated = replace(
        session,
        layout_revision_id=caret_map.layout_revision_id,
        caret_map_hash=caret_map_hash_v1(caret_map),
        current_frame_id=candidate.frame_id,
        selection=selection,
        typing_state=typing_state,
        composition_session=composition,
    )
    return TextSessionTransitionV1(
        protocol_version="chaptera.text-session-transition.v1",
        kind="same_story_handoff",
        previous_story_id=session.story_id,
        current_story_id=session.story_id,
        session=updated,
        focus_context_discontinuity=False,
        undo_group_boundary=False,
        lifecycle_document_mutation_count=0,
    )


def switch_text_edit_session_v1(
    session: TextEditSessionV1,
    *,
    entry_candidates: tuple[TextEntryCandidateV1, ...],
    revision_id: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
    pointer_context: TextPointerEntryContextV1 | None = None,
    initial_position: TextInitialPositionV1 | None = None,
    composition_resolution: CompositionResolutionV1 | None = None,
    pending_interaction_metadata: tuple[tuple[str, str], ...] = (),
) -> TextSessionTransitionV1:
    if not isinstance(session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 is required")
    _require_composition_resolved(session, composition_resolution)
    candidate = _resolve_entry_candidate(entry_candidates)
    if candidate.story_id == session.story_id:
        _fail(
            "same_story_handoff_required",
            "same Story frame activation must preserve the existing session",
        )
    entered = enter_text_edit_session_v1(
        session_id=session.session_id,
        incarnation=session.incarnation + 1,
        document_id=session.document_id,
        entry_candidates=(candidate,),
        revision_id=revision_id,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
        expected_layout_revision_id=expected_layout_revision_id,
        pointer_context=pointer_context,
        initial_position=initial_position,
        pending_interaction_metadata=pending_interaction_metadata,
    )
    return TextSessionTransitionV1(
        protocol_version="chaptera.text-session-transition.v1",
        kind="story_switch",
        previous_story_id=session.story_id,
        current_story_id=entered.session.story_id,
        session=entered.session,
        focus_context_discontinuity=True,
        undo_group_boundary=True,
        lifecycle_document_mutation_count=0,
    )


def rebind_text_edit_session_authority_v1(
    session: TextEditSessionV1,
    *,
    revision_id: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    selection: TextSelectionStateV1,
    typing_state: TextTypingFormatStateV1 | None,
    expected_layout_revision_id: str,
) -> TextSessionTransitionV1:
    if not isinstance(session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 is required")
    if session.composition_session is not None:
        _fail(
            "composition_transition_required",
            "authority rebind during active composition must be reconciled by composition owner",
        )
    _validate_authoritative_context(
        story_id=session.story_id,
        revision_id=revision_id,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
        expected_layout_revision_id=expected_layout_revision_id,
    )
    try:
        validate_selection_state_v1(
            state=selection,
            domain=domain,
            expected_revision_id=revision_id,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))
    if selection.story_id != session.story_id:
        _fail("selection_story_mismatch", "rebound selection targets a different Story")
    if typing_state is not None:
        try:
            validate_typing_format_state_v1(
                state=typing_state,
                selection=selection,
                domain=domain,
                format_state=format_state,
            )
        except ValueError as exc:
            _fail(getattr(exc, "code", "typing_context_mismatch"), str(exc))

    updated = replace(
        session,
        revision_id=revision_id,
        edit_domain_id=edit_domain_id_v1(domain),
        layout_revision_id=caret_map.layout_revision_id,
        caret_map_hash=caret_map_hash_v1(caret_map),
        selection=selection,
        typing_state=typing_state,
    )
    return TextSessionTransitionV1(
        protocol_version="chaptera.text-session-transition.v1",
        kind="authority_rebind",
        previous_story_id=session.story_id,
        current_story_id=session.story_id,
        session=updated,
        focus_context_discontinuity=False,
        undo_group_boundary=False,
        lifecycle_document_mutation_count=0,
    )


def exit_text_edit_session_v1(
    session: TextEditSessionV1,
    *,
    reason: str,
    composition_resolution: CompositionResolutionV1 | None = None,
    submitted_durable_operation_ids: tuple[str, ...] = (),
) -> TextSessionExitV1:
    if not isinstance(session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 is required")
    _required_string(reason, "exit reason")
    _require_composition_resolved(session, composition_resolution)
    if not isinstance(submitted_durable_operation_ids, tuple):
        _fail("invalid_pending_operations", "submitted operation ids must be tuple")
    seen = set()
    pending = []
    for value in submitted_durable_operation_ids:
        _required_string(value, "submitted operation id")
        if value in seen:
            _fail("invalid_pending_operations", "submitted operation ids must be unique")
        seen.add(value)
        pending.append(value)
    return TextSessionExitV1(
        protocol_version="chaptera.text-session-exit.v1",
        session_id=session.session_id,
        incarnation=session.incarnation,
        closed_story_id=session.story_id,
        reason=reason,
        composition_resolution=composition_resolution,
        focus_context_discontinuity=True,
        undo_group_boundary=True,
        lifecycle_document_mutation_count=0,
        still_pending_operation_ids=tuple(pending),
    )
