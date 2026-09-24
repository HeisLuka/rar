#!/usr/bin/env python3
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "services" / "editor-api"))

from resolved_text_caret_map_v1 import (
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_keyboard_policy_v1 import (
    UNICODE_GRAPHEME_VERSION,
    TextKeyboardPolicyError,
    apply_text_keyboard_policy_v1,
    grapheme_boundaries_v1,
)
from text_selection_state_v1 import build_text_selection_state_v1


STORY_ID = "story:keyboard"
REVISION_ID = "sha256:" + "a" * 64
PAGE_ID = "page:1"
FRAME_ID = "frame:1"


def caret_map_for_clusters(story_text, clusters):
    resolved = []
    x = 0
    for start, end in clusters:
        width = max(1, end - start) * 1000
        resolved.append(
            ResolvedClusterV1(
                start_scalar=start,
                end_scalar=end,
                page_x_start_emu=x,
                page_x_end_emu=x + width,
                frame_x_start_emu=x,
                frame_x_end_emu=x + width,
                painted=True,
            )
        )
        x += width
    line = ResolvedLineFragmentV1(
        story_id=STORY_ID,
        page_id=PAGE_ID,
        frame_id=FRAME_ID,
        line_id="line:0",
        flow_ordinal=0,
        previous_line_id=None,
        next_line_id=None,
        page_y_top_emu=0,
        page_y_bottom_emu=1000,
        frame_y_top_emu=0,
        frame_y_bottom_emu=1000,
        clusters=tuple(resolved),
    )
    return build_resolved_text_caret_map_v1(
        layout_revision_id=REVISION_ID,
        story_id=STORY_ID,
        story_scalar_len=len(story_text),
        lines=(line,),
    )


def full_scalar_caret_map(story_text):
    return caret_map_for_clusters(
        story_text,
        [(index, index + 1) for index in range(len(story_text))],
    )


def selection(domain, anchor, focus):
    return build_text_selection_state_v1(
        domain=domain,
        revision_id=REVISION_ID,
        anchor_scalar=anchor,
        focus_scalar=focus,
    )


class GraphemePolicyTests(unittest.TestCase):
    def test_profile_is_pinned(self):
        self.assertEqual("15.0.0", UNICODE_GRAPHEME_VERSION)

    def test_ascii_boundaries(self):
        self.assertEqual((0, 1, 2, 3), grapheme_boundaries_v1("abc"))

    def test_combining_sequence_is_one_grapheme(self):
        self.assertEqual((0, 2, 3), grapheme_boundaries_v1("a\u0301b"))

    def test_zwj_family_is_one_grapheme(self):
        family = "👨‍👩‍👧‍👦"
        self.assertEqual((0, len(family)), grapheme_boundaries_v1(family))

    def test_regional_indicator_pairs(self):
        text = "🇺🇸🇨🇦"
        self.assertEqual((0, 2, 4), grapheme_boundaries_v1(text))

    def test_crlf_is_one_grapheme_boundary_unit(self):
        self.assertEqual((0, 1, 3, 4), grapheme_boundaries_v1("a\r\nb"))


class KeyboardPolicyTests(unittest.TestCase):
    def test_move_previous_and_next_use_grapheme_not_scalar(self):
        text = "a\u0301b"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        caret_map = caret_map_for_clusters(text, [(0, 2), (2, 3)])

        previous = apply_text_keyboard_policy_v1(
            command="move_previous",
            story_text=text,
            domain=domain,
            selection=selection(domain, 2, 2),
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual("selection", previous.action)
        self.assertEqual(0, previous.selection.focus_scalar)

        next_result = apply_text_keyboard_policy_v1(
            command="move_next",
            story_text=text,
            domain=domain,
            selection=selection(domain, 0, 0),
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual(2, next_result.selection.focus_scalar)

    def test_nonempty_selection_delete_is_exact_scalar_range(self):
        text = "abcd"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        result = apply_text_keyboard_policy_v1(
            command="delete_backward",
            story_text=text,
            domain=domain,
            selection=selection(domain, 3, 1),
            caret_map=full_scalar_caret_map(text),
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual("delete", result.action)
        self.assertEqual(1, result.delete_intent.start_scalar)
        self.assertEqual(3, result.delete_intent.end_scalar)
        self.assertEqual("", result.delete_intent.replacement_text)

    def test_backward_delete_removes_whole_zwj_family(self):
        family = "👨‍👩‍👧‍👦"
        text = family + "x"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        caret_map = caret_map_for_clusters(text, [(0, len(family)), (len(family), len(text))])
        result = apply_text_keyboard_policy_v1(
            command="delete_backward",
            story_text=text,
            domain=domain,
            selection=selection(domain, len(family), len(family)),
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual((0, len(family)), (
            result.delete_intent.start_scalar,
            result.delete_intent.end_scalar,
        ))

    def test_forward_delete_removes_whole_flag_pair(self):
        text = "🇺🇸x"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        caret_map = caret_map_for_clusters(text, [(0, 2), (2, 3)])
        result = apply_text_keyboard_policy_v1(
            command="delete_forward",
            story_text=text,
            domain=domain,
            selection=selection(domain, 0, 0),
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual((0, 2), (
            result.delete_intent.start_scalar,
            result.delete_intent.end_scalar,
        ))

    def test_protected_terminal_cr_is_boundary_noop(self):
        text = "abc\r"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="imported_mature_quill_terminal_cr",
        )
        caret_map = full_scalar_caret_map(text)

        forward = apply_text_keyboard_policy_v1(
            command="delete_forward",
            story_text=text,
            domain=domain,
            selection=selection(domain, 3, 3),
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual("boundary_noop", forward.action)

        next_result = apply_text_keyboard_policy_v1(
            command="move_next",
            story_text=text,
            domain=domain,
            selection=selection(domain, 3, 3),
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual(3, next_result.selection.focus_scalar)

    def test_extend_preserves_anchor(self):
        text = "abc"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        result = apply_text_keyboard_policy_v1(
            command="extend_next",
            story_text=text,
            domain=domain,
            selection=selection(domain, 1, 1),
            caret_map=full_scalar_caret_map(text),
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual(1, result.selection.anchor_scalar)
        self.assertEqual(2, result.selection.focus_scalar)

    def test_move_collapses_nonempty_selection_toward_direction(self):
        text = "abcd"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        caret_map = full_scalar_caret_map(text)
        selected = selection(domain, 3, 1)

        left = apply_text_keyboard_policy_v1(
            command="move_previous",
            story_text=text,
            domain=domain,
            selection=selected,
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual((1, 1), (left.selection.anchor_scalar, left.selection.focus_scalar))

        right = apply_text_keyboard_policy_v1(
            command="move_next",
            story_text=text,
            domain=domain,
            selection=selected,
            caret_map=caret_map,
            expected_revision_id=REVISION_ID,
        )
        self.assertEqual((3, 3), (right.selection.anchor_scalar, right.selection.focus_scalar))

    def test_ligature_internal_logical_boundary_fails_without_shaping_caret(self):
        text = "fi"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        # One shaping cluster covers both scalar graphemes, with no explicit
        # internal caret authority at scalar boundary 1.
        caret_map = caret_map_for_clusters(text, [(0, 2)])
        with self.assertRaisesRegex(TextKeyboardPolicyError, "cluster interior"):
            apply_text_keyboard_policy_v1(
                command="move_next",
                story_text=text,
                domain=domain,
                selection=selection(domain, 0, 0),
                caret_map=caret_map,
                expected_revision_id=REVISION_ID,
            )

    def test_stale_revision_fails_closed(self):
        text = "abc"
        domain = derive_story_edit_domain_v1(
            story_id=STORY_ID,
            story_text=text,
            provenance="chaptera_created",
        )
        with self.assertRaises(TextKeyboardPolicyError) as caught:
            apply_text_keyboard_policy_v1(
                command="move_next",
                story_text=text,
                domain=domain,
                selection=selection(domain, 0, 0),
                caret_map=full_scalar_caret_map(text),
                expected_revision_id="sha256:" + "b" * 64,
            )
        self.assertEqual("stale_selection_revision", caught.exception.code)


if __name__ == "__main__":
    unittest.main()
