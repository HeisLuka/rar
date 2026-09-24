#!/usr/bin/env python3
"""Desktop tool/focus routing and Escape precedence V1.

This module owns only transient routing. Feature authoring semantics remain in
their feature controllers. Exactly one owner receives pointer/keyboard input.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal

from canvas_tool_state_v1 import (
    LINK_TEXTBOX_TOOL_V1,
    PICTURE_CREATE_TOOL_V1,
    PICTURE_CROP_TOOL_V1,
    RECTANGLE_CREATE_TOOL_V1,
    SELECT_TOOL_V1,
    TEXTBOX_CREATE_TOOL_V1,
    CanvasToolIdV1,
    CanvasToolStateV1,
    activate_canvas_tool_v1,
    escape_canvas_tool_v1,
)
from editor_text_session_gestures_v1 import (
    CompositionResolutionV1,
    DesktopTextExitResultV1,
    TextEditSessionV1,
    exit_desktop_text_mode_v1,
)
from group_member_selection_v1 import (
    GroupMembersSelectionScopeV1,
    ObjectSelectionScopeV1,
    TopLevelSelectionScopeV1,
    escape_parent_v1,
)


FocusOwnerV1 = Literal[
    "canvas",
    "story_text",
    "inspector_input",
    "find_replace",
    "modal",
    "ime",
]
PointerOwnerV1 = Literal["none", "canvas_tool", "canvas_gesture", "story_text", "host_control"]
EscapeActionV1 = Literal[
    "host_consumed",
    "text_session_exit",
    "composition_resolution_required",
    "canvas_gesture_cancelled",
    "temporary_tool_to_select",
    "escape_parent_group",
    "clear_top_level_selection",
    "no_op",
]
ToolCompletionV1 = Literal["accepted", "cancelled", "selection_invalidated"]


class EditorToolRoutingError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class EditorToolRoutingStateV1:
    protocol_version: Literal["chaptera.editor-tool-routing-state.v1"]
    canvas: CanvasToolStateV1
    focus_owner: FocusOwnerV1
    selection_scope: ObjectSelectionScopeV1
    active_text_session: TextEditSessionV1 | None
    document_mutation_count: Literal[0] = 0


@dataclass(frozen=True)
class EditorToolRoutingResultV1:
    protocol_version: Literal["chaptera.editor-tool-routing-result.v1"]
    state: EditorToolRoutingStateV1
    action: str
    pointer_owner: PointerOwnerV1
    cancelled_gesture_token: str | None
    selection_clear_requested: bool
    document_mutation_count: Literal[0]


def _fail(code: str, message: str) -> None:
    raise EditorToolRoutingError(code, message)


def _validate_focus(owner: str) -> None:
    if owner not in {
        "canvas",
        "story_text",
        "inspector_input",
        "find_replace",
        "modal",
        "ime",
    }:
        _fail("invalid_focus_owner", "unsupported desktop focus owner")


def pointer_owner_v1(state: EditorToolRoutingStateV1) -> PointerOwnerV1:
    _validate_state(state)
    if state.focus_owner in {"modal", "ime", "inspector_input", "find_replace"}:
        return "host_control"
    if state.focus_owner == "story_text" and state.active_text_session is not None:
        return "story_text"
    if state.canvas.active_gesture is not None:
        return "canvas_gesture"
    return "canvas_tool"


def _validate_state(state: EditorToolRoutingStateV1) -> None:
    if not isinstance(state, EditorToolRoutingStateV1):
        _fail("invalid_routing_state", "EditorToolRoutingStateV1 is required")
    if state.protocol_version != "chaptera.editor-tool-routing-state.v1":
        _fail("invalid_routing_state", "routing protocol mismatch")
    _validate_focus(state.focus_owner)
    if state.focus_owner == "story_text" and state.active_text_session is None:
        _fail("invalid_routing_state", "story_text focus requires active TextEditSessionV1")


def build_editor_tool_routing_state_v1(
    *,
    canvas: CanvasToolStateV1,
    selection_scope: ObjectSelectionScopeV1,
    focus_owner: FocusOwnerV1 = "canvas",
    active_text_session: TextEditSessionV1 | None = None,
) -> EditorToolRoutingStateV1:
    _validate_focus(focus_owner)
    state = EditorToolRoutingStateV1(
        protocol_version="chaptera.editor-tool-routing-state.v1",
        canvas=canvas,
        focus_owner=focus_owner,
        selection_scope=selection_scope,
        active_text_session=active_text_session,
        document_mutation_count=0,
    )
    _validate_state(state)
    return state


def set_host_focus_owner_v1(
    state: EditorToolRoutingStateV1,
    *,
    focus_owner: FocusOwnerV1,
) -> EditorToolRoutingResultV1:
    _validate_state(state)
    _validate_focus(focus_owner)
    if focus_owner == "story_text" and state.active_text_session is None:
        _fail("invalid_focus_owner", "story_text focus requires active text session")
    next_state = replace(state, focus_owner=focus_owner)
    return EditorToolRoutingResultV1(
        protocol_version="chaptera.editor-tool-routing-result.v1",
        state=next_state,
        action="focus_owner_changed" if focus_owner != state.focus_owner else "no_change",
        pointer_owner=pointer_owner_v1(next_state),
        cancelled_gesture_token=None,
        selection_clear_requested=False,
        document_mutation_count=0,
    )


def activate_desktop_tool_v1(
    state: EditorToolRoutingStateV1,
    *,
    tool: CanvasToolIdV1,
    composition_resolution: CompositionResolutionV1 | None = None,
    submitted_durable_operation_ids: tuple[str, ...] = (),
) -> EditorToolRoutingResultV1:
    """Activate one canvas tool after resolving any active Story text session."""
    _validate_state(state)
    active_session = state.active_text_session
    focus_owner = state.focus_owner

    if active_session is not None:
        exit_result = exit_desktop_text_mode_v1(
            active_session=active_session,
            trigger="non_text_tool",
            focus_owner=(
                "story_text"
                if focus_owner == "story_text"
                else "modal"
                if focus_owner in {"modal", "ime"}
                else "find_replace"
                if focus_owner == "find_replace"
                else "inspector"
                if focus_owner == "inspector_input"
                else "canvas"
            ),
            composition_resolution=composition_resolution,
            submitted_durable_operation_ids=submitted_durable_operation_ids,
        )
        if exit_result.status == "composition_resolution_required":
            return EditorToolRoutingResultV1(
                protocol_version="chaptera.editor-tool-routing-result.v1",
                state=state,
                action="composition_resolution_required",
                pointer_owner=pointer_owner_v1(state),
                cancelled_gesture_token=None,
                selection_clear_requested=False,
                document_mutation_count=0,
            )
        if exit_result.status != "exited":
            _fail(
                "text_session_exit_failed",
                "non-text tool activation must explicitly resolve active Story session",
            )
        active_session = None
        focus_owner = "canvas"

    transition = activate_canvas_tool_v1(state.canvas, tool=tool)
    next_state = EditorToolRoutingStateV1(
        protocol_version=state.protocol_version,
        canvas=transition.state,
        focus_owner=focus_owner,
        selection_scope=state.selection_scope,
        active_text_session=active_session,
        document_mutation_count=0,
    )
    return EditorToolRoutingResultV1(
        protocol_version="chaptera.editor-tool-routing-result.v1",
        state=next_state,
        action=(
            "text_session_exited_then_" + transition.action
            if state.active_text_session is not None
            else transition.action
        ),
        pointer_owner=pointer_owner_v1(next_state),
        cancelled_gesture_token=transition.cancelled_gesture_token,
        selection_clear_requested=False,
        document_mutation_count=0,
    )


def route_escape_v1(
    state: EditorToolRoutingStateV1,
    *,
    host_consumed: bool = False,
    composition_resolution: CompositionResolutionV1 | None = None,
    submitted_durable_operation_ids: tuple[str, ...] = (),
) -> EditorToolRoutingResultV1:
    _validate_state(state)

    # 1. Modal / IME / host field owns Escape before any Story/canvas state.
    if state.focus_owner in {"modal", "ime", "inspector_input", "find_replace"}:
        return EditorToolRoutingResultV1(
            protocol_version="chaptera.editor-tool-routing-result.v1",
            state=state,
            action="host_consumed",
            pointer_owner=pointer_owner_v1(state),
            cancelled_gesture_token=None,
            selection_clear_requested=False,
            document_mutation_count=0,
        )

    # 2. Active Story text session.
    if state.active_text_session is not None:
        exit_result = exit_desktop_text_mode_v1(
            active_session=state.active_text_session,
            trigger="escape",
            focus_owner="story_text",
            composition_resolution=composition_resolution,
            submitted_durable_operation_ids=submitted_durable_operation_ids,
            host_consumed=host_consumed,
        )
        if exit_result.status == "composition_resolution_required":
            return EditorToolRoutingResultV1(
                protocol_version="chaptera.editor-tool-routing-result.v1",
                state=state,
                action="composition_resolution_required",
                pointer_owner=pointer_owner_v1(state),
                cancelled_gesture_token=None,
                selection_clear_requested=False,
                document_mutation_count=0,
            )
        if exit_result.status != "exited":
            _fail("text_session_exit_failed", "Story text Escape did not resolve deterministically")
        next_state = replace(
            state,
            focus_owner="canvas",
            active_text_session=None,
        )
        return EditorToolRoutingResultV1(
            protocol_version="chaptera.editor-tool-routing-result.v1",
            state=next_state,
            action="text_session_exit",
            pointer_owner=pointer_owner_v1(next_state),
            cancelled_gesture_token=None,
            selection_clear_requested=False,
            document_mutation_count=0,
        )

    # 3 / 4. Active canvas gesture first; then temporary tool -> Select.
    canvas_escape = escape_canvas_tool_v1(state.canvas)
    if canvas_escape.action != "no_change":
        next_state = replace(state, canvas=canvas_escape.state, focus_owner="canvas")
        return EditorToolRoutingResultV1(
            protocol_version="chaptera.editor-tool-routing-result.v1",
            state=next_state,
            action=(
                "canvas_gesture_cancelled"
                if canvas_escape.action == "gesture_cancelled"
                else "temporary_tool_to_select"
            ),
            pointer_owner=pointer_owner_v1(next_state),
            cancelled_gesture_token=canvas_escape.cancelled_gesture_token,
            selection_clear_requested=False,
            document_mutation_count=0,
        )

    # 5. Nested Group member scope exits exactly to root Group top-level selection.
    if isinstance(state.selection_scope, GroupMembersSelectionScopeV1):
        next_scope = escape_parent_v1(state.selection_scope)
        next_state = replace(state, selection_scope=next_scope)
        return EditorToolRoutingResultV1(
            protocol_version="chaptera.editor-tool-routing-result.v1",
            state=next_state,
            action="escape_parent_group",
            pointer_owner=pointer_owner_v1(next_state),
            cancelled_gesture_token=None,
            selection_clear_requested=False,
            document_mutation_count=0,
        )

    # 6. Top-level nonempty canvas selection delegates clearing to the dedicated
    # downstream selection controller. Routing owns only precedence.
    if (
        isinstance(state.selection_scope, TopLevelSelectionScopeV1)
        and bool(state.selection_scope.selected)
    ):
        return EditorToolRoutingResultV1(
            protocol_version="chaptera.editor-tool-routing-result.v1",
            state=state,
            action="clear_top_level_selection",
            pointer_owner=pointer_owner_v1(state),
            cancelled_gesture_token=None,
            selection_clear_requested=True,
            document_mutation_count=0,
        )

    # 7. No owner has an Escape transition.
    return EditorToolRoutingResultV1(
        protocol_version="chaptera.editor-tool-routing-result.v1",
        state=state,
        action="no_op",
        pointer_owner=pointer_owner_v1(state),
        cancelled_gesture_token=None,
        selection_clear_requested=False,
        document_mutation_count=0,
    )


def complete_canvas_tool_v1(
    state: EditorToolRoutingStateV1,
    *,
    outcome: ToolCompletionV1,
) -> EditorToolRoutingResultV1:
    """Normalize one-shot and transient modal tools back to Select."""
    _validate_state(state)
    if outcome not in {"accepted", "cancelled", "selection_invalidated"}:
        _fail("invalid_tool_completion", "unsupported tool completion outcome")

    tool = state.canvas.active_tool
    if tool == SELECT_TOOL_V1:
        return EditorToolRoutingResultV1(
            protocol_version="chaptera.editor-tool-routing-result.v1",
            state=state,
            action="no_change",
            pointer_owner=pointer_owner_v1(state),
            cancelled_gesture_token=None,
            selection_clear_requested=False,
            document_mutation_count=0,
        )

    return_to_select = (
        tool in {
            RECTANGLE_CREATE_TOOL_V1,
            PICTURE_CREATE_TOOL_V1,
            PICTURE_CROP_TOOL_V1,
            LINK_TEXTBOX_TOOL_V1,
        }
        or tool == TEXTBOX_CREATE_TOOL_V1
    )
    if not return_to_select:
        _fail("unsupported_tool_completion", "tool completion policy is not defined in V1")

    transition = activate_canvas_tool_v1(state.canvas, tool=SELECT_TOOL_V1)
    focus_owner: FocusOwnerV1 = state.focus_owner
    if tool == TEXTBOX_CREATE_TOOL_V1 and outcome == "accepted":
        # Authoring/TextBox UI owns creation + session opening. This router keeps
        # canvas Select and accepts Story focus only when the caller already
        # supplied the resulting session into state.
        focus_owner = "story_text" if state.active_text_session is not None else "canvas"
    else:
        focus_owner = "canvas"

    next_state = replace(
        state,
        canvas=transition.state,
        focus_owner=focus_owner,
    )
    return EditorToolRoutingResultV1(
        protocol_version="chaptera.editor-tool-routing-result.v1",
        state=next_state,
        action=(
            "textbox_created_to_text_session"
            if tool == TEXTBOX_CREATE_TOOL_V1
            and outcome == "accepted"
            and state.active_text_session is not None
            else "tool_completed_to_select"
        ),
        pointer_owner=pointer_owner_v1(next_state),
        cancelled_gesture_token=transition.cancelled_gesture_token,
        selection_clear_requested=False,
        document_mutation_count=0,
    )
