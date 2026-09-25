#!/usr/bin/env python3
import unittest
from dataclasses import replace

from canvas_tool_state_v1 import (
    RECTANGLE_CREATE_TOOL_V1,
    activate_canvas_tool_v1,
    default_canvas_tool_state_v1,
    start_pointer_gesture_v1,
)
from editor_multi_select_v1 import (
    AuthoredSelectableNodeV1,
    empty_multi_selection_v1,
)
from editor_object_select_all_v1 import (
    EditorObjectSelectAllError,
    route_editor_select_all_v1,
)
from editor_tool_routing_v1 import build_editor_tool_routing_state_v1
from group_member_selection_v1 import empty_top_level_scope_v1
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_selection_state_v1 import build_text_selection_state_v1


def candidate(node_id, *, page="page:1", authored=True, x=0, y=0):
    return AuthoredSelectableNodeV1(
        page_id=page,
        node_id=node_id,
        authored_direct=authored,
        x_emu=x,
        y_emu=y,
        width_emu=10,
        height_emu=10,
    )


def canvas_state():
    return build_editor_tool_routing_state_v1(
        canvas=default_canvas_tool_state_v1(),
        selection_scope=empty_top_level_scope_v1(),
        focus_owner="canvas",
    )


class EditorObjectSelectAllV1Tests(unittest.TestCase):
    def test_canvas_selects_all_current_page_authored_direct_objects(self):
        result=route_editor_select_all_v1(
            routing_state=canvas_state(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            candidates=(
                candidate("b"),
                candidate("a",x=-999999,y=500000),
                candidate("other",page="page:2"),
            ),
        )
        self.assertEqual("objects_selected",result.status)
        self.assertEqual(("a","b"),result.object_selection.selected_node_ids)
        self.assertIsNone(result.object_selection.primary_node_id)
        self.assertEqual(2,result.eligible_count)
        self.assertEqual(0,result.document_mutation_count)
        self.assertFalse(result.revision_created)
        self.assertFalse(result.scroll_changed)
        self.assertFalse(result.zoom_changed)
        self.assertFalse(result.authored_stack_changed)

    def test_single_object_becomes_primary(self):
        result=route_editor_select_all_v1(
            routing_state=canvas_state(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            candidates=(candidate("solo"),),
        )
        self.assertEqual(("solo",),result.object_selection.selected_node_ids)
        self.assertEqual("solo",result.object_selection.primary_node_id)

    def test_non_authored_instances_are_explicitly_excluded(self):
        result=route_editor_select_all_v1(
            routing_state=canvas_state(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            candidates=(
                candidate("authored"),
                candidate("projected-a",authored=False),
                candidate("projected-b",authored=False),
            ),
        )
        self.assertEqual(("authored",),result.object_selection.selected_node_ids)
        self.assertEqual(2,result.excluded_non_authored_count)
        self.assertIn("excluded",result.reason)
        self.assertEqual(
            "current_page_authored_direct_only",result.authored_scope_label
        )

    def test_zero_eligible_objects_clears_existing_selection(self):
        current=empty_multi_selection_v1(page_id="page:1")
        current=replace(
            current,selected_node_ids=("old",),primary_node_id="old"
        )
        result=route_editor_select_all_v1(
            routing_state=canvas_state(),
            current_page_id="page:1",
            current_object_selection=current,
            candidates=(candidate("x",authored=False),),
        )
        self.assertEqual("objects_cleared",result.status)
        self.assertEqual((),result.object_selection.selected_node_ids)

    def test_story_focus_routes_exclusively_to_text_select_all(self):
        text="abc"
        domain=derive_story_edit_domain_v1(
            story_id="story:1",story_text=text,provenance="chaptera_created"
        )
        selection=build_text_selection_state_v1(
            domain=domain,revision_id="rev:1",anchor_scalar=1,focus_scalar=1
        )
        routing=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=empty_top_level_scope_v1(),
            focus_owner="canvas",
        )
        routing=replace(
            routing,focus_owner="story_text",active_text_session=object()
        )
        objects=empty_multi_selection_v1(page_id="page:1")
        result=route_editor_select_all_v1(
            routing_state=routing,
            current_page_id="page:1",
            current_object_selection=objects,
            candidates=(candidate("a"),),
            current_text_selection=selection,
            text_domain=domain,
        )
        self.assertEqual("story_text_selected",result.status)
        self.assertEqual((0,3),result.text_result.selection.normalized_range)
        self.assertEqual(objects,result.object_selection)

    def test_modal_and_temporary_tool_suppress_canvas_select_all(self):
        base=canvas_state()
        modal=replace(base,focus_owner="modal")
        result=route_editor_select_all_v1(
            routing_state=modal,
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            candidates=(candidate("a"),),
        )
        self.assertEqual("suppressed",result.status)

        tool=replace(
            base,
            canvas=activate_canvas_tool_v1(
                base.canvas,tool=RECTANGLE_CREATE_TOOL_V1
            ).state,
        )
        result=route_editor_select_all_v1(
            routing_state=tool,
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            candidates=(candidate("a"),),
        )
        self.assertEqual("suppressed",result.status)

    def test_active_gesture_suppresses_canvas_select_all(self):
        base=canvas_state()
        canvas=activate_canvas_tool_v1(
            base.canvas,tool=RECTANGLE_CREATE_TOOL_V1
        ).state
        canvas=start_pointer_gesture_v1(
            canvas,tool=RECTANGLE_CREATE_TOOL_V1,token="g1"
        ).state
        state=replace(base,canvas=canvas)
        result=route_editor_select_all_v1(
            routing_state=state,
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            candidates=(candidate("a"),),
        )
        self.assertEqual("suppressed",result.status)

    def test_duplicate_authored_nodeids_fail_closed(self):
        with self.assertRaises(EditorObjectSelectAllError):
            route_editor_select_all_v1(
                routing_state=canvas_state(),
                current_page_id="page:1",
                current_object_selection=empty_multi_selection_v1(page_id="page:1"),
                candidates=(candidate("a"),candidate("a",x=99)),
            )


if __name__=="__main__":
    unittest.main()
