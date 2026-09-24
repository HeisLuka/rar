#!/usr/bin/env python3
import json
import unittest

from story_range_v1 import (
    StoryRangeError,
    apply_story_range_inverse_v1,
    replace_story_range_v1,
    replay_story_range_operation_v1,
    story_state_id_v1,
)


class StoryRangeV1Tests(unittest.TestCase):
    def test_ascii_insert_delete_replace(self):
        inserted = replace_story_range_v1(
            story_id="s",
            story_text="AB",
            start_scalar=1,
            end_scalar=1,
            expected_before="",
            replacement_text="x",
        )
        self.assertEqual("AxB", inserted.after_text)

        deleted = replace_story_range_v1(
            story_id="s",
            story_text="AxB",
            start_scalar=1,
            end_scalar=2,
            expected_before="x",
            replacement_text="",
        )
        self.assertEqual("AB", deleted.after_text)

        replaced = replace_story_range_v1(
            story_id="s",
            story_text="ABC",
            start_scalar=1,
            end_scalar=2,
            expected_before="B",
            replacement_text="xy",
        )
        self.assertEqual("AxyC", replaced.after_text)

    def test_bmp_and_supplementary_emoji_use_scalar_indices(self):
        result = replace_story_range_v1(
            story_id="s",
            story_text="A漢😀B",
            start_scalar=2,
            end_scalar=3,
            expected_before="😀",
            replacement_text="Z",
        )
        self.assertEqual("A漢ZB", result.after_text)
        self.assertEqual(2, result.operation["start_scalar"])
        self.assertEqual(3, result.operation["end_scalar"])

    def test_combining_sequence_is_multiple_scalars_and_not_normalized(self):
        decomposed = "e\u0301"
        result = replace_story_range_v1(
            story_id="s",
            story_text="A" + decomposed + "B",
            start_scalar=1,
            end_scalar=3,
            expected_before=decomposed,
            replacement_text="é",
        )
        self.assertEqual("AéB", result.after_text)
        self.assertEqual(decomposed, result.operation["expected_before"])
        self.assertEqual("é", result.operation["replacement_text"])

    def test_u000d_paragraph_boundary_is_an_ordinary_explicit_scalar(self):
        result = replace_story_range_v1(
            story_id="s",
            story_text="one\rtwo",
            start_scalar=3,
            end_scalar=4,
            expected_before="\r",
            replacement_text="\r\r",
        )
        self.assertEqual("one\r\rtwo", result.after_text)

    def test_expected_before_forgery_and_out_of_bounds_fail_closed(self):
        with self.assertRaisesRegex(StoryRangeError, "expected_before"):
            replace_story_range_v1(
                story_id="s",
                story_text="ABC",
                start_scalar=1,
                end_scalar=2,
                expected_before="X",
                replacement_text="Y",
            )
        with self.assertRaisesRegex(StoryRangeError, "outside"):
            replace_story_range_v1(
                story_id="s",
                story_text="ABC",
                start_scalar=1,
                end_scalar=4,
                expected_before="BC",
                replacement_text="Y",
            )

    def test_surrogate_is_not_a_canonical_scalar(self):
        with self.assertRaisesRegex(StoryRangeError, "surrogate"):
            replace_story_range_v1(
                story_id="s",
                story_text="A\ud800B",
                start_scalar=1,
                end_scalar=2,
                expected_before="\ud800",
                replacement_text="X",
            )

    def test_exact_inverse_restores_scalar_identical_story(self):
        before = "A😀e\u0301\rB"
        result = replace_story_range_v1(
            story_id="s",
            story_text=before,
            start_scalar=1,
            end_scalar=4,
            expected_before="😀e\u0301",
            replacement_text="漢字",
        )
        restored = apply_story_range_inverse_v1(
            story_id="s",
            story_text=result.after_text,
            operation=result.operation,
        )
        self.assertEqual(before, restored)

    def test_serialized_replay_is_deterministic(self):
        result = replace_story_range_v1(
            story_id="s",
            story_text="A😀B",
            start_scalar=1,
            end_scalar=2,
            expected_before="😀",
            replacement_text="漢",
        )
        serialized = json.loads(json.dumps(result.operation, ensure_ascii=False))
        replayed = replay_story_range_operation_v1(
            story_text="A😀B",
            operation=serialized,
        )
        self.assertEqual("A漢B", replayed)

    def test_equivalent_final_story_has_same_story_state_identity(self):
        first = replace_story_range_v1(
            story_id="s",
            story_text="ABC",
            start_scalar=0,
            end_scalar=1,
            expected_before="A",
            replacement_text="X",
        )
        second = replace_story_range_v1(
            story_id="s",
            story_text="XBC",
            start_scalar=1,
            end_scalar=2,
            expected_before="B",
            replacement_text="B",
        )
        self.assertEqual("XBC", first.after_text)
        self.assertEqual("XBC", second.after_text)
        self.assertEqual(
            story_state_id_v1("s", first.after_text),
            story_state_id_v1("s", second.after_text),
        )

    def test_source_backed_terminal_cr_policy_is_explicit_not_global(self):
        author_created = replace_story_range_v1(
            story_id="a",
            story_text="ABC",
            start_scalar=3,
            end_scalar=3,
            expected_before="",
            replacement_text="D",
            requires_terminal_cr=False,
        )
        self.assertEqual("ABCD", author_created.after_text)

        with self.assertRaisesRegex(StoryRangeError, "terminal CR"):
            replace_story_range_v1(
                story_id="source",
                story_text="ABC\r",
                start_scalar=3,
                end_scalar=4,
                expected_before="\r",
                replacement_text="",
                requires_terminal_cr=True,
            )

        preserved = replace_story_range_v1(
            story_id="source",
            story_text="ABC\r",
            start_scalar=0,
            end_scalar=1,
            expected_before="A",
            replacement_text="Z",
            requires_terminal_cr=True,
        )
        self.assertEqual("ZBC\r", preserved.after_text)


if __name__ == "__main__":
    unittest.main()
