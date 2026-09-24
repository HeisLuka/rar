#!/usr/bin/env python3
import unittest

from story_edit_domain_v1 import derive_story_edit_domain_v1
from text_select_all_v1 import TextSelectAllError, select_all_story_text_v1
from text_selection_state_v1 import build_text_selection_state_v1
from text_typing_format_state_v1 import TextTypingFormatStateV1


STORY="story:1"


def domain(text,provenance="chaptera_created"):
    return derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=text,
        provenance=provenance,
    )


def selection(d,a,f,preferred=None):
    return build_text_selection_state_v1(
        domain=d,
        revision_id="rev:1",
        anchor_scalar=a,
        focus_scalar=f,
        preferred_inline_x_emu=preferred,
    )


def typing(s):
    return TextTypingFormatStateV1(
        protocol_version="chaptera.text-typing-format-state.v1",
        story_id=s.story_id,
        revision_id=s.revision_id,
        caret_scalar=s.focus_scalar,
        edit_domain_id=s.edit_domain_id,
        format_state_hash="fixture",
        pending_explicit_properties=(("bold",True),),
    )


class TextSelectAllV1Tests(unittest.TestCase):
    def test_chaptera_story_selects_full_editable_story_not_visible_slice(self):
        text="ABCDEFGHIJ"
        d=domain(text)
        s=selection(d,4,4,preferred=777)
        r=select_all_story_text_v1(
            current_selection=s,
            domain=d,
            input_owner="story_text",
            typing_state=typing(s),
        )
        self.assertEqual("selected",r.status)
        self.assertEqual((0,len(text)),r.selection.normalized_range)
        self.assertEqual(0,r.selection.anchor_scalar)
        self.assertEqual(len(text),r.selection.focus_scalar)
        self.assertIsNone(r.selection.preferred_inline_x_emu)
        self.assertIsNone(r.typing_state)
        self.assertTrue(r.preferred_inline_x_cleared)
        self.assertTrue(r.typing_state_cleared)

    def test_imported_terminal_source_mark_is_excluded(self):
        text="AB\r"
        d=domain(text,"imported_mature_quill_terminal_cr")
        s=selection(d,1,1)
        r=select_all_story_text_v1(
            current_selection=s,
            domain=d,
            input_owner="story_text",
        )
        self.assertEqual((0,2),r.selection.normalized_range)
        self.assertEqual(2,r.selection.focus_scalar)
        self.assertNotEqual(len(text),r.selection.focus_scalar)

    def test_linked_or_overset_visibility_cannot_shrink_semantic_extent(self):
        text="placed-visible|linked-next-frame|overset-tail"
        d=domain(text)
        s=selection(d,3,3)
        r=select_all_story_text_v1(
            current_selection=s,
            domain=d,
            input_owner="story_text",
        )
        self.assertEqual((0,len(text)),r.selection.normalized_range)
        self.assertEqual("layout_pending",r.selection.projection_state)

    def test_empty_editable_story_collapses_at_editable_start(self):
        d=domain("")
        s=selection(d,0,0,preferred=55)
        t=typing(s)
        r=select_all_story_text_v1(
            current_selection=s,
            domain=d,
            input_owner="story_text",
            typing_state=t,
        )
        self.assertTrue(r.selection.is_collapsed)
        self.assertEqual((0,0),r.selection.normalized_range)
        self.assertIsNone(r.selection.preferred_inline_x_emu)
        self.assertIsNone(r.typing_state)

    def test_modal_composition_and_other_focus_owners_suppress_without_state_change(self):
        d=domain("ABC")
        s=selection(d,1,1,preferred=123)
        t=typing(s)
        for owner in ("composition","modal","inspector","canvas","page_navigator"):
            r=select_all_story_text_v1(
                current_selection=s,
                domain=d,
                input_owner=owner,
                typing_state=t,
            )
            self.assertEqual("suppressed",r.status,owner)
            self.assertIs(s,r.selection,owner)
            self.assertIs(t,r.typing_state,owner)
            self.assertFalse(r.preferred_inline_x_cleared,owner)
            self.assertFalse(r.typing_state_cleared,owner)
            self.assertEqual(0,r.document_mutation_count,owner)

    def test_unknown_imported_edit_domain_fails_closed(self):
        d=domain("ABC\r","imported_unknown")
        # Cannot construct a valid ordinary selection under an unknown domain,
        # so prove the command refuses a stale selection from a known domain too.
        known=domain("ABC\r")
        s=selection(known,1,1)
        with self.assertRaises(TextSelectAllError) as caught:
            select_all_story_text_v1(
                current_selection=s,
                domain=d,
                input_owner="story_text",
            )
        self.assertIn(caught.exception.code,{"edit_domain_unknown","selection_story_mismatch","selection_reconcile_required"})

    def test_typing_state_from_other_context_is_rejected_not_silently_dropped(self):
        d=domain("ABC")
        s=selection(d,1,1)
        foreign=TextTypingFormatStateV1(
            protocol_version="chaptera.text-typing-format-state.v1",
            story_id=STORY,
            revision_id="rev:other",
            caret_scalar=1,
            edit_domain_id=s.edit_domain_id,
            format_state_hash="fixture",
            pending_explicit_properties=(),
        )
        with self.assertRaises(TextSelectAllError) as caught:
            select_all_story_text_v1(
                current_selection=s,
                domain=d,
                input_owner="story_text",
                typing_state=foreign,
            )
        self.assertEqual("typing_context_changed",caught.exception.code)

    def test_select_all_is_transient_and_creates_no_revision_or_undo(self):
        d=domain("ABC")
        s=selection(d,1,1)
        r=select_all_story_text_v1(
            current_selection=s,
            domain=d,
            input_owner="story_text",
        )
        self.assertEqual(0,r.document_mutation_count)
        self.assertEqual(0,r.undo_history_entry_count)


if __name__=="__main__":
    unittest.main()
