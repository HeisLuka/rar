#!/usr/bin/env python3
import unittest

from story_edit_domain_v1 import derive_story_edit_domain_v1
from story_edit_transaction_v1 import validate_story_edit_transaction_request_v1
from text_composition_session_v1 import (
    TextCompositionSessionError,
    cancel_text_composition_v1,
    composition_suppresses_document_shortcuts_v1,
    plan_text_composition_commit_v1,
    start_text_composition_v1,
    update_text_composition_v1,
)
from text_format_overlay_v1 import BaseCharacterFormatV1, BaseFormatRunV1, build_text_format_overlay_state_v1
from text_selection_state_v1 import build_text_selection_state_v1
from text_typing_format_state_v1 import derive_typing_format_state_v1, set_pending_typing_property_v1


STORY = "story:1"


def domain(text):
    return derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=text,
        provenance="chaptera_created",
    )


def selection(text, a, f, revision="rev:1"):
    return build_text_selection_state_v1(
        domain=domain(text),
        revision_id=revision,
        anchor_scalar=a,
        focus_scalar=f,
    )


def format_state(text):
    base = BaseCharacterFormatV1("font:1", 152400, False, False, "#000000")
    return build_text_format_overlay_state_v1(
        story_id=STORY,
        base_revision_id="fmt:1",
        story_scalar_len=len(text),
        base_runs=(() if not text else (BaseFormatRunV1(0, len(text), base),)),
    )


class TextCompositionSessionV1Tests(unittest.TestCase):
    def test_updates_are_provisional_and_create_no_document_mutation(self):
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcd",
            domain=domain("abcd"),
            selection=selection("abcd", 2, 2),
        )
        updated = update_text_composition_v1(
            session,
            provisional_external_text="漢字",
            provisional_selection_start_scalar=1,
            provisional_selection_end_scalar=2,
        )
        self.assertEqual("rev:1", updated.base_revision_id)
        self.assertEqual("漢字", updated.provisional_external_text)
        self.assertTrue(composition_suppresses_document_shortcuts_v1(updated))

    def test_cancel_restores_captured_authoritative_selection_without_mutation(self):
        captured = selection("abcd", 1, 3)
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcd",
            domain=domain("abcd"),
            selection=captured,
        )
        result = cancel_text_composition_v1(
            session,
            current_revision_id="rev:1",
            current_story_text="abcd",
            current_domain=domain("abcd"),
        )
        self.assertEqual(captured, result.restored_selection)
        self.assertEqual(0, result.document_mutation_count)
        self.assertFalse(result.undo_group_boundary)

    def test_stale_revision_fails_closed_instead_of_merging_provisional_state(self):
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcd",
            domain=domain("abcd"),
            selection=selection("abcd", 2, 2),
        )
        with self.assertRaises(TextCompositionSessionError) as caught:
            plan_text_composition_commit_v1(
                session,
                current_revision_id="rev:2",
                current_story_text="abcd",
                current_domain=domain("abcd"),
                final_external_text="漢",
                document_id="doc:1",
                source_hash="a" * 64,
                client_operation_id="ime-op-0001",
            )
        self.assertEqual("reconcile_required", caught.exception.code)

    def test_commit_is_exactly_one_existing_story_transaction(self):
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcd",
            domain=domain("abcd"),
            selection=selection("abcd", 2, 2),
        )
        plan = plan_text_composition_commit_v1(
            session,
            current_revision_id="rev:1",
            current_story_text="abcd",
            current_domain=domain("abcd"),
            final_external_text="漢",
            document_id="doc:1",
            source_hash="a" * 64,
            client_operation_id="ime-op-0001",
        )
        validate_story_edit_transaction_request_v1(plan.request)
        self.assertEqual(1, plan.document_mutation_count)
        self.assertTrue(plan.undo_group_boundary)
        self.assertEqual("story_edit_transaction", plan.request["command"]["kind"])
        self.assertEqual("collapse_after_edit", plan.post_edit_selection_intent.kind)

    def test_commit_normalizes_external_newlines_once_and_requires_paragraph_ids(self):
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcd",
            domain=domain("abcd"),
            selection=selection("abcd", 2, 2),
        )
        with self.assertRaises(TextCompositionSessionError) as caught:
            plan_text_composition_commit_v1(
                session,
                current_revision_id="rev:1",
                current_story_text="abcd",
                current_domain=domain("abcd"),
                final_external_text="x\r\ny",
                document_id="doc:1",
                source_hash="a" * 64,
                client_operation_id="ime-op-0001",
            )
        self.assertEqual("paragraph_ids_required", caught.exception.code)

        plan = plan_text_composition_commit_v1(
            session,
            current_revision_id="rev:1",
            current_story_text="abcd",
            current_domain=domain("abcd"),
            final_external_text="x\r\ny",
            document_id="doc:1",
            source_hash="a" * 64,
            client_operation_id="ime-op-0001",
            paragraph_inserted_ids=("paragraph:new",),
        )
        self.assertEqual("x\ry", plan.canonical_replacement_text)
        self.assertEqual("x\ry", plan.request["command"]["replacement_text"])
        validate_story_edit_transaction_request_v1(plan.request)

    def test_composition_start_freezes_typing_snapshot(self):
        sel = selection("abcd", 2, 2)
        typing = derive_typing_format_state_v1(
            selection=sel,
            domain=domain("abcd"),
            format_state=format_state("abcd"),
        )
        typing = set_pending_typing_property_v1(
            state=typing,
            prop="bold",
            value=True,
        )
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcd",
            domain=domain("abcd"),
            selection=sel,
            typing_state=typing,
        )
        later = set_pending_typing_property_v1(
            state=typing,
            prop="italic",
            value=True,
        )
        self.assertEqual((("bold", True),), session.typing_snapshot.items)
        self.assertEqual(
            (("bold", True), ("italic", True)),
            later.pending_explicit_properties,
        )

        plan = plan_text_composition_commit_v1(
            session,
            current_revision_id="rev:1",
            current_story_text="abcd",
            current_domain=domain("abcd"),
            final_external_text="Z",
            document_id="doc:1",
            source_hash="a" * 64,
            client_operation_id="ime-op-0001",
        )
        self.assertEqual({"bold": True}, plan.request["command"]["typing_format"])

    def test_noncollapsed_selection_replaces_exact_captured_range_without_typing_state(self):
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcdef",
            domain=domain("abcdef"),
            selection=selection("abcdef", 2, 5),
        )
        plan = plan_text_composition_commit_v1(
            session,
            current_revision_id="rev:1",
            current_story_text="abcdef",
            current_domain=domain("abcdef"),
            final_external_text="X",
            document_id="doc:1",
            source_hash="a" * 64,
            client_operation_id="ime-op-0001",
        )
        command = plan.request["command"]
        self.assertEqual((2, 5, "cde", "X"), (
            command["start_scalar"],
            command["end_scalar"],
            command["expected_before"],
            command["replacement_text"],
        ))
        self.assertIsNone(command["typing_format"])

    def test_changed_canonical_range_requires_reconcile(self):
        session = start_text_composition_v1(
            composition_id="ime:1",
            canonical_story_text="abcdef",
            domain=domain("abcdef"),
            selection=selection("abcdef", 2, 5),
        )
        with self.assertRaises(TextCompositionSessionError) as caught:
            cancel_text_composition_v1(
                session,
                current_revision_id="rev:1",
                current_story_text="abZZef",
                current_domain=domain("abZZef"),
            )
        self.assertEqual("reconcile_required", caught.exception.code)


if __name__ == "__main__":
    unittest.main()
