#!/usr/bin/env python3
"""Desktop Text Box tool orchestration over canonical Chaptera V1 seams.

This module owns only the create -> accepted durable commit -> text-session
handoff composition. BoxDraw owns transient pointer geometry, RevisionKernel
owns document mutation, TextEditSessionV1 owns transient text focus, and the
shared desktop router owns tool/focus precedence.
"""

from __future__ import annotations

from dataclasses import dataclass, replace

from canvas_box_draw_v1 import (
    BoxDrawTransactionV1,
    PointEmu,
    commit_box_draw_v1,
    preview_box_draw_v1,
    start_box_draw_v1,
    update_box_draw_v1,
)
from canvas_tool_state_v1 import (
    TEXTBOX_CREATE_TOOL_V1,
    end_pointer_gesture_v1,
    start_pointer_gesture_v1,
    update_pointer_gesture_v1,
)
from editor_tool_routing_v1 import (
    EditorToolRoutingStateV1,
    activate_desktop_tool_v1,
    complete_canvas_tool_v1,
    pointer_owner_v1,
)
from resolved_text_caret_map_v1 import build_resolved_text_caret_map_v1
from story_edit_domain_v1 import derive_story_edit_domain_v1
from story_edit_transaction_v1 import story_edit_core_state_from_dict
from text_edit_session_v1 import (
    TextEditSessionV1,
    TextEntryCandidateV1,
    TextInitialPositionV1,
    enter_text_edit_session_v1,
)


class InsertTextBoxUiError(ValueError):
    pass


@dataclass(frozen=True)
class TextBoxToolSessionV1:
    routing_state: EditorToolRoutingStateV1
    gesture_token: str | None = None
    draw: BoxDrawTransactionV1 | None = None


@dataclass(frozen=True)
class TextBoxToolResultV1:
    session: TextBoxToolSessionV1
    action: str
    preview_bounds: dict | None = None
    create_textbox_request: dict | None = None
    text_session: TextEditSessionV1 | None = None


def _rect_dict(rect) -> dict:
    return {
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
    }


def _require_textbox_pointer_owner(session: TextBoxToolSessionV1) -> None:
    state = session.routing_state
    if state.canvas.active_tool != TEXTBOX_CREATE_TOOL_V1:
        raise InsertTextBoxUiError("TextBoxCreate is not the active canvas tool")
    if pointer_owner_v1(state) not in {"canvas_tool", "canvas_gesture"}:
        raise InsertTextBoxUiError("TextBoxCreate does not own canvas pointer input")


def activate_textbox_tool_v1(
    state: EditorToolRoutingStateV1,
    *,
    composition_resolution: str | None = None,
    submitted_durable_operation_ids: tuple[str, ...] = (),
) -> TextBoxToolResultV1:
    transition = activate_desktop_tool_v1(
        state,
        tool=TEXTBOX_CREATE_TOOL_V1,
        composition_resolution=composition_resolution,
        submitted_durable_operation_ids=submitted_durable_operation_ids,
    )
    return TextBoxToolResultV1(
        session=TextBoxToolSessionV1(routing_state=transition.state),
        action=transition.action,
    )


def textbox_pointer_down_v1(
    session: TextBoxToolSessionV1,
    *,
    page_id: str,
    point: PointEmu,
    gesture_token: str,
) -> TextBoxToolResultV1:
    _require_textbox_pointer_owner(session)
    if session.draw is not None or session.gesture_token is not None:
        raise InsertTextBoxUiError("TextBox gesture already active")

    transition = start_pointer_gesture_v1(
        session.routing_state.canvas,
        tool=TEXTBOX_CREATE_TOOL_V1,
        token=gesture_token,
    )
    draw = start_box_draw_v1(page_id=page_id, anchor=point)
    routing = replace(session.routing_state, canvas=transition.state)
    return TextBoxToolResultV1(
        session=TextBoxToolSessionV1(
            routing_state=routing,
            gesture_token=gesture_token,
            draw=draw,
        ),
        action="gesture_started",
    )


