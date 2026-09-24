#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_selection_state_v1 import (
    PostEditSelectionIntentV1,
    RawTextEditV1,
    TextSelectionStateError,
    build_text_edit_receipt_v1,
    build_text_selection_state_v1,
    project_selection_state_v1,
    reconcile_post_edit_selection_v1,
    reconcile_rejected_edit_v1,
    selection_discontinuity_v1,
)


STORY = "story:1"


def domain(text):
    return derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=text,
        provenance="chaptera_created",
    )


def line(line_id, ordinal, start, end, x0, x1, *, prev=None, next=None, y=0):
    return ResolvedLineFragmentV1(
        story_id=STORY,
        page_id="page:1",
        frame_id="frame:1",
        line_id=line_id,
        flow_ordinal=ordinal,
        previous_line_id=prev,
        next_line_id=next,
        page_y_top_emu=y,
        page_y_bottom_emu=y + 20,
        frame_y_top_emu=y,
        frame_y_bottom_emu=y + 20,
        clusters=(
            ResolvedClusterV1(
                start_scalar=start,
                end_scalar=end,
                page_x_start_emu=x0,
                page_x_end_emu=x1,
                frame_x_start_emu=x0,
                frame_x_end_emu=x1,
            ),
        ),
    )


def simple_map(story_len=4, layout_revision="layout:1"):
    return build_resolved_text_caret_map_v1(
        layout_revision_id=layout_revision,
        story_id=STORY,
        story_scalar_len=story_len,
        lines=(
            line("l1", 0, 0, 1, 0, 10, next="l2", y=0),
            line("l2", 1, 1, story_len, 0, 30, prev="l1", y=30),
        ),
    )


