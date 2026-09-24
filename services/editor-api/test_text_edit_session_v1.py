#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_composition_session_v1 import TextCompositionSessionV1
from text_edit_session_v1 import (
    TextEditSessionError,
    TextEntryCandidateV1,
    TextInitialPositionV1,
    TextPointerEntryContextV1,
    attach_text_composition_v1,
    enter_text_edit_session_v1,
    exit_text_edit_session_v1,
    handoff_same_story_frame_v1,
    rebind_text_edit_session_authority_v1,
    switch_text_edit_session_v1,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)
from text_selection_state_v1 import build_text_selection_state_v1
from text_typing_format_state_v1 import derive_typing_format_state_v1


def cluster(a,b,x0,x1):
    return ResolvedClusterV1(a,b,x0,x1,x0,x1,True,())


def line(story,line_id,ordinal,clusters,*,frame="frame:A",prev=None,next=None,y=0):
    return ResolvedLineFragmentV1(
        story_id=story,page_id="page:1",frame_id=frame,line_id=line_id,
        flow_ordinal=ordinal,previous_line_id=prev,next_line_id=next,
        page_y_top_emu=y,page_y_bottom_emu=y+20,
        frame_y_top_emu=y,frame_y_bottom_emu=y+20,
        clusters=tuple(clusters),
    )


def context(story,text,*,provenance="chaptera_created",linked=False,layout="layout:1"):
    d=derive_story_edit_domain_v1(
        story_id=story,story_text=text,provenance=provenance
    )
    if not text:
        lines=()
    elif linked and len(text)>=2:
        lines=(
            line(story,"l1",0,(cluster(0,1,0,10),),frame="frame:A",next="l2",y=0),
            line(story,"l2",1,(cluster(1,len(text),0,10),),frame="frame:B",prev="l1",y=30),
        )
    else:
        lines=(line(story,"l1",0,tuple(
            cluster(i,i+1,i*10,(i+1)*10) for i in range(len(text))
        )),)
    m=build_resolved_text_caret_map_v1(
        layout_revision_id=layout,story_id=story,
        story_scalar_len=len(text),lines=lines
    )
    base=BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")
    f=build_text_format_overlay_state_v1(
        story_id=story,base_revision_id="rev:1",story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),base),),
    )
    return d,m,f


def candidate(story,frame="frame:A",capability="editable"):
    return TextEntryCandidateV1(
        target_id=frame or story,story_id=story,frame_id=frame,
        capability=capability,
    )


