#!/usr/bin/env python3
import unittest

from editor_text_find_replace_ui_v1 import (
    build_find_navigation_jump_v1,
    open_editor_text_find_replace_panel_v1,
    set_panel_input_state_v1,
    set_replacement_text_v1,
)
from editor_text_find_scope_ui_v1 import (
    after_scoped_replace_all_v1,
    after_scoped_replace_current_v1,
    build_scoped_replace_request_v1,
    enter_find_in_selection_v1,
    invalidate_scoped_find_mode_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_selection_state_v1 import build_text_selection_state_v1


DOC="doc:1"
STORY="story:1"
SOURCE="a"*64


def domain(text):
    return derive_story_edit_domain_v1(
        story_id=STORY,story_text=text,provenance="chaptera_created"
    )


def selection(text,a,f,rev="rev:1"):
    return build_text_selection_state_v1(
        domain=domain(text),revision_id=rev,anchor_scalar=a,focus_scalar=f
    )


def panel(text,query="one"):
    p=open_editor_text_find_replace_panel_v1(
        document_id=DOC,revision_id="rev:1",story_id=STORY,
        story_text=text,domain=domain(text),find_text=query,
    )
    return set_panel_input_state_v1(
        p,panel_focus="story",composition_active=False
    )


def operation(start,end,replacement,ordinal=0):
    return {
        "protocol_version":"chaptera.story-find-replace.v1",
        "kind":"story_find_replace",
        "story_id":STORY,
        "base_story_revision_id":"rev:1",
        "query":"one",
        "selected_match_ordinals":[ordinal],
        "replacement_text":replacement,
        "normalized_edits":[{
            "edit_ordinal":0,
            "snapshot_match_ordinal":ordinal,
            "base_start_scalar":start,
            "base_end_scalar":end,
            "inserted_start_scalar":start,
            "inserted_end_scalar":start+len(replacement),
            "inserted_paragraph_ids":[],
        }],
        "before_state_id":"before","after_state_id":"after",
        "paragraph_before_hash":"p0","paragraph_after_hash":"p1",
        "format_before_hash":"f0","format_after_hash":"f1",
        "inverse_state":{},"after_state":{},
        "snapshot_staled":True,"layout_status":"layout_unknown",
    }


class EditorTextFindScopeUIV1Tests(unittest.TestCase):
    def test_enter_scope_searches_only_frozen_selection(self):
        text="one XX one YY one"
        mode=enter_find_in_selection_v1(
            panel=panel(text),
            selection=selection(text,4,13),
            domain=domain(text),
            story_text=text,
        )
        self.assertEqual("active",mode.status)
        self.assertEqual((4,13),(mode.scope.start_scalar,mode.scope.end_scalar))
        self.assertEqual(
            [(7,10)],
            [(m.start_scalar,m.end_scalar) for m in mode.panel.snapshot.matches],
        )

    def test_find_next_changes_live_result_not_scope(self):
        text="one XX one YY one"
        mode=enter_find_in_selection_v1(
            panel=panel(text),
            selection=selection(text,4,13),
            domain=domain(text),
            story_text=text,
        )
        before=(mode.scope.start_scalar,mode.scope.end_scalar)
        p,jump=build_find_navigation_jump_v1(
            panel=mode.panel,direction="next",navigation_origin=4
        )
        self.assertEqual((7,10),(jump.start_scalar,jump.end_scalar))
        self.assertEqual(before,(mode.scope.start_scalar,mode.scope.end_scalar))

    def test_scoped_replace_request_uses_scoped_snapshot_only(self):
        text="one XX one YY one"
        mode=enter_find_in_selection_v1(
            panel=set_replacement_text_v1(panel(text),"ONE"),
            selection=selection(text,4,13),
            domain=domain(text),
            story_text=text,
        )
        mode=type(mode)(
            protocol_version=mode.protocol_version,
            panel=build_find_navigation_jump_v1(
                panel=mode.panel,direction="next",navigation_origin=4
            )[0],
            scope=mode.scope,status=mode.status,status_reason=mode.status_reason
        )
        req=build_scoped_replace_request_v1(
            mode=mode,replace_mode="all",
            base_revision_id="rev:1",
            client_operation_id="scoped-all-0001",
            source_hash=SOURCE,
            paragraph_ids_by_match=[],
            format_generation_id="fmt:1",
        )
        self.assertEqual([0],req["command"]["selected_match_ordinals"])

    def test_replace_current_rebases_scope_and_regenerates_inside_it(self):
        text="one XX one YY one"
        mode=enter_find_in_selection_v1(
            panel=panel(text),
            selection=selection(text,4,13),
            domain=domain(text),
            story_text=text,
        )
        result="one XX ONEONE YY one"
        mode=after_scoped_replace_current_v1(
            mode=mode,
            canonical_operation=operation(7,10,"ONEONE",0),
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            resulting_story_text=result,
            resulting_domain=domain(result),
        )
        self.assertEqual("active",mode.status)
        self.assertEqual((4,16),(mode.scope.start_scalar,mode.scope.end_scalar))
        self.assertEqual("rev:2",mode.panel.snapshot.revision_id)
        self.assertIsNone(mode.panel.current_ordinal)

    def test_replace_all_success_exits_scope(self):
        text="one XX one YY one"
        mode=enter_find_in_selection_v1(
            panel=panel(text),
            selection=selection(text,0,13),
            domain=domain(text),
            story_text=text,
        )
        result="ONE XX ONE YY one"
        mode=after_scoped_replace_all_v1(
            mode=mode,
            resulting_revision_id="rev:2",
            resulting_story_text=result,
            resulting_domain=domain(result),
        )
        self.assertEqual("inactive",mode.status)
        self.assertIsNone(mode.scope)
        self.assertEqual("rev:2",mode.panel.revision_id)

    def test_unrelated_edit_invalidates_scope_and_snapshot(self):
        text="one XX one"
        mode=enter_find_in_selection_v1(
            panel=panel(text),
            selection=selection(text,0,len(text)),
            domain=domain(text),
            story_text=text,
        )
        mode=invalidate_scoped_find_mode_v1(
            mode=mode,current_revision_id="rev:2",reason="unrelated_story_edit"
        )
        self.assertEqual("invalidated",mode.status)
        self.assertEqual("invalidated",mode.scope.status)
        self.assertIsNone(mode.panel.snapshot)
        self.assertEqual("stale",mode.panel.status)


if __name__=="__main__":
    unittest.main()
