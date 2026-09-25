#!/usr/bin/env python3
import copy
import unittest

from paragraph_alignment_export_v1 import (
    ParagraphAlignmentExportError,
    assess_paragraph_alignment_export_v1,
)
from revision_store import RevisionKernel, hash_id


DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64
P1 = "paragraph:1"
P2 = "paragraph:2"


def alignment_state(paragraph):
    return {
        "base_alignment": paragraph["base_alignment"],
        "override": paragraph["alignment_override"],
    }


class FakeParagraphAlignmentExecutor:
    def __call__(self, base_project, command):
        project = copy.deepcopy(base_project)
        changes = []
        for paragraph_id in command["paragraph_ids"]:
            paragraph = project["paragraphs"][paragraph_id]
            before = alignment_state(paragraph)
            if hash_id(before) != command["expected_state_ids"][paragraph_id]:
                raise ValueError("stale_paragraph_alignment")
            if command["kind"] == "clear_paragraph_alignment_override":
                override = None
            else:
                requested = command["value"]
                override = None if requested == before["base_alignment"] else requested
            after = {"base_alignment": before["base_alignment"], "override": override}
            if after == before:
                raise ValueError("paragraph_alignment_noop")
            paragraph["alignment_override"] = override
            changes.append(
                {
                    "paragraph_id": paragraph_id,
                    "before": before,
                    "before_state_id": hash_id(before),
                    "after": after,
                }
            )
        op = {"kind": command["kind"], "changes": changes}
        project["operations"] = list(project["operations"]) + [copy.deepcopy(op)]
        return op, project, [
            {"key": "paragraph.alignment", "state": "supported", "note": None},
            {"key": "layout.reflow", "state": "invalidated", "note": None},
            {"key": "story.overset", "state": "invalidated", "note": None},
        ]


class ParagraphAlignmentExportV1Tests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "paragraphs": {
                P1: {
                    "paragraph_id": P1,
                    "story_id": "story:1",
                    "text_range": [0, 5],
                    "base_alignment": "left",
                    "alignment_override": None,
                },
                P2: {
                    "paragraph_id": P2,
                    "story_id": "story:1",
                    "text_range": [6, 11],
                    "base_alignment": "ambiguous",
                    "alignment_override": None,
                },
            },
        }

    def _kernel(self):
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        return kernel, baseline

    def _set(self, kernel, base_revision_id, paragraph_id, value):
        project = kernel.current_revision(DOCUMENT_ID).project
        state = alignment_state(project["paragraphs"][paragraph_id])
        return kernel.commit_paragraph_alignment(
            {
                "protocol_version": "chaptera.paragraph-alignment-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": base_revision_id,
                "client_operation_id": f"set-{paragraph_id}-{value}",
                "command": {
                    "kind": "set_paragraph_alignment_override",
                    "paragraph_ids": [paragraph_id],
                    "expected_state_ids": {paragraph_id: hash_id(state)},
                    "value": value,
                },
            },
            FakeParagraphAlignmentExecutor(),
        )

    def test_supported_effective_base_alignment_is_explicit_loss_for_both_targets(self):
        for target in ("idml", "odg"):
            assessment = assess_paragraph_alignment_export_v1(
                self.project, target=target, paragraph_ids=[P1]
            )
            self.assertFalse(assessment.can_serialize_without_alignment_loss)
            self.assertEqual(1, len(assessment.losses))
            loss = assessment.losses[0]
            self.assertEqual("left", loss.effective_alignment)
            self.assertEqual("base", loss.source_layer)
            self.assertEqual("paragraph_alignment_not_materialized", loss.code)

    def test_chaptera_override_masks_unsupported_base_and_is_never_silent(self):
        kernel, baseline = self._kernel()
        accepted = self._set(kernel, baseline.revision_id, P2, "right")
        self.assertEqual("accepted", accepted["status"])

        assessment = assess_paragraph_alignment_export_v1(
            kernel.current_revision(DOCUMENT_ID).project,
            target="idml",
            paragraph_ids=[P2],
        )
        self.assertEqual(1, len(assessment.losses))
        self.assertEqual("right", assessment.losses[0].effective_alignment)
        self.assertEqual("chaptera_override", assessment.losses[0].source_layer)

    def test_clear_reveals_unsupported_base_and_removes_bounded_v1_loss_item(self):
        kernel, baseline = self._kernel()
        set_result = self._set(kernel, baseline.revision_id, P2, "center")
        edited = kernel.current_revision(DOCUMENT_ID).project
        before = alignment_state(edited["paragraphs"][P2])

        cleared = kernel.commit_paragraph_alignment(
            {
                "protocol_version": "chaptera.paragraph-alignment-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": set_result["revision_id"],
                "client_operation_id": "clear-p2",
                "command": {
                    "kind": "clear_paragraph_alignment_override",
                    "paragraph_ids": [P2],
                    "expected_state_ids": {P2: hash_id(before)},
                },
            },
            FakeParagraphAlignmentExecutor(),
        )
        self.assertEqual("accepted", cleared["status"])
        current = kernel.current_revision(DOCUMENT_ID).project
        self.assertIsNone(current["paragraphs"][P2]["alignment_override"])
        assessment = assess_paragraph_alignment_export_v1(
            current, target="odg", paragraph_ids=[P2]
        )
        self.assertEqual((), assessment.losses)

    def test_loss_order_is_canonical_and_input_order_independent(self):
        project = copy.deepcopy(self.project)
        project["paragraphs"][P2]["base_alignment"] = "right"
        a = assess_paragraph_alignment_export_v1(
            project, target="idml", paragraph_ids=[P2, P1]
        )
        b = assess_paragraph_alignment_export_v1(
            project, target="idml", paragraph_ids=[P1, P2]
        )
        self.assertEqual(a, b)
        self.assertEqual([P1, P2], [loss.paragraph_id for loss in a.losses])

    def test_invalid_target_duplicate_selection_and_missing_identity_fail_closed(self):
        with self.assertRaises(ParagraphAlignmentExportError) as caught:
            assess_paragraph_alignment_export_v1(self.project, target="pdf")
        self.assertEqual("target_unsupported", caught.exception.code)

        with self.assertRaises(ParagraphAlignmentExportError) as caught:
            assess_paragraph_alignment_export_v1(
                self.project, target="idml", paragraph_ids=[P1, P1]
            )
        self.assertEqual("paragraph_selection_duplicate", caught.exception.code)

        bad = copy.deepcopy(self.project)
        bad["paragraphs"][P1]["paragraph_id"] = P2
        with self.assertRaises(ParagraphAlignmentExportError) as caught:
            assess_paragraph_alignment_export_v1(
                bad, target="odg", paragraph_ids=[P1]
            )
        self.assertEqual("paragraph_identity_mismatch", caught.exception.code)


if __name__ == "__main__":
    unittest.main()
