#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_selection_state_v1 import (
    build_text_selection_state_v1,
    project_selection_state_v1,
)
from text_visual_line_navigation_v1 import navigate_visual_line_v1


STORY="story:1"


def cluster(a,b,px0,px1,fx0=None,fx1=None):
    return ResolvedClusterV1(
        a,b,px0,px1,
        px0 if fx0 is None else fx0,
        px1 if fx1 is None else fx1,
        True,(),
    )


def line(line_id,ordinal,clusters,*,frame="frame:A",page="page:1",prev=None,next=None,y=0):
    return ResolvedLineFragmentV1(
        story_id=STORY,
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


def domain(text,provenance="chaptera_created"):
    return derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=text,
        provenance=provenance,
    )


def cmap(text,lines,layout="layout:1"):
    return build_resolved_text_caret_map_v1(
        layout_revision_id=layout,
        story_id=STORY,
        story_scalar_len=len(text),
        lines=tuple(lines),
    )


def stop(m,scalar,line_id=None,x=None):
    matches=[
        s for s in m.caret_stops
        if s.scalar_boundary==scalar
        and (line_id is None or s.line_id==line_id)
        and (x is None or s.frame_x_emu==x)
    ]
    assert len(matches)==1,(scalar,line_id,x,[(s.scalar_boundary,s.line_id,s.frame_x_emu) for s in matches])
    return matches[0]


def state(text,m,a,f,*,anchor_line=None,focus_line=None,preferred=None,project=True):
    d=domain(text)
    s=build_text_selection_state_v1(
        domain=d,
        revision_id="rev:1",
        anchor_scalar=a,
        focus_scalar=f,
        preferred_inline_x_emu=preferred,
    )
    if not project:
        return s
    astop=stop(m,a,anchor_line).stop_id
    fstop=stop(m,f,focus_line).stop_id
    return project_selection_state_v1(
        state=s,
        domain=d,
        caret_map=m,
        anchor_stop_id=astop,
        focus_stop_id=fstop,
    ).state


