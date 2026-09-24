#!/usr/bin/env python3
"""Transient canvas tool/gesture ownership state machine V1."""

from __future__ import annotations

from dataclasses import dataclass
import re


class CanvasToolStateError(ValueError):
    pass


_TOOL_PART = re.compile(r"^[a-z][a-z0-9._-]*$")


@dataclass(frozen=True, order=True)
class CanvasToolIdV1:
    namespace: str
    name: str

    def __post_init__(self) -> None:
        if not isinstance(self.namespace, str) or not _TOOL_PART.fullmatch(self.namespace):
            raise CanvasToolStateError("tool namespace must be a typed lowercase identifier")
        if not isinstance(self.name, str) or not _TOOL_PART.fullmatch(self.name):
            raise CanvasToolStateError("tool name must be a typed lowercase identifier")


SELECT_TOOL_V1 = CanvasToolIdV1("chaptera", "select")
RECTANGLE_CREATE_TOOL_V1 = CanvasToolIdV1("chaptera", "rectangle_create")
TEXTBOX_CREATE_TOOL_V1 = CanvasToolIdV1("chaptera", "textbox_create")
PICTURE_CREATE_TOOL_V1 = CanvasToolIdV1("chaptera", "picture_create")
PICTURE_CROP_TOOL_V1 = CanvasToolIdV1("chaptera", "picture_crop")
LINK_TEXTBOX_TOOL_V1 = CanvasToolIdV1("chaptera", "link_textbox")

BASE_CANVAS_TOOLS_V1 = frozenset(
    {
        SELECT_TOOL_V1,
        RECTANGLE_CREATE_TOOL_V1,
        TEXTBOX_CREATE_TOOL_V1,
        PICTURE_CREATE_TOOL_V1,
        PICTURE_CROP_TOOL_V1,
        LINK_TEXTBOX_TOOL_V1,
    }
)


@dataclass(frozen=True)
class PointerGestureOwnershipV1:
    tool: CanvasToolIdV1
    token: str

    def __post_init__(self) -> None:
        if not isinstance(self.tool, CanvasToolIdV1):
            raise CanvasToolStateError("gesture tool must be CanvasToolIdV1")
        if not isinstance(self.token, str) or not self.token:
            raise CanvasToolStateError("gesture token is required")


@dataclass(frozen=True)
class CanvasToolStateV1:
    active_tool: CanvasToolIdV1 = SELECT_TOOL_V1
    active_gesture: PointerGestureOwnershipV1 | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.active_tool, CanvasToolIdV1):
            raise CanvasToolStateError("active_tool must be CanvasToolIdV1")
        if self.active_gesture is not None:
            if not isinstance(self.active_gesture, PointerGestureOwnershipV1):
                raise CanvasToolStateError(
                    "active_gesture must be PointerGestureOwnershipV1"
                )
            if self.active_gesture.tool != self.active_tool:
                raise CanvasToolStateError(
                    "active gesture must be owned by the active tool"
                )


@dataclass(frozen=True)
class CanvasToolTransitionV1:
    state: CanvasToolStateV1
    action: str
    cancelled_gesture_token: str | None = None
    commit_requested: bool = False


def default_canvas_tool_state_v1() -> CanvasToolStateV1:
    return CanvasToolStateV1()


def activate_canvas_tool_v1(
    state: CanvasToolStateV1,
    *,
    tool: CanvasToolIdV1,
) -> CanvasToolTransitionV1:
    _validate_state(state)
    if not isinstance(tool, CanvasToolIdV1):
        raise CanvasToolStateError("tool must be CanvasToolIdV1")

    cancelled = state.active_gesture.token if state.active_gesture is not None else None
    if state.active_tool == tool and cancelled is None:
        return CanvasToolTransitionV1(state=state, action="no_change")

    return CanvasToolTransitionV1(
        state=CanvasToolStateV1(active_tool=tool, active_gesture=None),
        action=(
            "gesture_cancelled_then_tool_changed"
            if cancelled is not None and tool != state.active_tool
            else "gesture_cancelled"
            if cancelled is not None
            else "tool_changed"
        ),
        cancelled_gesture_token=cancelled,
        commit_requested=False,
    )


