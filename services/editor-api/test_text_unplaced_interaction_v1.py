#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_selection_state_v1 import build_text_selection_state_v1
from text_unplaced_interaction_v1 import (
    decide_text_interaction_admissibility_v1,
)


def cluster(a,b,x0,x1):
    return ResolvedClusterV1(a,b,x0,x1,x0,x1,True,())


def line(clusters):
    return ResolvedLineFragmentV1(
        story_id="story:1",
        page_id="page:1",
        frame_id="frame:1",
        line_id="line:1",
        flow_ordinal=0,
        previous_line_id=None,
        next_line_id=None,
        page_y_top_emu=0,
        page_y_bottom_emu=20,
        frame_y_top_emu=0,
        frame_y_bottom_emu=20,
        clusters=tuple(clusters),
    )


def cmap(story_len, placed_len):
    clusters=tuple(cluster(i,i+1,i*10,(i+1)*10) for i in range(placed_len))
    return build_resolved_text_caret_map_v1(
        layout_revision_id="layout:1",
        story_id="story:1",
        story_scalar_len=story_len,
        lines=(line(clusters),),
    )


def selection(text,a,f):
    domain=derive_story_edit_domain_v1(
        story_id="story:1",
        story_text=text,
        provenance="chaptera_created",
    )
    return build_text_selection_state_v1(
        domain=domain,
        revision_id="revision:1",
        anchor_scalar=a,
        focus_scalar=f,
    )


class TextUnplacedInteractionV1Tests(unittest.TestCase):
    def test_wholly_unplaced_nonempty_range_is_semantic_not_empty(self):
        m=cmap(5,3)
        s=selection("ABCDE",3,5)
        r=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=m,
            command_kind="find_replace",
            expected_layout_revision_id="layout:1",
        )
        self.assertTrue(r.admitted)
        self.assertEqual("semantic_range",r.mode)
        self.assertEqual("unplaced",r.geometry_state)
        self.assertTrue(r.selection_nonempty)
        self.assertEqual("semantic_range_unplaced",r.reason)

    def test_partial_selection_keeps_full_canonical_range_for_copy(self):
        m=cmap(5,3)
        s=selection("ABCDE",1,5)
        r=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=m,
            command_kind="copy",
            expected_layout_revision_id="layout:1",
        )
        self.assertTrue(r.admitted)
        self.assertEqual("partial",r.geometry_state)
        self.assertEqual("semantic_range_partial",r.reason)

    def test_direct_typing_at_unplaced_focus_fails_without_mutation(self):
        m=cmap(5,3)
        s=selection("ABCDE",3,5)
        r=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=m,
            command_kind="typing",
            expected_layout_revision_id="layout:1",
        )
        self.assertFalse(r.admitted)
        self.assertEqual("direct_edit_unavailable:unplaced",r.reason)
        self.assertEqual(0,r.authoring_mutation_count)
        self.assertEqual(0,r.undo_history_entry_count)

    def test_collapsed_unplaced_ime_target_is_fenced(self):
        m=cmap(5,3)
        s=selection("ABCDE",5,5)
        r=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=m,
            command_kind="ime_start",
            expected_layout_revision_id="layout:1",
        )
        self.assertFalse(r.admitted)
        self.assertEqual("direct_edit_unavailable:unplaced",r.reason)

    def test_placed_collapsed_caret_admits_direct_commands(self):
        m=cmap(5,3)
        s=selection("ABCDE",2,2)
        for command in ("typing","ime_start","collapsed_format","caret_navigation"):
            r=decide_text_interaction_admissibility_v1(
                selection=s,
                caret_map=m,
                command_kind=command,
                expected_layout_revision_id="layout:1",
            )
            self.assertTrue(r.admitted,command)
            self.assertIsNotNone(r.focus_stop_id)

    def test_semantic_delete_of_unplaced_range_is_admitted(self):
        m=cmap(5,3)
        s=selection("ABCDE",3,5)
        r=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=m,
            command_kind="explicit_range_delete",
            expected_layout_revision_id="layout:1",
        )
        self.assertTrue(r.admitted)
        self.assertEqual("unplaced",r.geometry_state)

    def test_reflow_recovery_uses_fresh_map_not_old_visibility_guess(self):
        old=cmap(5,3)
        s=selection("ABCDE",3,5)
        blocked=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=old,
            command_kind="typing",
            expected_layout_revision_id="layout:1",
        )
        self.assertFalse(blocked.admitted)

        # Fresh authoritative projection now materializes the former focus.
        fresh=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:2",
            story_id="story:1",
            story_scalar_len=5,
            lines=(line(tuple(cluster(i,i+1,i*10,(i+1)*10) for i in range(5))),),
        )
        admitted=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=fresh,
            command_kind="typing",
            expected_layout_revision_id="layout:2",
        )
        self.assertTrue(admitted.admitted)

    def test_no_last_visible_line_fabrication(self):
        m=cmap(5,3)
        s=selection("ABCDE",4,4)
        r=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=m,
            command_kind="caret_navigation",
            expected_layout_revision_id="layout:1",
        )
        self.assertFalse(r.admitted)
        self.assertIsNone(r.focus_stop_id)

    def test_collapsed_range_command_requiring_nonempty_is_rejected(self):
        m=cmap(5,3)
        s=selection("ABCDE",2,2)
        r=decide_text_interaction_admissibility_v1(
            selection=s,
            caret_map=m,
            command_kind="copy",
            expected_layout_revision_id="layout:1",
        )
        self.assertFalse(r.admitted)
        self.assertEqual("semantic_range_required",r.reason)


if __name__=="__main__":
    unittest.main()
