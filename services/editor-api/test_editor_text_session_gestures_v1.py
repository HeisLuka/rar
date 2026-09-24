#!/usr/bin/env python3
import unittest

from editor_text_session_gestures_v1 import (
    EditorTextSessionGesturesError,
    activate_explicit_edit_text_v1,
    activate_pointer_text_v1,
    exit_desktop_text_mode_v1,
    story_shortcut_admitted_v1,
)
from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_composition_session_v1 import TextCompositionSessionV1
from text_edit_session_v1 import (
    TextEntryCandidateV1,
    TextPointerEntryContextV1,
    attach_text_composition_v1,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)


DOC="doc:1"


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


def context(story,text,*,linked=False,placed_len=None,layout="layout:1"):
    d=derive_story_edit_domain_v1(
        story_id=story,story_text=text,provenance="chaptera_created"
    )
    if not text:
        lines=()
    elif linked and len(text)>=2:
        lines=(
            line(story,"l1",0,(cluster(0,1,0,10),),frame="frame:A",next="l2",y=0),
            line(
                story,"l2",1,
                tuple(cluster(i,i+1,(i-1)*10,i*10) for i in range(1,len(text))),
                frame="frame:B",prev="l1",y=30,
            ),
        )
    else:
        n=len(text) if placed_len is None else placed_len
        lines=(line(
            story,"l1",0,
            tuple(cluster(i,i+1,i*10,(i+1)*10) for i in range(n)),
        ),) if n else ()
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
        target_id=frame or story,
        story_id=story,
        frame_id=frame,
        capability=capability,
    )


