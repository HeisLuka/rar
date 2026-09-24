#!/usr/bin/env python3
import unittest

from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    effective_property_segments_v1,
)
from text_insert_format_v1 import (
    RelativeFragmentFormatRunV1,
    TextInsertFormatError,
    TypingFormatSnapshotV1,
    plan_text_insert_format_v1,
    replay_text_insert_format_v1,
    undo_text_insert_format_v1,
)


def fmt(*, size=12000, bold=False, italic=False, color="#000000", font="font:resolved"):
    return BaseCharacterFormatV1(font, size, bold, italic, color)


def state(length=4, basefmt=None, overrides=()):
    if basefmt is None:
        basefmt = fmt()
    runs = () if length == 0 else (BaseFormatRunV1(0, length, basefmt),)
    return build_text_format_overlay_state_v1(
        story_id="story:1",
        base_revision_id="rev:before",
        story_scalar_len=length,
        base_runs=runs,
        overrides=tuple(overrides),
    )


def effective(state_value, prop, scalar):
    seg = effective_property_segments_v1(
        state=state_value,
        prop=prop,
        start_scalar=scalar,
        end_scalar=scalar + 1,
    )
    return seg[0].value


class TextInsertFormatV1Tests(unittest.TestCase):
    def test_insert_inside_existing_override_inherits_effective_format(self):
        s = state(
            overrides=(TextFormatOverrideRunV1(1, 3, "bold", True),),
        )
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=2,
            edit_end_scalar=2,
            replacement_text="X",
            post_edit_revision_id="rev:after",
        )
        self.assertEqual("inherited", receipt.source)
        self.assertTrue(effective(receipt.after_state, "bold", 2))
        self.assertEqual(5, receipt.after_state.story_scalar_len)

    def test_left_biased_boundary_excludes_right_span_at_its_start(self):
        s = state(
            overrides=(TextFormatOverrideRunV1(2, 4, "bold", True),),
        )
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=2,
            edit_end_scalar=2,
            replacement_text="X",
            post_edit_revision_id="rev:after",
        )
        self.assertFalse(effective(receipt.after_state, "bold", 2))
        self.assertTrue(effective(receipt.after_state, "bold", 3))

    def test_left_biased_boundary_includes_left_span_at_its_end(self):
        s = state(
            overrides=(TextFormatOverrideRunV1(0, 2, "bold", True),),
        )
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=2,
            edit_end_scalar=2,
            replacement_text="X",
            post_edit_revision_id="rev:after",
        )
        self.assertTrue(effective(receipt.after_state, "bold", 2))

    def test_story_start_falls_back_to_old_first_effective_format(self):
        s = state(basefmt=fmt(italic=True))
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=0,
            edit_end_scalar=0,
            replacement_text="X",
            post_edit_revision_id="rev:after",
        )
        self.assertTrue(effective(receipt.after_state, "italic", 0))

    def test_story_end_inherits_old_last_effective_format(self):
        s = state(
            overrides=(TextFormatOverrideRunV1(2, 4, "bold", True),),
        )
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=4,
            edit_end_scalar=4,
            replacement_text="X",
            post_edit_revision_id="rev:after",
        )
        self.assertTrue(effective(receipt.after_state, "bold", 4))

    def test_typing_snapshot_materializes_only_over_nonempty_inserted_range(self):
        s = state(basefmt=fmt(bold=True))
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=2,
            edit_end_scalar=2,
            replacement_text="XY",
            post_edit_revision_id="rev:after",
            typing_snapshot=TypingFormatSnapshotV1(
                (("bold", False), ("text_color_rgb", "#ff0000")),
            ),
        )
        self.assertEqual("typing_snapshot", receipt.source)
        self.assertFalse(effective(receipt.after_state, "bold", 2))
        self.assertFalse(effective(receipt.after_state, "bold", 3))
        self.assertEqual("#FF0000", effective(receipt.after_state, "text_color_rgb", 2))
        bold_runs = [r for r in receipt.after_state.overrides if r.property == "bold"]
        self.assertTrue(any(r.start_scalar <= 2 and r.end_scalar >= 4 for r in bold_runs))

    def test_semantic_fragment_format_is_relative_and_not_flattened(self):
        s = state(basefmt=fmt(color="#000000"))
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=1,
            edit_end_scalar=1,
            replacement_text="XYZ",
            post_edit_revision_id="rev:after",
            fragment_runs=(
                RelativeFragmentFormatRunV1(1, 3, "text_color_rgb", "#00ff00"),
            ),
        )
        self.assertEqual("semantic_fragment", receipt.source)
        self.assertEqual("#000000", effective(receipt.after_state, "text_color_rgb", 1))
        self.assertEqual("#00FF00", effective(receipt.after_state, "text_color_rgb", 2))
        self.assertEqual("#00FF00", effective(receipt.after_state, "text_color_rgb", 3))

    def test_plain_text_replacement_uses_same_left_inheritance_law(self):
        s = state(
            overrides=(TextFormatOverrideRunV1(0, 2, "italic", True),),
        )
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=2,
            edit_end_scalar=3,
            replacement_text="PQ",
            post_edit_revision_id="rev:after",
        )
        self.assertTrue(effective(receipt.after_state, "italic", 2))
        self.assertTrue(effective(receipt.after_state, "italic", 3))

    def test_full_story_replacement_uses_old_first_effective_format(self):
        s = state(basefmt=fmt(size=18000, color="#123456"))
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=0,
            edit_end_scalar=4,
            replacement_text="Z",
            post_edit_revision_id="rev:after",
        )
        self.assertEqual(18000, effective(receipt.after_state, "font_size_emu", 0))
        self.assertEqual("#123456", effective(receipt.after_state, "text_color_rgb", 0))

    def test_empty_story_requires_explicit_authoring_preset_base(self):
        s = state(length=0)
        with self.assertRaisesRegex(TextInsertFormatError, "AuthoringTextPreset"):
            plan_text_insert_format_v1(
                before_state=s,
                edit_start_scalar=0,
                edit_end_scalar=0,
                replacement_text="A",
                post_edit_revision_id="rev:after",
            )

        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=0,
            edit_end_scalar=0,
            replacement_text="A",
            post_edit_revision_id="rev:after",
            empty_story_preset_format=fmt(size=14000, color="#222222"),
        )
        self.assertEqual(14000, effective(receipt.after_state, "font_size_emu", 0))
        self.assertEqual("#222222", effective(receipt.after_state, "text_color_rgb", 0))

    def test_deletion_with_typing_snapshot_creates_no_zero_length_override(self):
        s = state(
            overrides=(TextFormatOverrideRunV1(1, 3, "bold", True),),
        )
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=1,
            edit_end_scalar=3,
            replacement_text="",
            post_edit_revision_id="rev:after",
            typing_snapshot=TypingFormatSnapshotV1((("italic", True),)),
        )
        self.assertEqual("deletion_only", receipt.source)
        self.assertEqual(2, receipt.after_state.story_scalar_len)
        self.assertTrue(all(r.start_scalar < r.end_scalar for r in receipt.after_state.overrides))
        self.assertFalse(any(r.property == "italic" for r in receipt.after_state.overrides))

    def test_fragment_and_typing_snapshot_are_mutually_exclusive(self):
        s = state()
        with self.assertRaisesRegex(TextInsertFormatError, "mutually exclusive"):
            plan_text_insert_format_v1(
                before_state=s,
                edit_start_scalar=1,
                edit_end_scalar=1,
                replacement_text="X",
                post_edit_revision_id="rev:after",
                typing_snapshot=TypingFormatSnapshotV1((("bold", True),)),
                fragment_runs=(RelativeFragmentFormatRunV1(0,1,"italic",True),),
            )

    def test_overlapping_fragment_runs_for_one_property_fail_closed(self):
        s = state()
        with self.assertRaisesRegex(TextInsertFormatError, "must not overlap"):
            plan_text_insert_format_v1(
                before_state=s,
                edit_start_scalar=1,
                edit_end_scalar=1,
                replacement_text="XYZ",
                post_edit_revision_id="rev:after",
                fragment_runs=(
                    RelativeFragmentFormatRunV1(0,2,"bold",True),
                    RelativeFragmentFormatRunV1(1,3,"bold",False),
                ),
            )

    def test_paragraph_boundary_scalar_uses_same_character_format_law(self):
        s = state(basefmt=fmt(bold=True))
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=2,
            edit_end_scalar=2,
            replacement_text="\r",
            post_edit_revision_id="rev:after",
        )
        self.assertTrue(effective(receipt.after_state, "bold", 2))

    def test_lf_must_be_normalized_before_insert_format(self):
        s = state()
        with self.assertRaisesRegex(TextInsertFormatError, "canonical"):
            plan_text_insert_format_v1(
                before_state=s,
                edit_start_scalar=1,
                edit_end_scalar=1,
                replacement_text="\n",
                post_edit_revision_id="rev:after",
            )

    def test_undo_and_replay_restore_exact_format_state(self):
        s = state(
            basefmt=fmt(bold=True),
            overrides=(TextFormatOverrideRunV1(0,2,"italic",True),),
        )
        receipt = plan_text_insert_format_v1(
            before_state=s,
            edit_start_scalar=2,
            edit_end_scalar=2,
            replacement_text="XY",
            post_edit_revision_id="rev:after",
            typing_snapshot=TypingFormatSnapshotV1((("bold",False),)),
        )
        self.assertEqual(s, undo_text_insert_format_v1(receipt))
        self.assertEqual(receipt.after_state, replay_text_insert_format_v1(receipt))


if __name__ == "__main__":
    unittest.main()
