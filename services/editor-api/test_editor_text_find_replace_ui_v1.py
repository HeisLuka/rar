#!/usr/bin/env python3
import unittest

from editor_text_find_replace_ui_v1 import (
    EditorTextFindReplaceUIError,
    build_find_navigation_jump_v1,
    build_story_find_replace_request_v1,
    open_editor_text_find_replace_panel_v1,
    refresh_after_accepted_story_revision_v1,
    set_panel_input_state_v1,
    set_replacement_text_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1


DOC="doc:1"
STORY="story:1"
SOURCE="a"*64


def domain(text):
    return derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=text,
        provenance="chaptera_created",
    )


def panel(text="one two one"):
    p=open_editor_text_find_replace_panel_v1(
        document_id=DOC,
        revision_id="rev:1",
        story_id=STORY,
        story_text=text,
        domain=domain(text),
        find_text="one",
    )
    return set_panel_input_state_v1(p,panel_focus="story",composition_active=False)


class EditorTextFindReplaceUIV1Tests(unittest.TestCase):
    def test_panel_builds_full_story_revision_bound_snapshot_and_count(self):
        p=panel()
        self.assertEqual(2,p.result_count)
        self.assertIsNone(p.current_ordinal)
        self.assertEqual("rev:1",p.snapshot.revision_id)

    def test_next_previous_lower_to_programmatic_exact_range_jump(self):
        p=panel()
        p,jump=build_find_navigation_jump_v1(
            panel=p,direction="next",navigation_origin=0
        )
        self.assertEqual(0,p.current_ordinal)
        self.assertEqual((0,3),(jump.start_scalar,jump.end_scalar))
        self.assertEqual("exact_range",jump.selection_mode)
        p,jump=build_find_navigation_jump_v1(
            panel=p,direction="previous",navigation_origin=11
        )
        self.assertEqual(1,p.current_ordinal)
        self.assertEqual((8,11),(jump.start_scalar,jump.end_scalar))

    def test_replace_current_submits_exact_current_snapshot_ordinal(self):
        p=panel()
        p,_=build_find_navigation_jump_v1(
            panel=p,direction="next",navigation_origin=0
        )
        p=set_replacement_text_v1(p,"ONE")
        req=build_story_find_replace_request_v1(
            panel=p,
            mode="current",
            base_revision_id="rev:1",
            client_operation_id="replace-current-0001",
            source_hash=SOURCE,
            paragraph_ids_by_match=[],
            format_generation_id="fmt:1",
        )
        self.assertEqual([0],req["command"]["selected_match_ordinals"])
        self.assertEqual("ONE",req["command"]["external_replacement_text"])

    def test_replace_all_submits_whole_snapshot_once(self):
        p=set_replacement_text_v1(panel(),"x")
        req=build_story_find_replace_request_v1(
            panel=p,
            mode="all",
            base_revision_id="rev:1",
            client_operation_id="replace-all-0001",
            source_hash=SOURCE,
            paragraph_ids_by_match=[],
            format_generation_id="fmt:1",
        )
        self.assertEqual([0,1],req["command"]["selected_match_ordinals"])

    def test_empty_replacement_is_not_missing_value(self):
        p=panel()
        p,_=build_find_navigation_jump_v1(
            panel=p,direction="next",navigation_origin=0
        )
        req=build_story_find_replace_request_v1(
            panel=p,
            mode="current",
            base_revision_id="rev:1",
            client_operation_id="replace-delete-0001",
            source_hash=SOURCE,
            paragraph_ids_by_match=[],
            format_generation_id="fmt:1",
        )
        self.assertEqual("",req["command"]["external_replacement_text"])

    def test_zero_matches_emit_no_mutation_request(self):
        text="abc"
        p=open_editor_text_find_replace_panel_v1(
            document_id=DOC,revision_id="rev:1",story_id=STORY,
            story_text=text,domain=domain(text),find_text="zzz",
        )
        req=build_story_find_replace_request_v1(
            panel=p,
            mode="all",
            base_revision_id="rev:1",
            client_operation_id="replace-zero-0001",
            source_hash=SOURCE,
            paragraph_ids_by_match=[],
            format_generation_id="fmt:1",
        )
        self.assertIsNone(req)

    def test_accepted_revision_discards_old_ordinal_and_regenerates_snapshot(self):
        p=panel()
        p,_=build_find_navigation_jump_v1(
            panel=p,direction="next",navigation_origin=0
        )
        self.assertEqual(0,p.current_ordinal)
        new="x two one"
        p=refresh_after_accepted_story_revision_v1(
            panel=p,
            revision_id="rev:2",
            story_text=new,
            domain=domain(new),
        )
        self.assertEqual("rev:2",p.snapshot.revision_id)
        self.assertIsNone(p.current_ordinal)
        self.assertEqual([(6,9)],[(m.start_scalar,m.end_scalar) for m in p.snapshot.matches])

    def test_panel_or_modal_focus_suppresses_story_navigation_shortcut(self):
        p=open_editor_text_find_replace_panel_v1(
            document_id=DOC,revision_id="rev:1",story_id=STORY,
            story_text="one",domain=domain("one"),find_text="one",
        )
        with self.assertRaises(EditorTextFindReplaceUIError) as caught:
            build_find_navigation_jump_v1(
                panel=p,direction="next",navigation_origin=0
            )
        self.assertEqual("shortcut_owned_by_panel",caught.exception.code)

    def test_active_story_composition_blocks_replace(self):
        p=set_panel_input_state_v1(
            panel(),panel_focus="story",composition_active=True
        )
        with self.assertRaises(EditorTextFindReplaceUIError) as caught:
            build_story_find_replace_request_v1(
                panel=p,
                mode="all",
                base_revision_id="rev:1",
                client_operation_id="replace-ime-0001",
                source_hash=SOURCE,
                paragraph_ids_by_match=[],
                format_generation_id="fmt:1",
            )
        self.assertEqual("composition_active",caught.exception.code)

    def test_replacement_containing_query_does_not_change_frozen_snapshot(self):
        p=set_replacement_text_v1(panel(),"oneone")
        req=build_story_find_replace_request_v1(
            panel=p,
            mode="all",
            base_revision_id="rev:1",
            client_operation_id="replace-self-0001",
            source_hash=SOURCE,
            paragraph_ids_by_match=[],
            format_generation_id="fmt:1",
        )
        self.assertEqual([0,1],req["command"]["selected_match_ordinals"])
        self.assertEqual(2,p.result_count)


if __name__=="__main__":
    unittest.main()