def start_pointer_gesture_v1(
    state: CanvasToolStateV1,
    *,
    tool: CanvasToolIdV1,
    token: str,
) -> CanvasToolTransitionV1:
    _validate_state(state)
    _require_owner(state, tool=tool, token=None, require_gesture=False)
    if state.active_gesture is not None:
        raise CanvasToolStateError("a pointer gesture is already active")
    gesture = PointerGestureOwnershipV1(tool=tool, token=token)
    return CanvasToolTransitionV1(
        state=CanvasToolStateV1(active_tool=state.active_tool, active_gesture=gesture),
        action="gesture_started",
    )


def update_pointer_gesture_v1(
    state: CanvasToolStateV1,
    *,
    tool: CanvasToolIdV1,
    token: str,
) -> CanvasToolTransitionV1:
    _validate_state(state)
    _require_owner(state, tool=tool, token=token, require_gesture=True)
    return CanvasToolTransitionV1(state=state, action="gesture_updated")


def end_pointer_gesture_v1(
    state: CanvasToolStateV1,
    *,
    tool: CanvasToolIdV1,
    token: str,
) -> CanvasToolTransitionV1:
    _validate_state(state)
    _require_owner(state, tool=tool, token=token, require_gesture=True)
    return CanvasToolTransitionV1(
        state=CanvasToolStateV1(active_tool=state.active_tool, active_gesture=None),
        action="gesture_ended",
        commit_requested=False,
    )


def cancel_pointer_gesture_v1(
    state: CanvasToolStateV1,
    *,
    tool: CanvasToolIdV1,
    token: str,
) -> CanvasToolTransitionV1:
    _validate_state(state)
    _require_owner(state, tool=tool, token=token, require_gesture=True)
    return CanvasToolTransitionV1(
        state=CanvasToolStateV1(active_tool=state.active_tool, active_gesture=None),
        action="gesture_cancelled",
        cancelled_gesture_token=token,
        commit_requested=False,
    )


def escape_canvas_tool_v1(state: CanvasToolStateV1) -> CanvasToolTransitionV1:
    _validate_state(state)
    if state.active_gesture is not None:
        return CanvasToolTransitionV1(
            state=CanvasToolStateV1(active_tool=state.active_tool, active_gesture=None),
            action="gesture_cancelled",
            cancelled_gesture_token=state.active_gesture.token,
            commit_requested=False,
        )
    if state.active_tool != SELECT_TOOL_V1:
        return CanvasToolTransitionV1(
            state=CanvasToolStateV1(active_tool=SELECT_TOOL_V1, active_gesture=None),
            action="deactivated_to_select",
            commit_requested=False,
        )
    return CanvasToolTransitionV1(state=state, action="no_change")


def _validate_state(state: CanvasToolStateV1) -> None:
    if not isinstance(state, CanvasToolStateV1):
        raise CanvasToolStateError("state must be CanvasToolStateV1")


def _require_owner(
    state: CanvasToolStateV1,
    *,
    tool: CanvasToolIdV1,
    token: str | None,
    require_gesture: bool,
) -> None:
    if not isinstance(tool, CanvasToolIdV1):
        raise CanvasToolStateError("tool must be CanvasToolIdV1")
    if tool != state.active_tool:
        raise CanvasToolStateError("pointer event tool does not own the active canvas tool")
    if not require_gesture:
        return
    gesture = state.active_gesture
    if gesture is None:
        raise CanvasToolStateError("no active pointer gesture")
    if gesture.tool != tool:
        raise CanvasToolStateError("pointer event tool does not own the active gesture")
    if not isinstance(token, str) or not token or gesture.token != token:
        raise CanvasToolStateError("pointer event gesture token mismatch")
