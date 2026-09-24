#!/usr/bin/env python3
import unittest

from document_text_find_v1 import DocumentStorySearchInputV1
from editor_document_text_find_replace_ui_v1 import (
    EditorDocumentTextFindReplaceUIError,
    build_document_find_navigation_jump_v1,
    build_document_text_replace_all_request_v1,
    open_editor_document_text_find_replace_panel_v1,
    ordered_document_result_refs_v1,
    reconcile_active_session_after_document_replace_all_v1,
    refresh_after_document_revision_v1,
    set_document_replacement_text_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_edit_session_v1 import TextEditSessionV1
from text_selection_state_v1 import build_text_selection_state_v1


DOC="doc:1"
SOURCE="a"*64


def story(story_id,text,provenance="chaptera_created"):
    return DocumentStorySearchInputV1(story_id,text,provenance)


def domain(story_id,text,provenance="chaptera_created"):
    return derive_story_edit_domain_v1(
        story_id=story_id,
        story_text=text,
        provenance=provenance,
    )


def session(story_id,text,a,f,rev="rev:1"):
    d=domain(story_id,text)
    selection=build_text_selection_state_v1(
        domain=d,
        revision_id=rev,
        anchor_scalar=a,
        focus_scalar=f,
        preferred_inline_x_emu=77,
    )
    return TextEditSessionV1(
        protocol_version="chaptera.text-edit-session.v1",
        session_id="session:1",
        incarnation=0,
        document_id=DOC,
        story_id=story_id,
        revision_id=rev,
        edit_domain_id="fixture",
        layout_revision_id="layout:1",
        caret_map_hash="map:1",
        entry_frame_id=None,
        current_frame_id=None,
        focus_owner="story_text",
        selection=selection,
        typing_state=None,
        composition_session=None,
        pending_interaction_metadata=(),
    )


def panel(stories,query="foo"):
    return open_editor_document_text_find_replace_panel_v1(
        document_id=DOC,
        revision_id="rev:1",
        stories=tuple(stories),
        find_text=query,
    )


def local_find_replace_operation(
    *,
    story_id,
    before_text,
    after_text,
    edits,
):
    return {
        "protocol_version":"chaptera.story-find-replace.v1",
        "kind":"story_find_replace",
        "story_id":story_id,
        "normalized_edits":[
            {
                "edit_ordinal":i,
                "snapshot_match_ordinal":source,
                "base_start_scalar":start,
                "base_end_scalar":end,
                "inserted_start_scalar":final_start,
                "inserted_end_scalar":final_end,
                "inserted_paragraph_ids":[],
            }
            for i,(source,start,end,final_start,final_end) in enumerate(edits)
        ],
        "inverse_state":{"paragraph_state":{"story_text":before_text}},
        "after_state":{"paragraph_state":{"story_text":after_text}},
    }


def document_operation(local_entries):
    return {
        "protocol_version":"chaptera.document-text-replace-all.v1",
        "kind":"document_text_replace_all",
        "base_revision_id":"rev:1",
        "snapshot_revision_id":"rev:1",
        "query":"foo",
        "replacement_text":"X",
        "total_match_count":sum(
            len(entry["operation"]["normalized_edits"]) for entry in local_entries
        ),
        "affected_story_ids":sorted(entry["story_id"] for entry in local_entries),
        "multi_story_operation":{
            "protocol_version":"chaptera.multi-story-text-transaction.v1",
            "kind":"multi_story_text_transaction",
            "base_revision_id":"rev:1",
            "story_ids":sorted(entry["story_id"] for entry in local_entries),
            "local_operations":sorted(local_entries,key=lambda x:x["story_id"]),
            "layout_invalidation_story_ids":sorted(entry["story_id"] for entry in local_entries),
        },
        "document_snapshot_staled":True,
    }


def local_entry(story_id,before,after,edits):
    return {
        "story_id":story_id,
        "producer_kind":"story_find_replace",
        "before_state_id":"before:"+story_id,
        "after_state_id":"after:"+story_id,
        "operation":local_find_replace_operation(
            story_id=story_id,before_text=before,after_text=after,edits=edits
        ),
    }


class EditorDocumentTextFindReplaceUIV1Tests(unittest.TestCase):
    def test_results_use_canonical_story_scalar_order_not_input_or_reading_order(self):
        p=panel((
            story("story:z","foo z foo"),
            story("story:a","x foo"),
            story("story:m","none"),
        ))
        refs=ordered_document_result_refs_v1(p)
        self.assertEqual(
            [("story:a",2),("story:z",0),("story:z",6)],
            [(r.story_id,r.start_scalar) for r in refs],
        )
        self.assertEqual("canonical_story_id_scalar",p.presentation_policy)
        self.assertEqual("non_reading_order",p.presentation_semantics)

    def test_next_result_lowers_to_programmatic_jump_and_switches_no_session_itself(self):
        p=panel((story("story:z","foo"),story("story:a","foo")))
        p,jump=build_document_find_navigation_jump_v1(panel=p,direction="next")
        self.assertEqual("story:a",jump.story_id)
        self.assertEqual((0,3),(jump.start_scalar,jump.end_scalar))
        self.assertEqual("exact_range",jump.selection_mode)
        self.assertEqual(0,p.current_result_index)
        p,jump=build_document_find_navigation_jump_v1(panel=p,direction="next")
        self.assertEqual("story:z",jump.story_id)
        self.assertEqual(1,p.current_result_index)

    def test_incomplete_search_is_visible_but_known_results_remain_navigable(self):
        p=panel((
            story("story:a","foo"),
            story("story:b","maybe\r","imported_unknown"),
        ))
        self.assertEqual("incomplete_search",p.status)
        self.assertFalse(p.exhaustive_searchable)
        self.assertEqual(1,p.total_result_count)
        p,jump=build_document_find_navigation_jump_v1(panel=p,direction="next")
        self.assertEqual("story:a",jump.story_id)
        with self.assertRaises(EditorDocumentTextFindReplaceUIError) as caught:
            build_document_text_replace_all_request_v1(
                panel=p,
                source_hash=SOURCE,
                client_operation_id="doc-replace-incomplete-1",
                paragraph_ids_by_match=[],
            )
        self.assertEqual("document_search_incomplete",caught.exception.code)

    def test_replace_all_submits_whole_document_snapshot_once(self):
        p=set_document_replacement_text_v1(
            panel((story("story:a","foo"),story("story:z","foo foo"))),
            "X",
        )
        req=build_document_text_replace_all_request_v1(
            panel=p,
            source_hash=SOURCE,
            client_operation_id="doc-replace-all-0001",
            paragraph_ids_by_match=[],
        )
        self.assertEqual("document_text_replace_all",req["command"]["kind"])
        self.assertEqual(3,p.total_result_count)
        self.assertEqual(
            ["story:a","story:z"],
            [x["story_id"] for x in req["command"]["document_find_snapshot"]["story_results"]],
        )

    def test_zero_match_still_lowers_to_engine_deterministic_noop_request(self):
        p=panel((story("story:a","abc"),story("story:b","xyz")),query="needle")
        self.assertEqual(0,p.total_result_count)
        req=build_document_text_replace_all_request_v1(
            panel=p,
            source_hash=SOURCE,
            client_operation_id="doc-replace-zero-0001",
            paragraph_ids_by_match=[],
        )
        self.assertEqual(
            0,
            sum(
                len(item["snapshot"]["matches"])
                for item in req["command"]["document_find_snapshot"]["story_results"]
                if item["snapshot"] is not None
            ),
        )

    def test_successful_document_revision_discards_old_result_index(self):
        p=panel((story("story:a","foo foo"),))
        p,_=build_document_find_navigation_jump_v1(panel=p,direction="next")
        self.assertEqual(0,p.current_result_index)
        refreshed=refresh_after_document_revision_v1(
            panel=p,
            revision_id="rev:2",
            stories=(story("story:a","X foo"),),
        )
        self.assertEqual("rev:2",refreshed.revision_id)
        self.assertIsNone(refreshed.current_result_index)
        self.assertEqual([(2,5)],[
            (r.start_scalar,r.end_scalar)
            for r in ordered_document_result_refs_v1(refreshed)
        ])

    def test_no_active_session_replace_all_does_not_create_one(self):
        p=panel((story("story:a","foo"),story("story:b","foo")))
        op=document_operation([
            local_entry("story:a","foo","X",[(0,0,3,0,1)]),
            local_entry("story:b","foo","X",[(0,0,3,0,1)]),
        ])
        r=reconcile_active_session_after_document_replace_all_v1(
            panel_before_command=p,
            canonical_operation=op,
            resulting_revision_id="rev:2",
            active_session=None,
        )
        self.assertEqual("no_active_session",r.status)
        self.assertEqual(0,r.session_creation_count)
        self.assertEqual(0,r.other_story_session_count)

    def test_active_unaffected_story_preserves_selection_and_preferred_x(self):
        p=panel((story("story:a","foo"),story("story:b","keep")))
        s=session("story:b","keep",2,2)
        op=document_operation([
            local_entry("story:a","foo","X",[(0,0,3,0,1)]),
        ])
        r=reconcile_active_session_after_document_replace_all_v1(
            panel_before_command=p,
            canonical_operation=op,
            resulting_revision_id="rev:2",
            active_session=s,
            base_domain=domain("story:b","keep"),
            resulting_domain=domain("story:b","keep"),
        )
        self.assertEqual("unaffected_story_rebind",r.status)
        self.assertEqual((2,2),r.resulting_selection.normalized_range)
        self.assertEqual("rev:2",r.resulting_selection.revision_id)
        self.assertEqual(77,r.resulting_selection.preferred_inline_x_emu)
        self.assertEqual("preserve",r.typing_state_policy)

    def test_active_affected_current_result_collapses_after_its_local_replacement(self):
        p=panel((story("story:a","foo foo"),story("story:b","foo")))
        p,jump=build_document_find_navigation_jump_v1(panel=p,direction="next")
        self.assertEqual("story:a",jump.story_id)
        s=session("story:a","foo foo",0,3)
        op=document_operation([
            local_entry(
                "story:a","foo foo","X X",
                [(0,0,3,0,1),(1,4,7,2,3)],
            ),
            local_entry("story:b","foo","X",[(0,0,3,0,1)]),
        ])
        r=reconcile_active_session_after_document_replace_all_v1(
            panel_before_command=p,
            canonical_operation=op,
            resulting_revision_id="rev:2",
            active_session=s,
            base_domain=domain("story:a","foo foo"),
            resulting_domain=domain("story:a","X X"),
        )
        self.assertEqual("affected_story_rebind",r.status)
        self.assertTrue(r.current_result_replaced)
        self.assertEqual((1,1),r.resulting_selection.normalized_range)
        self.assertEqual("rev:2",r.resulting_selection.revision_id)
        self.assertEqual("recompute",r.typing_state_policy)

    def test_active_affected_story_current_result_elsewhere_preserves_old_selection(self):
        p=panel((story("story:a","foo foo"),story("story:b","foo")))
        # Advance to story:b result: a#0, a#1, then b#0.
        for _ in range(3):
            p,_=build_document_find_navigation_jump_v1(panel=p,direction="next")
        self.assertEqual("story:b",ordered_document_result_refs_v1(p)[p.current_result_index].story_id)
        s=session("story:a","foo foo",7,7)
        op=document_operation([
            local_entry(
                "story:a","foo foo","X X",
                [(0,0,3,0,1),(1,4,7,2,3)],
            ),
            local_entry("story:b","foo","X",[(0,0,3,0,1)]),
        ])
        r=reconcile_active_session_after_document_replace_all_v1(
            panel_before_command=p,
            canonical_operation=op,
            resulting_revision_id="rev:2",
            active_session=s,
            base_domain=domain("story:a","foo foo"),
            resulting_domain=domain("story:a","X X"),
        )
        self.assertEqual("affected_story_rebind",r.status)
        self.assertFalse(r.current_result_replaced)
        self.assertEqual((3,3),r.resulting_selection.normalized_range)

    def test_successful_commit_with_ambiguous_active_selection_surfaces_reconcile_required_no_rollback(self):
        p=panel((story("story:a","foo"),story("story:b","foo")))
        # Current result is story:b, but active Story is a with caret inside replaced match.
        p,_=build_document_find_navigation_jump_v1(panel=p,direction="next")
        p,_=build_document_find_navigation_jump_v1(panel=p,direction="next")
        s=session("story:a","foo",1,1)
        op=document_operation([
            local_entry("story:a","foo","X",[(0,0,3,0,1)]),
            local_entry("story:b","foo","X",[(0,0,3,0,1)]),
        ])
        r=reconcile_active_session_after_document_replace_all_v1(
            panel_before_command=p,
            canonical_operation=op,
            resulting_revision_id="rev:2",
            active_session=s,
            base_domain=domain("story:a","foo"),
            resulting_domain=domain("story:a","X"),
        )
        self.assertEqual("selection_reconcile_required",r.status)
        self.assertIsNone(r.resulting_selection)
        self.assertFalse(r.rollback_document_revision)
        self.assertEqual(0,r.other_story_session_count)


if __name__=="__main__":
    unittest.main()
