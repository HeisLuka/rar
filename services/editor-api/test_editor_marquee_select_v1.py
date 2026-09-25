#!/usr/bin/env python3
import unittest

from box_select_plan_v1 import PointEmu
from editor_marquee_select_v1 import (
    begin_marquee_select_v1,
    cancel_marquee_select_v1,
    finish_marquee_select_v1,
    update_marquee_select_v1,
)
from editor_multi_select_v1 import (
    AuthoredMultiSelectionStateV1,
    AuthoredSelectableNodeV1,
    empty_multi_selection_v1,
)


def node(node_id,x,y,w,h,*,page="page:1",authored=True):
    return AuthoredSelectableNodeV1(page,node_id,authored,x,y,w,h)


def pre():
    return AuthoredMultiSelectionStateV1(
        "chaptera.authored-multi-selection.v1",
        "page:1",
        ("old",),
        "old",
    )


class EditorMarqueeSelectV1Tests(unittest.TestCase):
    def test_object_hit_declines_marquee_to_higher_priority_route(self):
        start=begin_marquee_select_v1(
            page_id="page:1",gesture_token="g1",
            start_document_point=PointEmu(0,0),
            start_screen_x=0,start_screen_y=0,drag_threshold_px=4,
            pre_gesture_selection=pre(),started_on_empty_canvas=False,
        )
        self.assertEqual("declined_nonempty_hit",start.status)
        self.assertIsNone(start.transaction)

    def test_below_threshold_keeps_selection_and_no_overlay(self):
        tx=begin_marquee_select_v1(
            page_id="page:1",gesture_token="g1",
            start_document_point=PointEmu(0,0),
            start_screen_x=10,start_screen_y=10,drag_threshold_px=5,
            pre_gesture_selection=pre(),started_on_empty_canvas=True,
        ).transaction
        update=update_marquee_select_v1(
            transaction=tx,current_document_point=PointEmu(100,100),
            current_screen_x=13,current_screen_y=13,
        )
        self.assertEqual("armed",update.transaction.stage)
        self.assertIsNone(update.transaction.overlay_bounds)
        self.assertEqual(pre(),update.canonical_selection)

    def test_motion_after_threshold_changes_overlay_only_until_release(self):
        tx=begin_marquee_select_v1(
            page_id="page:1",gesture_token="g1",
            start_document_point=PointEmu(0,0),
            start_screen_x=0,start_screen_y=0,drag_threshold_px=4,
            pre_gesture_selection=pre(),started_on_empty_canvas=True,
        ).transaction
        update=update_marquee_select_v1(
            transaction=tx,current_document_point=PointEmu(200,200),
            current_screen_x=10,current_screen_y=0,
        )
        self.assertEqual("dragging",update.transaction.stage)
        self.assertEqual(pre(),update.canonical_selection)
        self.assertIsNotNone(update.transaction.overlay_bounds)
        self.assertEqual(0,update.document_mutation_count)

    def test_release_calls_full_containment_and_replaces_selection(self):
        tx=begin_marquee_select_v1(
            page_id="page:1",gesture_token="g1",
            start_document_point=PointEmu(0,0),
            start_screen_x=0,start_screen_y=0,drag_threshold_px=4,
            pre_gesture_selection=pre(),started_on_empty_canvas=True,
        ).transaction
        tx=update_marquee_select_v1(
            transaction=tx,current_document_point=PointEmu(100,100),
            current_screen_x=10,current_screen_y=10,
        ).transaction
        result=finish_marquee_select_v1(
            transaction=tx,
            release_document_point=PointEmu(100,100),
            current_candidates=(
                node("inside:a",10,10,10,10),
                node("inside:b",50,50,20,20),
                node("partial",90,90,20,20),
                node("projected",20,20,10,10,authored=False),
                node("other-page",10,10,10,10,page="page:2"),
            ),
        )
        self.assertEqual("selected",result.status)
        self.assertEqual(("inside:a","inside:b"),result.selected_node_ids)
        self.assertIsNone(result.primary_node_id)
        self.assertFalse(result.revision_created)

    def test_single_result_becomes_primary_and_empty_result_clears(self):
        tx=begin_marquee_select_v1(
            page_id="page:1",gesture_token="g1",
            start_document_point=PointEmu(0,0),
            start_screen_x=0,start_screen_y=0,drag_threshold_px=1,
            pre_gesture_selection=pre(),started_on_empty_canvas=True,
        ).transaction
        tx=update_marquee_select_v1(
            transaction=tx,current_document_point=PointEmu(50,50),
            current_screen_x=2,current_screen_y=0,
        ).transaction
        one=finish_marquee_select_v1(
            transaction=tx,release_document_point=PointEmu(50,50),
            current_candidates=(node("one",10,10,10,10),),
        )
        self.assertEqual("one",one.primary_node_id)

        empty=finish_marquee_select_v1(
            transaction=tx,release_document_point=PointEmu(50,50),
            current_candidates=(node("outside",100,100,10,10),),
        )
        self.assertEqual("cleared",empty.status)
        self.assertEqual((),empty.selected_node_ids)

    def test_cancel_restores_exact_pre_gesture_selection(self):
        tx=begin_marquee_select_v1(
            page_id="page:1",gesture_token="g1",
            start_document_point=PointEmu(0,0),
            start_screen_x=0,start_screen_y=0,drag_threshold_px=1,
            pre_gesture_selection=pre(),started_on_empty_canvas=True,
        ).transaction
        tx=update_marquee_select_v1(
            transaction=tx,current_document_point=PointEmu(100,100),
            current_screen_x=100,current_screen_y=100,
        ).transaction
        cancelled=cancel_marquee_select_v1(tx)
        self.assertEqual("cancelled",cancelled.status)
        self.assertEqual(pre(),cancelled.selection)

    def test_drag_direction_is_semantically_irrelevant(self):
        base=empty_multi_selection_v1(page_id="page:1")
        candidates=(node("x",10,10,10,10),)
        out=[]
        for start_doc,end_doc in ((PointEmu(0,0),PointEmu(50,50)),(PointEmu(50,50),PointEmu(0,0))):
            tx=begin_marquee_select_v1(
                page_id="page:1",gesture_token="g",
                start_document_point=start_doc,
                start_screen_x=0,start_screen_y=0,drag_threshold_px=1,
                pre_gesture_selection=base,started_on_empty_canvas=True,
            ).transaction
            tx=update_marquee_select_v1(
                transaction=tx,current_document_point=end_doc,
                current_screen_x=2,current_screen_y=2,
            ).transaction
            out.append(finish_marquee_select_v1(
                transaction=tx,release_document_point=end_doc,current_candidates=candidates
            ).selected_node_ids)
        self.assertEqual(out[0],out[1])

    def test_modifier_marquee_is_explicitly_outside_v1(self):
        result=begin_marquee_select_v1(
            page_id="page:1",gesture_token="g1",
            start_document_point=PointEmu(0,0),
            start_screen_x=0,start_screen_y=0,drag_threshold_px=4,
            pre_gesture_selection=pre(),started_on_empty_canvas=True,
            modifier_state="shift",
        )
        self.assertEqual("suppressed_modifier",result.status)
        self.assertIsNone(result.transaction)


if __name__=="__main__":
    unittest.main()