class TextSelectionStateV1Tests(unittest.TestCase):
    def test_state_is_transient_canonical_story_coordinates(self):
        state = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=3,
            focus_scalar=1,
            preferred_inline_x_emu=1234,
        )
        self.assertEqual((1, 3), state.normalized_range)
        self.assertFalse(state.is_collapsed)
        self.assertEqual("layout_pending", state.projection_state)
        self.assertIsNone(state.layout_revision_id)

    def test_imported_protected_suffix_is_not_a_silent_caret_target(self):
        protected = derive_story_edit_domain_v1(
            story_id=STORY,
            story_text="abc\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        with self.assertRaises(TextSelectionStateError) as caught:
            build_text_selection_state_v1(
                domain=protected,
                revision_id="rev:1",
                anchor_scalar=4,
                focus_scalar=4,
            )
        self.assertEqual("selection_reconcile_required", caught.exception.code)

    def test_text_edit_receipt_normalizes_base_coordinate_edits_and_final_ranges(self):
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=10,
            edits=(
                RawTextEditV1(7, 6, 8, 1),
                RawTextEditV1(2, 1, 3, 4),
            ),
        )
        self.assertEqual(
            [(0, 2, 1, 3, 1, 5), (1, 7, 6, 8, 8, 9)],
            [
                (
                    item.edit_ordinal,
                    item.source_ordinal,
                    item.base_start_scalar,
                    item.base_end_scalar,
                    item.final_inserted_start_scalar,
                    item.final_inserted_end_scalar,
                )
                for item in receipt.edits
            ],
        )
        self.assertEqual(11, receipt.resulting_scalar_len)

    def test_overlapping_composite_edits_fail_closed(self):
        with self.assertRaises(TextSelectionStateError) as caught:
            build_text_edit_receipt_v1(
                story_id=STORY,
                base_revision_id="rev:1",
                resulting_revision_id="rev:2",
                base_scalar_len=8,
                edits=(
                    RawTextEditV1(0, 1, 4, 1),
                    RawTextEditV1(1, 3, 5, 1),
                ),
            )
        self.assertEqual("invalid_text_edit_receipt", caught.exception.code)

    def test_typing_and_paste_collapse_after_final_inserted_range(self):
        before = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=2,
            focus_scalar=2,
            preferred_inline_x_emu=900,
        )
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=4,
            edits=(RawTextEditV1(0, 2, 2, 2),),
        )
        after = reconcile_post_edit_selection_v1(
            state=before,
            base_domain=domain("abcd"),
            resulting_domain=domain("abXYcd"),
            receipt=receipt,
            intent=PostEditSelectionIntentV1(
                protocol_version="chaptera.post-edit-selection-intent.v1",
                kind="collapse_after_edit",
                edit_ordinal=0,
            ),
        )
        self.assertEqual((4, 4), (after.anchor_scalar, after.focus_scalar))
        self.assertIsNone(after.preferred_inline_x_emu)
        self.assertEqual("layout_pending", after.projection_state)

    def test_delete_collapses_at_edit_start(self):
        before = build_text_selection_state_v1(
            domain=domain("abcdef"),
            revision_id="rev:1",
            anchor_scalar=2,
            focus_scalar=5,
        )
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=6,
            edits=(RawTextEditV1(0, 2, 5, 0),),
        )
        after = reconcile_post_edit_selection_v1(
            state=before,
            base_domain=domain("abcdef"),
            resulting_domain=domain("abf"),
            receipt=receipt,
            intent=PostEditSelectionIntentV1(
                protocol_version="chaptera.post-edit-selection-intent.v1",
                kind="collapse_at_edit_start",
                edit_ordinal=0,
            ),
        )
        self.assertEqual((2, 2), (after.anchor_scalar, after.focus_scalar))

    def test_preserve_through_composite_edits_preserves_direction(self):
        before = build_text_selection_state_v1(
            domain=domain("abcdefghij"),
            revision_id="rev:1",
            anchor_scalar=9,
            focus_scalar=4,
        )
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=10,
            edits=(
                RawTextEditV1(0, 1, 3, 4),  # +2
                RawTextEditV1(1, 6, 8, 1),  # -1
            ),
        )
        after = reconcile_post_edit_selection_v1(
            state=before,
            base_domain=domain("abcdefghij"),
            resulting_domain=domain("aXXXXdefYij"),
            receipt=receipt,
            intent=PostEditSelectionIntentV1(
                protocol_version="chaptera.post-edit-selection-intent.v1",
                kind="preserve_through_edits",
                anchor_policy="right",
                focus_policy="right",
            ),
        )
        self.assertGreater(after.anchor_scalar, after.focus_scalar)
        self.assertEqual((10, 6), (after.anchor_scalar, after.focus_scalar))

    def test_endpoint_inside_replaced_content_requires_explicit_policy(self):
        before = build_text_selection_state_v1(
            domain=domain("abcdef"),
            revision_id="rev:1",
            anchor_scalar=3,
            focus_scalar=3,
        )
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=6,
            edits=(RawTextEditV1(0, 2, 5, 1),),
        )
        with self.assertRaises(TextSelectionStateError) as caught:
            reconcile_post_edit_selection_v1(
                state=before,
                base_domain=domain("abcdef"),
                resulting_domain=domain("abXf"),
                receipt=receipt,
                intent=PostEditSelectionIntentV1(
                    protocol_version="chaptera.post-edit-selection-intent.v1",
                    kind="preserve_through_edits",
                ),
            )
        self.assertEqual("selection_reconcile_required", caught.exception.code)

        right = reconcile_post_edit_selection_v1(
            state=before,
            base_domain=domain("abcdef"),
            resulting_domain=domain("abXf"),
            receipt=receipt,
            intent=PostEditSelectionIntentV1(
                protocol_version="chaptera.post-edit-selection-intent.v1",
                kind="preserve_through_edits",
                anchor_policy="right",
                focus_policy="right",
            ),
        )
        self.assertEqual((3, 3), (right.anchor_scalar, right.focus_scalar))

    def test_select_inserted_range_is_explicit_not_default(self):
        before = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=1,
            focus_scalar=3,
        )
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=4,
            edits=(RawTextEditV1(0, 1, 3, 3),),
        )
        after = reconcile_post_edit_selection_v1(
            state=before,
            base_domain=domain("abcd"),
            resulting_domain=domain("aXYZd"),
            receipt=receipt,
            intent=PostEditSelectionIntentV1(
                protocol_version="chaptera.post-edit-selection-intent.v1",
                kind="select_inserted_range",
                edit_ordinal=0,
            ),
        )
        self.assertEqual((1, 4), (after.anchor_scalar, after.focus_scalar))

    def test_non_text_revision_preserves_exact_selection_and_preferred_x(self):
        before = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=1,
            focus_scalar=3,
            preferred_inline_x_emu=777,
        )
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=4,
            edits=(),
        )
        after = reconcile_post_edit_selection_v1(
            state=before,
            base_domain=domain("abcd"),
            resulting_domain=domain("abcd"),
            receipt=receipt,
            intent=PostEditSelectionIntentV1(
                protocol_version="chaptera.post-edit-selection-intent.v1",
                kind="preserve_exact_for_non_text_mutation",
            ),
        )
        self.assertEqual((1, 3), (after.anchor_scalar, after.focus_scalar))
        self.assertEqual(777, after.preferred_inline_x_emu)
        self.assertEqual("rev:2", after.revision_id)

    def test_post_edit_domain_shrink_never_silently_clamps(self):
        before = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=4,
            focus_scalar=4,
        )
        receipt = build_text_edit_receipt_v1(
            story_id=STORY,
            base_revision_id="rev:1",
            resulting_revision_id="rev:2",
            base_scalar_len=4,
            edits=(),
        )
        protected = derive_story_edit_domain_v1(
            story_id=STORY,
            story_text="abc\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        with self.assertRaises(TextSelectionStateError) as caught:
            reconcile_post_edit_selection_v1(
                state=before,
                base_domain=domain("abcd"),
                resulting_domain=protected,
                receipt=receipt,
                intent=PostEditSelectionIntentV1(
                    protocol_version="chaptera.post-edit-selection-intent.v1",
                    kind="preserve_exact_for_non_text_mutation",
                ),
            )
        self.assertEqual("selection_reconcile_required", caught.exception.code)

    def test_projection_after_reflow_requires_grounded_affinity_for_same_scalar(self):
        state = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=1,
            focus_scalar=1,
        )
        caret_map = simple_map()
        with self.assertRaises(TextSelectionStateError) as caught:
            project_selection_state_v1(
                state=state,
                domain=domain("abcd"),
                caret_map=caret_map,
            )
        self.assertEqual("caret_affinity_required", caught.exception.code)

        candidates = [
            stop for stop in caret_map.caret_stops if stop.scalar_boundary == 1
        ]
        projected = project_selection_state_v1(
            state=state,
            domain=domain("abcd"),
            caret_map=caret_map,
            anchor_stop_id=candidates[1].stop_id,
        )
        self.assertEqual("projected", projected.state.projection_state)
        self.assertEqual(candidates[1].stop_id, projected.state.anchor_visual_stop_id)
        self.assertEqual(
            projected.state.anchor_visual_stop_id,
            projected.state.focus_visual_stop_id,
        )

    def test_projection_uses_authoritative_selection_geometry(self):
        state = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=0,
            focus_scalar=4,
        )
        projected = project_selection_state_v1(
            state=state,
            domain=domain("abcd"),
            caret_map=simple_map(),
        )
        self.assertEqual("complete", projected.geometry.coverage_state)
        self.assertEqual("layout:1", projected.state.layout_revision_id)

    def test_rejected_edit_restores_only_still_authoritative_state(self):
        state = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=2,
            focus_scalar=2,
        )
        self.assertEqual(
            state,
            reconcile_rejected_edit_v1(
                last_authoritative_state=state,
                current_domain=domain("abcd"),
                current_revision_id="rev:1",
            ),
        )
        with self.assertRaises(TextSelectionStateError) as caught:
            reconcile_rejected_edit_v1(
                last_authoritative_state=state,
                current_domain=domain("abcd"),
                current_revision_id="rev:2",
            )
        self.assertEqual("stale_selection_revision", caught.exception.code)

    def test_selection_discontinuity_is_stable_grouping_input(self):
        before = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=1,
            focus_scalar=1,
        )
        same = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:2",
            anchor_scalar=1,
            focus_scalar=1,
        )
        moved = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:2",
            anchor_scalar=2,
            focus_scalar=2,
        )
        self.assertFalse(selection_discontinuity_v1(before, same))
        self.assertTrue(selection_discontinuity_v1(before, moved))


if __name__ == "__main__":
    unittest.main()
