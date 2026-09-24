#!/usr/bin/env python3
import unittest
from dataclasses import replace
from unittest.mock import patch

from canvas_tool_state_v1 import (
    LINK_TEXTBOX_TOOL_V1,
    PICTURE_CROP_TOOL_V1,
    RECTANGLE_CREATE_TOOL_V1,
    SELECT_TOOL_V1,
    default_canvas_tool_state_v1,
    activate_canvas_tool_v1,
    start_pointer_gesture_v1,
)
from editor_tool_routing_v1 import (
    activate_desktop_tool_v1,
    build_editor_tool_routing_state_v1,
    complete_canvas_tool_v1,
    pointer_owner_v1,
    route_escape_v1,
    set_host_focus_owner_v1,
)
from group_member_selection_v1 import (
    GroupMembersSelectionScopeV1,
    TopLevelSelectionScopeV1,
    empty_top_level_scope_v1,
)
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
)


class FakeExit:
    def __init__(self,status):
        self.status=status


def top(selected=True):
    if not selected:
        return empty_top_level_scope_v1()
    node=DirectNodeSelectionV1("page:1","node:1")
    return TopLevelSelectionScopeV1((node,),node)


def grouped():
    root=DirectNodeSelectionV1("page:1","group:1")
    member=GroupMemberSelectionV1("page:1","group:1","node:child")
    return GroupMembersSelectionScopeV1(root,(member,),member)


class EditorToolRoutingV1Tests(unittest.TestCase):
    def test_host_control_suspends_canvas_pointer_and_escape(self):
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=top(),
        )
        state=set_host_focus_owner_v1(
            state,focus_owner="inspector_input"
        ).state
        self.assertEqual("host_control",pointer_owner_v1(state))
        escaped=route_escape_v1(state)
        self.assertEqual("host_consumed",escaped.action)
        self.assertEqual(state,escaped.state)

    def test_active_story_session_precedes_canvas_tool_escape(self):
        state=build_editor_tool_routing_state_v1(
            canvas=activate_canvas_tool_v1(
                default_canvas_tool_state_v1(),tool=RECTANGLE_CREATE_TOOL_V1
            ).state,
            selection_scope=top(),
        )
        fake_session=object()
        state=replace(state,focus_owner="story_text",active_text_session=fake_session)
        with patch("editor_tool_routing_v1.exit_desktop_text_mode_v1",return_value=FakeExit("exited")):
            result=route_escape_v1(state)
        self.assertEqual("text_session_exit",result.action)
        self.assertEqual(RECTANGLE_CREATE_TOOL_V1,result.state.canvas.active_tool)
        self.assertIsNone(result.state.active_text_session)

    def test_active_canvas_gesture_precedes_temporary_tool_deactivation(self):
        canvas=activate_canvas_tool_v1(
            default_canvas_tool_state_v1(),tool=RECTANGLE_CREATE_TOOL_V1
        ).state
        canvas=start_pointer_gesture_v1(
            canvas,tool=RECTANGLE_CREATE_TOOL_V1,token="drag:1"
        ).state
        state=build_editor_tool_routing_state_v1(
            canvas=canvas,selection_scope=top()
        )
        first=route_escape_v1(state)
        self.assertEqual("canvas_gesture_cancelled",first.action)
        self.assertEqual("drag:1",first.cancelled_gesture_token)
        self.assertEqual(RECTANGLE_CREATE_TOOL_V1,first.state.canvas.active_tool)

        second=route_escape_v1(first.state)
        self.assertEqual("temporary_tool_to_select",second.action)
        self.assertEqual(SELECT_TOOL_V1,second.state.canvas.active_tool)

    def test_group_member_escape_parent_precedes_top_level_clear(self):
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=grouped(),
        )
        result=route_escape_v1(state)
        self.assertEqual("escape_parent_group",result.action)
        self.assertIsInstance(result.state.selection_scope,TopLevelSelectionScopeV1)
        self.assertEqual(
            "group:1",
            result.state.selection_scope.primary.node_id,
        )
        self.assertFalse(result.selection_clear_requested)

    def test_top_level_selection_requests_dedicated_clear_after_all_higher_precedence(self):
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=top(),
        )
        result=route_escape_v1(state)
        self.assertEqual("clear_top_level_selection",result.action)
        self.assertTrue(result.selection_clear_requested)
        self.assertEqual(state.selection_scope,result.state.selection_scope)

    def test_empty_select_state_escape_is_noop(self):
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=top(False),
        )
        result=route_escape_v1(state)
        self.assertEqual("no_op",result.action)
        self.assertEqual(0,result.document_mutation_count)

    def test_non_text_tool_activation_resolves_story_session_first(self):
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=top(False),
        )
        state=replace(state,focus_owner="story_text",active_text_session=object())
        with patch("editor_tool_routing_v1.exit_desktop_text_mode_v1",return_value=FakeExit("exited")):
            result=activate_desktop_tool_v1(
                state,tool=RECTANGLE_CREATE_TOOL_V1
            )
        self.assertEqual(
            "text_session_exited_then_tool_changed",result.action
        )
        self.assertEqual(RECTANGLE_CREATE_TOOL_V1,result.state.canvas.active_tool)
        self.assertIsNone(result.state.active_text_session)
        self.assertEqual("canvas",result.state.focus_owner)

    def test_composition_resolution_blocks_tool_activation_without_state_change(self):
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=top(False),
        )
        state=replace(state,focus_owner="story_text",active_text_session=object())
        with patch(
            "editor_tool_routing_v1.exit_desktop_text_mode_v1",
            return_value=FakeExit("composition_resolution_required"),
        ):
            result=activate_desktop_tool_v1(
                state,tool=RECTANGLE_CREATE_TOOL_V1
            )
        self.assertEqual("composition_resolution_required",result.action)
        self.assertEqual(state,result.state)

    def test_one_shot_and_modal_tools_return_to_select(self):
        for tool in (RECTANGLE_CREATE_TOOL_V1,PICTURE_CROP_TOOL_V1,LINK_TEXTBOX_TOOL_V1):
            state=build_editor_tool_routing_state_v1(
                canvas=activate_canvas_tool_v1(
                    default_canvas_tool_state_v1(),tool=tool
                ).state,
                selection_scope=top(False),
            )
            result=complete_canvas_tool_v1(state,outcome="accepted")
            self.assertEqual(SELECT_TOOL_V1,result.state.canvas.active_tool)
            self.assertEqual("tool_completed_to_select",result.action)
            self.assertEqual(0,result.document_mutation_count)

    def test_routing_never_fans_pointer_to_canvas_while_host_owns_focus(self):
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=top(False),
            focus_owner="modal",
        )
        self.assertEqual("host_control",pointer_owner_v1(state))


if __name__=="__main__":
    unittest.main()
