#!/usr/bin/env python3
import unittest

from range_anchor_rebase_v1 import (
    AnchoredRangeV1,
    RangeAnchorPolicyV1,
    RangeAnchorRebaseError,
    StoryRangeEditV1,
    rebase_anchored_range_v1,
    rebase_anchored_ranges_v1,
    restore_range_from_rebase_receipt_v1,
)


FORMAT_POLICY = RangeAnchorPolicyV1(
    start_affinity="left",
    end_affinity="right",
    full_cover_policy="replacement",
)
COMMENT_POLICY = RangeAnchorPolicyV1(
    start_affinity="right",
    end_affinity="left",
    full_cover_policy="invalidate",
)


class RangeAnchorRebaseV1Tests(unittest.TestCase):
    def test_insert_before_shifts_span(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(2, 2, 3),
        ).result
        self.assertEqual((8, 13), (r.range.start_scalar, r.range.end_scalar))

    def test_insert_at_start_respects_start_affinity(self):
        include = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(5, 5, 2),
        ).result
        self.assertEqual((5, 12), (include.range.start_scalar, include.range.end_scalar))

        exclude = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("right", "right", "replacement"),
            edit=StoryRangeEditV1(5, 5, 2),
        ).result
        self.assertEqual((7, 12), (exclude.range.start_scalar, exclude.range.end_scalar))

    def test_insert_inside_expands_range(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(7, 7, 2),
        ).result
        self.assertEqual((5, 12), (r.range.start_scalar, r.range.end_scalar))

    def test_insert_at_end_respects_end_affinity(self):
        include = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(10, 10, 2),
        ).result
        self.assertEqual((5, 12), (include.range.start_scalar, include.range.end_scalar))

        exclude = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("left", "left", "replacement"),
            edit=StoryRangeEditV1(10, 10, 2),
        ).result
        self.assertEqual((5, 10), (exclude.range.start_scalar, exclude.range.end_scalar))

    def test_insert_after_does_not_change_span(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(12, 12, 3),
        ).result
        self.assertEqual((5, 10), (r.range.start_scalar, r.range.end_scalar))

    def test_delete_wholly_before_shifts_span_left(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(1, 3, 0),
        ).result
        self.assertEqual((3, 8), (r.range.start_scalar, r.range.end_scalar))

    def test_overlap_left_maps_start_to_replacement_boundary(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("right", "right", "replacement"),
            edit=StoryRangeEditV1(3, 7, 2),
        ).result
        self.assertEqual((5, 8), (r.range.start_scalar, r.range.end_scalar))

    def test_edit_inside_span_changes_end_by_delta(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 12),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(7, 9, 5),
        ).result
        self.assertEqual((5, 15), (r.range.start_scalar, r.range.end_scalar))

    def test_overlap_right_maps_end_by_affinity(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("left", "left", "replacement"),
            edit=StoryRangeEditV1(8, 12, 1),
        ).result
        self.assertEqual((5, 8), (r.range.start_scalar, r.range.end_scalar))

    def test_full_cover_policies_are_explicit(self):
        edit = StoryRangeEditV1(3, 12, 2)
        replacement = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("left", "right", "replacement"),
            edit=edit,
        ).result
        self.assertEqual((3, 5), (replacement.range.start_scalar, replacement.range.end_scalar))

        invalidated = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("left", "right", "invalidate"),
            edit=edit,
        ).result
        self.assertEqual("invalidated", invalidated.status)
        self.assertIsNone(invalidated.range)

        deleted = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("left", "right", "delete"),
            edit=edit,
        ).result
        self.assertEqual("deleted", deleted.status)

    def test_deletion_can_collapse_empty_owner_when_explicitly_allowed(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10, allow_empty=True),
            policy=RangeAnchorPolicyV1("left", "right", "collapse_left"),
            edit=StoryRangeEditV1(3, 12, 0),
        ).result
        self.assertEqual("survives", r.status)
        self.assertEqual((3, 3), (r.range.start_scalar, r.range.end_scalar))

    def test_nonempty_owner_deleted_when_replacement_becomes_empty(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=RangeAnchorPolicyV1("left", "right", "replacement"),
            edit=StoryRangeEditV1(3, 12, 0),
        ).result
        self.assertEqual("deleted", r.status)
        self.assertIsNone(r.range)

    def test_empty_span_uses_one_affinity(self):
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 5, allow_empty=True),
            policy=RangeAnchorPolicyV1("right", "right", "collapse_right"),
            edit=StoryRangeEditV1(5, 5, 2),
        ).result
        self.assertEqual((7, 7), (r.range.start_scalar, r.range.end_scalar))

        with self.assertRaises(RangeAnchorRebaseError):
            rebase_anchored_range_v1(
                anchored=AnchoredRangeV1(5, 5, allow_empty=True),
                policy=RangeAnchorPolicyV1("left", "right", "collapse_right"),
                edit=StoryRangeEditV1(5, 5, 2),
            )

    def test_comment_start_right_end_left_policy_excludes_boundary_insertions(self):
        at_start = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=COMMENT_POLICY,
            edit=StoryRangeEditV1(5, 5, 2),
        ).result
        self.assertEqual((7, 12), (at_start.range.start_scalar, at_start.range.end_scalar))

        at_end = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(5, 10),
            policy=COMMENT_POLICY,
            edit=StoryRangeEditV1(10, 10, 2),
        ).result
        self.assertEqual((5, 10), (at_end.range.start_scalar, at_end.range.end_scalar))

    def test_unicode_scalar_coordinates_are_not_utf16_units(self):
        story = "A😀e\u0301B"
        self.assertEqual(5, len(story))
        # Span covers emoji + e + combining acute => [1,4).
        r = rebase_anchored_range_v1(
            anchored=AnchoredRangeV1(1, 4),
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(1, 2, 1),
        ).result
        self.assertEqual((1, 4), (r.range.start_scalar, r.range.end_scalar))

    def test_multiple_adjacent_and_overlapping_ranges_share_one_transform(self):
        receipts = rebase_anchored_ranges_v1(
            anchored_ranges=(
                ("a", AnchoredRangeV1(0, 4), FORMAT_POLICY),
                ("b", AnchoredRangeV1(4, 8), FORMAT_POLICY),
                ("c", AnchoredRangeV1(2, 6), FORMAT_POLICY),
            ),
            edit=StoryRangeEditV1(4, 4, 2),
        )
        by_id = {semantic_id: receipt.result.range for semantic_id, receipt in receipts}
        self.assertEqual((0, 6), (by_id["a"].start_scalar, by_id["a"].end_scalar))
        self.assertEqual((4, 10), (by_id["b"].start_scalar, by_id["b"].end_scalar))
        self.assertEqual((2, 8), (by_id["c"].start_scalar, by_id["c"].end_scalar))

    def test_exact_undo_uses_persisted_before_state(self):
        before = AnchoredRangeV1(5, 10)
        receipt = rebase_anchored_range_v1(
            anchored=before,
            policy=FORMAT_POLICY,
            edit=StoryRangeEditV1(7, 9, 5),
        )
        self.assertNotEqual(before, receipt.result.range)
        self.assertEqual(before, restore_range_from_rebase_receipt_v1(receipt))


if __name__ == "__main__":
    unittest.main()
