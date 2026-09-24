#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    InternalCaretStopV1,
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    build_resolved_text_caret_map_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1
from story_edit_transaction_v1 import validate_story_edit_transaction_request_v1
from text_keyboard_policy_v1 import (
    GRAPHEME_PROFILE_V1,
    TextKeyboardPolicyError,
    apply_text_keyboard_policy_v1,
    grapheme_boundaries_v1,
)
from text_selection_state_v1 import (
    build_text_selection_state_v1,
    project_selection_state_v1,
)


STORY = "story:1"


def domain(text, provenance="chaptera_created"):
    return derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=text,
        provenance=provenance,
    )


def caret_map(text, *, clusters=None, internal=()):
    if clusters is None:
        clusters = tuple((i, i + 1) for i in range(len(text)))
    resolved = []
    x = 0
    for index, (start, end) in enumerate(clusters):
        width = 100 * max(1, end - start)
        stops = tuple(
            InternalCaretStopV1(
                scalar_boundary=scalar,
                page_x_emu=x + 100 * (scalar - start),
                frame_x_emu=x + 100 * (scalar - start),
            )
            for scalar in internal
            if start < scalar < end
        )
        resolved.append(
            ResolvedClusterV1(
                start_scalar=start,
                end_scalar=end,
                page_x_start_emu=x,
                page_x_end_emu=x + width,
                frame_x_start_emu=x,
                frame_x_end_emu=x + width,
                internal_caret_stops=stops,
            )
        )
        x += width
    return build_resolved_text_caret_map_v1(
        layout_revision_id="layout:1",
        story_id=STORY,
        story_scalar_len=len(text),
        lines=(
            ResolvedLineFragmentV1(
                story_id=STORY,
                page_id="page:1",
                frame_id="frame:1",
                line_id="line:1",
                flow_ordinal=0,
                previous_line_id=None,
                next_line_id=None,
                page_y_top_emu=0,
                page_y_bottom_emu=200,
                frame_y_top_emu=0,
                frame_y_bottom_emu=200,
                clusters=tuple(resolved),
            ),
        ),
    )


def selection(text, anchor, focus, *, cmap=None, provenance="chaptera_created"):
    d = domain(text, provenance)
    state = build_text_selection_state_v1(
        domain=d,
        revision_id="rev:1",
        anchor_scalar=anchor,
        focus_scalar=focus,
    )
    if cmap is not None:
        kwargs = {}
        if anchor == focus:
            matches = [s for s in cmap.caret_stops if s.scalar_boundary == anchor]
            if len(matches) == 1:
                kwargs["anchor_stop_id"] = matches[0].stop_id
        try:
            state = project_selection_state_v1(
                state=state,
                domain=d,
                caret_map=cmap,
                **kwargs,
            ).state
        except Exception:
            pass
    return state


class GraphemeProfileV1Tests(unittest.TestCase):
    def test_ascii_and_supplementary_scalar_boundaries(self):
        self.assertEqual((0, 1, 2, 3), grapheme_boundaries_v1("abc"))
        self.assertEqual((0, 1, 2), grapheme_boundaries_v1("😀x"))

    def test_combining_sequence_is_one_grapheme(self):
        self.assertEqual((0, 2), grapheme_boundaries_v1("a\u0301"))

    def test_zwj_family_is_one_grapheme(self):
        family = "👨‍👩‍👧‍👦"
        self.assertEqual((0, len(family)), grapheme_boundaries_v1(family))

    def test_regional_indicator_flag_is_one_grapheme(self):
        flag = "🇺🇸"
        self.assertEqual((0, 2), grapheme_boundaries_v1(flag))

    def test_crlf_is_one_control_cluster_but_canonical_cr_is_independent(self):
        self.assertEqual((0, 2), grapheme_boundaries_v1("\r\n"))
        self.assertEqual((0, 1, 2, 3), grapheme_boundaries_v1("a\rb"))