class TextEditSessionV1Tests(unittest.TestCase):
    def test_enter_existing_story_by_pointer_creates_transient_text_focus_only(self):
        d,m,f=context("story:1","ABC")
        tr=enter_text_edit_session_v1(
            session_id="session:1",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),
            revision_id="rev:1",domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            pointer_context=TextPointerEntryContextV1("page:1",19,10),
        )
        s=tr.session
        self.assertEqual("story_text",s.focus_owner)
        self.assertEqual("story:1",s.story_id)
        self.assertEqual("frame:A",s.entry_frame_id)
        self.assertEqual(2,s.selection.focus_scalar)
        self.assertEqual(0,tr.lifecycle_document_mutation_count)
        self.assertFalse(tr.undo_group_boundary)

    def test_new_empty_textbox_enters_at_zero_without_fake_visual_stop(self):
        d,m,f=context("story:new","")
        tr=enter_text_edit_session_v1(
            session_id="session:new",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:new","frame:new"),),
            revision_id="rev:1",domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(0),
        )
        self.assertEqual(0,tr.session.selection.focus_scalar)
        self.assertEqual("layout_pending",tr.session.selection.projection_state)
        self.assertIsNotNone(tr.session.typing_state)
        self.assertEqual(0,tr.lifecycle_document_mutation_count)

    def test_same_linked_story_frame_handoff_keeps_session_identity(self):
        d,m,f=context("story:1","AB",linked=True)
        entered=enter_text_edit_session_v1(
            session_id="session:1",incarnation=4,document_id="doc:1",
            entry_candidates=(candidate("story:1","frame:A"),),
            revision_id="rev:1",domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            pointer_context=TextPointerEntryContextV1("page:1",0,10),
        ).session
        handoff=handoff_same_story_frame_v1(
            entered,
            entry_candidate=candidate("story:1","frame:B"),
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            pointer_context=TextPointerEntryContextV1("page:1",10,40),
        )
        self.assertEqual("session:1",handoff.session.session_id)
        self.assertEqual(4,handoff.session.incarnation)
        self.assertEqual("story:1",handoff.session.story_id)
        self.assertEqual("frame:B",handoff.session.current_frame_id)
        self.assertEqual(2,handoff.session.selection.focus_scalar)
        self.assertFalse(handoff.focus_context_discontinuity)

    def test_switch_story_restarts_incarnation_and_closes_undo_context(self):
        d1,m1,f1=context("story:1","A")
        s1=enter_text_edit_session_v1(
            session_id="session:1",incarnation=2,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),revision_id="rev:1",
            domain=d1,caret_map=m1,format_state=f1,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(1),
        ).session
        d2,m2,f2=context("story:2","B")
        switched=switch_text_edit_session_v1(
            s1,entry_candidates=(candidate("story:2","frame:2"),),
            revision_id="rev:2",domain=d2,caret_map=m2,format_state=f2,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(0),
        )
        self.assertEqual("story_switch",switched.kind)
        self.assertEqual("story:1",switched.previous_story_id)
        self.assertEqual("story:2",switched.current_story_id)
        self.assertEqual("session:1",switched.session.session_id)
        self.assertEqual(3,switched.session.incarnation)
        self.assertTrue(switched.focus_context_discontinuity)
        self.assertTrue(switched.undo_group_boundary)
        self.assertEqual(0,switched.lifecycle_document_mutation_count)

    def test_same_story_must_use_handoff_not_switch_restart(self):
        d,m,f=context("story:1","A")
        s=enter_text_edit_session_v1(
            session_id="session:1",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),revision_id="rev:1",
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(0),
        ).session
        with self.assertRaises(TextEditSessionError) as caught:
            switch_text_edit_session_v1(
                s,entry_candidates=(candidate("story:1","frame:B"),),
                revision_id="rev:1",domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                initial_position=TextInitialPositionV1(1),
            )
        self.assertEqual("same_story_handoff_required",caught.exception.code)

    def test_active_composition_fences_switch_and_exit_until_resolved(self):
        d1,m1,f1=context("story:1","A")
        base=enter_text_edit_session_v1(
            session_id="session:1",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),revision_id="rev:1",
            domain=d1,caret_map=m1,format_state=f1,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(1),
        ).session
        comp=TextCompositionSessionV1(
            protocol_version="chaptera.text-composition-session.v1",
            composition_id="ime:1",story_id="story:1",base_revision_id="rev:1",
            edit_domain_id=base.edit_domain_id,start_scalar=1,end_scalar=1,
            expected_before="",captured_selection=base.selection,typing_snapshot=None,
            provisional_external_text="",provisional_selection_start_scalar=0,
            provisional_selection_end_scalar=0,
        )
        active=attach_text_composition_v1(base,comp)
        with self.assertRaises(TextEditSessionError) as switch_err:
            d2,m2,f2=context("story:2","B")
            switch_text_edit_session_v1(
                active,entry_candidates=(candidate("story:2"),),
                revision_id="rev:2",domain=d2,caret_map=m2,format_state=f2,
                expected_layout_revision_id="layout:1",
                initial_position=TextInitialPositionV1(0),
            )
        self.assertEqual("composition_transition_required",switch_err.exception.code)
        with self.assertRaises(TextEditSessionError) as exit_err:
            exit_text_edit_session_v1(active,reason="canvas")
        self.assertEqual("composition_transition_required",exit_err.exception.code)

        exit_receipt=exit_text_edit_session_v1(
            active,reason="canvas",composition_resolution="cancelled"
        )
        self.assertEqual("cancelled",exit_receipt.composition_resolution)
        self.assertEqual(0,exit_receipt.lifecycle_document_mutation_count)

    def test_exit_does_not_cancel_already_submitted_durable_operations(self):
        d,m,f=context("story:1","A")
        s=enter_text_edit_session_v1(
            session_id="session:1",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),revision_id="rev:1",
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(0),
        ).session
        receipt=exit_text_edit_session_v1(
            s,reason="inspector",
            submitted_durable_operation_ids=("op:pending-1","op:pending-2"),
        )
        self.assertEqual(
            ("op:pending-1","op:pending-2"),
            receipt.still_pending_operation_ids,
        )
        self.assertTrue(receipt.undo_group_boundary)
        self.assertEqual(0,receipt.lifecycle_document_mutation_count)

    def test_reentry_same_story_does_not_resurrect_old_caret_or_typing(self):
        d,m,f=context("story:1","ABC")
        first=enter_text_edit_session_v1(
            session_id="session:1",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),revision_id="rev:1",
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(3),
        ).session
        exit_text_edit_session_v1(first,reason="canvas")
        second=enter_text_edit_session_v1(
            session_id="session:2",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),revision_id="rev:1",
            domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(0),
        ).session
        self.assertEqual(3,first.selection.focus_scalar)
        self.assertEqual(0,second.selection.focus_scalar)
        self.assertNotEqual(first.session_id,second.session_id)
        self.assertEqual((),second.typing_state.pending_explicit_properties)

    def test_ambiguous_readonly_and_unknown_domain_entry_fail_closed(self):
        d,m,f=context("story:1","A")
        with self.assertRaises(TextEditSessionError) as ambiguous:
            enter_text_edit_session_v1(
                session_id="s",incarnation=0,document_id="d",
                entry_candidates=(candidate("story:1"),candidate("story:1","frame:B")),
                revision_id="rev:1",domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                initial_position=TextInitialPositionV1(0),
            )
        self.assertEqual("entry_owner_ambiguous",ambiguous.exception.code)
        with self.assertRaises(TextEditSessionError) as readonly:
            enter_text_edit_session_v1(
                session_id="s",incarnation=0,document_id="d",
                entry_candidates=(candidate("story:1",capability="read_only"),),
                revision_id="rev:1",domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                initial_position=TextInitialPositionV1(0),
            )
        self.assertEqual("read_only_story",readonly.exception.code)

        du=derive_story_edit_domain_v1(
            story_id="story:1",story_text="A\r",provenance="imported_unknown"
        )
        mu=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",story_id="story:1",story_scalar_len=2,
            lines=(line("story:1","u1",0,(cluster(0,1,0,10),cluster(1,2,10,20))),),
        )
        fu=build_text_format_overlay_state_v1(
            story_id="story:1",base_revision_id="rev:1",story_scalar_len=2,
            base_runs=(BaseFormatRunV1(
                0,2,BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")
            ),),
        )
        with self.assertRaises(TextEditSessionError) as unknown:
            enter_text_edit_session_v1(
                session_id="s",incarnation=0,document_id="d",
                entry_candidates=(candidate("story:1"),),revision_id="rev:1",
                domain=du,caret_map=mu,format_state=fu,
                expected_layout_revision_id="layout:1",
                initial_position=TextInitialPositionV1(0),
            )
        self.assertEqual("edit_domain_unknown",unknown.exception.code)

    def test_authority_rebind_accepts_current_selection_and_typing_receipts(self):
        d1,m1,f1=context("story:1","A",layout="layout:1")
        session=enter_text_edit_session_v1(
            session_id="session:1",incarnation=0,document_id="doc:1",
            entry_candidates=(candidate("story:1"),),revision_id="rev:1",
            domain=d1,caret_map=m1,format_state=f1,
            expected_layout_revision_id="layout:1",
            initial_position=TextInitialPositionV1(1),
        ).session

        d2,m2,f2=context("story:1","AB",layout="layout:2")
        sel=build_text_selection_state_v1(
            domain=d2,revision_id="rev:2",anchor_scalar=2,focus_scalar=2
        )
        typing=derive_typing_format_state_v1(
            selection=sel,domain=d2,format_state=f2
        )
        rebound=rebind_text_edit_session_authority_v1(
            session,revision_id="rev:2",domain=d2,caret_map=m2,format_state=f2,
            selection=sel,typing_state=typing,
            expected_layout_revision_id="layout:2",
        )
        self.assertEqual("rev:2",rebound.session.revision_id)
        self.assertEqual("layout:2",rebound.session.layout_revision_id)
        self.assertEqual(2,rebound.session.selection.focus_scalar)
        self.assertFalse(rebound.focus_context_discontinuity)


if __name__=="__main__":
    unittest.main()
