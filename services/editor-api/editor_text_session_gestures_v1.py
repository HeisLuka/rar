#!/usr/bin/env python3
"""Deterministic desktop gesture routing into canonical text sessions V1.

Toolkit focus/click conventions are inputs, never semantic authority. This
adapter delegates all canonical selection/session work to TextEditSessionV1
and TextPointerSelectionV1.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from resolved_text_caret_map_v1 import (
    ResolvedTextCaretMapError,
    ResolvedTextCaretMapV1,
    hit_test_story_position_v1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from text_edit_session_v1 import (
    CompositionResolutionV1,
    TextEditSessionError,
    TextEditSessionV1,
    TextEntryCandidateV1,
    TextInitialPositionV1,
    TextPointerEntryContextV1,
    TextSessionExitV1,
    TextSessionTransitionV1,
    enter_text_edit_session_v1,
    exit_text_edit_session_v1,
    handoff_same_story_frame_v1,
    switch_text_edit_session_v1,
)
from text_format_overlay_v1 import TextFormatOverlayStateV1


HostFocusOwnerV1 = Literal[
    "canvas",
    "story_text",
    "inspector",
    "find_replace",
    "modal",
]
ExitTriggerV1 = Literal[
    "escape",
    "canvas_non_text_click",
    "non_text_tool",
    "explicit_exit",
]


class EditorTextSessionGesturesError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class DesktopTextActivationResultV1:
    protocol_version: Literal["chaptera.desktop-text-activation-result.v1"]
    status: Literal[
        "canvas_object_selection",
        "entered",
        "same_story_handoff",
        "story_switch",
        "direct_edit_unavailable",
        "multiclick_unavailable",
    ]
    transition: TextSessionTransitionV1 | None
    active_session: TextEditSessionV1 | None
    story_shortcuts_owned: bool
    canvas_object_shortcuts_owned: bool
    document_mutation_count: Literal[0]
    reason: str | None


@dataclass(frozen=True)
class DesktopTextExitResultV1:
    protocol_version: Literal["chaptera.desktop-text-exit-result.v1"]
    status: Literal[
        "no_active_session",
        "host_focus_owned",
        "composition_resolution_required",
        "exited",
    ]
    exit_receipt: TextSessionExitV1 | None
    active_session: TextEditSessionV1 | None
    story_shortcuts_owned: bool
    canvas_object_shortcuts_owned: bool
    transient_text_state_cleared: bool
    document_mutation_count: Literal[0]
    reason: str | None


def _fail(code: str, message: str) -> None:
    raise EditorTextSessionGesturesError(code, message)


def story_shortcut_admitted_v1(
    *,
    active_session: TextEditSessionV1 | None,
    focus_owner: HostFocusOwnerV1,
) -> bool:
    if focus_owner not in {
        "canvas",
        "story_text",
        "inspector",
        "find_replace",
        "modal",
    }:
        _fail("invalid_focus_owner", "unsupported desktop focus owner")
    if active_session is None:
        return False
    if not isinstance(active_session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 or None is required")
    return focus_owner == "story_text"


def _admitted_stop_in_frame(
    *,
    candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
):
    if domain.story_id != candidate.story_id or caret_map.story_id != candidate.story_id:
        _fail("entry_story_mismatch", "candidate/domain/caret map target different Stories")
    if domain.status != "known":
        _fail("edit_domain_unknown", "ordinary Story edit domain is unavailable")
    if domain.caret_start_boundary is None or domain.caret_end_boundary is None:
        _fail("edit_domain_unknown", "ordinary caret boundaries are unavailable")
    stops = [
        stop
        for stop in caret_map.caret_stops
        if domain.caret_start_boundary
        <= stop.scalar_boundary
        <= domain.caret_end_boundary
        and (candidate.frame_id is None or stop.frame_id == candidate.frame_id)
    ]
    if not stops:
        return None
    return min(
        stops,
        key=lambda stop: (
            stop.flow_ordinal,
            stop.scalar_boundary,
            stop.page_y_top_emu,
            stop.page_x_emu,
            stop.stop_id,
        ),
    )


def _explicit_activation_position(
    *,
    candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
) -> tuple[TextPointerEntryContextV1 | None, TextInitialPositionV1 | None]:
    stop = _admitted_stop_in_frame(
        candidate=candidate,
        domain=domain,
        caret_map=caret_map,
    )
    if stop is not None:
        return (
            TextPointerEntryContextV1(
                page_id=stop.page_id,
                page_x_emu=stop.page_x_emu,
                page_y_emu=(stop.page_y_top_emu + stop.page_y_bottom_emu) // 2,
            ),
            None,
        )
    if (
        domain.raw_scalar_len == 0
        and domain.caret_start_boundary == 0
        and domain.caret_end_boundary == 0
    ):
        return None, TextInitialPositionV1(0)
    _fail(
        "direct_edit_unavailable",
        "selected TextFrame/Story has no authoritative placed caret stop",
    )


def _preflight_pointer_target(
    *,
    candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    pointer_context: TextPointerEntryContextV1,
    expected_layout_revision_id: str,
) -> None:
    if caret_map.layout_revision_id != expected_layout_revision_id:
        _fail("stale_layout_map", "caret map belongs to a different layout revision")
    if caret_map.story_id != candidate.story_id or domain.story_id != candidate.story_id:
        _fail("entry_story_mismatch", "pointer target authority targets a different Story")
    try:
        stop = hit_test_story_position_v1(
            caret_map=caret_map,
            page_id=pointer_context.page_id,
            page_x_emu=pointer_context.page_x_emu,
            page_y_emu=pointer_context.page_y_emu,
            expected_layout_revision_id=expected_layout_revision_id,
        )
    except ResolvedTextCaretMapError as exc:
        _fail(exc.code, str(exc))
    if candidate.frame_id is not None and stop.frame_id != candidate.frame_id:
        _fail(
            "direct_edit_unavailable",
            "requested TextFrame has no authoritative caret hit at the pointer location",
        )
    if domain.caret_start_boundary is None or domain.caret_end_boundary is None:
        _fail("edit_domain_unknown", "ordinary caret boundaries are unavailable")
    if not domain.caret_start_boundary <= stop.scalar_boundary <= domain.caret_end_boundary:
        _fail(
            "protected_story_structure",
            "pointer target lies outside ordinary Story edit domain",
        )


def _transition_result(
    transition: TextSessionTransitionV1,
) -> DesktopTextActivationResultV1:
    status = {
        "enter": "entered",
        "same_story_handoff": "same_story_handoff",
        "story_switch": "story_switch",
    }.get(transition.kind)
    if status is None:
        _fail(
            "invalid_session_transition",
            "gesture activation received non-entry session transition",
        )
    return DesktopTextActivationResultV1(
        protocol_version="chaptera.desktop-text-activation-result.v1",
        status=status,
        transition=transition,
        active_session=transition.session,
        story_shortcuts_owned=True,
        canvas_object_shortcuts_owned=False,
        document_mutation_count=0,
        reason=None,
    )


def activate_explicit_edit_text_v1(
    *,
    session_id: str,
    document_id: str,
    revision_id: str,
    candidate: TextEntryCandidateV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
    active_session: TextEditSessionV1 | None = None,
    composition_resolution: CompositionResolutionV1 | None = None,
) -> DesktopTextActivationResultV1:
    try:
        pointer_context, initial_position = _explicit_activation_position(
            candidate=candidate,
            domain=domain,
            caret_map=caret_map,
        )
        if active_session is None:
            transition = enter_text_edit_session_v1(
                session_id=session_id,
                incarnation=0,
                document_id=document_id,
                entry_candidates=(candidate,),
                revision_id=revision_id,
                domain=domain,
                caret_map=caret_map,
                format_state=format_state,
                expected_layout_revision_id=expected_layout_revision_id,
                pointer_context=pointer_context,
                initial_position=initial_position,
                pending_interaction_metadata=(("entry_reason", "explicit_edit_text"),),
            )
        elif active_session.story_id == candidate.story_id:
            transition = handoff_same_story_frame_v1(
                active_session,
                entry_candidate=candidate,
                domain=domain,
                caret_map=caret_map,
                format_state=format_state,
                expected_layout_revision_id=expected_layout_revision_id,
                pointer_context=pointer_context,
                composition_resolution=composition_resolution,
            )
        else:
            transition = switch_text_edit_session_v1(
                active_session,
                entry_candidates=(candidate,),
                revision_id=revision_id,
                domain=domain,
                caret_map=caret_map,
                format_state=format_state,
                expected_layout_revision_id=expected_layout_revision_id,
                pointer_context=pointer_context,
                initial_position=initial_position,
                composition_resolution=composition_resolution,
                pending_interaction_metadata=(("entry_reason", "explicit_edit_text"),),
            )
    except EditorTextSessionGesturesError:
        raise
    except TextEditSessionError as exc:
        _fail(exc.code, str(exc))
    return _transition_result(transition)


def activate_pointer_text_v1(
    *,
    candidate: TextEntryCandidateV1,
    revision_id: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    expected_layout_revision_id: str,
    pointer_context: TextPointerEntryContextV1,
    active_session: TextEditSessionV1 | None,
    explicit_edit_requested: bool = False,
    click_count: int = 1,
    session_id: str = "desktop:text-session",
    document_id: str = "desktop:document",
    composition_resolution: CompositionResolutionV1 | None = None,
) -> DesktopTextActivationResultV1:
    if (
        not isinstance(click_count, int)
        or isinstance(click_count, bool)
        or click_count < 1
    ):
        _fail("invalid_click_count", "click_count must be positive integer")
    if click_count != 1:
        return DesktopTextActivationResultV1(
            protocol_version="chaptera.desktop-text-activation-result.v1",
            status="multiclick_unavailable",
            transition=None,
            active_session=active_session,
            story_shortcuts_owned=active_session is not None,
            canvas_object_shortcuts_owned=active_session is None,
            document_mutation_count=0,
            reason="double/triple-click text semantics are gated by EXP-TEXT-MULTICLICK-SELECTION-01",
        )
    if active_session is None and not explicit_edit_requested:
        return DesktopTextActivationResultV1(
            protocol_version="chaptera.desktop-text-activation-result.v1",
            status="canvas_object_selection",
            transition=None,
            active_session=None,
            story_shortcuts_owned=False,
            canvas_object_shortcuts_owned=True,
            document_mutation_count=0,
            reason="inactive single-click remains canvas object selection",
        )

    try:
        _preflight_pointer_target(
            candidate=candidate,
            domain=domain,
            caret_map=caret_map,
            pointer_context=pointer_context,
            expected_layout_revision_id=expected_layout_revision_id,
        )
        if active_session is None:
            transition = enter_text_edit_session_v1(
                session_id=session_id,
                incarnation=0,
                document_id=document_id,
                entry_candidates=(candidate,),
                revision_id=revision_id,
                domain=domain,
                caret_map=caret_map,
                format_state=format_state,
                expected_layout_revision_id=expected_layout_revision_id,
                pointer_context=pointer_context,
                pending_interaction_metadata=(("entry_reason", "pointer_edit_text"),),
            )
        elif active_session.story_id == candidate.story_id:
            transition = handoff_same_story_frame_v1(
                active_session,
                entry_candidate=candidate,
                domain=domain,
                caret_map=caret_map,
                format_state=format_state,
                expected_layout_revision_id=expected_layout_revision_id,
                pointer_context=pointer_context,
                composition_resolution=composition_resolution,
            )
        else:
            transition = switch_text_edit_session_v1(
                active_session,
                entry_candidates=(candidate,),
                revision_id=revision_id,
                domain=domain,
                caret_map=caret_map,
                format_state=format_state,
                expected_layout_revision_id=expected_layout_revision_id,
                pointer_context=pointer_context,
                composition_resolution=composition_resolution,
                pending_interaction_metadata=(("entry_reason", "pointer_edit_text"),),
            )
    except EditorTextSessionGesturesError as exc:
        if exc.code in {
            "direct_edit_unavailable",
            "unplaced_hit_test",
            "unplaced_story_position",
            "internal_cluster_unsupported",
        }:
            return DesktopTextActivationResultV1(
                protocol_version="chaptera.desktop-text-activation-result.v1",
                status="direct_edit_unavailable",
                transition=None,
                active_session=active_session,
                story_shortcuts_owned=active_session is not None,
                canvas_object_shortcuts_owned=active_session is None,
                document_mutation_count=0,
                reason=str(exc),
            )
        raise
    except TextEditSessionError as exc:
        if exc.code in {
            "unplaced_hit_test",
            "unplaced_story_position",
            "internal_cluster_unsupported",
        }:
            return DesktopTextActivationResultV1(
                protocol_version="chaptera.desktop-text-activation-result.v1",
                status="direct_edit_unavailable",
                transition=None,
                active_session=active_session,
                story_shortcuts_owned=active_session is not None,
                canvas_object_shortcuts_owned=active_session is None,
                document_mutation_count=0,
                reason=str(exc),
            )
        _fail(exc.code, str(exc))
    return _transition_result(transition)


def exit_desktop_text_mode_v1(
    *,
    active_session: TextEditSessionV1 | None,
    trigger: ExitTriggerV1,
    focus_owner: HostFocusOwnerV1,
    composition_resolution: CompositionResolutionV1 | None = None,
    submitted_durable_operation_ids: tuple[str, ...] = (),
    host_consumed: bool = False,
) -> DesktopTextExitResultV1:
    if trigger not in {
        "escape",
        "canvas_non_text_click",
        "non_text_tool",
        "explicit_exit",
    }:
        _fail("invalid_exit_trigger", "unsupported text-session exit trigger")
    if focus_owner not in {
        "canvas",
        "story_text",
        "inspector",
        "find_replace",
        "modal",
    }:
        _fail("invalid_focus_owner", "unsupported desktop focus owner")
    if active_session is None:
        return DesktopTextExitResultV1(
            protocol_version="chaptera.desktop-text-exit-result.v1",
            status="no_active_session",
            exit_receipt=None,
            active_session=None,
            story_shortcuts_owned=False,
            canvas_object_shortcuts_owned=True,
            transient_text_state_cleared=False,
            document_mutation_count=0,
            reason=None,
        )
    if not isinstance(active_session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 or None is required")

    # Escape/keyboard ownership stays with the focused host field/modal until
    # that owner explicitly declines the event. Explicit Exit Text and direct
    # canvas/tool transitions are product commands and are not blocked here.
    if trigger == "escape" and (
        host_consumed
        or focus_owner in {"inspector", "find_replace", "modal"}
    ):
        return DesktopTextExitResultV1(
            protocol_version="chaptera.desktop-text-exit-result.v1",
            status="host_focus_owned",
            exit_receipt=None,
            active_session=active_session,
            story_shortcuts_owned=False,
            canvas_object_shortcuts_owned=False,
            transient_text_state_cleared=False,
            document_mutation_count=0,
            reason="focused host control owns Escape/input",
        )

    try:
        receipt = exit_text_edit_session_v1(
            active_session,
            reason=trigger,
            composition_resolution=composition_resolution,
            submitted_durable_operation_ids=submitted_durable_operation_ids,
        )
    except TextEditSessionError as exc:
        if exc.code == "composition_transition_required":
            return DesktopTextExitResultV1(
                protocol_version="chaptera.desktop-text-exit-result.v1",
                status="composition_resolution_required",
                exit_receipt=None,
                active_session=active_session,
                story_shortcuts_owned=True,
                canvas_object_shortcuts_owned=False,
                transient_text_state_cleared=False,
                document_mutation_count=0,
                reason=str(exc),
            )
        _fail(exc.code, str(exc))

    return DesktopTextExitResultV1(
        protocol_version="chaptera.desktop-text-exit-result.v1",
        status="exited",
        exit_receipt=receipt,
        active_session=None,
        story_shortcuts_owned=False,
        canvas_object_shortcuts_owned=True,
        transient_text_state_cleared=True,
        document_mutation_count=0,
        reason=None,
    )
