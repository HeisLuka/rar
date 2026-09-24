#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_composition_session_v1 import TextCompositionSessionV1
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)
from text_pointer_selection_v1 import (
    TextPointerSelectionError,
    begin_pointer_drag_v1,
    end_pointer_drag_v1,
    pointer_click_v1,
    update_pointer_drag_v1,
)
from text_selection_state_v1 import build_text_selection_state_v1


STORY="story:1"


def cluster(a,b,x0,x1):
    return ResolvedClusterV1(a,b,x0,x1,x0,x1,True,())


def line(line_id,ordinal,clusters,*,frame="frame:1",page="page:1",prev=None,next=None,y=0,story=STORY):
    return ResolvedLineFragmentV1(
        story_id=story,
        page_id=page,
        frame_id=frame,
        line_id=line_id,
        flow_ordinal=ordinal,
        previous_line_id=prev,
        next_line_id=next,
        page_y_top_emu=y,
        page_y_bottom_emu=y+20,
        frame_y_top_emu=y,
        frame_y_bottom_emu=y+20,
        clusters=tuple(clusters),
    )


def caret_map(text_len,lines,*,story=STORY,layout="layout:1"):
    return build_resolved_text_caret_map_v1(
        layout_revision_id=layout,
        story_id=story,
        story_scalar_len=text_len,
        lines=tuple(lines),
    )


def domain(text,provenance="chaptera_created",story=STORY):
    return derive_story_edit_domain_v1(
        story_id=story,story_text=text,provenance=provenance
    )


def format_state(text,story=STORY):
    base=BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")
    return build_text_format_overlay_state_v1(
        story_id=story,
        base_revision_id="rev:1",
        story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),base),),
    )


def simple_map(text="ABC"):
    return caret_map(
        len(text),
        (line("l1",0,(
            cluster(0,1,0,10),
            cluster(1,2,10,20),
            cluster(2,3,20,30),
        )),),
    )


