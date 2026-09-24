#!/usr/bin/env python3
import unittest

from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)
from text_insert_format_v1 import plan_text_insert_format_v1
from text_selection_state_v1 import build_text_selection_state_v1
from text_typing_format_state_v1 import (
    advance_typing_state_after_continuous_insert_v1,
    clear_pending_typing_property_v1,
    clear_typing_state_for_context_change_v1,
    derive_typing_format_state_v1,
    displayed_character_properties_v1,
    set_pending_typing_property_v1,
    snapshot_typing_format_v1,
)


STORY = "story:1"


def domain(text):
    return derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=text,
        provenance="chaptera_created",
    )


def fmt_state(text, revision="fmt:1", *, bold=False, italic=False):
    base = BaseCharacterFormatV1(
        font_resource_id="font:default",
        font_size_emu=152400,
        bold=bold,
        italic=italic,
        text_color_rgb="#112233",
    )
    return build_text_format_overlay_state_v1(
        story_id=STORY,
        base_revision_id=revision,
        story_scalar_len=len(text),
        base_runs=(() if not text else (BaseFormatRunV1(0, len(text), base),)),
    )


def caret(text, scalar, revision="rev:1"):
    return build_text_selection_state_v1(
        domain=domain(text),
        revision_id=revision,
        anchor_scalar=scalar,
        focus_scalar=scalar,
    )


