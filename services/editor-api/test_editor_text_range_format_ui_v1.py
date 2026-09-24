#!/usr/bin/env python3
import unittest

from editor_text_range_format_ui_v1 import (
    EditorTextRangeFormatUIError,
    apply_editor_text_format_control_v1,
    build_editor_keyboard_delete_request_v1,
    build_editor_text_input_request_v1,
    build_editor_text_range_format_ui_state_v1,
    route_editor_text_navigation_v1,
)
from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
    caret_map_hash_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_edit_session_v1 import TextEditSessionV1
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    replay_text_format_operation_v1,
    undo_text_format_operation_v1,
)
from text_selection_state_v1 import (
    build_text_selection_state_v1,
    edit_domain_id_v1,
    project_selection_state_v1,
)
from text_typing_format_state_v1 import derive_typing_format_state_v1


STORY="story:1"
DOC="doc:1"
SOURCE="d"*64


def cluster(a,b,x0,x1):
    return ResolvedClusterV1(a,b,x0,x1,x0,x1,True,())


def line(line_id,ordinal,clusters,*,frame="frame:A",prev=None,next=None,y=0):
    return ResolvedLineFragmentV1(
        story_id=STORY,page_id="page:1",frame_id=frame,line_id=line_id,
        flow_ordinal=ordinal,previous_line_id=prev,next_line_id=next,
        page_y_top_emu=y,page_y_bottom_emu=y+20,
        frame_y_top_emu=y,frame_y_bottom_emu=y+20,
        clusters=tuple(clusters),
    )


def domain(text):
    return derive_story_edit_domain_v1(
        story_id=STORY,story_text=text,provenance="chaptera_created"
    )


def caret_map(text,*,placed_len=None,linked=False):
    if placed_len is None:
        placed_len=len(text)
    if not text or placed_len==0:
        lines=()
    elif linked:
        lines=(
            line("l1",0,(cluster(0,1,0,10),),frame="frame:A",next="l2"),
            line(
                "l2",1,
                tuple(cluster(i,i+1,(i-1)*10,i*10) for i in range(1,placed_len)),
                frame="frame:B",prev="l1",y=30,
            ),
        )
    else:
        split=min(3,placed_len)
        if placed_len<=3:
            lines=(line(
                "l1",0,tuple(cluster(i,i+1,i*10,(i+1)*10) for i in range(placed_len))
            ),)
        else:
            lines=(
                line(
                    "l1",0,
                    tuple(cluster(i,i+1,i*10,(i+1)*10) for i in range(split)),
                    next="l2",y=0,
                ),
                line(
                    "l2",1,
                    tuple(cluster(i,i+1,(i-split)*10,(i-split+1)*10) for i in range(split,placed_len)),
                    prev="l1",y=30,
                ),
            )
    return build_resolved_text_caret_map_v1(
        layout_revision_id="layout:1",story_id=STORY,
        story_scalar_len=len(text),lines=lines
    )


def fmt(*,bold=False,italic=False,size=12000,color="#000000"):
    return BaseCharacterFormatV1("font:resolved",size,bold,italic,color)


def format_state(text,*,base_runs=None,overrides=()):
    if base_runs is None:
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),fmt()),)
    return build_text_format_overlay_state_v1(
        story_id=STORY,base_revision_id="rev:1",story_scalar_len=len(text),
        base_runs=tuple(base_runs),overrides=tuple(overrides)
    )


def stop(m,scalar,line_id=None):
    hits=[
        s for s in m.caret_stops
        if s.scalar_boundary==scalar and (line_id is None or s.line_id==line_id)
    ]
    assert len(hits)==1,(scalar,line_id,[(s.scalar_boundary,s.line_id) for s in hits])
    return hits[0]


