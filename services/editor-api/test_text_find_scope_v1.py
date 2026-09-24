#!/usr/bin/env python3
import unittest

from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_find_scope_v1 import (
    TextFindScopeError,
    capture_text_find_scope_v1,
    invalidate_text_find_scope_v1,
    rebase_scope_after_replace_current_v1,
    scope_search_extent_v1,
    scoped_replace_current_delta_from_operation_v1,
    terminate_scope_after_replace_all_v1,
)
from text_selection_state_v1 import build_text_selection_state_v1


STORY="story:1"


def domain(text):
    return derive_story_edit_domain_v1(
        story_id=STORY,story_text=text,provenance="chaptera_created"
    )


def selection(text,a,f,rev="rev:1"):
    return build_text_selection_state_v1(
        domain=domain(text),revision_id=rev,anchor_scalar=a,focus_scalar=f
    )


def operation(start,end,replacement,ordinal=0):
    return {
        "protocol_version":"chaptera.story-find-replace.v1",
        "kind":"story_find_replace",
        "story_id":STORY,
        "base_story_revision_id":"rev:1",
        "query":"x",
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


class TextFindScopeV1Tests(unittest.TestCase):
    def test_capture_freezes_nonempty_selection_independent_of_later_live_selection(self):
        text="0123456789"
        s=selection(text,2,8)
        scope=capture_text_find_scope_v1(
            selection=s,domain=domain(text),base_revision_id="rev:1"
        )
        self.assertEqual((2,8),(scope.start_scalar,scope.end_scalar))
        extent=scope_search_extent_v1(scope)
        self.assertEqual((2,8),(extent.start_scalar,extent.end_scalar))

        # A later ordinary selection object does not mutate the captured scope.
        later=selection(text,4,5)
        self.assertEqual((4,5),later.normalized_range)
        self.assertEqual((2,8),(scope.start_scalar,scope.end_scalar))

    def test_collapsed_selection_cannot_create_scope(self):
        text="abc"
        with self.assertRaises(TextFindScopeError) as caught:
            capture_text_find_scope_v1(
                selection=selection(text,1,1),
                domain=domain(text),
                base_revision_id="rev:1",
            )
        self.assertEqual("nonempty_scope_required",caught.exception.code)

    def test_replace_current_inside_scope_rebases_end_with_delta(self):
        text="abcdefghij"
        scope=capture_text_find_scope_v1(
            selection=selection(text,2,8),
            domain=domain(text),
            base_revision_id="rev:1",
        )
        op=operation(4,6,"WXYZ")
        delta=scoped_replace_current_delta_from_operation_v1(
            scope=scope,operation=op,
            base_revision_id="rev:1",resulting_revision_id="rev:2",
        )
        result=rebase_scope_after_replace_current_v1(
            scope=scope,delta=delta,
            resulting_domain=domain("abcdWXYZghij"),
        )
        self.assertTrue(result.is_active)
        self.assertEqual((2,10),(result.start_scalar,result.end_scalar))
        self.assertEqual("rev:2",result.revision_id)

    def test_replace_at_scope_start_uses_start_left_affinity(self):
        text="abcdefghij"
        scope=capture_text_find_scope_v1(
            selection=selection(text,2,8),domain=domain(text),base_revision_id="rev:1"
        )
        delta=scoped_replace_current_delta_from_operation_v1(
            scope=scope,operation=operation(2,4,"XYZ"),
            base_revision_id="rev:1",resulting_revision_id="rev:2",
        )
        result=rebase_scope_after_replace_current_v1(
            scope=scope,delta=delta,resulting_domain=domain("abXYZefghij")
        )
        self.assertEqual((2,9),(result.start_scalar,result.end_scalar))

    def test_replace_at_scope_end_uses_end_right_affinity(self):
        text="abcdefghij"
        scope=capture_text_find_scope_v1(
            selection=selection(text,2,8),domain=domain(text),base_revision_id="rev:1"
        )
        delta=scoped_replace_current_delta_from_operation_v1(
            scope=scope,operation=operation(6,8,"XYZ"),
            base_revision_id="rev:1",resulting_revision_id="rev:2",
        )
        result=rebase_scope_after_replace_current_v1(
            scope=scope,delta=delta,resulting_domain=domain("abcdefXYZij")
        )
        self.assertEqual((2,9),(result.start_scalar,result.end_scalar))

    def test_multi_match_replace_all_cannot_masquerade_as_replace_current_delta(self):
        scope=capture_text_find_scope_v1(
            selection=selection("abcdef",1,5),domain=domain("abcdef"),base_revision_id="rev:1"
        )
        op=operation(1,2,"x")
        op["selected_match_ordinals"]=[0,1]
        op["normalized_edits"].append({
            "edit_ordinal":1,"snapshot_match_ordinal":1,
            "base_start_scalar":3,"base_end_scalar":4,
            "inserted_start_scalar":3,"inserted_end_scalar":4,
            "inserted_paragraph_ids":[],
        })
        with self.assertRaises(TextFindScopeError) as caught:
            scoped_replace_current_delta_from_operation_v1(
                scope=scope,operation=op,
                base_revision_id="rev:1",resulting_revision_id="rev:2",
            )
        self.assertEqual("not_replace_current",caught.exception.code)

    def test_replace_all_success_terminates_scope(self):
        scope=capture_text_find_scope_v1(
            selection=selection("abcdef",1,5),domain=domain("abcdef"),base_revision_id="rev:1"
        )
        ended=terminate_scope_after_replace_all_v1(
            scope,resulting_revision_id="rev:2"
        )
        self.assertEqual("terminated",ended.status)
        with self.assertRaises(TextFindScopeError):
            scope_search_extent_v1(ended)

    def test_unrelated_edit_and_history_changes_invalidate_not_rebase(self):
        scope=capture_text_find_scope_v1(
            selection=selection("abcdef",1,5),domain=domain("abcdef"),base_revision_id="rev:1"
        )
        for reason in ("unrelated_story_edit","undo_redo","history_jump","session_change"):
            invalid=invalidate_text_find_scope_v1(
                scope,current_revision_id="rev:2",reason=reason
            )
            self.assertEqual("invalidated",invalid.status)
            self.assertEqual(reason,invalid.status_reason)

    def test_delta_from_different_scope_is_rejected(self):
        a=capture_text_find_scope_v1(
            selection=selection("abcdef",1,5),domain=domain("abcdef"),base_revision_id="rev:1"
        )
        b=capture_text_find_scope_v1(
            selection=selection("abcdef",2,5),domain=domain("abcdef"),base_revision_id="rev:1"
        )
        delta=scoped_replace_current_delta_from_operation_v1(
            scope=a,operation=operation(2,3,"XX"),
            base_revision_id="rev:1",resulting_revision_id="rev:2",
        )
        with self.assertRaises(TextFindScopeError) as caught:
            rebase_scope_after_replace_current_v1(
                scope=b,delta=delta,resulting_domain=domain("abXXdef")
            )
        self.assertEqual("scope_delta_mismatch",caught.exception.code)


if __name__=="__main__":
    unittest.main()
