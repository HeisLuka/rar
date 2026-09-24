#!/usr/bin/env python3
import unittest

from dataclasses import replace

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_caret_reveal_v1 import (
    PageCanvasPlacementV1,
    TextViewportReceiptV1,
)
from text_composition_session_v1 import TextCompositionSessionV1
from text_edit_session_v1 import (
    TextEntryCandidateV1,
    TextInitialPositionV1,
    attach_text_composition_v1,
    enter_text_edit_session_v1,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)
from text_programmatic_jump_v1 import (
    TextProgrammaticJumpError,
    TextProgrammaticJumpRequestV1,
    execute_text_programmatic_jump_v1,
)


def cluster(a,b,x0,x1):
    return ResolvedClusterV1(a,b,x0,x1,x0,x1,True,())


def line(story,line_id,ordinal,clusters,*,page="page:1",frame="frame:1",prev=None,next=None,y=0):
    return ResolvedLineFragmentV1(
        story_id=story,page_id=page,frame_id=frame,line_id=line_id,
        flow_ordinal=ordinal,previous_line_id=prev,next_line_id=next,
        page_y_top_emu=y,page_y_bottom_emu=y+20,
        frame_y_top_emu=y,frame_y_bottom_emu=y+20,
        clusters=tuple(clusters),
    )


def authority(story,text,*,lines=None,provenance="chaptera_created",layout="layout:1"):
    d=derive_story_edit_domain_v1(
        story_id=story,story_text=text,provenance=provenance
    )
    if lines is None:
        lines=(line(story,"l1",0,tuple(
            cluster(i,i+1,i*10,(i+1)*10) for i in range(len(text))
        ),y=0),) if text else ()
    m=build_resolved_text_caret_map_v1(
        layout_revision_id=layout,story_id=story,
        story_scalar_len=len(text),lines=tuple(lines)
    )
    fmt=BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")
    f=build_text_format_overlay_state_v1(
        story_id=story,base_revision_id="rev:1",story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),fmt),),
    )
    return d,m,f


def candidate(story,frame="frame:1",cap="editable"):
    return TextEntryCandidateV1(frame or story,story,frame,cap)


def viewport(*,x=0,y=0,placements=None):
    if placements is None:
        placements=(PageCanvasPlacementV1("page:1",0,0),)
    return TextViewportReceiptV1(
        protocol_version="chaptera.text-viewport-receipt.v1",
        view_revision_id="view:1",
        viewport_x_emu=x,viewport_y_emu=y,
        viewport_width_emu=200,viewport_height_emu=200,
        emu_per_css_px=1.0,
        safe_inset_left_emu=10,safe_inset_right_emu=10,
        safe_inset_top_emu=10,safe_inset_bottom_emu=10,
        page_placements=tuple(placements),
    )


def request(story,start,end,mode="exact_range",rev="rev:1",reason="find"):
    return TextProgrammaticJumpRequestV1(
        protocol_version="chaptera.text-programmatic-jump.v1",
        document_id="doc:1",revision_id=rev,story_id=story,
        start_scalar=start,end_scalar=end,selection_mode=mode,reason=reason,
    )


def enter(story,text,focus=0,lines=None):
    d,m,f=authority(story,text,lines=lines)
    s=enter_text_edit_session_v1(
        session_id="session:1",incarnation=0,document_id="doc:1",
        entry_candidates=(candidate(story),),revision_id="rev:1",
        domain=d,caret_map=m,format_state=f,
        expected_layout_revision_id="layout:1",
        initial_position=TextInitialPositionV1(focus),
    ).session
    return d,m,f,s