def session(text,m,fstate,a,f,*,anchor_line=None,focus_line=None,pending=()):
    d=domain(text)
    s=build_text_selection_state_v1(
        domain=d,revision_id="rev:1",anchor_scalar=a,focus_scalar=f
    )
    ahits=[
        x for x in m.caret_stops
        if x.scalar_boundary==a and (anchor_line is None or x.line_id==anchor_line)
    ]
    fhits=[
        x for x in m.caret_stops
        if x.scalar_boundary==f and (focus_line is None or x.line_id==focus_line)
    ]
    if len(ahits)==1 and len(fhits)==1:
        s=project_selection_state_v1(
            state=s,domain=d,caret_map=m,
            anchor_stop_id=ahits[0].stop_id,focus_stop_id=fhits[0].stop_id,
        ).state
    typing=derive_typing_format_state_v1(
        selection=s,domain=d,format_state=fstate
    )
    if typing is not None and pending:
        from text_typing_format_state_v1 import set_pending_typing_property_v1
        for prop,value in pending:
            typing=set_pending_typing_property_v1(state=typing,prop=prop,value=value)
    return TextEditSessionV1(
        protocol_version="chaptera.text-edit-session.v1",
        session_id="session:1",incarnation=0,document_id=DOC,
        story_id=STORY,revision_id="rev:1",
        edit_domain_id=edit_domain_id_v1(d),
        layout_revision_id=m.layout_revision_id,
        caret_map_hash=caret_map_hash_v1(m),
        entry_frame_id="frame:A",current_frame_id="frame:A",
        focus_owner="story_text",selection=s,typing_state=typing,
        composition_session=None,pending_interaction_metadata=(),
    )


def control(ui,prop):
    return [x for x in ui.controls if x.property==prop][0]