class TextPointerSelectionV1Tests(unittest.TestCase):
    def test_plain_click_before_inside_after_line_collapses_to_nearest_admitted_stop(self):
        text="ABC"
        d=domain(text)
        m=simple_map(text)
        fs=format_state(text)
        for x,expected in ((-100,0),(19,2),(100,3)):
            result=pointer_click_v1(
                domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
                page_id="page:1",page_x_emu=x,page_y_emu=10,
                expected_layout_revision_id="layout:1",
            )
            self.assertEqual((expected,expected),(
                result.selection.anchor_scalar,result.selection.focus_scalar
            ))
            self.assertEqual("projected",result.selection.projection_state)
            self.assertEqual(result.selection.anchor_visual_stop_id,result.selection.focus_visual_stop_id)
            self.assertIsNone(result.selection.preferred_inline_x_emu)
            self.assertEqual(0,result.document_mutation_count)
            self.assertIsNotNone(result.typing_state)
            self.assertEqual((),result.typing_state.pending_explicit_properties)

    def test_protected_terminal_mark_stop_is_filtered_not_selected(self):
        text="A\r"
        d=domain(text,"imported_mature_quill_terminal_cr")
        m=caret_map(2,(line("l1",0,(cluster(0,1,0,10),cluster(1,2,10,20))),))
        result=pointer_click_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=format_state(text),
            page_id="page:1",page_x_emu=100,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(1,result.selection.focus_scalar)

    def test_shift_click_preserves_anchor_and_moves_focus_forward_and_backward(self):
        text="ABC"
        d=domain(text)
        m=simple_map(text)
        fs=format_state(text)
        initial=pointer_click_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=10,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        ).selection
        forward=pointer_click_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=100,page_y_emu=10,
            expected_layout_revision_id="layout:1",
            existing_selection=initial,shift=True,
        )
        self.assertEqual((1,3),(forward.selection.anchor_scalar,forward.selection.focus_scalar))
        backward=pointer_click_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=-100,page_y_emu=10,
            expected_layout_revision_id="layout:1",
            existing_selection=initial,shift=True,
        )
        self.assertEqual((1,0),(backward.selection.anchor_scalar,backward.selection.focus_scalar))
        self.assertIsNone(forward.typing_state)

    def test_drag_keeps_original_anchor_when_focus_crosses_it(self):
        text="ABC"
        d=domain(text)
        m=simple_map(text)
        fs=format_state(text)
        begun=begin_pointer_drag_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=20,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        left=update_pointer_drag_v1(
            begun.drag_session,domain=d,caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=0,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual((2,0),(left.result.selection.anchor_scalar,left.result.selection.focus_scalar))
        right=update_pointer_drag_v1(
            left.drag_session,domain=d,caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=30,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual((2,3),(right.result.selection.anchor_scalar,right.result.selection.focus_scalar))
        self.assertEqual(right.result.selection,end_pointer_drag_v1(right.drag_session,domain=d))

    def test_drag_across_linked_frames_of_same_story_is_allowed(self):
        text="AB"
        d=domain(text)
        l1=line("l1",0,(cluster(0,1,0,10),),frame="frame:A",next="l2",y=0)
        l2=line("l2",1,(cluster(1,2,0,10),),frame="frame:B",prev="l1",y=30)
        m=caret_map(2,(l1,l2))
        fs=format_state(text)
        begun=begin_pointer_drag_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=0,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        moved=update_pointer_drag_v1(
            begun.drag_session,domain=d,caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=10,page_y_emu=40,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual((0,2),(moved.result.selection.anchor_scalar,moved.result.selection.focus_scalar))
        self.assertEqual("frame:B",moved.result.hit_stop.frame_id)

    def test_cross_story_drag_fails_explicitly(self):
        text="A"
        d=domain(text)
        m=caret_map(1,(line("l1",0,(cluster(0,1,0,10),)),))
        fs=format_state(text)
        begun=begin_pointer_drag_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=0,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        other_d=domain("B",story="story:2")
        other_m=caret_map(
            1,(line("o1",0,(cluster(0,1,0,10),),story="story:2"),),
            story="story:2",
        )
        with self.assertRaises(TextPointerSelectionError) as caught:
            update_pointer_drag_v1(
                begun.drag_session,domain=other_d,caret_map=other_m,
                format_state=format_state("B",story="story:2"),
                page_id="page:1",page_x_emu=0,page_y_emu=10,
                expected_layout_revision_id="layout:1",
            )
        self.assertEqual("cross_story_selection_unsupported",caught.exception.code)

    def test_stale_layout_receipt_fails_before_selection_change(self):
        text="ABC"
        with self.assertRaises(TextPointerSelectionError) as caught:
            pointer_click_v1(
                domain=domain(text),revision_id="rev:1",caret_map=simple_map(text),
                format_state=format_state(text),page_id="page:1",
                page_x_emu=10,page_y_emu=10,
                expected_layout_revision_id="layout:stale",
            )
        self.assertEqual("stale_layout_map",caught.exception.code)

    def test_ligature_like_cluster_never_invents_unsupported_internal_scalar(self):
        text="AB"
        m=caret_map(2,(line("l1",0,(cluster(0,2,0,20),)),))
        result=pointer_click_v1(
            domain=domain(text),revision_id="rev:1",caret_map=m,
            format_state=format_state(text),page_id="page:1",
            page_x_emu=11,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        self.assertIn(result.selection.focus_scalar,{0,2})
        self.assertNotEqual(1,result.selection.focus_scalar)

    def test_active_composition_requires_acknowledged_transition(self):
        text="ABC"
        d=domain(text)
        captured=build_text_selection_state_v1(
            domain=d,revision_id="rev:1",anchor_scalar=1,focus_scalar=1
        )
        comp=TextCompositionSessionV1(
            protocol_version="chaptera.text-composition-session.v1",
            composition_id="ime:1",story_id=STORY,base_revision_id="rev:1",
            edit_domain_id=captured.edit_domain_id,start_scalar=1,end_scalar=1,
            expected_before="",captured_selection=captured,typing_snapshot=None,
            provisional_external_text="",provisional_selection_start_scalar=0,
            provisional_selection_end_scalar=0,
        )
        with self.assertRaises(TextPointerSelectionError) as caught:
            pointer_click_v1(
                domain=d,revision_id="rev:1",caret_map=simple_map(text),
                format_state=format_state(text),page_id="page:1",
                page_x_emu=0,page_y_emu=10,
                expected_layout_revision_id="layout:1",
                composition_session=comp,
            )
        self.assertEqual("composition_transition_required",caught.exception.code)
        accepted=pointer_click_v1(
            domain=d,revision_id="rev:1",caret_map=simple_map(text),
            format_state=format_state(text),page_id="page:1",
            page_x_emu=0,page_y_emu=10,
            expected_layout_revision_id="layout:1",
            composition_session=comp,composition_resolution_acknowledged=True,
        )
        self.assertEqual(0,accepted.selection.focus_scalar)

    def test_viewport_edge_handoff_retains_anchor_and_last_focus(self):
        text="ABC"
        d=domain(text)
        m=simple_map(text)
        fs=format_state(text)
        begun=begin_pointer_drag_v1(
            domain=d,revision_id="rev:1",caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=10,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        moved=update_pointer_drag_v1(
            begun.drag_session,domain=d,caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=30,page_y_emu=10,
            expected_layout_revision_id="layout:1",
        )
        edge=update_pointer_drag_v1(
            moved.drag_session,domain=d,caret_map=m,format_state=fs,
            page_id="page:1",page_x_emu=1000,page_y_emu=10,
            expected_layout_revision_id="layout:1",
            pointer_in_hittable_view=False,
        )
        self.assertTrue(edge.result.autoscroll_needed)
        self.assertIsNone(edge.result.hit_stop)
        self.assertEqual(moved.result.selection,edge.result.selection)
        self.assertEqual(0,edge.result.document_mutation_count)

    def test_edit_domain_unknown_fails_closed(self):
        text="A\r"
        with self.assertRaises(TextPointerSelectionError) as caught:
            pointer_click_v1(
                domain=domain(text,"imported_unknown"),revision_id="rev:1",
                caret_map=caret_map(2,(line("l1",0,(cluster(0,1,0,10),cluster(1,2,10,20))),)),
                format_state=format_state(text),page_id="page:1",
                page_x_emu=10,page_y_emu=10,
                expected_layout_revision_id="layout:1",
            )
        self.assertEqual("edit_domain_unknown",caught.exception.code)


if __name__=="__main__":
    unittest.main()