class TextProgrammaticJumpV1Tests(unittest.TestCase):
    def test_same_story_exact_search_match_reuses_session_and_forward_selection(self):
        d,m,f,s=enter("story:1","ABC",focus=0)
        result=execute_text_programmatic_jump_v1(
            request=request("story:1",1,3),
            target_candidate=candidate("story:1"),
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            viewport=viewport(),expected_view_revision_id="view:1",
            active_session=s,
        )
        self.assertEqual("reuse",result.transition_kind)
        self.assertEqual("session:1",result.session.session_id)
        self.assertEqual((1,3),(
            result.session.selection.anchor_scalar,
            result.session.selection.focus_scalar,
        ))
        self.assertEqual("projected",result.geometry_state)
        self.assertEqual(0,result.authoring_mutation_count)
        self.assertEqual(0,result.undo_history_entry_count)
        self.assertIsNone(result.session.typing_state)

    def test_jump_story_a_to_b_switches_one_session_context(self):
        d1,m1,f1,s1=enter("story:A","A",focus=1)
        d2,m2,f2=authority("story:B","B")
        result=execute_text_programmatic_jump_v1(
            request=request("story:B",0,1),
            target_candidate=candidate("story:B","frame:B"),
            domain=d2,caret_map=m2,format_state=f2,
            expected_layout_revision_id="layout:1",
            viewport=viewport(),expected_view_revision_id="view:1",
            active_session=s1,
        )
        self.assertEqual("story_switch",result.transition_kind)
        self.assertEqual("story:B",result.session.story_id)
        self.assertEqual("session:1",result.session.session_id)
        self.assertEqual(1,result.session.incarnation)
        self.assertTrue(result.focus_context_discontinuity)
        self.assertTrue(result.undo_group_boundary)

    def test_linked_story_focus_endpoint_uses_frame_b_page_2_and_reveals_it(self):
        story="story:1"
        l1=line(story,"l1",0,(cluster(0,1,0,10),),page="page:1",frame="frame:A",next="l2",y=0)
        l2=line(story,"l2",1,(cluster(1,2,0,10),),page="page:2",frame="frame:B",prev="l1",y=0)
        d,m,f,s=enter(story,"AB",focus=0,lines=(l1,l2))
        result=execute_text_programmatic_jump_v1(
            request=request(story,1,2),
            target_candidate=candidate(story,"frame:A"),
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            viewport=viewport(placements=(
                PageCanvasPlacementV1("page:1",0,0),
                PageCanvasPlacementV1("page:2",0,1000),
            )),
            expected_view_revision_id="view:1",
            active_session=s,
        )
        self.assertEqual("frame:B",result.reveal.target_frame_id)
        self.assertEqual("page:2",result.reveal.target_page_id)
        self.assertEqual("frame:B",result.session.current_frame_id)
        self.assertEqual("pan",result.reveal.status)

    def test_range_adjacent_to_protected_terminal_is_allowed_but_overlap_rejects(self):
        story="story:1"
        d,m,f=authority(
            story,"A\r",provenance="imported_mature_quill_terminal_cr"
        )
        ok=execute_text_programmatic_jump_v1(
            request=request(story,0,1),
            target_candidate=candidate(story),
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            viewport=viewport(),expected_view_revision_id="view:1",
        )
        self.assertEqual((0,1),ok.session.selection.normalized_range)
        with self.assertRaises(TextProgrammaticJumpError) as caught:
            execute_text_programmatic_jump_v1(
                request=request(story,0,2),
                target_candidate=candidate(story),
                domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                viewport=viewport(),expected_view_revision_id="view:1",
            )
        self.assertEqual("selection_reconcile_required",caught.exception.code)

    def test_readonly_and_stale_active_revision_fail_closed(self):
        d,m,f,s=enter("story:1","A")
        with self.assertRaises(TextProgrammaticJumpError) as readonly:
            execute_text_programmatic_jump_v1(
                request=request("story:1",0,1),
                target_candidate=candidate("story:1",cap="read_only"),
                domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                viewport=viewport(),expected_view_revision_id="view:1",
                active_session=s,
            )
        self.assertEqual("read_only_story",readonly.exception.code)

        with self.assertRaises(TextProgrammaticJumpError) as stale:
            execute_text_programmatic_jump_v1(
                request=request("story:1",0,1,rev="rev:2"),
                target_candidate=candidate("story:1"),
                domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                viewport=viewport(),expected_view_revision_id="view:1",
                active_session=s,
            )
        self.assertEqual("jump_stale",stale.exception.code)

    def test_active_composition_requires_explicit_resolution(self):
        d,m,f,s=enter("story:1","A",focus=1)
        comp=TextCompositionSessionV1(
            protocol_version="chaptera.text-composition-session.v1",
            composition_id="ime:1",story_id="story:1",base_revision_id="rev:1",
            edit_domain_id=s.edit_domain_id,start_scalar=1,end_scalar=1,
            expected_before="",captured_selection=s.selection,typing_snapshot=None,
            provisional_external_text="",provisional_selection_start_scalar=0,
            provisional_selection_end_scalar=0,
        )
        active=attach_text_composition_v1(s,comp)
        with self.assertRaises(TextProgrammaticJumpError) as caught:
            execute_text_programmatic_jump_v1(
                request=request("story:1",0,1),
                target_candidate=candidate("story:1"),
                domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                viewport=viewport(),expected_view_revision_id="view:1",
                active_session=active,
            )
        self.assertEqual("composition_transition_required",caught.exception.code)
        ok=execute_text_programmatic_jump_v1(
            request=request("story:1",0,1),
            target_candidate=candidate("story:1"),
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            viewport=viewport(),expected_view_revision_id="view:1",
            active_session=active,composition_resolution="cancelled",
        )
        self.assertIsNone(ok.session.composition_session)

    def test_already_visible_collapsed_caret_reveal_is_noop(self):
        d,m,f,s=enter("story:1","A",focus=0)
        result=execute_text_programmatic_jump_v1(
            request=request("story:1",0,0,mode="collapsed_caret"),
            target_candidate=candidate("story:1"),
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            viewport=viewport(),expected_view_revision_id="view:1",
            active_session=s,
        )
        self.assertEqual("no_op",result.reveal.status)
        self.assertEqual((0,0),(
            result.session.selection.anchor_scalar,
            result.session.selection.focus_scalar,
        ))

    def test_nonempty_unplaced_target_keeps_semantic_selection_and_reports_pending(self):
        story="story:1"
        lines=(line(story,"l1",0,(cluster(0,1,0,10),),y=0),)
        d,m,f=authority(story,"AB",lines=lines)
        result=execute_text_programmatic_jump_v1(
            request=request(story,1,2),
            target_candidate=candidate(story),
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            viewport=viewport(),expected_view_revision_id="view:1",
        )
        self.assertEqual("semantic_geometry_pending",result.geometry_state)
        self.assertEqual((1,2),result.session.selection.normalized_range)
        self.assertEqual("reveal_unavailable",result.reveal.status)

    def test_collapsed_unplaced_target_fails_instead_of_invisible_editable_caret(self):
        story="story:1"
        lines=(line(story,"l1",0,(cluster(0,1,0,10),),y=0),)
        d,m,f=authority(story,"AB",lines=lines)
        with self.assertRaises(TextProgrammaticJumpError) as caught:
            execute_text_programmatic_jump_v1(
                request=request(story,2,2,mode="collapsed_caret"),
                target_candidate=candidate(story),
                domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                viewport=viewport(),expected_view_revision_id="view:1",
            )
        self.assertEqual("collapsed_target_unplaced",caught.exception.code)

    def test_same_scalar_affinity_ambiguity_fails_without_first_stop_guess(self):
        story="story:1"
        l1=line(story,"l1",0,(cluster(0,1,0,10),),next="l2",y=0)
        l2=line(story,"l2",1,(cluster(1,2,0,10),),prev="l1",y=30)
        d,m,f=authority(story,"AB",lines=(l1,l2))
        with self.assertRaises(TextProgrammaticJumpError) as caught:
            execute_text_programmatic_jump_v1(
                request=request(story,1,1,mode="collapsed_caret"),
                target_candidate=candidate(story),
                domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                viewport=viewport(),expected_view_revision_id="view:1",
            )
        self.assertEqual("caret_affinity_required",caught.exception.code)

    def test_repeated_identical_same_story_jump_is_session_selection_idempotent(self):
        d,m,f,s=enter("story:1","ABC")
        first=execute_text_programmatic_jump_v1(
            request=request("story:1",1,2),
            target_candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",viewport=viewport(),
            expected_view_revision_id="view:1",active_session=s,
        )
        second=execute_text_programmatic_jump_v1(
            request=request("story:1",1,2),
            target_candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",viewport=viewport(),
            expected_view_revision_id="view:1",active_session=first.session,
        )
        self.assertEqual(first.session,second.session)
        self.assertEqual(first.reveal,second.reveal)
        self.assertEqual(0,second.authoring_mutation_count)


if __name__=="__main__":
    unittest.main()