class TextKeyboardPolicyV1Tests(unittest.TestCase):
    def run_policy(self, command, text, state, cmap, provenance="chaptera_created", op="keyboard-op-0001"):
        return apply_text_keyboard_policy_v1(
            command=command,
            story_text=text,
            domain=domain(text, provenance),
            selection=state,
            caret_map=cmap,
            document_id="doc:1",
            source_hash="a" * 64,
            client_operation_id=op,
        )

    def test_move_previous_next_and_extend_are_transient(self):
        text = "abc"
        cmap = caret_map(text)
        state = selection(text, 1, 1, cmap=cmap)
        moved = self.run_policy("move_next", text, state, cmap)
        self.assertEqual("selection", moved.result_kind)
        self.assertEqual((2, 2), (
            moved.selection.anchor_scalar,
            moved.selection.focus_scalar,
        ))
        self.assertIsNone(moved.request)
        extended = self.run_policy("extend_next", text, moved.selection, cmap)
        self.assertEqual((2, 3), (
            extended.selection.anchor_scalar,
            extended.selection.focus_scalar,
        ))
        self.assertEqual(GRAPHEME_PROFILE_V1, extended.grapheme_profile)

    def test_unmodified_arrow_collapses_nonempty_selection_without_extra_step(self):
        text = "abcd"
        cmap = caret_map(text)
        state = selection(text, 1, 3, cmap=cmap)
        left = self.run_policy("move_previous", text, state, cmap)
        right = self.run_policy("move_next", text, state, cmap)
        self.assertEqual((1, 1), (left.selection.anchor_scalar, left.selection.focus_scalar))
        self.assertEqual((3, 3), (right.selection.anchor_scalar, right.selection.focus_scalar))

    def test_boundary_navigation_and_delete_are_deterministic_noop(self):
        text = "ab"
        cmap = caret_map(text)
        start = selection(text, 0, 0, cmap=cmap)
        end = selection(text, 2, 2, cmap=cmap)
        self.assertEqual(
            "boundary_noop",
            self.run_policy("move_previous", text, start, cmap).result_kind,
        )
        self.assertEqual(
            "boundary_noop",
            self.run_policy("delete_backward", text, start, cmap).result_kind,
        )
        self.assertEqual(
            "boundary_noop",
            self.run_policy("move_next", text, end, cmap).result_kind,
        )
        self.assertEqual(
            "boundary_noop",
            self.run_policy("delete_forward", text, end, cmap).result_kind,
        )

    def test_combining_and_zwj_delete_lower_to_one_canonical_transaction(self):
        for text in ("a\u0301x", "👨‍👩‍👧‍👦x", "🇺🇸x"):
            cmap = caret_map(text)
            boundary = grapheme_boundaries_v1(text)[1]
            state = selection(text, boundary, boundary, cmap=cmap)
            result = self.run_policy("delete_backward", text, state, cmap)
            self.assertEqual((0, boundary), (
                result.delete_start_scalar,
                result.delete_end_scalar,
            ))
            self.assertEqual("", result.request["command"]["replacement_text"])
            self.assertEqual(text[:boundary], result.request["command"]["expected_before"])
            validate_story_edit_transaction_request_v1(result.request)

    def test_nonempty_selection_deletes_exact_canonical_range(self):
        text = "abcdef"
        cmap = caret_map(text)
        state = selection(text, 4, 2, cmap=cmap)
        result = self.run_policy("delete_forward", text, state, cmap)
        self.assertEqual((2, 4), (
            result.delete_start_scalar,
            result.delete_end_scalar,
        ))
        self.assertEqual("cd", result.request["command"]["expected_before"])

    def test_protected_terminal_cr_is_never_crossed_or_clamped(self):
        text = "ab\r"
        cmap = caret_map(text, clusters=((0, 1), (1, 2)))
        state = selection(
            text,
            2,
            2,
            cmap=cmap,
            provenance="imported_mature_quill_terminal_cr",
        )
        forward = self.run_policy(
            "delete_forward",
            text,
            state,
            cmap,
            provenance="imported_mature_quill_terminal_cr",
        )
        self.assertEqual("boundary_noop", forward.result_kind)
        back = self.run_policy(
            "delete_backward",
            text,
            state,
            cmap,
            provenance="imported_mature_quill_terminal_cr",
        )
        self.assertEqual((1, 2), (back.delete_start_scalar, back.delete_end_scalar))

    def test_real_ligature_logical_boundary_without_internal_caret_fails_closed(self):
        text = "fi"
        cmap = caret_map(text, clusters=((0, 2),))
        state = selection(text, 0, 0, cmap=cmap)
        with self.assertRaises(TextKeyboardPolicyError) as caught:
            self.run_policy("move_next", text, state, cmap)
        self.assertEqual("caret_geometry_unsupported", caught.exception.code)

    def test_internal_caret_authority_allows_logical_fi_boundary(self):
        text = "fi"
        cmap = caret_map(text, clusters=((0, 2),), internal=(1,))
        state = selection(text, 0, 0, cmap=cmap)
        moved = self.run_policy("move_next", text, state, cmap)
        self.assertEqual(1, moved.selection.focus_scalar)

    def test_collapsed_delete_from_non_grapheme_internal_caret_requires_reconcile(self):
        text = "a\u0301"
        cmap = caret_map(text, clusters=((0, 2),), internal=(1,))
        state = selection(text, 1, 1, cmap=cmap)
        with self.assertRaises(TextKeyboardPolicyError) as caught:
            self.run_policy("delete_backward", text, state, cmap)
        self.assertEqual("reconcile_required", caught.exception.code)


if __name__ == "__main__":
    unittest.main()
