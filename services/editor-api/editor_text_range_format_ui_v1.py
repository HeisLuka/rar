#!/usr/bin/env python3
"""Desktop one-frame Story selection/edit/format controller V1.

The widget is a consumer, never document truth:
- canonical session/selection owns Story identity and scalar ranges;
- keyboard/visual-line policies own navigation;
- TextIngressV1 owns external newline normalization;
- StoryEditTransactionV1 owns durable text mutation;
- TextFormatOverlayV1 owns durable range formatting;
- TextTypingFormatStateV1 owns collapsed-caret formatting intentions.

No DOM range, widget line model, durable generic Toggle, or zero-length
formatting span is introduced here.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any, Literal

from resolved_text_caret_map_v1 import (
    ResolvedTextCaretMapV1,
    project_selection_state_v1 if False else ResolvedTextCaretMapV1,
)
from story_edit_domain_v1 import StoryEditDomainV1
from story_edit_transaction_v1 import validate_story_edit_transaction_request_v1
from text_edit_session_v1 import TextEditSessionV1
from text_format_overlay_v1 import (
    EffectivePropertySegmentV1,
    FormatPropertyV1,
    TextFormatOperationReceiptV1,
    TextFormatOverlayError,
    TextFormatOverlayStateV1,
    clear_text_format_property_override_v1,
    effective_property_segments_v1,
    set_text_format_property_v1,
    state_hash_v1,
)
from text_ingress_v1 import TextIngressError, normalize_external_text_v1
from text_keyboard_policy_v1 import (
    TextKeyboardPolicyError,
    apply_text_keyboard_policy_v1,
)
from text_selection_state_v1 import (
    TextSelectionStateError,
    TextSelectionStateV1,
    project_selection_state_v1,
)
from text_typing_format_state_v1 import (
    TextTypingFormatStateV1,
    clear_pending_typing_property_v1,
    clear_typing_state_for_context_change_v1,
    derive_typing_format_state_v1,
    displayed_character_properties_v1,
    set_pending_typing_property_v1,
    snapshot_typing_format_v1,
)
from text_unplaced_interaction_v1 import decide_text_interaction_admissibility_v1
from text_visual_line_navigation_v1 import navigate_visual_line_v1


FormatDisplayStateV1 = Literal[
    "inherited_base",
    "explicit",
    "mixed_effective",
    "mixed_provenance",
    "unavailable",
]
FormatControlActionV1 = Literal["press", "set", "clear_override"]
NavigationCommandV1 = Literal[
    "move_previous",
    "move_next",
    "extend_previous",
    "extend_next",
    "delete_backward",
    "delete_forward",
    "move_line_start",
    "move_line_end",
    "extend_line_start",
    "extend_line_end",
    "move_visual_line_up",
    "move_visual_line_down",
    "extend_visual_line_up",
    "extend_visual_line_down",
]

_PROPS: tuple[FormatPropertyV1, ...] = (
    "bold",
    "italic",
    "font_size_emu",
    "text_color_rgb",
)
_KEYBOARD = {
    "move_previous",
    "move_next",
    "extend_previous",
    "extend_next",
    "delete_backward",
    "delete_forward",
}
_VISUAL = {
    "move_line_start",
    "move_line_end",
    "extend_line_start",
    "extend_line_end",
    "move_visual_line_up",
    "move_visual_line_down",
    "extend_visual_line_up",
    "extend_visual_line_down",
}


class EditorTextRangeFormatUIError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextFormatControlStateV1:
    property: FormatPropertyV1
    display_state: FormatDisplayStateV1
    effective_value: Any | None
    effective_values: tuple[Any, ...]
    provenance: tuple[str, ...]
    pending_explicit: bool
    enabled: bool
    reason: str | None


@dataclass(frozen=True)
class EditorTextRangeFormatUIStateV1:
    protocol_version: Literal["chaptera.editor-text-range-format-ui-state.v1"]
    story_id: str
    revision_id: str
    selection_kind: Literal["collapsed", "range"]
    selection_start_scalar: int
    selection_end_scalar: int
    geometry_state: Literal["complete", "partial", "unplaced", "unsupported"]
    direct_edit_available: bool
    range_format_available: bool
    one_frame_surface: Literal[True]
    controls: tuple[TextFormatControlStateV1, ...]
    status: Literal["ready", "unsupported"]
    reason: str | None


@dataclass(frozen=True)
class EditorTextFormatActionResultV1:
    protocol_version: Literal["chaptera.editor-text-format-action-result.v1"]
    status: Literal["range_operation", "typing_state", "unsupported"]
    property: FormatPropertyV1
    action: FormatControlActionV1
    range_receipt: TextFormatOperationReceiptV1 | None
    typing_state: TextTypingFormatStateV1 | None
    durable_command_kind: str | None
    requires_authoritative_relayout: bool
    authoring_revision_count: Literal[0]
    zero_length_durable_span_created: Literal[False]
    reason: str | None


@dataclass(frozen=True)
class EditorTextNavigationResultV1:
    protocol_version: Literal["chaptera.editor-text-navigation-result.v1"]
    status: Literal["selection", "delete", "boundary_noop", "unsupported"]
    command: NavigationCommandV1
    session: TextEditSessionV1
    delete_start_scalar: int | None
    delete_end_scalar: int | None
    typing_state_cleared: bool
    authoring_revision_count: Literal[0]
    reason: str | None


@dataclass(frozen=True)
class EditorTextMutationPlanV1:
    protocol_version: Literal["chaptera.editor-text-mutation-plan.v1"]
    status: Literal["ready", "boundary_noop", "unsupported"]
    request: dict | None
    start_scalar: int | None
    end_scalar: int | None
    canonical_replacement_text: str | None
    post_edit_selection_intent: Literal["collapse_after_edit"] | None
    authoring_revision_count: Literal[0]
    reason: str | None


def _fail(code: str, message: str) -> None:
    raise EditorTextRangeFormatUIError(code, message)


def _validate_context(
    *,
    session: TextEditSessionV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
) -> None:
    if not isinstance(session, TextEditSessionV1):
        _fail("invalid_session", "TextEditSessionV1 is required")
    if session.focus_owner != "story_text":
        _fail("invalid_session", "Story text must own focus")
    if session.composition_session is not None:
        _fail(
            "composition_active",
            "format/edit surface must resolve active composition before ordinary commands",
        )
    if domain.story_id != session.story_id:
        _fail("story_mismatch", "session and StoryEditDomainV1 target different Stories")
    if caret_map.story_id != session.story_id:
        _fail("story_mismatch", "session and caret map target different Stories")
    if format_state.story_id != session.story_id:
        _fail("story_mismatch", "session and format state target different Stories")
    if domain.raw_scalar_len != caret_map.story_scalar_len:
        _fail("reconcile_required", "caret map extent differs from Story edit domain")
    if domain.raw_scalar_len != format_state.story_scalar_len:
        _fail("reconcile_required", "format extent differs from Story edit domain")
    if session.revision_id != session.selection.revision_id:
        _fail("reconcile_required", "session selection revision is stale")
    if session.layout_revision_id != caret_map.layout_revision_id:
        _fail("reconcile_required", "session layout receipt is stale")

    # This task intentionally owns the first ordinary one-frame desktop slice.
    frames = {line.frame_id for line in caret_map.lines}
    if len(frames) > 1:
        _fail(
            "linked_story_ui_unsupported",
            "base range-format UI slice supports one physical TextFrame only",
        )
    if frames and session.current_frame_id is not None and session.current_frame_id not in frames:
        _fail("reconcile_required", "session current frame differs from authoritative caret map")


def _normalized_selection(session: TextEditSessionV1) -> tuple[int, int]:
    return session.selection.normalized_range


def _range_control(
    *,
    prop: FormatPropertyV1,
    segments: tuple[EffectivePropertySegmentV1, ...],
    enabled: bool,
    reason: str | None,
) -> TextFormatControlStateV1:
    values = tuple(dict.fromkeys(segment.value for segment in segments))
    sources = tuple(sorted({segment.source for segment in segments}))
    if len(values) > 1:
        display = "mixed_effective"
        value = None
    elif not values:
        display = "unavailable"
        value = None
    elif len(sources) > 1:
        display = "mixed_provenance"
        value = values[0]
    elif sources == ("chaptera_override",):
        display = "explicit"
        value = values[0]
    else:
        display = "inherited_base"
        value = values[0]
    return TextFormatControlStateV1(
        property=prop,
        display_state=display,
        effective_value=value,
        effective_values=values,
        provenance=sources,
        pending_explicit=False,
        enabled=enabled,
        reason=reason,
    )


def _caret_scalar_for_display(
    selection: TextSelectionStateV1,
    format_state: TextFormatOverlayStateV1,
) -> int | None:
    if format_state.story_scalar_len == 0:
        return None
    caret = selection.focus_scalar
    scalar = caret - 1 if caret > 0 else 0
    if scalar >= format_state.story_scalar_len:
        scalar = format_state.story_scalar_len - 1
    return scalar


def build_editor_text_range_format_ui_state_v1(
    *,
    session: TextEditSessionV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
) -> EditorTextRangeFormatUIStateV1:
    _validate_context(
        session=session,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
    )
    selection = session.selection
    start, end = selection.normalized_range

    if selection.is_collapsed:
        admissibility = decide_text_interaction_admissibility_v1(
            selection=selection,
            caret_map=caret_map,
            command_kind="collapsed_format",
            expected_layout_revision_id=caret_map.layout_revision_id,
        )
        direct = admissibility.admitted
        geometry = admissibility.geometry_state
        try:
            typing = (
                session.typing_state
                if session.typing_state is not None
                else derive_typing_format_state_v1(
                    selection=selection,
                    domain=domain,
                    format_state=format_state,
                )
            )
            displayed = displayed_character_properties_v1(
                state=typing,
                selection=selection,
                domain=domain,
                format_state=format_state,
            )
        except ValueError as exc:
            _fail(getattr(exc, "code", "typing_context_mismatch"), str(exc))

        pending = {} if typing is None else dict(typing.pending_explicit_properties)
        scalar = _caret_scalar_for_display(selection, format_state)
        controls = []
        for prop in _PROPS:
            if prop in pending:
                controls.append(
                    TextFormatControlStateV1(
                        property=prop,
                        display_state="explicit",
                        effective_value=displayed.get(prop),
                        effective_values=(
                            () if prop not in displayed else (displayed[prop],)
                        ),
                        provenance=("typing_pending",),
                        pending_explicit=True,
                        enabled=direct,
                        reason=None if direct else admissibility.reason,
                    )
                )
                continue
            if scalar is None or prop not in displayed:
                controls.append(
                    TextFormatControlStateV1(
                        property=prop,
                        display_state="unavailable",
                        effective_value=None,
                        effective_values=(),
                        provenance=(),
                        pending_explicit=False,
                        enabled=False,
                        reason=admissibility.reason or "no materialized character context",
                    )
                )
                continue
            try:
                segs = effective_property_segments_v1(
                    state=format_state,
                    prop=prop,
                    start_scalar=scalar,
                    end_scalar=scalar + 1,
                )
            except TextFormatOverlayError as exc:
                _fail("format_state_unsupported", str(exc))
            source = segs[0].source
            controls.append(
                TextFormatControlStateV1(
                    property=prop,
                    display_state=(
                        "explicit" if source == "chaptera_override" else "inherited_base"
                    ),
                    effective_value=displayed[prop],
                    effective_values=(displayed[prop],),
                    provenance=(source,),
                    pending_explicit=False,
                    enabled=direct,
                    reason=None if direct else admissibility.reason,
                )
            )
        return EditorTextRangeFormatUIStateV1(
            protocol_version="chaptera.editor-text-range-format-ui-state.v1",
            story_id=session.story_id,
            revision_id=session.revision_id,
            selection_kind="collapsed",
            selection_start_scalar=start,
            selection_end_scalar=end,
            geometry_state=geometry,
            direct_edit_available=direct,
            range_format_available=False,
            one_frame_surface=True,
            controls=tuple(controls),
            status="ready" if direct else "unsupported",
            reason=None if direct else admissibility.reason,
        )

    admissibility = decide_text_interaction_admissibility_v1(
        selection=selection,
        caret_map=caret_map,
        command_kind="range_format",
        expected_layout_revision_id=caret_map.layout_revision_id,
    )
    controls = []
    for prop in _PROPS:
        try:
            segs = effective_property_segments_v1(
                state=format_state,
                prop=prop,
                start_scalar=start,
                end_scalar=end,
            )
        except TextFormatOverlayError as exc:
            _fail("format_state_unsupported", str(exc))
        controls.append(
            _range_control(
                prop=prop,
                segments=segs,
                enabled=admissibility.admitted,
                reason=None if admissibility.admitted else admissibility.reason,
            )
        )
    return EditorTextRangeFormatUIStateV1(
        protocol_version="chaptera.editor-text-range-format-ui-state.v1",
        story_id=session.story_id,
        revision_id=session.revision_id,
        selection_kind="range",
        selection_start_scalar=start,
        selection_end_scalar=end,
        geometry_state=admissibility.geometry_state,
        direct_edit_available=False,
        range_format_available=admissibility.admitted,
        one_frame_surface=True,
        controls=tuple(controls),
        status="ready" if admissibility.admitted else "unsupported",
        reason=None if admissibility.admitted else admissibility.reason,
    )


def _control(
    state: EditorTextRangeFormatUIStateV1,
    prop: FormatPropertyV1,
) -> TextFormatControlStateV1:
    matches = [item for item in state.controls if item.property == prop]
    if len(matches) != 1:
        _fail("unsupported_format_property", f"unsupported UI format property {prop}")
    return matches[0]


def apply_editor_text_format_control_v1(
    *,
    session: TextEditSessionV1,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    prop: FormatPropertyV1,
    action: FormatControlActionV1,
    value: Any | None = None,
) -> EditorTextFormatActionResultV1:
    ui = build_editor_text_range_format_ui_state_v1(
        session=session,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
    )
    control = _control(ui, prop)
    if action not in {"press", "set", "clear_override"}:
        _fail("unsupported_format_action", "format control action is unsupported")
    if not control.enabled:
        return EditorTextFormatActionResultV1(
            protocol_version="chaptera.editor-text-format-action-result.v1",
            status="unsupported",
            property=prop,
            action=action,
            range_receipt=None,
            typing_state=session.typing_state,
            durable_command_kind=None,
            requires_authoritative_relayout=False,
            authoring_revision_count=0,
            zero_length_durable_span_created=False,
            reason=control.reason or "format control is unavailable",
        )

    if session.selection.is_collapsed:
        try:
            typing = (
                session.typing_state
                if session.typing_state is not None
                else derive_typing_format_state_v1(
                    selection=session.selection,
                    domain=domain,
                    format_state=format_state,
                )
            )
            if typing is None:
                _fail("typing_context_mismatch", "collapsed caret requires typing state")
            if action == "clear_override":
                updated = clear_pending_typing_property_v1(state=typing, prop=prop)
            elif action == "press":
                if prop not in {"bold", "italic"}:
                    _fail(
                        "unsupported_format_action",
                        "press toggle is only defined for Bold/Italic",
                    )
                displayed = displayed_character_properties_v1(
                    state=typing,
                    selection=session.selection,
                    domain=domain,
                    format_state=format_state,
                )
                target = not bool(displayed[prop])
                updated = set_pending_typing_property_v1(
                    state=typing,
                    prop=prop,
                    value=target,
                )
            else:
                updated = set_pending_typing_property_v1(
                    state=typing,
                    prop=prop,
                    value=value,
                )
        except ValueError as exc:
            if isinstance(exc, EditorTextRangeFormatUIError):
                raise
            _fail(getattr(exc, "code", "typing_format_rejected"), str(exc))
        return EditorTextFormatActionResultV1(
            protocol_version="chaptera.editor-text-format-action-result.v1",
            status="typing_state",
            property=prop,
            action=action,
            range_receipt=None,
            typing_state=updated,
            durable_command_kind=None,
            requires_authoritative_relayout=False,
            authoring_revision_count=0,
            zero_length_durable_span_created=False,
            reason=None,
        )

    start, end = session.selection.normalized_range
    try:
        if action == "clear_override":
            receipt = clear_text_format_property_override_v1(
                state=format_state,
                start_scalar=start,
                end_scalar=end,
                prop=prop,
                expected_state_hash=state_hash_v1(format_state),
            )
        else:
            if action == "press":
                if prop not in {"bold", "italic"}:
                    _fail(
                        "unsupported_format_action",
                        "press toggle is only defined for Bold/Italic",
                    )
                # Product click is resolved now, not persisted as Toggle:
                # mixed/false => explicit true, uniform true => explicit false.
                target = not (
                    control.display_state != "mixed_effective"
                    and len(control.effective_values) == 1
                    and control.effective_values[0] is True
                )
            else:
                target = value
            receipt = set_text_format_property_v1(
                state=format_state,
                start_scalar=start,
                end_scalar=end,
                prop=prop,
                value=target,
                expected_state_hash=state_hash_v1(format_state),
            )
    except TextFormatOverlayError as exc:
        _fail("format_operation_rejected", str(exc))

    return EditorTextFormatActionResultV1(
        protocol_version="chaptera.editor-text-format-action-result.v1",
        status="range_operation",
        property=prop,
        action=action,
        range_receipt=receipt,
        typing_state=None,
        durable_command_kind=receipt.command["kind"],
        requires_authoritative_relayout=receipt.requires_authoritative_relayout,
        authoring_revision_count=0,
        zero_length_durable_span_created=False,
        reason=None,
    )


def route_editor_text_navigation_v1(
    *,
    session: TextEditSessionV1,
    command: NavigationCommandV1,
    story_text: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
) -> EditorTextNavigationResultV1:
    _validate_context(
        session=session,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
    )
    if command in _KEYBOARD:
        try:
            decision = apply_text_keyboard_policy_v1(
                command=command,
                story_text=story_text,
                domain=domain,
                selection=session.selection,
                caret_map=caret_map,
                expected_revision_id=session.revision_id,
            )
        except TextKeyboardPolicyError as exc:
            return EditorTextNavigationResultV1(
                protocol_version="chaptera.editor-text-navigation-result.v1",
                status="unsupported",
                command=command,
                session=session,
                delete_start_scalar=None,
                delete_end_scalar=None,
                typing_state_cleared=False,
                authoring_revision_count=0,
                reason=f"{exc.code}:{exc}",
            )
        if decision.action == "boundary_noop":
            return EditorTextNavigationResultV1(
                protocol_version="chaptera.editor-text-navigation-result.v1",
                status="boundary_noop",
                command=command,
                session=session,
                delete_start_scalar=None,
                delete_end_scalar=None,
                typing_state_cleared=False,
                authoring_revision_count=0,
                reason=None,
            )
        if decision.action == "delete":
            intent = decision.delete_intent
            assert intent is not None
            return EditorTextNavigationResultV1(
                protocol_version="chaptera.editor-text-navigation-result.v1",
                status="delete",
                command=command,
                session=session,
                delete_start_scalar=intent.start_scalar,
                delete_end_scalar=intent.end_scalar,
                typing_state_cleared=False,
                authoring_revision_count=0,
                reason=None,
            )
        assert decision.selection is not None
        try:
            projected = project_selection_state_v1(
                state=decision.selection,
                domain=domain,
                caret_map=caret_map,
            ).state
            typing = clear_typing_state_for_context_change_v1(
                selection=projected,
                domain=domain,
                format_state=format_state,
            )
        except TextSelectionStateError as exc:
            return EditorTextNavigationResultV1(
                protocol_version="chaptera.editor-text-navigation-result.v1",
                status="unsupported",
                command=command,
                session=session,
                delete_start_scalar=None,
                delete_end_scalar=None,
                typing_state_cleared=False,
                authoring_revision_count=0,
                reason=f"{exc.code}:{exc}",
            )
        updated = replace(session, selection=projected, typing_state=typing)
        return EditorTextNavigationResultV1(
            protocol_version="chaptera.editor-text-navigation-result.v1",
            status="selection",
            command=command,
            session=updated,
            delete_start_scalar=None,
            delete_end_scalar=None,
            typing_state_cleared=bool(
                session.typing_state is not None
                and session.typing_state.pending_explicit_properties
            ),
            authoring_revision_count=0,
            reason=None,
        )

    if command not in _VISUAL:
        _fail("unsupported_navigation_command", "navigation command is outside V1")
    result = navigate_visual_line_v1(
        command=command,
        state=session.selection,
        domain=domain,
        caret_map=caret_map,
        expected_layout_revision_id=caret_map.layout_revision_id,
    )
    if result.status in {"navigation_unsupported", "reconcile_required"}:
        return EditorTextNavigationResultV1(
            protocol_version="chaptera.editor-text-navigation-result.v1",
            status="unsupported",
            command=command,
            session=session,
            delete_start_scalar=None,
            delete_end_scalar=None,
            typing_state_cleared=False,
            authoring_revision_count=0,
            reason=result.reason,
        )
    moved = result.selection != session.selection
    typing = session.typing_state
    if moved:
        try:
            typing = clear_typing_state_for_context_change_v1(
                selection=result.selection,
                domain=domain,
                format_state=format_state,
            )
        except ValueError as exc:
            _fail(getattr(exc, "code", "typing_context_mismatch"), str(exc))
    updated = replace(session, selection=result.selection, typing_state=typing)
    return EditorTextNavigationResultV1(
        protocol_version="chaptera.editor-text-navigation-result.v1",
        status="selection" if moved else "boundary_noop",
        command=command,
        session=updated,
        delete_start_scalar=None,
        delete_end_scalar=None,
        typing_state_cleared=bool(
            moved
            and session.typing_state is not None
            and session.typing_state.pending_explicit_properties
        ),
        authoring_revision_count=0,
        reason=result.reason,
    )


def _story_transaction_request(
    *,
    session: TextEditSessionV1,
    source_hash: str,
    client_operation_id: str,
    story_text: str,
    start: int,
    end: int,
    replacement_text: str,
    paragraph_inserted_ids: tuple[str, ...],
    typing_format: dict[str, Any] | None,
) -> dict:
    request = {
        "protocol_version": "chaptera.story-edit-transaction-intent.v1",
        "document_id": session.document_id,
        "source_hash": source_hash,
        "base_revision_id": session.revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "story_edit_transaction",
            "story_id": session.story_id,
            "start_scalar": start,
            "end_scalar": end,
            "expected_before": story_text[start:end],
            "replacement_text": replacement_text,
            "paragraph_inserted_ids": list(paragraph_inserted_ids),
            "paragraph_inserted_property_presets": [],
            "typing_format": typing_format,
            "fragment_format_runs": [],
            "incoming_semantic_kinds": [],
        },
    }
    try:
        validate_story_edit_transaction_request_v1(request)
    except ValueError as exc:
        _fail("invalid_story_edit_request", str(exc))
    return request


def build_editor_text_input_request_v1(
    *,
    session: TextEditSessionV1,
    story_text: str,
    domain: StoryEditDomainV1,
    caret_map: ResolvedTextCaretMapV1,
    format_state: TextFormatOverlayStateV1,
    source_hash: str,
    client_operation_id: str,
    external_text: str,
    paragraph_inserted_ids: tuple[str, ...] = (),
) -> EditorTextMutationPlanV1:
    _validate_context(
        session=session,
        domain=domain,
        caret_map=caret_map,
        format_state=format_state,
    )
    admissibility = decide_text_interaction_admissibility_v1(
        selection=session.selection,
        caret_map=caret_map,
        command_kind="typing",
        expected_layout_revision_id=caret_map.layout_revision_id,
    )
    if not admissibility.admitted:
        return EditorTextMutationPlanV1(
            protocol_version="chaptera.editor-text-mutation-plan.v1",
            status="unsupported",
            request=None,
            start_scalar=None,
            end_scalar=None,
            canonical_replacement_text=None,
            post_edit_selection_intent=None,
            authoring_revision_count=0,
            reason=admissibility.reason,
        )
    try:
        canonical = normalize_external_text_v1(external_text)
    except TextIngressError as exc:
        _fail("text_ingress_rejected", str(exc))
    typing_snapshot = snapshot_typing_format_v1(session.typing_state)
    typing = None if typing_snapshot is None else dict(typing_snapshot.items)
    start, end = session.selection.normalized_range
    request = _story_transaction_request(
        session=session,
        source_hash=source_hash,
        client_operation_id=client_operation_id,
        story_text=story_text,
        start=start,
        end=end,
        replacement_text=canonical.text,
        paragraph_inserted_ids=paragraph_inserted_ids,
        typing_format=typing,
    )
    return EditorTextMutationPlanV1(
        protocol_version="chaptera.editor-text-mutation-plan.v1",
        status="ready",
        request=request,
        start_scalar=start,
        end_scalar=end,
        canonical_replacement_text=canonical.text,
        post_edit_selection_intent="collapse_after_edit",
        authoring_revision_count=0,
        reason=None,
    )


def build_editor_keyboard_delete_request_v1(
    *,
    navigation: EditorTextNavigationResultV1,
    story_text: str,
    source_hash: str,
    client_operation_id: str,
) -> EditorTextMutationPlanV1:
    if not isinstance(navigation, EditorTextNavigationResultV1):
        _fail("invalid_navigation_result", "EditorTextNavigationResultV1 is required")
    if navigation.status == "boundary_noop":
        return EditorTextMutationPlanV1(
            protocol_version="chaptera.editor-text-mutation-plan.v1",
            status="boundary_noop",
            request=None,
            start_scalar=None,
            end_scalar=None,
            canonical_replacement_text="",
            post_edit_selection_intent=None,
            authoring_revision_count=0,
            reason=None,
        )
    if navigation.status != "delete":
        _fail("invalid_navigation_result", "keyboard delete plan requires delete decision")
    assert navigation.delete_start_scalar is not None
    assert navigation.delete_end_scalar is not None
    request = _story_transaction_request(
        session=navigation.session,
        source_hash=source_hash,
        client_operation_id=client_operation_id,
        story_text=story_text,
        start=navigation.delete_start_scalar,
        end=navigation.delete_end_scalar,
        replacement_text="",
        paragraph_inserted_ids=(),
        typing_format=None,
    )
    return EditorTextMutationPlanV1(
        protocol_version="chaptera.editor-text-mutation-plan.v1",
        status="ready",
        request=request,
        start_scalar=navigation.delete_start_scalar,
        end_scalar=navigation.delete_end_scalar,
        canonical_replacement_text="",
        post_edit_selection_intent="collapse_after_edit",
        authoring_revision_count=0,
        reason=None,
    )
