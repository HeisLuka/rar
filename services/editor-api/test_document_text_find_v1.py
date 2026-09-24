#!/usr/bin/env python3
import unittest

from document_text_find_v1 import (
    DocumentStorySearchInputV1,
    DocumentTextFindError,
    build_document_text_find_snapshot_v1,
    ordered_document_matches_v1,
    serialize_document_text_find_snapshot_v1,
    validate_document_text_find_snapshot_current_v1,
)


def story(story_id, text, provenance="chaptera_created"):
    return DocumentStorySearchInputV1(story_id, text, provenance)


class DocumentTextFindV1Tests(unittest.TestCase):
    def test_story_order_and_match_order_are_canonical(self):
        snap = build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(
                story("story:z", "x x"),
                story("story:a", "xx"),
                story("story:m", "none"),
            ),
            external_query="x",
        )
        self.assertEqual(
            ["story:a", "story:m", "story:z"],
            [item.story_id for item in snap.story_results],
        )
        self.assertEqual(
            [
                ("story:a", 0),
                ("story:a", 1),
                ("story:z", 0),
                ("story:z", 2),
            ],
            [(sid, match.start_scalar) for sid, match in ordered_document_matches_v1(snap)],
        )

    def test_linked_story_registry_entry_is_counted_once(self):
        snap = build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(story("story:linked", "needle needle"),),
            external_query="needle",
        )
        self.assertEqual(1, len(snap.story_results))
        self.assertEqual(2, snap.total_match_count)

    def test_unknown_edit_domain_is_explicitly_accounted_not_skipped(self):
        snap = build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(
                story("story:a", "needle"),
                story("story:b", "needle\r", "imported_unknown"),
            ),
            external_query="needle",
        )
        self.assertFalse(snap.exhaustive_searchable)
        self.assertEqual("searched", snap.story_results[0].status)
        self.assertEqual("unsupported", snap.story_results[1].status)
        self.assertEqual("edit_domain_unknown", snap.story_results[1].reason)
        self.assertEqual(1, snap.total_match_count)

    def test_protected_terminal_cr_is_not_searchable(self):
        snap = build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(story("story:a", "A\r", "imported_mature_quill_terminal_cr"),),
            external_query="\n",
        )
        self.assertTrue(snap.exhaustive_searchable)
        self.assertEqual(0, snap.total_match_count)

    def test_external_newline_query_normalizes_once_for_all_stories(self):
        snap = build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(story("story:a", "A\rB"), story("story:b", "\r")),
            external_query="\r\n",
        )
        self.assertEqual("\r", snap.query)
        self.assertEqual(2, snap.total_match_count)
        self.assertTrue(all(
            item.snapshot is None or item.snapshot.query == "\r"
            for item in snap.story_results
        ))

    def test_revision_change_stales_entire_document_snapshot(self):
        stories=(story("story:a", "needle"),story("story:b","none"))
        snap=build_document_text_find_snapshot_v1(
            revision_id="r1", stories=stories, external_query="needle"
        )
        with self.assertRaises(DocumentTextFindError) as caught:
            validate_document_text_find_snapshot_current_v1(
                snapshot=snap,
                revision_id="r2",
                stories=stories,
            )
        self.assertEqual("document_find_snapshot_stale", caught.exception.code)

    def test_story_change_under_same_claimed_revision_is_detected(self):
        snap=build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(story("story:a","needle"),),
            external_query="needle",
        )
        with self.assertRaises(DocumentTextFindError) as caught:
            validate_document_text_find_snapshot_current_v1(
                snapshot=snap,
                revision_id="r1",
                stories=(story("story:a","changed"),),
            )
        self.assertEqual("document_find_snapshot_stale", caught.exception.code)

    def test_duplicate_story_id_rejects_instead_of_double_counting_frames(self):
        with self.assertRaises(DocumentTextFindError) as caught:
            build_document_text_find_snapshot_v1(
                revision_id="r1",
                stories=(story("story:a","x"),story("story:a","x")),
                external_query="x",
            )
        self.assertEqual("duplicate_story", caught.exception.code)

    def test_serialization_is_independent_of_input_registry_order(self):
        a=build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(story("story:b","x"),story("story:a","x")),
            external_query="x",
        )
        b=build_document_text_find_snapshot_v1(
            revision_id="r1",
            stories=(story("story:a","x"),story("story:b","x")),
            external_query="x",
        )
        self.assertEqual(
            serialize_document_text_find_snapshot_v1(a),
            serialize_document_text_find_snapshot_v1(b),
        )


if __name__=="__main__":
    unittest.main()