class EditorTextRangeFormatUIV1Tests(unittest.TestCase):
    def test_range_controls_distinguish_inherited_explicit_and_mixed(self):
        text="ABCD"
        m=caret_map(text)
        fs=format_state(
            text,
            base_runs=(
                BaseFormatRunV1(0,2,fmt(bold=False)),
                BaseFormatRunV1(2,4,fmt(bold=True)),
            ),
            overrides=(TextFormatOverrideRunV1(0,1,"italic",True),),
        )
        s=session(text,m,fs,0,4,anchor_line="l1",focus_line="l2")
        ui=build_editor_text_range_format_ui_state_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs
        )
        self.assertEqual("range",ui.selection_kind)
        self.assertEqual("mixed_effective",control(ui,"bold").display_state)
        self.assertEqual("mixed_effective",control(ui,"italic").display_state)
        self.assertEqual("inherited_base",control(ui,"font_size_emu").display_state)

    def test_mixed_bold_press_resolves_now_to_explicit_true_not_toggle_command(self):
        text="ABCD"
        m=caret_map(text)
        fs=format_state(
            text,
            base_runs=(
                BaseFormatRunV1(0,2,fmt(bold=False)),
                BaseFormatRunV1(2,4,fmt(bold=True)),
            ),
        )
        s=session(text,m,fs,0,4,anchor_line="l1",focus_line="l2")
        r=apply_editor_text_format_control_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs,
            prop="bold",action="press",
        )
        self.assertEqual("range_operation",r.status)
        self.assertEqual("set_text_format_property",r.durable_command_kind)
        self.assertEqual(True,r.range_receipt.command["value"])
        self.assertNotEqual("toggle",r.range_receipt.command["kind"])
        self.assertTrue(r.requires_authoritative_relayout)
        self.assertEqual(fs,undo_text_format_operation_v1(r.range_receipt))
        self.assertEqual(
            r.range_receipt.after_state,replay_text_format_operation_v1(r.range_receipt)
        )

    def test_uniform_true_bold_press_becomes_explicit_false(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(text,base_runs=(BaseFormatRunV1(0,3,fmt(bold=True)),))
        s=session(text,m,fs,0,3)
        r=apply_editor_text_format_control_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs,
            prop="bold",action="press",
        )
        self.assertFalse(r.range_receipt.command["value"])
        self.assertEqual("set_text_format_property",r.range_receipt.command["kind"])

    def test_clear_override_reveals_base_instead_of_writing_default(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(
            text,
            base_runs=(BaseFormatRunV1(0,3,fmt(bold=True)),),
            overrides=(TextFormatOverrideRunV1(0,3,"bold",False),),
        )
        s=session(text,m,fs,0,3)
        before=build_editor_text_range_format_ui_state_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs
        )
        self.assertEqual("explicit",control(before,"bold").display_state)
        r=apply_editor_text_format_control_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs,
            prop="bold",action="clear_override",
        )
        self.assertEqual("clear_text_format_property_override",r.durable_command_kind)
        after=build_editor_text_range_format_ui_state_v1(
            session=s,domain=domain(text),caret_map=m,
            format_state=r.range_receipt.after_state,
        )
        self.assertEqual("inherited_base",control(after,"bold").display_state)
        self.assertTrue(control(after,"bold").effective_value)

    def test_font_size_and_color_set_are_canonical_range_operations(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(text)
        s=session(text,m,fs,0,3)
        size=apply_editor_text_format_control_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs,
            prop="font_size_emu",action="set",value=18000,
        )
        self.assertEqual(18000,size.range_receipt.command["value"])
        color=apply_editor_text_format_control_v1(
            session=s,domain=domain(text),caret_map=m,
            format_state=size.range_receipt.after_state,
            prop="text_color_rgb",action="set",value="#aa00cc",
        )
        self.assertEqual("#AA00CC",color.range_receipt.command["value"])

    def test_collapsed_bold_changes_typing_state_only_no_zero_length_span(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(text)
        s=session(text,m,fs,1,1)
        r=apply_editor_text_format_control_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs,
            prop="bold",action="press",
        )
        self.assertEqual("typing_state",r.status)
        self.assertIsNone(r.range_receipt)
        self.assertIn(("bold",True),r.typing_state.pending_explicit_properties)
        self.assertEqual(0,r.authoring_revision_count)
        self.assertFalse(r.zero_length_durable_span_created)

    def test_collapsed_explicit_false_is_distinct_from_inherit_and_clear_restores_inherit(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(text,base_runs=(BaseFormatRunV1(0,3,fmt(bold=True)),))
        s=session(text,m,fs,1,1)
        off=apply_editor_text_format_control_v1(
            session=s,domain=domain(text),caret_map=m,format_state=fs,
            prop="bold",action="press",
        )
        self.assertIn(("bold",False),off.typing_state.pending_explicit_properties)
        s2=replace_session_typing(s,off.typing_state)
        ui=build_editor_text_range_format_ui_state_v1(
            session=s2,domain=domain(text),caret_map=m,format_state=fs
        )
        self.assertEqual("explicit",control(ui,"bold").display_state)
        cleared=apply_editor_text_format_control_v1(
            session=s2,domain=domain(text),caret_map=m,format_state=fs,
            prop="bold",action="clear_override",
        )
        self.assertEqual((),cleared.typing_state.pending_explicit_properties)

    def test_unplaced_nonempty_range_can_format_but_collapsed_unplaced_cannot(self):
        text="ABCDE"
        m=caret_map(text,placed_len=3)
        fs=format_state(text)
        ranged=session(text,m,fs,3,5)
        ui=build_editor_text_range_format_ui_state_v1(
            session=ranged,domain=domain(text),caret_map=m,format_state=fs
        )
        self.assertEqual("unplaced",ui.geometry_state)
        self.assertTrue(ui.range_format_available)
        r=apply_editor_text_format_control_v1(
            session=ranged,domain=domain(text),caret_map=m,format_state=fs,
            prop="italic",action="press",
        )
        self.assertEqual("range_operation",r.status)

        collapsed=session(text,m,fs,5,5)
        ui2=build_editor_text_range_format_ui_state_v1(
            session=collapsed,domain=domain(text),caret_map=m,format_state=fs
        )
        self.assertFalse(ui2.direct_edit_available)
        self.assertEqual("unsupported",ui2.status)
        blocked=apply_editor_text_format_control_v1(
            session=collapsed,domain=domain(text),caret_map=m,format_state=fs,
            prop="italic",action="press",
        )
        self.assertEqual("unsupported",blocked.status)

    def test_linked_frame_map_is_explicit_non_goal_for_this_ui_slice(self):
        text="AB"
        m=caret_map(text,linked=True)
        fs=format_state(text)
        s=session(text,m,fs,0,0,anchor_line="l1",focus_line="l1")
        with self.assertRaises(EditorTextRangeFormatUIError) as caught:
            build_editor_text_range_format_ui_state_v1(
                session=s,domain=domain(text),caret_map=m,format_state=fs
            )
        self.assertEqual("linked_story_ui_unsupported",caught.exception.code)

    def test_keyboard_navigation_delegates_canonical_policy_and_clears_pending_typing(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(text)
        s=session(text,m,fs,1,1,pending=(("italic",True),))
        r=route_editor_text_navigation_v1(
            session=s,command="move_next",story_text=text,
            domain=domain(text),caret_map=m,format_state=fs,
        )
        self.assertEqual("selection",r.status)
        self.assertEqual((2,2),r.session.selection.normalized_range)
        self.assertEqual((),r.session.typing_state.pending_explicit_properties)
        self.assertTrue(r.typing_state_cleared)

    def test_visual_line_navigation_preserves_preferred_x_from_canonical_policy(self):
        text="ABCDEF"
        m=caret_map(text)
        fs=format_state(text)
        s=session(text,m,fs,2,2,anchor_line="l1",focus_line="l1")
        r=route_editor_text_navigation_v1(
            session=s,command="move_visual_line_down",story_text=text,
            domain=domain(text),caret_map=m,format_state=fs,
        )
        self.assertEqual("selection",r.status)
        self.assertEqual("l2",stop(m,r.session.selection.focus_scalar,"l2").line_id)
        self.assertIsNotNone(r.session.selection.preferred_inline_x_emu)

    def test_external_input_uses_text_ingress_and_pending_typing_snapshot(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(text)
        s=session(text,m,fs,1,1,pending=(("bold",True),))
        plan=build_editor_text_input_request_v1(
            session=s,story_text=text,domain=domain(text),caret_map=m,
            format_state=fs,source_hash=SOURCE,
            client_operation_id="typing-input-0001",
            external_text="X\nY",
            paragraph_inserted_ids=("p:new",),
        )
        self.assertEqual("ready",plan.status)
        self.assertEqual("X\rY",plan.canonical_replacement_text)
        self.assertEqual("X\rY",plan.request["command"]["replacement_text"])
        self.assertEqual({"bold":True},plan.request["command"]["typing_format"])
        self.assertEqual(["p:new"],plan.request["command"]["paragraph_inserted_ids"])
        self.assertEqual("collapse_after_edit",plan.post_edit_selection_intent)

    def test_keyboard_delete_lowers_exact_policy_range_to_story_transaction(self):
        text="ABC"
        m=caret_map(text)
        fs=format_state(text)
        s=session(text,m,fs,2,2)
        nav=route_editor_text_navigation_v1(
            session=s,command="delete_backward",story_text=text,
            domain=domain(text),caret_map=m,format_state=fs,
        )
        self.assertEqual("delete",nav.status)
        plan=build_editor_keyboard_delete_request_v1(
            navigation=nav,story_text=text,source_hash=SOURCE,
            client_operation_id="keyboard-delete-0001",
        )
        self.assertEqual((1,2),(plan.start_scalar,plan.end_scalar))
        self.assertEqual("B",plan.request["command"]["expected_before"])
        self.assertEqual("",plan.request["command"]["replacement_text"])


def replace_session_typing(s,typing):
    from dataclasses import replace
    return replace(s,typing_state=typing)


if __name__=="__main__":
    unittest.main()