class TextTypingFormatStateV1Tests(unittest.TestCase):
    def test_collapsed_caret_starts_with_no_pending_properties(self):
        selection = caret("abcd", 2)
        state = derive_typing_format_state_v1(
            selection=selection,
            domain=domain("abcd"),
            format_state=fmt_state("abcd"),
        )
        self.assertIsNotNone(state)
        self.assertEqual((), state.pending_explicit_properties)
        self.assertIsNone(snapshot_typing_format_v1(state))

    def test_noncollapsed_selection_has_no_typing_state(self):
        selection = build_text_selection_state_v1(
            domain=domain("abcd"),
            revision_id="rev:1",
            anchor_scalar=1,
            focus_scalar=3,
        )
        self.assertIsNone(
            derive_typing_format_state_v1(
                selection=selection,
                domain=domain("abcd"),
                format_state=fmt_state("abcd"),
            )
        )

    def test_explicit_false_is_distinct_from_absent(self):
        selection = caret("abcd", 2)
        state = derive_typing_format_state_v1(
            selection=selection,
            domain=domain("abcd"),
            format_state=fmt_state("abcd", bold=True),
        )
        state = set_pending_typing_property_v1(
            state=state,
            prop="bold",
            value=False,
        )
        self.assertEqual((("bold", False),), state.pending_explicit_properties)
        self.assertEqual(
            (("bold", False),),
            snapshot_typing_format_v1(state).items,
        )
        cleared = clear_pending_typing_property_v1(state=state, prop="bold")
        self.assertEqual((), cleared.pending_explicit_properties)
        self.assertIsNone(snapshot_typing_format_v1(cleared))

    def test_displayed_format_overlays_pending_on_canonical_effective_context(self):
        selection = caret("abcd", 2)
        format_state = fmt_state("abcd", bold=True, italic=False)
        state = derive_typing_format_state_v1(
            selection=selection,
            domain=domain("abcd"),
            format_state=format_state,
        )
        state = set_pending_typing_property_v1(state=state, prop="italic", value=True)
        shown = displayed_character_properties_v1(
            state=state,
            selection=selection,
            domain=domain("abcd"),
            format_state=format_state,
        )
        self.assertTrue(shown["bold"])
        self.assertTrue(shown["italic"])
        self.assertEqual(152400, shown["font_size_emu"])
        self.assertEqual("#112233", shown["text_color_rgb"])

    def test_collapsed_format_command_creates_no_durable_zero_length_span(self):
        selection = caret("abcd", 2)
        format_state = fmt_state("abcd")
        state = derive_typing_format_state_v1(
            selection=selection,
            domain=domain("abcd"),
            format_state=format_state,
        )
        state = set_pending_typing_property_v1(state=state, prop="bold", value=True)
        self.assertEqual((), format_state.overrides)
        self.assertEqual((("bold", True),), state.pending_explicit_properties)

    def test_pending_snapshot_materializes_only_over_committed_nonempty_insert(self):
        selection = caret("abcd", 2)
        before = fmt_state("abcd")
        state = derive_typing_format_state_v1(
            selection=selection,
            domain=domain("abcd"),
            format_state=before,
        )
        state = set_pending_typing_property_v1(state=state, prop="bold", value=True)
        receipt = plan_text_insert_format_v1(
            before_state=before,
            edit_start_scalar=2,
            edit_end_scalar=2,
            replacement_text="XY",
            post_edit_revision_id="fmt:2",
            typing_snapshot=snapshot_typing_format_v1(state),
        )
        bold_runs = [
            run for run in receipt.after_state.overrides if run.property == "bold"
        ]
        self.assertEqual(1, len(bold_runs))
        self.assertEqual((2, 4, True), (
            bold_runs[0].start_scalar,
            bold_runs[0].end_scalar,
            bold_runs[0].value,
        ))

    def test_continuous_typing_advances_context_but_keeps_pending_properties(self):
        old_selection = caret("abcd", 2, "rev:1")
        old_format = fmt_state("abcd", "fmt:1")
        state = derive_typing_format_state_v1(
            selection=old_selection,
            domain=domain("abcd"),
            format_state=old_format,
        )
        state = set_pending_typing_property_v1(state=state, prop="italic", value=True)

        new_selection = caret("abXcd", 3, "rev:2")
        new_format = fmt_state("abXcd", "fmt:2")
        advanced = advance_typing_state_after_continuous_insert_v1(
            state=state,
            previous_selection=old_selection,
            previous_domain=domain("abcd"),
            previous_format_state=old_format,
            resulting_selection=new_selection,
            resulting_domain=domain("abXcd"),
            resulting_format_state=new_format,
        )
        self.assertEqual((("italic", True),), advanced.pending_explicit_properties)
        self.assertEqual("rev:2", advanced.revision_id)
        self.assertEqual(3, advanced.caret_scalar)

    def test_pointer_or_keyboard_relocation_clears_pending_properties(self):
        original = caret("abcd", 2, "rev:1")
        state = derive_typing_format_state_v1(
            selection=original,
            domain=domain("abcd"),
            format_state=fmt_state("abcd", "fmt:1"),
        )
        state = set_pending_typing_property_v1(state=state, prop="bold", value=True)

        relocated = caret("abcd", 1, "rev:1")
        cleared = clear_typing_state_for_context_change_v1(
            selection=relocated,
            domain=domain("abcd"),
            format_state=fmt_state("abcd", "fmt:1"),
        )
        self.assertEqual((), cleared.pending_explicit_properties)
        self.assertNotEqual(state.caret_scalar, cleared.caret_scalar)

    def test_history_revision_change_clears_and_rederives(self):
        selection = caret("abcd", 2, "rev:1")
        state = derive_typing_format_state_v1(
            selection=selection,
            domain=domain("abcd"),
            format_state=fmt_state("abcd", "fmt:1"),
        )
        state = set_pending_typing_property_v1(state=state, prop="bold", value=True)
        after_history = caret("abcd", 2, "rev:history")
        rederived = clear_typing_state_for_context_change_v1(
            selection=after_history,
            domain=domain("abcd"),
            format_state=fmt_state("abcd", "fmt:history"),
        )
        self.assertEqual((), rederived.pending_explicit_properties)
        self.assertEqual("rev:history", rederived.revision_id)

    def test_composition_snapshot_is_immutable_after_later_transient_change(self):
        selection = caret("abcd", 2)
        state = derive_typing_format_state_v1(
            selection=selection,
            domain=domain("abcd"),
            format_state=fmt_state("abcd"),
        )
        state = set_pending_typing_property_v1(state=state, prop="bold", value=True)
        frozen = snapshot_typing_format_v1(state)
        later = set_pending_typing_property_v1(state=state, prop="italic", value=True)
        self.assertEqual((("bold", True),), frozen.items)
        self.assertEqual(
            (("bold", True), ("italic", True)),
            later.pending_explicit_properties,
        )


if __name__ == "__main__":
    unittest.main()
