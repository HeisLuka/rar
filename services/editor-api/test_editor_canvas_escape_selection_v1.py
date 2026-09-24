#!/usr/bin/env python3
import unittest

from canvas_tool_state_v1 import default_canvas_tool_state_v1
from editor_canvas_escape_selection_v1 import (
    CanvasEscapeSelectionError,
    apply_canvas_escape_selection_v1,
)
from editor_tool_routing_v1 import (
    build_editor_tool_routing_state_v1,
    route_escape_v1,
)
from group_member_selection_v1 import (
    GroupMembersSelectionScopeV1,
    TopLevelSelectionScopeV1,
)
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ProjectedInstanceSelectionV1,
)


def top(*targets):
    return TopLevelSelectionScopeV1(
        selected=tuple(targets),
        primary=targets[0] if len(targets)==1 else None,
    )


class CanvasEscapeSelectionV1Tests(unittest.TestCase):
    def test_single_and_multi_top_level_selection_clear_without_mutation(self):
        for scope in (
            top(DirectNodeSelectionV1("page:1","a")),
            top(
                DirectNodeSelectionV1("page:1","a"),
                DirectNodeSelectionV1("page:1","b"),
            ),
        ):
            state=build_editor_tool_routing_state_v1(
                canvas=default_canvas_tool_state_v1(),
                selection_scope=scope,
            )
            routing=route_escape_v1(state)
            result=apply_canvas_escape_selection_v1(
                routing=routing,selection=scope
            )
            self.assertEqual("cleared",result.status)
            self.assertEqual((),result.selection.selected)
            self.assertEqual(0,result.document_mutation_count)
            self.assertFalse(result.revision_created)
            self.assertTrue(result.focus_owner_unchanged)
            self.assertTrue(result.tool_state_unchanged)
            self.assertTrue(result.viewport_unchanged)

    def test_future_instance_aware_top_level_target_clears_without_origin_collapse(self):
        a=ProjectedInstanceSelectionV1(
            "page:1","inst:1","origin:1","projection","inspect_only"
        )
        b=ProjectedInstanceSelectionV1(
            "page:1","inst:2","origin:1","projection","inspect_only"
        )
        scope=top(a,b)
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=scope,
        )
        result=apply_canvas_escape_selection_v1(
            routing=route_escape_v1(state),
            selection=scope,
        )
        self.assertEqual("cleared",result.status)
        self.assertEqual((),result.selection.selected)

    def test_group_member_escape_is_consumed_before_top_level_clear(self):
        root=DirectNodeSelectionV1("page:1","group:1")
        member=GroupMemberSelectionV1("page:1","group:1","child:1")
        nested=GroupMembersSelectionScopeV1(root,(member,),member)
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=nested,
        )
        routing=route_escape_v1(state)
        self.assertEqual("escape_parent_group",routing.action)
        result=apply_canvas_escape_selection_v1(
            routing=routing,
            selection=TopLevelSelectionScopeV1((root,),root),
        )
        self.assertEqual("not_applicable",result.status)
        self.assertEqual((root,),result.selection.selected)

    def test_router_clear_action_requires_nonempty_matching_selection(self):
        scope=top(DirectNodeSelectionV1("page:1","a"))
        state=build_editor_tool_routing_state_v1(
            canvas=default_canvas_tool_state_v1(),
            selection_scope=scope,
        )
        routing=route_escape_v1(state)
        with self.assertRaises(CanvasEscapeSelectionError):
            apply_canvas_escape_selection_v1(
                routing=routing,
                selection=TopLevelSelectionScopeV1((),None),
            )


if __name__=="__main__":
    unittest.main()
