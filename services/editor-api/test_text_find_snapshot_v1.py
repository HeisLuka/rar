#!/usr/bin/env python3
import unittest

from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_find_snapshot_v1 import (
    TextFindError,
    TextFindExtentV1,
    build_text_find_snapshot_v1,
    find_next_v1,
    find_previous_v1,
    serialize_text_find_snapshot_v1,
    validate_text_find_snapshot_current_v1,
)


def domain(text, provenance="chaptera_created"):
    return derive_story_edit_domain_v1(
        story_id="s",
        story_text=text,
        provenance=provenance,
    )


class TextFindSnapshotV1Tests(unittest.TestCase):
    def test_ascii_repeated_matches_are_left_to_right_nonoverlapping(self):
        snap = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text="abc abc abc",
            domain=domain("abc abc abc"),
            external_query="abc",
            extent=TextFindExtentV1("full_editable_story"),
        )
        self.assertEqual([(0,3),(4,7),(8,11)], [
            (m.start_scalar,m.end_scalar) for m in snap.matches
        ])

    def test_overlap_candidate_aa_in_aaa_yields_one_match(self):
        snap = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text="aaa",
            domain=domain("aaa"),
            external_query="aa",
            extent=TextFindExtentV1("full_editable_story"),
        )
        self.assertEqual([(0,2)], [(m.start_scalar,m.end_scalar) for m in snap.matches])

    def test_emoji_and_combining_are_scalar_exact(self):
        text = "A😀e\u0301é😀"
        emoji = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="😀",
            extent=TextFindExtentV1("full_editable_story"),
        )
        self.assertEqual([(1,2),(5,6)], [(m.start_scalar,m.end_scalar) for m in emoji.matches])

        pre = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="é",
            extent=TextFindExtentV1("full_editable_story"),
        )
        decomp = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="e\u0301",
            extent=TextFindExtentV1("full_editable_story"),
        )
        self.assertEqual([(4,5)], [(m.start_scalar,m.end_scalar) for m in pre.matches])
        self.assertEqual([(2,4)], [(m.start_scalar,m.end_scalar) for m in decomp.matches])

    def test_external_lf_query_matches_canonical_cr(self):
        text = "one\rtwo"
        snap = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="\n",
            extent=TextFindExtentV1("full_editable_story"),
        )
        self.assertEqual("\r", snap.query)
        self.assertEqual([(3,4)], [(m.start_scalar,m.end_scalar) for m in snap.matches])

    def test_protected_terminal_cr_is_excluded(self):
        text = "A\r"
        snap = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text, "imported_mature_quill_terminal_cr"),
            external_query="\r",
            extent=TextFindExtentV1("full_editable_story"),
        )
        self.assertEqual((), snap.matches)

    def test_explicit_extent_requires_wholly_contained_matches(self):
        text = "xxabcxxabc"
        snap = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="abc",
            extent=TextFindExtentV1("range", 1, 7),
        )
        self.assertEqual([(2,5)], [(m.start_scalar,m.end_scalar) for m in snap.matches])

    def test_empty_query_and_empty_explicit_extent_reject(self):
        with self.assertRaises(TextFindError) as q:
            build_text_find_snapshot_v1(
                revision_id="r1",
                story_id="s",
                story_text="abc",
                domain=domain("abc"),
                external_query="",
                extent=TextFindExtentV1("full_editable_story"),
            )
        self.assertEqual("empty_query", q.exception.code)

        with self.assertRaises(TextFindError) as e:
            build_text_find_snapshot_v1(
                revision_id="r1",
                story_id="s",
                story_text="abc",
                domain=domain("abc"),
                external_query="a",
                extent=TextFindExtentV1("range", 1, 1),
            )
        self.assertEqual("invalid_extent", e.exception.code)

    def test_revision_change_invalidates_snapshot(self):
        text = "abc abc"
        snap = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="abc",
            extent=TextFindExtentV1("full_editable_story"),
        )
        with self.assertRaises(TextFindError) as caught:
            validate_text_find_snapshot_current_v1(
                snapshot=snap,
                revision_id="r2",
                story_id="s",
                story_text=text,
                domain=domain(text),
            )
        self.assertEqual("find_snapshot_stale", caught.exception.code)

    def test_navigation_uses_origin_and_explicit_wrap_not_old_ordinal(self):
        text = "a a a"
        snap = build_text_find_snapshot_v1(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="a",
            extent=TextFindExtentV1("full_editable_story"),
        )
        self.assertEqual(2, find_next_v1(snapshot=snap, navigation_origin=2, wrap=False).start_scalar)
        self.assertIsNone(find_next_v1(snapshot=snap, navigation_origin=5, wrap=False))
        self.assertEqual(0, find_next_v1(snapshot=snap, navigation_origin=5, wrap=True).start_scalar)

        self.assertEqual(2, find_previous_v1(snapshot=snap, navigation_origin=3, wrap=False).start_scalar)
        self.assertIsNone(find_previous_v1(snapshot=snap, navigation_origin=0, wrap=False))
        self.assertEqual(4, find_previous_v1(snapshot=snap, navigation_origin=0, wrap=True).start_scalar)

    def test_repeated_identical_snapshot_serializes_identically(self):
        text = "one\rtwo one"
        kwargs = dict(
            revision_id="r1",
            story_id="s",
            story_text=text,
            domain=domain(text),
            external_query="one",
            extent=TextFindExtentV1("full_editable_story"),
        )
        a = build_text_find_snapshot_v1(**kwargs)
        b = build_text_find_snapshot_v1(**kwargs)
        self.assertEqual(serialize_text_find_snapshot_v1(a), serialize_text_find_snapshot_v1(b))


if __name__ == "__main__":
    unittest.main()
