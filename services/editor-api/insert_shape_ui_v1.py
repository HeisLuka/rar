#!/usr/bin/env python3
"""Source-neutral Rectangle tool orchestration for Chaptera Editor V1."""

from __future__ import annotations

from dataclasses import dataclass

from canvas_box_draw_v1 import (
    BoxDrawTransactionV1,
    PointEmu,
    commit_box_draw_v1,
    preview_box_draw_v1,
    start_box_draw_v1,
    update_box_draw_v1,
)
from canvas_tool_state_v1 import (
    RECTANGLE_CREATE_TOOL_V1,
    SELECT_TOOL_V1,
    CanvasToolStateV1,
    activate_canvas_tool_v1,
    cancel_pointer_gesture_v1,
    start_pointer_gesture_v1,
    update_pointer_gesture_v1,
)


class InsertShapeUiError(ValueError):
    pass


@dataclass(frozen=True)
class RectangleToolSessionV1:
    tool_state: CanvasToolStateV1
    gesture_token: str | None = None
    draw: BoxDrawTransactionV1 | None = None


@dataclass(frozen=True)
class RectangleToolResultV1:
    session: RectangleToolSessionV1
    action: str
    preview_bounds: dict | None = None
    create_shape_request: dict | None = None
    selected_node_id: str | None = None


def _rect_dict(rect) -> dict:
    return {
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
    }


def activate_rectangle_tool_v1(state: CanvasToolStateV1) -> RectangleToolSessionV1:
    transition = activate_canvas_tool_v1(state, tool=RECTANGLE_CREATE_TOOL_V1)
    return RectangleToolSessionV1(tool_state=transition.state)


def rectangle_pointer_down_v1(
    session: RectangleToolSessionV1,
    *,
    page_id: str,
    point: PointEmu,
    gesture_token: str,
) -> RectangleToolResultV1:
    if session.draw is not None or session.gesture_token is not None:
        raise InsertShapeUiError("rectangle gesture already active")
    transition = start_pointer_gesture_v1(
        session.tool_state,
        tool=RECTANGLE_CREATE_TOOL_V1,
        token=gesture_token,
    )
    draw = start_box_draw_v1(page_id=page_id, anchor=point)
    return RectangleToolResultV1(
        session=RectangleToolSessionV1(
            tool_state=transition.state,
            gesture_token=gesture_token,
            draw=draw,
        ),
        action="gesture_started",
    )


def rectangle_pointer_move_v1(
    session: RectangleToolSessionV1,
    *,
    point: PointEmu,
) -> RectangleToolResultV1:
    if session.draw is None or session.gesture_token is None:
        raise InsertShapeUiError("no active rectangle gesture")
    transition = update_pointer_gesture_v1(
        session.tool_state,
        tool=RECTANGLE_CREATE_TOOL_V1,
        token=session.gesture_token,
    )
    draw = update_box_draw_v1(session.draw, current=point)
    preview = preview_box_draw_v1(draw)
    return RectangleToolResultV1(
        session=RectangleToolSessionV1(
            tool_state=transition.state,
            gesture_token=session.gesture_token,
            draw=draw,
        ),
        action=preview.status,
        preview_bounds=None if preview.bounds is None else _rect_dict(preview.bounds),
    )


def rectangle_pointer_up_v1(
    session: RectangleToolSessionV1,
    *,
    point: PointEmu,
    document_id: str,
    source_hash: str,
    base_revision_id: str,
    client_operation_id: str,
    node_id: str,
    paint: dict,
) -> RectangleToolResultV1:
    moved = rectangle_pointer_move_v1(session, point=point)
    draw_result = commit_box_draw_v1(moved.session.draw)
    token = moved.session.gesture_token
    if token is None:
        raise InsertShapeUiError("missing gesture token")

    ended_state = cancel_pointer_gesture_v1(
        moved.session.tool_state,
        tool=RECTANGLE_CREATE_TOOL_V1,
        token=token,
    ).state

    if draw_result.status != "commit" or draw_result.bounds is None:
        return RectangleToolResultV1(
            session=RectangleToolSessionV1(tool_state=ended_state),
            action="no_change",
        )

    request = {
        "protocol_version": "chaptera.create-shape-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "create_shape",
            "node_id": node_id,
            "page_id": draw_result.page_id,
            "bounds": _rect_dict(draw_result.bounds),
            "paint": paint,
        },
    }
    return RectangleToolResultV1(
        session=RectangleToolSessionV1(tool_state=ended_state),
        action="submit_create_shape",
        create_shape_request=request,
    )


def rectangle_cancel_v1(session: RectangleToolSessionV1) -> RectangleToolResultV1:
    if session.gesture_token is None:
        return RectangleToolResultV1(session=session, action="no_change")
    cancelled = cancel_pointer_gesture_v1(
        session.tool_state,
        tool=RECTANGLE_CREATE_TOOL_V1,
        token=session.gesture_token,
    )
    return RectangleToolResultV1(
        session=RectangleToolSessionV1(tool_state=cancelled.state),
        action="cancelled",
    )


def rectangle_commit_accepted_v1(
    session: RectangleToolSessionV1,
    *,
    accepted_node_id: str,
) -> RectangleToolResultV1:
    transition = activate_canvas_tool_v1(session.tool_state, tool=SELECT_TOOL_V1)
    return RectangleToolResultV1(
        session=RectangleToolSessionV1(tool_state=transition.state),
        action="accepted",
        selected_node_id=accepted_node_id,
    )
