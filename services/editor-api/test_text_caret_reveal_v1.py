#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_caret_reveal_v1 import (
    PageCanvasPlacementV1,
    TextCaretRevealError,
    TextViewportReceiptV1,
    plan_text_caret_reveal_v1,
)
from text_edit_session_v1 import (
    TextEntryCandidateV1,
    TextInitialPositionV1,
    enter_text_edit_session_v1,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
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


def make_context(*,text="AB",story="story:1",lines=None,focus=0):
    d=derive_story_edit_domain_v1(
        story_id=story,story_text=text,provenance="chaptera_created"
    )
    if lines is None:
        lines=(line(story,"l1",0,tuple(
            cluster(i,i+1,i*10,(i+1)*10) for i in range(len(text))
        ),y=100),)
    m=build_resolved_text_caret_map_v1(
        layout_revision_id="layout:1",story_id=story,
        story_scalar_len=len(text),lines=tuple(lines)
    )
    base=BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")
    f=build_text_format_overlay_state_v1(
        story_id=story,base_revision_id="rev:1",story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),base),),
    )
    s=enter_text_edit_session_v1(
        session_id="session:1",incarnation=0,document_id="doc:1",
        entry_candidates=(TextEntryCandidateV1("frame:1",story,"frame:1","editable"),),
        revision_id="rev:1",domain=d,caret_map=m,format_state=f,
        expected_layout_revision_id="layout:1",
        initial_position=TextInitialPositionV1(focus),
    ).session
    return d,m,f,s


def viewport(*,x=0,y=0,w=200,h=200,inset=10,placements=None,rev="view:1",scale=2.0):
    if placements is None:
        placements=(PageCanvasPlacementV1("page:1",0,0),)
    return TextViewportReceiptV1(
        protocol_version="chaptera.text-viewport-receipt.v1",
        view_revision_id=rev,
        viewport_x_emu=x,viewport_y_emu=y,
        viewport_width_emu=w,viewport_height_emu=h,
        emu_per_css_px=scale,
        safe_inset_left_emu=inset,safe_inset_right_emu=inset,
        safe_inset_top_emu=inset,safe_inset_bottom_emu=inset,
        page_placements=tuple(placements),
    )