class TextVisualLineNavigationV1Tests(unittest.TestCase):
    def two_lines(self):
        text="ABCDEF"
        m=cmap(text,(
            line(
                "l1",0,
                (
                    cluster(0,1,0,10),
                    cluster(1,2,10,30),
                    cluster(2,3,30,80),
                ),
                next="l2",y=100,
            ),
            line(
                "l2",1,
                (
                    cluster(3,4,0,20),
                    cluster(4,5,20,45),
                    cluster(5,6,45,90),
                ),
                prev="l1",y=0,
            ),
        ))
        return text,m

    def test_home_end_use_current_visual_line_not_story_extent_or_scene_y(self):
        text,m=self.two_lines()
        d=domain(text)
        s=state(text,m,1,1,anchor_line="l1",focus_line="l1",preferred=77)
        home=navigate_visual_line_v1(
            command="move_line_start",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual("moved",home.status)
        self.assertEqual((0,0),home.selection.normalized_range)
        self.assertEqual("l1",home.target_line_id)
        self.assertIsNone(home.selection.preferred_inline_x_emu)

        end=navigate_visual_line_v1(
            command="move_line_end",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual((3,3),end.selection.normalized_range)
        self.assertEqual("l1",end.selection.focus_visual_stop_id.split(":stop:")[0])

    def test_extend_home_end_preserve_anchor_and_move_focus_only(self):
        text,m=self.two_lines()
        d=domain(text)
        s=state(text,m,1,2,anchor_line="l1",focus_line="l1")
        r=navigate_visual_line_v1(
            command="extend_line_end",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(1,r.selection.anchor_scalar)
        self.assertEqual(3,r.selection.focus_scalar)
        self.assertEqual(stop(m,1,"l1").stop_id,r.selection.anchor_visual_stop_id)
        self.assertEqual(stop(m,3,"l1").stop_id,r.selection.focus_visual_stop_id)

    def test_vertical_uses_story_flow_adjacency_not_scene_y_order(self):
        text,m=self.two_lines()
        d=domain(text)
        # l1 is visually lower in page-y than l2, but Story flow says l1 -> l2.
        s=state(text,m,2,2,anchor_line="l1",focus_line="l1")
        down=navigate_visual_line_v1(
            command="move_visual_line_down",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual("l2",down.target_line_id)
        self.assertEqual(5,down.selection.focus_scalar)
        up=navigate_visual_line_v1(
            command="move_visual_line_up",state=down.selection,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual("l1",up.target_line_id)

    def test_preferred_inline_x_is_frame_local_across_linked_frames(self):
        text="ABCDEFG"
        m=cmap(text,(
            line(
                "l1",0,
                (
                    cluster(0,1,500,530,0,30),
                    cluster(1,2,530,580,30,80),
                    cluster(2,3,580,600,80,100),
                ),
                frame="frame:A",page="page:1",next="l2",y=0,
            ),
            line(
                "l2",1,
                (
                    cluster(3,4,1500,1520,0,20),
                    cluster(4,5,1520,1550,20,50),
                    cluster(5,6,1550,1600,50,100),
                    cluster(6,7,1600,1620,100,120),
                ),
                frame="frame:B",page="page:2",prev="l1",y=0,
            ),
        ))
        d=domain(text)
        s=state(text,m,2,2,anchor_line="l1",focus_line="l1")
        r=navigate_visual_line_v1(
            command="move_visual_line_down",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(80,r.preferred_inline_x_emu)
        self.assertEqual("frame:B",stop(m,r.selection.focus_scalar,"l2").frame_id)
        self.assertEqual(100,stop(m,r.selection.focus_scalar,"l2").frame_x_emu)
        self.assertEqual(80,r.selection.preferred_inline_x_emu)

    def test_vertical_tie_break_is_lower_inline_x_then_scalar(self):
        text="ABCDE"
        m=cmap(text,(
            line(
                "l1",0,
                (cluster(0,1,0,50),cluster(1,2,50,100)),
                next="l2",y=0,
            ),
            line(
                "l2",1,
                (
                    cluster(2,3,0,40),
                    cluster(3,4,40,60),
                    cluster(4,5,60,100),
                ),
                prev="l1",y=30,
            ),
        ))
        d=domain(text)
        s=state(text,m,1,1,anchor_line="l1",focus_line="l1")
        r=navigate_visual_line_v1(
            command="move_visual_line_down",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(50,r.preferred_inline_x_emu)
        target=stop(m,r.selection.focus_scalar,"l2")
        self.assertEqual(40,target.frame_x_emu)
        self.assertEqual(3,target.scalar_boundary)

    def test_preferred_x_survives_short_line_then_next_vertical_move(self):
        text="ABCDEFG"
        m=cmap(text,(
            line(
                "l1",0,
                (cluster(0,1,0,50),cluster(1,2,50,100)),
                next="l2",y=0,
            ),
            line(
                "l2",1,
                (cluster(2,3,0,20),cluster(3,4,20,30)),
                prev="l1",next="l3",y=30,
            ),
            line(
                "l3",2,
                (
                    cluster(4,5,0,40),
                    cluster(5,6,40,100),
                    cluster(6,7,100,120),
                ),
                prev="l2",y=60,
            ),
        ))
        d=domain(text)
        s=state(text,m,2,2,anchor_line="l1",focus_line="l1",preferred=90)
        one=navigate_visual_line_v1(
            command="move_visual_line_down",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(90,one.selection.preferred_inline_x_emu)
        self.assertEqual(30,stop(m,one.selection.focus_scalar,"l2").frame_x_emu)
        two=navigate_visual_line_v1(
            command="move_visual_line_down",state=one.selection,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(90,two.selection.preferred_inline_x_emu)
        self.assertEqual(100,stop(m,two.selection.focus_scalar,"l3").frame_x_emu)

    def test_shift_vertical_preserves_anchor_and_preferred_x(self):
        text,m=self.two_lines()
        d=domain(text)
        s=state(text,m,0,2,anchor_line="l1",focus_line="l1")
        r=navigate_visual_line_v1(
            command="extend_visual_line_down",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(0,r.selection.anchor_scalar)
        self.assertEqual("l1",stop(m,r.selection.anchor_scalar,"l1").line_id)
        self.assertEqual("l2",r.target_line_id)
        self.assertIsNotNone(r.selection.preferred_inline_x_emu)

    def test_vertical_at_story_flow_boundary_is_transient_noop(self):
        text,m=self.two_lines()
        d=domain(text)
        s=state(text,m,1,1,anchor_line="l1",focus_line="l1")
        r=navigate_visual_line_v1(
            command="move_visual_line_up",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual("boundary_no_op",r.status)
        self.assertEqual((1,1),r.selection.normalized_range)
        self.assertEqual(stop(m,1,"l1").frame_x_emu,r.selection.preferred_inline_x_emu)
        self.assertEqual(0,r.authoring_mutation_count)

    def test_protected_terminal_source_scalar_is_not_home_end_target(self):
        text="AB\r"
        d=domain(text,"imported_mature_quill_terminal_cr")
        m=cmap(text,(
            line(
                "l1",0,
                (
                    cluster(0,1,0,10),
                    cluster(1,2,10,20),
                    cluster(2,3,20,30),
                ),
            ),
        ))
        # Explicitly project focus before the protected suffix.
        s=build_text_selection_state_v1(
            domain=d,revision_id="rev:1",anchor_scalar=1,focus_scalar=1
        )
        s=project_selection_state_v1(
            state=s,domain=d,caret_map=m,
            anchor_stop_id=stop(m,1,"l1").stop_id,
            focus_stop_id=stop(m,1,"l1").stop_id,
        ).state
        r=navigate_visual_line_v1(
            command="move_line_end",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual(2,r.selection.focus_scalar)
        self.assertNotEqual(3,r.selection.focus_scalar)

    def test_same_scalar_multi_stop_without_grounded_affinity_fails_closed(self):
        text="AB"
        m=cmap(text,(
            line("l1",0,(cluster(0,1,0,10),),next="l2",y=0),
            line("l2",1,(cluster(1,2,0,10),),prev="l1",y=30),
        ))
        d=domain(text)
        # Layout-pending semantic scalar 1 intentionally carries no physical stop id.
        s=state(text,m,1,1,project=False)
        r=navigate_visual_line_v1(
            command="move_visual_line_down",state=s,domain=d,caret_map=m,
            expected_layout_revision_id="layout:1",
        )
        self.assertEqual("navigation_unsupported",r.status)
        self.assertIn("caret_affinity_required",r.reason)
        self.assertEqual((1,1),r.selection.normalized_range)

    def test_stale_layout_or_edit_domain_requires_reconciliation_not_guessing(self):
        text,m=self.two_lines()
        d=domain(text)
        s=state(text,m,1,1,anchor_line="l1",focus_line="l1")
        stale_map=cmap(text,m.lines,layout="layout:2")
        r=navigate_visual_line_v1(
            command="move_line_end",state=s,domain=d,caret_map=stale_map,
            expected_layout_revision_id="layout:2",
        )
        self.assertEqual("reconcile_required",r.status)
        self.assertEqual((1,1),r.selection.normalized_range)

    def test_all_commands_create_zero_authoring_or_undo_entries(self):
        text,m=self.two_lines()
        d=domain(text)
        for command in (
            "move_line_start","move_line_end",
            "extend_line_start","extend_line_end",
            "move_visual_line_up","move_visual_line_down",
            "extend_visual_line_up","extend_visual_line_down",
        ):
            s=state(text,m,1,1,anchor_line="l1",focus_line="l1")
            r=navigate_visual_line_v1(
                command=command,state=s,domain=d,caret_map=m,
                expected_layout_revision_id="layout:1",
            )
            self.assertEqual(0,r.authoring_mutation_count,command)
            self.assertEqual(0,r.undo_history_entry_count,command)


if __name__=="__main__":
    unittest.main()