def textbox_pointer_move_v1(
    session: TextBoxToolSessionV1,
    *,
    point: PointEmu,
) -> TextBoxToolResultV1:
    _require_textbox_pointer_owner(session)
    if session.draw is None or session.gesture_token is None:
        raise InsertTextBoxUiError("no active TextBox gesture")

    transition = update_pointer_gesture_v1(
        session.routing_state.canvas,
        tool=TEXTBOX_CREATE_TOOL_V1,
        token=session.gesture_token,
    )
    draw = update_box_draw_v1(session.draw, current=point)
    preview = preview_box_draw_v1(draw)
    routing = replace(session.routing_state, canvas=transition.state)
    return TextBoxToolResultV1(
        session=TextBoxToolSessionV1(
            routing_state=routing,
            gesture_token=session.gesture_token,
            draw=draw,
        ),
        action=preview.status,
        preview_bounds=None if preview.bounds is None else _rect_dict(preview.bounds),
    )


def _finish_gesture(
    session: TextBoxToolSessionV1,
) -> EditorToolRoutingStateV1:
    if session.gesture_token is None:
        raise InsertTextBoxUiError("missing TextBox gesture token")
    ended = end_pointer_gesture_v1(
        session.routing_state.canvas,
        tool=TEXTBOX_CREATE_TOOL_V1,
        token=session.gesture_token,
    )
    return replace(session.routing_state, canvas=ended.state)


def textbox_pointer_up_v1(
    session: TextBoxToolSessionV1,
    *,
    point: PointEmu,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
    node_id: str,
    story_id: str,
    text_preset: dict,
) -> TextBoxToolResultV1:
    moved = textbox_pointer_move_v1(session, point=point)
    if moved.session.draw is None:
        raise InsertTextBoxUiError("missing TextBox BoxDraw transaction")
    draw_result = commit_box_draw_v1(moved.session.draw)
    routing = _finish_gesture(moved.session)

    if draw_result.status != "commit" or draw_result.bounds is None:
        completed = complete_canvas_tool_v1(routing, outcome="cancelled")
        return TextBoxToolResultV1(
            session=TextBoxToolSessionV1(routing_state=completed.state),
            action="no_change",
        )

    request = {
        "protocol_version": "chaptera.create-textbox-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "create_textbox",
            "node_id": node_id,
            "story_id": story_id,
            "page_id": draw_result.page_id,
            "bounds": _rect_dict(draw_result.bounds),
            "text_preset": text_preset,
            "initial_text": None,
        },
    }
    return TextBoxToolResultV1(
        session=TextBoxToolSessionV1(routing_state=routing),
        action="submit_create_textbox",
        create_textbox_request=request,
    )


def textbox_cancel_v1(session: TextBoxToolSessionV1) -> TextBoxToolResultV1:
    _require_textbox_pointer_owner(session)
    routing = session.routing_state
    if session.gesture_token is not None:
        ended = end_pointer_gesture_v1(
            routing.canvas,
            tool=TEXTBOX_CREATE_TOOL_V1,
            token=session.gesture_token,
        )
        routing = replace(routing, canvas=ended.state)
    completed = complete_canvas_tool_v1(routing, outcome="cancelled")
    return TextBoxToolResultV1(
        session=TextBoxToolSessionV1(routing_state=completed.state),
        action="cancelled",
    )


