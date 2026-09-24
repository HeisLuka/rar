#!/usr/bin/env python3
import unittest

from canvas_tool_state_v1 import (
    BASE_CANVAS_TOOLS_V1,
    LINK_TEXTBOX_TOOL_V1,
    PICTURE_CREATE_TOOL_V1,
    RECTANGLE_CREATE_TOOL_V1,
    SELECT_TOOL_V1,
    CanvasToolIdV1,
    CanvasToolStateError,
    activate_canvas_tool_v1,
    cancel_pointer_gesture_v1,
    default_canvas_tool_state_v1,
    end_pointer_gesture_v1,
    escape_canvas_tool_v1,
    start_pointer_gesture_v1,
    update_pointer_gesture_v1,
)


class CanvasToolStateV1Tests(unittest.TestCase):
    def test_base_registry_and_extension_safe_identity(self):
        self.assertEqual(6, len(BASE_CANVAS_TOOLS_V1))
        self.assertIn(SELECT_TOOL_V1, BASE_CANVAS_TOOLS_V1)
        extension = CanvasToolIdV1("plugin_vendor", "star_polygon")
        self.assertNotIn(extension, BASE_CANVAS_TOOLS_V1)
        with self.assertRaises(CanvasToolStateError):
            CanvasToolIdV1("Widget#12", "button")

    def test_reopen_default_is_select_without_gesture(self):
        state = default_canvas_tool_state_v1()
        self.assertEqual(SELECT_TOOL_V1, state.active_tool)
        self.assertIsNone(state.active_gesture)

    def test_start_update_end_require_exact_tool_and_token(self):
        state = activate_canvas_tool_v1(
            default_canvas_tool_state_v1(),
            tool=RECTANGLE_CREATE_TOOL_V1,
        ).state
        started = start_pointer_gesture_v1(
            state,
            tool=RECTANGLE_CREATE_TOOL_V1,
            token="gesture-1",
        ).state

        updated = update_pointer_gesture_v1(
            started,
            tool=RECTANGLE_CREATE_TOOL_V1,
            token="gesture-1",
        )
        self.assertEqual(started, updated.state)

        with self.assertRaisesRegex(CanvasToolStateError, "token mismatch"):
            update_pointer_gesture_v1(
                started,
                tool=RECTANGLE_CREATE_TOOL_V1,
                token="other",
            )
        with self.assertRaisesRegex(CanvasToolStateError, "active canvas tool"):
            update_pointer_gesture_v1(
                started,
                tool=PICTURE_CREATE_TOOL_V1,
                token="gesture-1",
            )

        ended = end_pointer_gesture_v1(
            started,
            tool=RECTANGLE_CREATE_TOOL_V1,
            token="gesture-1",
        )
        self.assertIsNone(ended.state.active_gesture)
        self.assertFalse(ended.commit_requested)

    def test_second_gesture_is_rejected(self):
        state = activate_canvas_tool_v1(
            default_canvas_tool_state_v1(),
            tool=RECTANGLE_CREATE_TOOL_V1,
        ).state
        state = start_pointer_gesture_v1(
            state,
            tool=RECTANGLE_CREATE_TOOL_V1,
            token="g1",
        ).state
        with self.assertRaisesRegex(CanvasToolStateError, "already active"):
            start_pointer_gesture_v1(
                state,
                tool=RECTANGLE_CREATE_TOOL_V1,
                token="g2",
            )

    def test_switch_tool_cancels_gesture_without_commit(self):
        state = activate_canvas_tool_v1(
            default_canvas_tool_state_v1(),
            tool=RECTANGLE_CREATE_TOOL_V1,
        ).state
        state = start_pointer_gesture_v1(
            state,
            tool=RECTANGLE_CREATE_TOOL_V1,
            token="draw-7",
        ).state
        transition = activate_canvas_tool_v1(state, tool=PICTURE_CREATE_TOOL_V1)
        self.assertEqual("gesture_cancelled_then_tool_changed", transition.action)
        self.assertEqual("draw-7", transition.cancelled_gesture_token)
        self.assertEqual(PICTURE_CREATE_TOOL_V1, transition.state.active_tool)
        self.assertIsNone(transition.state.active_gesture)
        self.assertFalse(transition.commit_requested)

    def test_escape_cancels_gesture_then_deactivates_to_select(self):
        state = activate_canvas_tool_v1(
            default_canvas_tool_state_v1(),
            tool=LINK_TEXTBOX_TOOL_V1,
        ).state
        state = start_pointer_gesture_v1(
            state,
            tool=LINK_TEXTBOX_TOOL_V1,
            token="link-1",
        ).state

        first = escape_canvas_tool_v1(state)
        self.assertEqual("gesture_cancelled", first.action)
        self.assertEqual(LINK_TEXTBOX_TOOL_V1, first.state.active_tool)
        self.assertIsNone(first.state.active_gesture)

        second = escape_canvas_tool_v1(first.state)
        self.assertEqual("deactivated_to_select", second.action)
        self.assertEqual(SELECT_TOOL_V1, second.state.active_tool)

        third = escape_canvas_tool_v1(second.state)
        self.assertEqual("no_change", third.action)

    def test_explicit_cancel_requires_owner_and_never_commits(self):
        state = activate_canvas_tool_v1(
            default_canvas_tool_state_v1(),
            tool=RECTANGLE_CREATE_TOOL_V1,
        ).state
        state = start_pointer_gesture_v1(
            state,
            tool=RECTANGLE_CREATE_TOOL_V1,
            token="g",
        ).state
        result = cancel_pointer_gesture_v1(
            state,
            tool=RECTANGLE_CREATE_TOOL_V1,
            token="g",
        )
        self.assertEqual("gesture_cancelled", result.action)
        self.assertFalse(result.commit_requested)
        self.assertIsNone(result.state.active_gesture)


if __name__ == "__main__":
    unittest.main()