class EditorTextSessionGesturesV1Tests(unittest.TestCase):
    def test_inactive_plain_single_click_remains_canvas_object_selection(self):
        d,m,f=context("story:1","ABC")
        r=activate_pointer_text_v1(
            candidate=candidate("story:1"),
            revision_id="rev:1",domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            pointer_context=TextPointerEntryContextV1("page:1",19,10),
            active_session=None,
            explicit_edit_requested=False,
            document_id=DOC,
        )
        self.assertEqual("canvas_object_selection",r.status)
        self.assertIsNone(r.active_session)
        self.assertTrue(r.canvas_object_shortcuts_owned)
        self.assertEqual(0,r.document_mutation_count)

    def test_explicit_edit_text_enters_at_authoritative_first_frame_stop(self):
        d,m,f=context("story:1","ABC")
        r=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual("entered",r.status)
        self.assertEqual(0,r.active_session.selection.focus_scalar)
        self.assertEqual("frame:A",r.active_session.current_frame_id)
        self.assertTrue(r.story_shortcuts_owned)
        self.assertFalse(r.canvas_object_shortcuts_owned)
        self.assertEqual(0,r.transition.lifecycle_document_mutation_count)

    def test_explicit_edit_text_on_linked_second_frame_reuses_session_and_relocates_there(self):
        d,m,f=context("story:1","AB",linked=True)
        first=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1","frame:A"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        ).active_session
        second=activate_explicit_edit_text_v1(
            session_id="ignored",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1","frame:B"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            active_session=first,
        )
        self.assertEqual("same_story_handoff",second.status)
        self.assertEqual(first.session_id,second.active_session.session_id)
        self.assertEqual(first.incarnation,second.active_session.incarnation)
        self.assertEqual("frame:B",second.active_session.current_frame_id)
        self.assertEqual(1,second.active_session.selection.focus_scalar)

    def test_pointer_activation_in_active_text_mode_uses_exact_target_frame_hit(self):
        d,m,f=context("story:1","AB",linked=True)
        active=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1","frame:A"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        ).active_session
        r=activate_pointer_text_v1(
            candidate=candidate("story:1","frame:B"),
            revision_id="rev:1",domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            pointer_context=TextPointerEntryContextV1("page:1",10,40),
            active_session=active,
        )
        self.assertEqual("same_story_handoff",r.status)
        self.assertEqual("frame:B",r.active_session.current_frame_id)
        self.assertEqual(2,r.active_session.selection.focus_scalar)

    def test_pointer_target_mismatch_does_not_snap_to_other_frame(self):
        d,m,f=context("story:1","A")
        r=activate_pointer_text_v1(
            candidate=candidate("story:1","frame:B"),
            revision_id="rev:1",domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
            pointer_context=TextPointerEntryContextV1("page:1",5,200),
            active_session=None,
            explicit_edit_requested=True,
            document_id=DOC,
        )
        self.assertEqual("direct_edit_unavailable",r.status)
        self.assertIsNone(r.active_session)

    def test_different_story_activation_switches_one_session_not_two(self):
        d1,m1,f1=context("story:1","A")
        active=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1"),domain=d1,caret_map=m1,format_state=f1,
            expected_layout_revision_id="layout:1",
        ).active_session
        d2,m2,f2=context("story:2","B")
        r=activate_pointer_text_v1(
            candidate=candidate("story:2"),
            revision_id="rev:2",domain=d2,caret_map=m2,format_state=f2,
            expected_layout_revision_id="layout:1",
            pointer_context=TextPointerEntryContextV1("page:1",0,10),
            active_session=active,
        )
        self.assertEqual("story_switch",r.status)
        self.assertEqual("story:2",r.active_session.story_id)
        self.assertEqual(active.session_id,r.active_session.session_id)
        self.assertEqual(active.incarnation+1,r.active_session.incarnation)

    def test_nonempty_unplaced_target_cannot_gain_invisible_caret(self):
        d,m,f=context("story:1","ABCDE",placed_len=3)
        with self.assertRaises(EditorTextSessionGesturesError) as caught:
            activate_explicit_edit_text_v1(
                session_id="session:1",document_id=DOC,revision_id="rev:1",
                candidate=candidate("story:1","frame:B"),domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
            )
        self.assertEqual("direct_edit_unavailable",caught.exception.code)

    def test_double_triple_click_semantics_are_explicitly_gated(self):
        d,m,f=context("story:1","ABC")
        for count in (2,3):
            r=activate_pointer_text_v1(
                candidate=candidate("story:1"),
                revision_id="rev:1",domain=d,caret_map=m,format_state=f,
                expected_layout_revision_id="layout:1",
                pointer_context=TextPointerEntryContextV1("page:1",10,10),
                active_session=None,
                explicit_edit_requested=True,
                click_count=count,
                document_id=DOC,
            )
            self.assertEqual("multiclick_unavailable",r.status)
            self.assertIsNone(r.active_session)

    def test_find_inspector_modal_focus_fences_story_shortcuts(self):
        d,m,f=context("story:1","A")
        active=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        ).active_session
        self.assertTrue(story_shortcut_admitted_v1(
            active_session=active,focus_owner="story_text"
        ))
        for owner in ("inspector","find_replace","modal","canvas"):
            self.assertFalse(story_shortcut_admitted_v1(
                active_session=active,focus_owner=owner
            ))

    def test_escape_owned_by_find_field_does_not_exit_story_session(self):
        d,m,f=context("story:1","A")
        active=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        ).active_session
        r=exit_desktop_text_mode_v1(
            active_session=active,
            trigger="escape",
            focus_owner="find_replace",
        )
        self.assertEqual("host_focus_owned",r.status)
        self.assertIs(active,r.active_session)
        self.assertFalse(r.transient_text_state_cleared)

    def test_escape_story_focus_exits_and_clears_transient_session_state_only(self):
        d,m,f=context("story:1","ABC")
        active=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        ).active_session
        r=exit_desktop_text_mode_v1(
            active_session=active,
            trigger="escape",
            focus_owner="story_text",
            submitted_durable_operation_ids=("op:already-submitted",),
        )
        self.assertEqual("exited",r.status)
        self.assertIsNone(r.active_session)
        self.assertTrue(r.transient_text_state_cleared)
        self.assertEqual(("op:already-submitted",),r.exit_receipt.still_pending_operation_ids)
        self.assertEqual(0,r.document_mutation_count)

    def test_active_composition_requires_resolution_before_exit_then_can_cancel(self):
        d,m,f=context("story:1","A")
        base=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        ).active_session
        comp=TextCompositionSessionV1(
            protocol_version="chaptera.text-composition-session.v1",
            composition_id="ime:1",story_id="story:1",base_revision_id="rev:1",
            edit_domain_id=base.edit_domain_id,start_scalar=0,end_scalar=0,
            expected_before="",captured_selection=base.selection,typing_snapshot=None,
            provisional_external_text="",provisional_selection_start_scalar=0,
            provisional_selection_end_scalar=0,
        )
        active=attach_text_composition_v1(base,comp)
        blocked=exit_desktop_text_mode_v1(
            active_session=active,
            trigger="canvas_non_text_click",
            focus_owner="story_text",
        )
        self.assertEqual("composition_resolution_required",blocked.status)
        self.assertIs(active,blocked.active_session)
        exited=exit_desktop_text_mode_v1(
            active_session=active,
            trigger="canvas_non_text_click",
            focus_owner="story_text",
            composition_resolution="cancelled",
        )
        self.assertEqual("exited",exited.status)
        self.assertEqual("cancelled",exited.exit_receipt.composition_resolution)

    def test_switching_non_text_tool_exits_without_document_revision(self):
        d,m,f=context("story:1","A")
        active=activate_explicit_edit_text_v1(
            session_id="session:1",document_id=DOC,revision_id="rev:1",
            candidate=candidate("story:1"),domain=d,caret_map=m,format_state=f,
            expected_layout_revision_id="layout:1",
        ).active_session
        r=exit_desktop_text_mode_v1(
            active_session=active,
            trigger="non_text_tool",
            focus_owner="story_text",
        )
        self.assertEqual("exited",r.status)
        self.assertEqual(0,r.exit_receipt.lifecycle_document_mutation_count)
        self.assertEqual(0,r.document_mutation_count)


if __name__=="__main__":
    unittest.main()
