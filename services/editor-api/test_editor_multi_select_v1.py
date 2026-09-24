#!/usr/bin/env python3
import unittest

from editor_multi_select_v1 import (
    AuthoredSelectableNodeV1,
    EditorMultiSelectError,
    aggregate_selection_bounds_v1,
    change_selection_page_v1,
    click_authored_node_v1,
    click_empty_canvas_v1,
    empty_multi_selection_v1,
    inspector_selection_summary_v1,
    shift_click_authored_node_v1,
)


def node(node_id,x=0,y=0,w=10,h=10,*,page="page:1",authored=True):
    return AuthoredSelectableNodeV1(page,node_id,authored,x,y,w,h)


class EditorMultiSelectV1Tests(unittest.TestCase):
    def test_plain_click_replaces_selection_and_sets_primary(self):
        state=empty_multi_selection_v1(page_id="page:1")
        state=click_authored_node_v1(state,candidate=node("b"))
        self.assertEqual(("b",),state.selected_node_ids)
        self.assertEqual("b",state.primary_node_id)
        state=click_authored_node_v1(state,candidate=node("a"))
        self.assertEqual(("a",),state.selected_node_ids)
        self.assertEqual("a",state.primary_node_id)

    def test_shift_click_toggles_deterministically(self):
        state=empty_multi_selection_v1(page_id="page:1")
        state=shift_click_authored_node_v1(state,candidate=node("b"))
        state=shift_click_authored_node_v1(state,candidate=node("a"))
        self.assertEqual(("a","b"),state.selected_node_ids)
        self.assertEqual("b",state.primary_node_id)
        state=shift_click_authored_node_v1(state,candidate=node("a"))
        self.assertEqual(("b",),state.selected_node_ids)
        self.assertEqual("b",state.primary_node_id)

    def test_empty_click_and_page_change_clear_transient_selection(self):
        state=click_authored_node_v1(
            empty_multi_selection_v1(page_id="page:1"),candidate=node("a")
        )
        cleared=click_empty_canvas_v1(state)
        self.assertEqual((),cleared.selected_node_ids)
        changed=change_selection_page_v1(state,page_id="page:2")
        self.assertEqual("page:2",changed.page_id)
        self.assertEqual((),changed.selected_node_ids)

    def test_cross_page_and_projected_source_backed_are_rejected(self):
        state=empty_multi_selection_v1(page_id="page:1")
        with self.assertRaisesRegex(EditorMultiSelectError,"cross-page"):
            click_authored_node_v1(state,candidate=node("x",page="page:2"))
        with self.assertRaisesRegex(EditorMultiSelectError,"not admitted"):
            click_authored_node_v1(state,candidate=node("x",authored=False))

    def test_aggregate_bounds_cover_individual_outlines_without_z_order_meaning(self):
        state=empty_multi_selection_v1(page_id="page:1")
        state=shift_click_authored_node_v1(state,candidate=node("a",10,20,30,40))
        state=shift_click_authored_node_v1(state,candidate=node("b",50,5,10,20))
        bounds=aggregate_selection_bounds_v1(
            state=state,
            candidates=(node("a",10,20,30,40),node("b",50,5,10,20)),
        )
        self.assertEqual((10,5,50,55),(bounds.x_emu,bounds.y_emu,bounds.width_emu,bounds.height_emu))

    def test_inspector_exposes_count_and_primary_only(self):
        state=click_authored_node_v1(
            empty_multi_selection_v1(page_id="page:1"),candidate=node("n1")
        )
        summary=inspector_selection_summary_v1(state)
        self.assertEqual(1,summary.count)
        self.assertEqual("n1",summary.primary_node_id)

    def test_selection_is_pure_transient_state(self):
        state=empty_multi_selection_v1(page_id="page:1")
        self.assertFalse(hasattr(state,"editor_project"))
        self.assertFalse(hasattr(state,"revision_id"))


if __name__=="__main__":
    unittest.main()
