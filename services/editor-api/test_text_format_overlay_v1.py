#!/usr/bin/env python3
import unittest

from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverlayError,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    clear_text_format_property_override_v1,
    effective_property_segments_v1,
    replay_text_format_operation_v1,
    set_text_format_property_v1,
    state_hash_v1,
    undo_text_format_operation_v1,
)


def fmt(*, size=12000, bold=False, italic=False, color="#000000", font="font:resolved"):
    return BaseCharacterFormatV1(font, size, bold, italic, color)


def state(base_runs, overrides=(), story_len=6):
    return build_text_format_overlay_state_v1(
        story_id="story:1",
        base_revision_id="rev:1",
        story_scalar_len=story_len,
        base_runs=tuple(base_runs),
        overrides=tuple(overrides),
    )


class TextFormatOverlayV1Tests(unittest.TestCase):
    def test_explicit_false_differs_from_inherit_when_base_is_true(self):
        s = state((BaseFormatRunV1(0, 6, fmt(bold=True)),))
        receipt = set_text_format_property_v1(
            state=s,
            start_scalar=1,
            end_scalar=5,
            prop="bold",
            value=False,
            expected_state_hash=state_hash_v1(s),
        )
        self.assertEqual(
            (TextFormatOverrideRunV1(1, 5, "bold", False),),
            receipt.after_state.overrides,
        )
        seg = effective_property_segments_v1(
            state=receipt.after_state,
            prop="bold",
            start_scalar=1,
            end_scalar=5,
        )
        self.assertEqual(False, seg[0].value)
        self.assertEqual("chaptera_override", seg[0].source)

    def test_redundant_explicit_value_equal_to_base_normalizes_away(self):
        s = state((BaseFormatRunV1(0, 6, fmt(bold=False)),))
        receipt = set_text_format_property_v1(
            state=s,
            start_scalar=0,
            end_scalar=6,
            prop="bold",
            value=False,
            expected_state_hash=state_hash_v1(s),
        )
        self.assertEqual((), receipt.after_state.overrides)

    def test_clear_reveals_immutable_base_only_on_requested_range(self):
        s = state(
            (BaseFormatRunV1(0, 6, fmt(bold=True)),),
            (TextFormatOverrideRunV1(0, 6, "bold", False),),
        )
        receipt = clear_text_format_property_override_v1(
            state=s,
            start_scalar=2,
            end_scalar=4,
            prop="bold",
            expected_state_hash=state_hash_v1(s),
        )
        self.assertEqual(
            (
                TextFormatOverrideRunV1(0, 2, "bold", False),
                TextFormatOverrideRunV1(4, 6, "bold", False),
            ),
            receipt.after_state.overrides,
        )
        middle = effective_property_segments_v1(
            state=receipt.after_state,
            prop="bold",
            start_scalar=2,
            end_scalar=4,
        )
        self.assertEqual(True, middle[0].value)
        self.assertEqual("base", middle[0].source)

    def test_adjacent_equal_overrides_coalesce_across_base_run_boundary(self):
        s = state((
            BaseFormatRunV1(0, 3, fmt(bold=False)),
            BaseFormatRunV1(3, 6, fmt(bold=False, color="#111111")),
        ))
        receipt = set_text_format_property_v1(
            state=s,
            start_scalar=0,
            end_scalar=6,
            prop="bold",
            value=True,
            expected_state_hash=state_hash_v1(s),
        )
        self.assertEqual(
            (TextFormatOverrideRunV1(0, 6, "bold", True),),
            receipt.after_state.overrides,
        )

    def test_overlap_history_normalizes_to_same_final_state(self):
        base = (BaseFormatRunV1(0, 6, fmt(bold=False)),)
        a0 = state(base)
        a1 = set_text_format_property_v1(
            state=a0,
            start_scalar=0,
            end_scalar=6,
            prop="bold",
            value=True,
            expected_state_hash=state_hash_v1(a0),
        ).after_state
        a2 = clear_text_format_property_override_v1(
            state=a1,
            start_scalar=2,
            end_scalar=4,
            prop="bold",
            expected_state_hash=state_hash_v1(a1),
        ).after_state

        b = state(
            base,
            (
                TextFormatOverrideRunV1(0, 2, "bold", True),
                TextFormatOverrideRunV1(4, 6, "bold", True),
            ),
        )
        self.assertEqual(b, a2)
        self.assertEqual(state_hash_v1(b), state_hash_v1(a2))

    def test_zero_length_durable_override_is_rejected(self):
        s = state((BaseFormatRunV1(0, 6, fmt()),))
        with self.assertRaisesRegex(TextFormatOverlayError, "non-empty"):
            set_text_format_property_v1(
                state=s,
                start_scalar=2,
                end_scalar=2,
                prop="italic",
                value=True,
                expected_state_hash=state_hash_v1(s),
            )

    def test_stale_state_hash_is_rejected(self):
        s = state((BaseFormatRunV1(0, 6, fmt()),))
        with self.assertRaisesRegex(TextFormatOverlayError, "stale"):
            set_text_format_property_v1(
                state=s,
                start_scalar=0,
                end_scalar=2,
                prop="italic",
                value=True,
                expected_state_hash="deadbeef",
            )

    def test_unsupported_font_family_and_invalid_values_fail_closed(self):
        s = state((BaseFormatRunV1(0, 6, fmt()),))
        with self.assertRaisesRegex(TextFormatOverlayError, "unsupported"):
            set_text_format_property_v1(
                state=s,
                start_scalar=0,
                end_scalar=2,
                prop="font_family",
                value="Arial",
                expected_state_hash=state_hash_v1(s),
            )
        with self.assertRaises(TextFormatOverlayError):
            set_text_format_property_v1(
                state=s,
                start_scalar=0,
                end_scalar=2,
                prop="text_color_rgb",
                value="red",
                expected_state_hash=state_hash_v1(s),
            )

    def test_missing_resolved_font_resource_fails_closed(self):
        with self.assertRaisesRegex(TextFormatOverlayError, "font_resource_id"):
            state((BaseFormatRunV1(0, 6, fmt(font="")),))

    def test_all_v1_properties_are_canonicalized(self):
        s = state((BaseFormatRunV1(0, 6, fmt()),))
        current = s
        commands = (
            ("font_size_emu", 15000),
            ("bold", True),
            ("italic", True),
            ("text_color_rgb", "#aa00cc"),
        )
        for prop, value in commands:
            current = set_text_format_property_v1(
                state=current,
                start_scalar=1,
                end_scalar=5,
                prop=prop,
                value=value,
                expected_state_hash=state_hash_v1(current),
            ).after_state
        self.assertEqual(4, len(current.overrides))
        color = [r for r in current.overrides if r.property == "text_color_rgb"][0]
        self.assertEqual("#AA00CC", color.value)

    def test_receipt_undo_and_replay_are_exact(self):
        s = state((BaseFormatRunV1(0, 6, fmt(bold=True)),))
        receipt = set_text_format_property_v1(
            state=s,
            start_scalar=1,
            end_scalar=5,
            prop="bold",
            value=False,
            expected_state_hash=state_hash_v1(s),
        )
        self.assertEqual(s, undo_text_format_operation_v1(receipt))
        self.assertEqual(receipt.after_state, replay_text_format_operation_v1(receipt))
        self.assertTrue(receipt.requires_authoritative_relayout)
        self.assertEqual("chaptera_override_or_explicit_loss", receipt.export_policy)


if __name__ == "__main__":
    unittest.main()