def textbox_commit_accepted_v1(
    session: TextBoxToolSessionV1,
    *,
    create_textbox_request: dict,
    accepted_commit: dict,
    current_project: dict,
    text_session_id: str,
    layout_revision_id: str,
) -> TextBoxToolResultV1:
    """Open the newly created empty Story only after its durable commit is accepted."""
    state = session.routing_state
    if state.canvas.active_tool != TEXTBOX_CREATE_TOOL_V1:
        raise InsertTextBoxUiError("accepted TextBox handoff requires active TextBoxCreate tool")

    if create_textbox_request.get("protocol_version") != "chaptera.create-textbox-intent.v1":
        raise InsertTextBoxUiError("CreateTextBox request protocol mismatch")
    command = create_textbox_request.get("command")
    if not isinstance(command, dict) or command.get("initial_text") is not None:
        raise InsertTextBoxUiError("base TextBox UI must create an empty canonical Story")

    if accepted_commit.get("protocol_version") != "chaptera.commit-accepted.v1":
        raise InsertTextBoxUiError("accepted durable commit receipt is required")
    revision_id = accepted_commit.get("revision_id")
    operation = accepted_commit.get("canonical_operation")
    if not isinstance(revision_id, str) or not revision_id:
        raise InsertTextBoxUiError("accepted TextBox commit revision_id is required")
    if not isinstance(operation, dict) or operation.get("kind") != "create_textbox":
        raise InsertTextBoxUiError("accepted commit is not CreateTextBox")
    for key in ("node_id", "story_id", "page_id"):
        if operation.get(key) != command.get(key):
            raise InsertTextBoxUiError(f"accepted CreateTextBox {key} differs from submitted request")
    if operation.get("story_text") != "":
        raise InsertTextBoxUiError("base TextBox UI accepted Story must be empty before typing")

    node_id = command["node_id"]
    story_id = command["story_id"]
    frames = current_project.get("text_frames")
    stories = current_project.get("stories")
    models = current_project.get("story_models")
    frame = frames.get(node_id) if isinstance(frames, dict) else None
    if (
        not isinstance(frame, dict)
        or frame.get("story_id") != story_id
        or frame.get("provenance") != {"kind": "author_created"}
    ):
        raise InsertTextBoxUiError("accepted TextBox frame is missing from current project")
    if not isinstance(stories, dict) or stories.get(story_id) != "":
        raise InsertTextBoxUiError("accepted empty Story mirror is missing from current project")
    raw_model = models.get(story_id) if isinstance(models, dict) else None
    if not isinstance(raw_model, dict):
        raise InsertTextBoxUiError("accepted Story model is missing from current project")

    core = story_edit_core_state_from_dict(raw_model)
    if core.provenance != "chaptera_created" or core.paragraph_state.story_text != "":
        raise InsertTextBoxUiError("accepted Story model is not canonical empty author-created state")

    domain = derive_story_edit_domain_v1(
        story_id=story_id,
        story_text="",
        provenance="chaptera_created",
    )
    caret_map = build_resolved_text_caret_map_v1(
        layout_revision_id=layout_revision_id,
        story_id=story_id,
        story_scalar_len=0,
        lines=(),
    )
    entered = enter_text_edit_session_v1(
        session_id=text_session_id,
        incarnation=0,
        document_id=create_textbox_request["document_id"],
        entry_candidates=(
            TextEntryCandidateV1(
                target_id=node_id,
                story_id=story_id,
                frame_id=node_id,
                capability="editable",
            ),
        ),
        revision_id=revision_id,
        domain=domain,
        caret_map=caret_map,
        format_state=core.format_state,
        expected_layout_revision_id=layout_revision_id,
        initial_position=TextInitialPositionV1(0),
        pending_interaction_metadata=(
            ("create_operation_id", create_textbox_request["client_operation_id"]),
        ),
    )

    with_session = replace(
        state,
        active_text_session=entered.session,
        focus_owner="canvas",
    )
    completed = complete_canvas_tool_v1(with_session, outcome="accepted")
    if completed.action != "textbox_created_to_text_session":
        raise InsertTextBoxUiError("TextBox accepted handoff did not enter Story text focus")
    return TextBoxToolResultV1(
        session=TextBoxToolSessionV1(routing_state=completed.state),
        action=completed.action,
        text_session=entered.session,
    )
