#!/usr/bin/env python3
"""Focus-routed canvas/object Select All V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from canvas_tool_state_v1 import SELECT_TOOL_V1
from editor_multi_select_v1 import (
    AuthoredMultiSelectionStateV1,
    AuthoredSelectableNodeV1,
    empty_multi_selection_v1,
)
from editor_tool_routing_v1 import EditorToolRoutingStateV1
from story_edit_domain_v1 import StoryEditDomainV1
from text_select_all_v1 import TextSelectAllResultV1, select_all_story_text_v1
from text_selection_state_v1 import TextSelectionStateV1
from text_typing_format_state_v1 import TextTypingFormatStateV1


class EditorObjectSelectAllError(ValueError):
    pass


@dataclass(frozen=True)
class EditorObjectSelectAllResultV1:
    protocol_version: Literal["chaptera.editor-object-select-all-result.v1"]
    status: Literal[
        "objects_selected",
        "objects_cleared",
        "story_text_selected",
        "suppressed",
    ]
    object_selection: AuthoredMultiSelectionStateV1
    text_result: TextSelectAllResultV1 | None
    authored_scope_label: Literal["current_page_authored_direct_only"]
    eligible_count: int
    excluded_non_authored_count: int
    document_mutation_count: Literal[0]
    revision_created: Literal[False]
    scroll_changed: Literal[False]
    zoom_changed: Literal[False]
    authored_stack_changed: Literal[False]
    reason: str | None


def _validate_page_id(page_id: str) -> None:
    if not isinstance(page_id, str) or not page_id:
        raise EditorObjectSelectAllError("current_page_id is required")


def _page_domain(
    *,
    current_page_id: str,
    candidates: tuple[AuthoredSelectableNodeV1, ...],
) -> tuple[tuple[str, ...], int]:
    ids: list[str] = []
    excluded = 0
    for candidate in candidates:
        if not isinstance(candidate, AuthoredSelectableNodeV1):
            raise EditorObjectSelectAllError(
                "candidates must be AuthoredSelectableNodeV1 values"
            )
        if candidate.page_id != current_page_id:
            continue
        if not candidate.authored_direct:
            excluded += 1
            continue
        ids.append(candidate.node_id)
    if len(set(ids)) != len(ids):
        raise EditorObjectSelectAllError(
            "current-page authored domain contains duplicate NodeId"
        )
    return tuple(sorted(ids)), excluded


def route_editor_select_all_v1(
    *,
    routing_state: EditorToolRoutingStateV1,
    current_page_id: str,
    current_object_selection: AuthoredMultiSelectionStateV1,
    candidates: tuple[AuthoredSelectableNodeV1, ...],
    current_text_selection: TextSelectionStateV1 | None = None,
    text_domain: StoryEditDomainV1 | None = None,
    typing_state: TextTypingFormatStateV1 | None = None,
) -> EditorObjectSelectAllResultV1:
    """Route Ctrl/Cmd+A to exactly one owner.

    Story focus delegates to TextSelectAllV1. Canvas focus selects the semantic
    authored-direct object domain of the current page only. Host controls,
    active gestures, and temporary tools suppress the canvas command.
    """
    if not isinstance(routing_state, EditorToolRoutingStateV1):
        raise EditorObjectSelectAllError("EditorToolRoutingStateV1 is required")
    if not isinstance(current_object_selection, AuthoredMultiSelectionStateV1):
        raise EditorObjectSelectAllError(
            "AuthoredMultiSelectionStateV1 is required"
        )
    _validate_page_id(current_page_id)

    if current_object_selection.page_id != current_page_id:
        raise EditorObjectSelectAllError(
            "object selection belongs to a different current page"
        )

    if routing_state.focus_owner == "story_text":
        if current_text_selection is None or text_domain is None:
            raise EditorObjectSelectAllError(
                "Story focus requires current text selection and edit domain"
            )
        text_result = select_all_story_text_v1(
            current_selection=current_text_selection,
            domain=text_domain,
            input_owner="story_text",
            typing_state=typing_state,
        )
        return EditorObjectSelectAllResultV1(
            protocol_version="chaptera.editor-object-select-all-result.v1",
            status="story_text_selected",
            object_selection=current_object_selection,
            text_result=text_result,
            authored_scope_label="current_page_authored_direct_only",
            eligible_count=0,
            excluded_non_authored_count=0,
            document_mutation_count=0,
            revision_created=False,
            scroll_changed=False,
            zoom_changed=False,
            authored_stack_changed=False,
            reason=None,
        )

    if routing_state.focus_owner in {"modal", "ime", "inspector_input", "find_replace"}:
        return EditorObjectSelectAllResultV1(
            protocol_version="chaptera.editor-object-select-all-result.v1",
            status="suppressed",
            object_selection=current_object_selection,
            text_result=None,
            authored_scope_label="current_page_authored_direct_only",
            eligible_count=0,
            excluded_non_authored_count=0,
            document_mutation_count=0,
            revision_created=False,
            scroll_changed=False,
            zoom_changed=False,
            authored_stack_changed=False,
            reason=f"{routing_state.focus_owner} owns Select All input",
        )

    if routing_state.focus_owner != "canvas":
        return EditorObjectSelectAllResultV1(
            protocol_version="chaptera.editor-object-select-all-result.v1",
            status="suppressed",
            object_selection=current_object_selection,
            text_result=None,
            authored_scope_label="current_page_authored_direct_only",
            eligible_count=0,
            excluded_non_authored_count=0,
            document_mutation_count=0,
            revision_created=False,
            scroll_changed=False,
            zoom_changed=False,
            authored_stack_changed=False,
            reason="canvas does not own Select All input",
        )

    if routing_state.canvas.active_gesture is not None:
        return EditorObjectSelectAllResultV1(
            protocol_version="chaptera.editor-object-select-all-result.v1",
            status="suppressed",
            object_selection=current_object_selection,
            text_result=None,
            authored_scope_label="current_page_authored_direct_only",
            eligible_count=0,
            excluded_non_authored_count=0,
            document_mutation_count=0,
            revision_created=False,
            scroll_changed=False,
            zoom_changed=False,
            authored_stack_changed=False,
            reason="active canvas gesture owns keyboard routing",
        )

    if routing_state.canvas.active_tool != SELECT_TOOL_V1:
        return EditorObjectSelectAllResultV1(
            protocol_version="chaptera.editor-object-select-all-result.v1",
            status="suppressed",
            object_selection=current_object_selection,
            text_result=None,
            authored_scope_label="current_page_authored_direct_only",
            eligible_count=0,
            excluded_non_authored_count=0,
            document_mutation_count=0,
            revision_created=False,
            scroll_changed=False,
            zoom_changed=False,
            authored_stack_changed=False,
            reason="temporary canvas tool owns keyboard routing",
        )

    selected_ids, excluded = _page_domain(
        current_page_id=current_page_id,
        candidates=candidates,
    )
    if not selected_ids:
        next_selection = empty_multi_selection_v1(page_id=current_page_id)
        return EditorObjectSelectAllResultV1(
            protocol_version="chaptera.editor-object-select-all-result.v1",
            status="objects_cleared",
            object_selection=next_selection,
            text_result=None,
            authored_scope_label="current_page_authored_direct_only",
            eligible_count=0,
            excluded_non_authored_count=excluded,
            document_mutation_count=0,
            revision_created=False,
            scroll_changed=False,
            zoom_changed=False,
            authored_stack_changed=False,
            reason=(
                "no eligible authored-direct objects on current page"
                if excluded == 0
                else "no eligible authored-direct objects; non-authored instances excluded"
            ),
        )

    primary = selected_ids[0] if len(selected_ids) == 1 else None
    next_selection = AuthoredMultiSelectionStateV1(
        protocol_version="chaptera.authored-multi-selection.v1",
        page_id=current_page_id,
        selected_node_ids=selected_ids,
        primary_node_id=primary,
    )
    return EditorObjectSelectAllResultV1(
        protocol_version="chaptera.editor-object-select-all-result.v1",
        status="objects_selected",
        object_selection=next_selection,
        text_result=None,
        authored_scope_label="current_page_authored_direct_only",
        eligible_count=len(selected_ids),
        excluded_non_authored_count=excluded,
        document_mutation_count=0,
        revision_created=False,
        scroll_changed=False,
        zoom_changed=False,
        authored_stack_changed=False,
        reason=(
            None
            if excluded == 0
            else f"{excluded} current-page non-authored/projected instance(s) excluded from V1 scope"
        ),
    )