class TextCaretRevealV1Tests(unittest.TestCase):
    def test_already_visible_target_is_idempotent_noop(self):
        _,m,_,s=make_context(focus=1)
        v=viewport()
        a=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=v,
            expected_view_revision_id="view:1",reveal_reason="navigation",
        )
        b=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=v,
            expected_view_revision_id="view:1",reveal_reason="navigation",
        )
        self.assertEqual(a,b)
        self.assertEqual("no_op",a.status)
        self.assertEqual((0,0),(a.pan_delta_x_emu,a.pan_delta_y_emu))
        self.assertEqual(2.0,a.emu_per_css_px)

    def test_typing_near_viewport_bottom_causes_minimum_downward_pan(self):
        story="story:1"
        lines=(line(story,"l1",0,(cluster(0,1,20,30),),y=180),)
        _,m,_,s=make_context(text="A",lines=lines,focus=1)
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=viewport(),
            expected_view_revision_id="view:1",reveal_reason="accepted_edit",
        )
        # Safe bottom is 190; caret line bottom is 200 -> minimum +10.
        self.assertEqual("pan",result.status)
        self.assertEqual(10,result.pan_delta_y_emu)
        self.assertEqual(10,result.resulting_viewport_y_emu)

    def test_upward_reveal_is_minimum_and_zoom_is_preserved(self):
        story="story:1"
        lines=(line(story,"l1",0,(cluster(0,1,20,30),),y=0),)
        _,m,_,s=make_context(text="A",lines=lines,focus=0)
        v=viewport(y=100,scale=3.25)
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=v,
            expected_view_revision_id="view:1",reveal_reason="undo_redo",
        )
        self.assertEqual("pan",result.status)
        self.assertEqual(-110,result.pan_delta_y_emu)
        self.assertEqual(-10,result.resulting_viewport_y_emu)
        self.assertEqual(3.25,result.emu_per_css_px)

    def test_reversed_selection_reveals_focus_not_normalized_end(self):
        d,m,f,s=make_context(text="AB",focus=2)
        # Re-enter with selection later replaced directly for the test.
        from text_selection_state_v1 import build_text_selection_state_v1, project_selection_state_v1
        from dataclasses import replace
        state=build_text_selection_state_v1(
            domain=d,revision_id="rev:1",anchor_scalar=2,focus_scalar=0
        )
        state=project_selection_state_v1(
            state=state,domain=d,caret_map=m
        ).state
        s=replace(s,selection=state)
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=viewport(x=50),
            expected_view_revision_id="view:1",reveal_reason="navigation",
        )
        self.assertEqual(0,result.focus_scalar)
        self.assertEqual(state.focus_visual_stop_id,result.target_stop_id)
        self.assertLess(result.pan_delta_x_emu,0)

    def test_linked_story_uses_focus_stop_page_and_frame_provenance(self):
        story="story:1"
        l1=line(story,"l1",0,(cluster(0,1,0,10),),page="page:1",frame="frame:A",next="l2",y=0)
        l2=line(story,"l2",1,(cluster(1,2,0,10),),page="page:2",frame="frame:B",prev="l1",y=0)
        _,m,_,s=make_context(text="AB",lines=(l1,l2),focus=2)
        v=viewport(
            w=300,h=300,
            placements=(
                PageCanvasPlacementV1("page:1",0,0),
                PageCanvasPlacementV1("page:2",0,1000),
            ),
        )
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=v,
            expected_view_revision_id="view:1",reveal_reason="authoritative_reflow",
        )
        self.assertEqual("page:2",result.target_page_id)
        self.assertEqual("frame:B",result.target_frame_id)
        self.assertGreater(result.pan_delta_y_emu,0)

    def test_stale_layout_receipt_returns_reconcile_required_without_selection_change(self):
        _,m,_,s=make_context(focus=1)
        from dataclasses import replace
        stale=replace(s,layout_revision_id="layout:old")
        result=plan_text_caret_reveal_v1(
            session=stale,caret_map=m,viewport=viewport(),
            expected_view_revision_id="view:1",reveal_reason="navigation",
        )
        self.assertEqual("reconcile_required",result.status)
        self.assertEqual(stale.selection,result.selection)
        self.assertEqual(0,result.document_mutation_count)

    def test_stale_view_receipt_returns_reconcile_required(self):
        _,m,_,s=make_context(focus=1)
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=viewport(rev="view:2"),
            expected_view_revision_id="view:1",reveal_reason="navigation",
        )
        self.assertEqual("reconcile_required",result.status)

    def test_unplaced_focus_reports_unavailable_without_nearest_frame_fallback(self):
        story="story:1"
        lines=(line(story,"l1",0,(cluster(0,1,0,10),),y=0),)
        d=derive_story_edit_domain_v1(
            story_id=story,story_text="AB",provenance="chaptera_created"
        )
        m=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",story_id=story,story_scalar_len=2,lines=lines
        )
        base=BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")
        f=build_text_format_overlay_state_v1(
            story_id=story,base_revision_id="rev:1",story_scalar_len=2,
            base_runs=(BaseFormatRunV1(0,2,base),),
        )
        # Session may retain a semantic layout-pending selection after reconciliation.
        from text_edit_session_v1 import TextEditSessionV1
        from text_selection_state_v1 import build_text_selection_state_v1, edit_domain_id_v1
        sel=build_text_selection_state_v1(
            domain=d,revision_id="rev:1",anchor_scalar=2,focus_scalar=2
        )
        from resolved_text_caret_map_v1 import caret_map_hash_v1
        s=TextEditSessionV1(
            protocol_version="chaptera.text-edit-session.v1",
            session_id="session:1",incarnation=0,document_id="doc:1",
            story_id=story,revision_id="rev:1",
            edit_domain_id=edit_domain_id_v1(d),
            layout_revision_id="layout:1",caret_map_hash=caret_map_hash_v1(m),
            entry_frame_id="frame:1",current_frame_id="frame:1",
            focus_owner="story_text",selection=sel,typing_state=None,
            composition_session=None,pending_interaction_metadata=(),
        )
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=viewport(),
            expected_view_revision_id="view:1",reveal_reason="programmatic_jump",
        )
        self.assertEqual("reveal_unavailable",result.status)
        self.assertIsNone(result.target_frame_id)
        self.assertEqual(sel,result.selection)

    def test_same_scalar_affinity_ambiguity_is_unsupported_not_arbitrary(self):
        story="story:1"
        l1=line(story,"l1",0,(cluster(0,1,0,10),),next="l2",y=0)
        l2=line(story,"l2",1,(cluster(1,2,0,10),),prev="l1",y=30)
        d=derive_story_edit_domain_v1(
            story_id=story,story_text="AB",provenance="chaptera_created"
        )
        m=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",story_id=story,story_scalar_len=2,lines=(l1,l2)
        )
        base=BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")
        f=build_text_format_overlay_state_v1(
            story_id=story,base_revision_id="rev:1",story_scalar_len=2,
            base_runs=(BaseFormatRunV1(0,2,base),),
        )
        from text_edit_session_v1 import TextEditSessionV1
        from text_selection_state_v1 import build_text_selection_state_v1, edit_domain_id_v1
        from resolved_text_caret_map_v1 import caret_map_hash_v1
        sel=build_text_selection_state_v1(
            domain=d,revision_id="rev:1",anchor_scalar=1,focus_scalar=1
        )
        s=TextEditSessionV1(
            protocol_version="chaptera.text-edit-session.v1",
            session_id="s",incarnation=0,document_id="d",story_id=story,
            revision_id="rev:1",edit_domain_id=edit_domain_id_v1(d),
            layout_revision_id="layout:1",caret_map_hash=caret_map_hash_v1(m),
            entry_frame_id=None,current_frame_id=None,focus_owner="story_text",
            selection=sel,typing_state=None,composition_session=None,
            pending_interaction_metadata=(),
        )
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=viewport(),
            expected_view_revision_id="view:1",reveal_reason="programmatic_jump",
        )
        self.assertEqual("reveal_unsupported",result.status)

    def test_missing_target_page_placement_is_unavailable_not_guessed(self):
        story="story:1"
        lines=(line(story,"l1",0,(cluster(0,1,0,10),),page="page:2",frame="frame:B",y=0),)
        _,m,_,s=make_context(text="A",lines=lines,focus=1)
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=viewport(
                placements=(PageCanvasPlacementV1("page:1",0,0),)
            ),
            expected_view_revision_id="view:1",reveal_reason="programmatic_jump",
        )
        self.assertEqual("reveal_unavailable",result.status)

    def test_reveal_never_changes_selection_typing_preferred_x_or_history(self):
        _,m,_,s=make_context(focus=1)
        result=plan_text_caret_reveal_v1(
            session=s,caret_map=m,viewport=viewport(x=1000,y=1000),
            expected_view_revision_id="view:1",reveal_reason="ime_commit",
        )
        self.assertIs(result.selection,s.selection)
        self.assertTrue(result.selection_unchanged)
        self.assertTrue(result.typing_state_unchanged)
        self.assertTrue(result.preferred_inline_x_unchanged)
        self.assertEqual(0,result.document_mutation_count)
        self.assertFalse(result.undo_history_changed)

    def test_pointer_is_not_an_admitted_reveal_reason(self):
        _,m,_,s=make_context(focus=1)
        with self.assertRaises(TextCaretRevealError) as caught:
            plan_text_caret_reveal_v1(
                session=s,caret_map=m,viewport=viewport(),
                expected_view_revision_id="view:1",reveal_reason="pointer",
            )
        self.assertEqual("unsupported_reveal_reason",caught.exception.code)


if __name__=="__main__":
    unittest.main()
