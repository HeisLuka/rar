#!/usr/bin/env python3
import unittest
from dataclasses import replace

from canvas_tool_state_v1 import (
    RECTANGLE_CREATE_TOOL_V1,
    activate_canvas_tool_v1,
    default_canvas_tool_state_v1,
)
from editor_multi_select_v1 import AuthoredSelectableNodeV1, empty_multi_selection_v1
from editor_nested_select_all_v1 import route_editor_nested_select_all_v1
from editor_tool_routing_v1 import build_editor_tool_routing_state_v1
from group_member_selection_v1 import empty_top_level_scope_v1
from nested_group_selection_v1 import (
    NestedGroupPathEdgeV1,
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionScopeV1,
    NestedGroupSelectionTargetV1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_selection_state_v1 import build_text_selection_state_v1


def routing(focus="canvas"):
    return build_editor_tool_routing_state_v1(
        canvas=default_canvas_tool_state_v1(),
        selection_scope=empty_top_level_scope_v1(),
        focus_owner=focus,
    )


def page_node(node_id,page="page:1"):
    return AuthoredSelectableNodeV1(page,node_id,True,0,0,10,10)


def nested_scope(path=("group:root",),selected=()):
    targets=tuple(
        NestedGroupSelectionTargetV1("page:1",path,node_id)
        for node_id in selected
    )
    primary=targets[0] if len(targets)==1 else None
    return NestedGroupSelectionScopeV1("page:1",path,targets,primary)


def snapshot(children,path=("group:root",)):
    edges=[]
    for i,group_id in enumerate(path):
        parent=None if i==0 else path[i-1]
        edge_children=(path[i+1],) if i+1<len(path) else tuple(children)
        edges.append(NestedGroupPathEdgeV1(group_id,parent,edge_children))
    return NestedGroupPathSnapshotV1("page:1",tuple(edges))


class EditorNestedSelectAllV1Tests(unittest.TestCase):
    def test_nested_select_all_replaces_with_direct_children_only(self):
        scope=nested_scope(selected=("old",))
        snap=snapshot(("old","child:b","child:a","child:group"))
        result=route_editor_nested_select_all_v1(
            routing_state=routing(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(page_node("page-sibling"),),
            nested_scope=scope,
            nested_snapshot=snap,
            eligible_direct_child_ids=("child:b","child:a","child:group"),
        )
        self.assertEqual("nested_selected",result.status)
        self.assertEqual(
            ("child:a","child:b","child:group"),
            tuple(t.node_id for t in result.nested_scope.selected),
        )
        self.assertIsNone(result.nested_scope.primary)
        self.assertEqual(("group:root",),result.nested_scope.container_path)

    def test_nested_child_group_is_selected_as_direct_child_not_recursed(self):
        path=("group:root","group:child")
        scope=nested_scope(path=path)
        snap=snapshot(("leaf:1","leaf:2"),path=path)
        result=route_editor_nested_select_all_v1(
            routing_state=routing(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(),
            nested_scope=scope,
            nested_snapshot=snap,
            eligible_direct_child_ids=("leaf:1","leaf:2"),
        )
        self.assertEqual(path,result.nested_scope.container_path)
        self.assertEqual(
            ("leaf:1","leaf:2"),
            tuple(t.node_id for t in result.nested_scope.selected),
        )

    def test_single_direct_child_becomes_primary_and_zero_clears(self):
        scope=nested_scope(selected=("old",))
        snap=snapshot(("old","only"))
        one=route_editor_nested_select_all_v1(
            routing_state=routing(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(),
            nested_scope=scope,
            nested_snapshot=snap,
            eligible_direct_child_ids=("only",),
        )
        self.assertEqual("only",one.nested_scope.primary.node_id)

        zero=route_editor_nested_select_all_v1(
            routing_state=routing(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(),
            nested_scope=one.nested_scope,
            nested_snapshot=snap,
            eligible_direct_child_ids=(),
        )
        self.assertEqual("nested_cleared",zero.status)
        self.assertEqual((),zero.nested_scope.selected)

    def test_broken_path_fails_closed_without_page_fallback(self):
        scope=nested_scope(selected=("old",))
        stale=NestedGroupPathSnapshotV1(
            "page:1",
            (NestedGroupPathEdgeV1("different:group",None,("old",)),),
        )
        result=route_editor_nested_select_all_v1(
            routing_state=routing(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(page_node("page:a"),page_node("page:b")),
            nested_scope=scope,
            nested_snapshot=stale,
            eligible_direct_child_ids=("old",),
        )
        self.assertEqual("nested_scope_invalid",result.status)
        self.assertIsNone(result.delegated_result)
        self.assertEqual(scope,result.nested_scope)

    def test_story_focus_delegates_to_text_select_all_and_keeps_nested_scope(self):
        text="abcd"
        domain=derive_story_edit_domain_v1(
            story_id="story:1",story_text=text,provenance="chaptera_created"
        )
        selection=build_text_selection_state_v1(
            domain=domain,revision_id="rev:1",anchor_scalar=2,focus_scalar=2
        )
        state=replace(
            routing(),
            focus_owner="story_text",
            active_text_session=object(),
        )
        scope=nested_scope()
        result=route_editor_nested_select_all_v1(
            routing_state=state,
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(page_node("page:a"),),
            nested_scope=scope,
            nested_snapshot=snapshot(("child:a",)),
            eligible_direct_child_ids=("child:a",),
            current_text_selection=selection,
            text_domain=domain,
        )
        self.assertEqual("delegated",result.status)
        self.assertEqual("story_text_selected",result.delegated_result.status)
        self.assertEqual((0,4),result.delegated_result.text_result.selection.normalized_range)
        self.assertEqual(scope,result.nested_scope)

    def test_top_level_without_nested_scope_delegates_page_select_all(self):
        result=route_editor_nested_select_all_v1(
            routing_state=routing(),
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(page_node("b"),page_node("a")),
        )
        self.assertEqual("delegated",result.status)
        self.assertEqual("objects_selected",result.delegated_result.status)
        self.assertEqual(("a","b"),result.delegated_result.object_selection.selected_node_ids)

    def test_temporary_tool_suppresses_nested_scope_without_page_fallback(self):
        base=routing()
        state=replace(
            base,
            canvas=activate_canvas_tool_v1(
                base.canvas,tool=RECTANGLE_CREATE_TOOL_V1
            ).state,
        )
        scope=nested_scope()
        result=route_editor_nested_select_all_v1(
            routing_state=state,
            current_page_id="page:1",
            current_object_selection=empty_multi_selection_v1(page_id="page:1"),
            page_candidates=(page_node("page:a"),),
            nested_scope=scope,
            nested_snapshot=snapshot(("child:a",)),
            eligible_direct_child_ids=("child:a",),
        )
        self.assertEqual("suppressed",result.status)
        self.assertIsNone(result.delegated_result)
        self.assertEqual(scope,result.nested_scope)


if __name__=="__main__":
    unittest.main()
