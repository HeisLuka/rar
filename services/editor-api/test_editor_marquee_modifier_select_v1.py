#!/usr/bin/env python3
import unittest

from box_select_plan_v1 import PointEmu
from editor_marquee_modifier_select_v1 import (
    begin_modifier_capable_marquee_v1,
    finish_marquee_with_release_modifier_v1,
)
from editor_marquee_select_v1 import update_marquee_select_v1
from editor_multi_select_v1 import (
    AuthoredMultiSelectionStateV1,
    AuthoredSelectableNodeV1,
)


def node(node_id,x,y,w=10,h=10):
    return AuthoredSelectableNodeV1("page:1",node_id,True,x,y,w,h)


def pre(ids,primary=None):
    return AuthoredMultiSelectionStateV1(
        "chaptera.authored-multi-selection.v1",
        "page:1",
        tuple(sorted(ids)),
        primary,
    )


def drag_tx(selection):
    tx=begin_modifier_capable_marquee_v1(
        page_id="page:1",
        gesture_token="g1",
        start_document_point=PointEmu(0,0),
        start_screen_x=0,
        start_screen_y=0,
        drag_threshold_px=1,
        pre_gesture_selection=selection,
        started_on_empty_canvas=True,
    ).transaction
    return update_marquee_select_v1(
        transaction=tx,
        current_document_point=PointEmu(50,50),
        current_screen_x=2,
        current_screen_y=2,
    ).transaction


class EditorMarqueeModifierSelectV1Tests(unittest.TestCase):
    def test_plain_release_remains_replace(self):
        tx=drag_tx(pre(("old",),"old"))
        result=finish_marquee_with_release_modifier_v1(
            transaction=tx,
            release_document_point=PointEmu(50,50),
            current_candidates=(node("new",10,10),),
            release_modifier="none",
        )
        self.assertEqual("replace",result.composition_mode)
        self.assertEqual(("new",),result.selection.selected_node_ids)
        self.assertEqual("new",result.selection.primary_node_id)

    def test_shift_release_toggles_against_frozen_pre_gesture_selection(self):
        tx=drag_tx(pre(("a","b"),"a"))
        result=finish_marquee_with_release_modifier_v1(
            transaction=tx,
            release_document_point=PointEmu(50,50),
            current_candidates=(
                node("b",10,10),
                node("c",20,20),
            ),
            release_modifier="shift",
        )
        self.assertEqual("toggle",result.composition_mode)
        self.assertEqual(("a","c"),result.selection.selected_node_ids)
        self.assertEqual("a",result.selection.primary_node_id)

    def test_shift_empty_candidate_preserves_preselection(self):
        tx=drag_tx(pre(("a","b"),"a"))
        result=finish_marquee_with_release_modifier_v1(
            transaction=tx,
            release_document_point=PointEmu(50,50),
            current_candidates=(node("outside",100,100),),
            release_modifier="shift",
        )
        self.assertEqual(("a","b"),result.selection.selected_node_ids)
        self.assertEqual("a",result.selection.primary_node_id)

    def test_shift_can_clear_when_candidates_equal_preselection(self):
        tx=drag_tx(pre(("a","b"),"a"))
        result=finish_marquee_with_release_modifier_v1(
            transaction=tx,
            release_document_point=PointEmu(50,50),
            current_candidates=(node("a",10,10),node("b",20,20)),
            release_modifier="shift",
        )
        self.assertEqual("cleared",result.status)
        self.assertEqual((),result.selection.selected_node_ids)
        self.assertIsNone(result.selection.primary_node_id)

    def test_ctrl_cmd_alt_are_reserved_and_keep_replace_semantics(self):
        for modifier in ("ctrl","cmd","alt"):
            tx=drag_tx(pre(("old",),"old"))
            result=finish_marquee_with_release_modifier_v1(
                transaction=tx,
                release_document_point=PointEmu(50,50),
                current_candidates=(node("new",10,10),),
                release_modifier=modifier,
            )
            self.assertEqual("replace",result.composition_mode)
            self.assertEqual(("new",),result.selection.selected_node_ids)
            self.assertIn("reserved",result.reason)

    def test_release_modifier_is_sampled_once_not_start_state(self):
        tx=drag_tx(pre(("a",),"a"))
        # No modifier was captured by begin/update. Final release alone decides.
        shifted=finish_marquee_with_release_modifier_v1(
            transaction=tx,
            release_document_point=PointEmu(50,50),
            current_candidates=(node("a",10,10),node("b",20,20)),
            release_modifier="shift",
        )
        plain=finish_marquee_with_release_modifier_v1(
            transaction=tx,
            release_document_point=PointEmu(50,50),
            current_candidates=(node("a",10,10),node("b",20,20)),
            release_modifier="none",
        )
        self.assertEqual(("b",),shifted.selection.selected_node_ids)
        self.assertEqual(("a","b"),plain.selection.selected_node_ids)

    def test_no_authoring_mutation(self):
        tx=drag_tx(pre(()))
        result=finish_marquee_with_release_modifier_v1(
            transaction=tx,
            release_document_point=PointEmu(50,50),
            current_candidates=(node("x",10,10),),
            release_modifier="shift",
        )
        self.assertEqual(0,result.document_mutation_count)
        self.assertFalse(result.revision_created)


if __name__=="__main__":
    unittest.main()
