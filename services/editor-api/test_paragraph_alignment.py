#!/usr/bin/env python3
import copy
import unittest

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
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        changes = []
        project = copy.deepcopy(base_project)
        project["paragraphs"] = copy.deepcopy(project["paragraphs"])

        for paragraph_id in command["paragraph_ids"]:
            paragraph = project["paragraphs"].get(paragraph_id)
            if paragraph is None:
                raise ValueError("missing_paragraph")
            before = alignment_state(paragraph)
            if hash_id(before) != command["expected_state_ids"][paragraph_id]:
                raise ValueError("stale_paragraph_alignment")

            if command["kind"] == "clear_paragraph_alignment_override":
                next_override = None
            else:
                requested = command["value"]
                next_override = None if before["base_alignment"] == requested else requested

            after = {
                "base_alignment": before["base_alignment"],
                "override": next_override,
            }
            if after == before:
                raise ValueError("paragraph_alignment_noop")

            paragraph["alignment_override"] = next_override
            changes.append({
                "paragraph_id": paragraph_id,
                "before": before,
                "before_state_id": hash_id(before),
                "after": after,
            })

        operation = {
            "kind": command["kind"],
            "changes": copy.deepcopy(changes),
        }
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        return operation, project, [
            {"key": "paragraph.alignment", "state": "supported", "note": None},
            {"key": "layout.reflow", "state": "invalidated", "note": None},
            {"key": "story.overset", "state": "invalidated", "note": None},
        ]


class ParagraphAlignmentCommitTests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "stories": {"story:1": "First paragraph\nSecond paragraph"},
            "paragraphs": {
                P1: {
                    "paragraph_id": P1,
                    "story_id": "story:1",
                    "text_range": [0, 15],
                    "base_alignment": "left",
                    "alignment_override": None,
                },
                P2: {
                    "paragraph_id": P2,
                    "story_id": "story:1",
                    "text_range": [16, 32],
                    "base_alignment": "ambiguous",
                    "alignment_override": None,
                },
            },
        }
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = FakeParagraphAlignmentExecutor()

    def expected(self, project=None, paragraph_ids=(P1, P2)):
        project = project or self.project
        return {
            paragraph_id: hash_id(alignment_state(project["paragraphs"][paragraph_id]))
            for paragraph_id in paragraph_ids
        }

    def set_request(self, op_id, value="center", paragraph_ids=(P1, P2), expected=None, base=None):
        paragraph_ids = list(paragraph_ids)
        return {
            "protocol_version": "chaptera.paragraph-alignment-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "set_paragraph_alignment_override",
                "paragraph_ids": paragraph_ids,
                "expected_state_ids": expected or self.expected(paragraph_ids=paragraph_ids),
                "value": value,
            },
        }

    def clear_request(self, op_id, project, paragraph_ids=(P1,), base=None):
        paragraph_ids = list(paragraph_ids)
        return {
            "protocol_version": "chaptera.paragraph-alignment-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base,
            "client_operation_id": op_id,
            "command": {
                "kind": "clear_paragraph_alignment_override",
                "paragraph_ids": paragraph_ids,
                "expected_state_ids": self.expected(project, paragraph_ids),
            },
        }

    def test_set_changes_only_override_and_preserves_story_and_paragraph_identity(self):
        result = self.kernel.commit_paragraph_alignment(
            self.set_request("align-op-00000001"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(self.project["stories"], current["stories"])
        for paragraph_id in (P1, P2):
            before = self.project["paragraphs"][paragraph_id]
            after = current["paragraphs"][paragraph_id]
            self.assertEqual(before["paragraph_id"], after["paragraph_id"])
            self.assertEqual(before["story_id"], after["story_id"])
            self.assertEqual(before["text_range"], after["text_range"])
            self.assertEqual(before["base_alignment"], after["base_alignment"])
            self.assertEqual("center", after["alignment_override"])
        self.assertEqual(
            ["paragraph.alignment", "layout.reflow", "story.overset"],
            [item["key"] for item in result["consequences"]],
        )

    def test_set_equal_to_supported_base_normalizes_to_inherited_state(self):
        project = copy.deepcopy(self.project)
        project["paragraphs"][P1]["alignment_override"] = "center"
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        executor = FakeParagraphAlignmentExecutor()
        before = alignment_state(project["paragraphs"][P1])
        req = {
            "protocol_version": "chaptera.paragraph-alignment-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": baseline.revision_id,
            "client_operation_id": "align-op-00000002",
            "command": {
                "kind": "set_paragraph_alignment_override",
                "paragraph_ids": [P1],
                "expected_state_ids": {P1: hash_id(before)},
                "value": "left",
            },
        }
        result = kernel.commit_paragraph_alignment(req, executor)
        self.assertIsNone(kernel.current_revision(DOCUMENT_ID).project["paragraphs"][P1]["alignment_override"])
        self.assertIsNone(result["canonical_operation"]["changes"][0]["after"]["override"])

    def test_unsupported_base_is_preserved_and_can_be_masked_then_revealed(self):
        set_result = self.kernel.commit_paragraph_alignment(
            self.set_request("align-op-00000003", value="right", paragraph_ids=(P2,)),
            self.executor,
        )
        set_project = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)
        self.assertEqual("ambiguous", set_project["paragraphs"][P2]["base_alignment"])
        self.assertEqual("right", set_project["paragraphs"][P2]["alignment_override"])

        clear = self.clear_request(
            "align-op-00000004",
            set_project,
            paragraph_ids=(P2,),
            base=set_result["revision_id"],
        )
        self.kernel.commit_paragraph_alignment(clear, self.executor)
        current = self.kernel.current_revision(DOCUMENT_ID).project["paragraphs"][P2]
        self.assertEqual("ambiguous", current["base_alignment"])
        self.assertIsNone(current["alignment_override"])

    def test_exact_retry_and_stale_revision_are_fail_closed(self):
        req = self.set_request("align-op-00000005")
        first = self.kernel.commit_paragraph_alignment(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_paragraph_alignment(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        calls = self.executor.calls
        stale = self.kernel.commit_paragraph_alignment(
            self.set_request("align-op-00000006", value="right"),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(calls, self.executor.calls)

    def test_stale_semantic_precondition_and_missing_paragraph_reject(self):
        bad = self.set_request(
            "align-op-00000007",
            expected={P1: "sha256:" + "b" * 64, P2: self.expected()[P2]},
        )
        with self.assertRaisesRegex(ValueError, "stale_paragraph_alignment"):
            self.kernel.commit_paragraph_alignment(bad, self.executor)

        missing = self.set_request("align-op-00000008", paragraph_ids=(P1,))
        missing["command"]["paragraph_ids"] = ["paragraph:missing"]
        missing["command"]["expected_state_ids"] = {"paragraph:missing": "sha256:" + "c" * 64}
        with self.assertRaisesRegex(ValueError, "missing_paragraph"):
            self.kernel.commit_paragraph_alignment(missing, self.executor)

    def test_duplicate_ids_and_extra_authoritative_fields_reject_before_executor(self):
        req = self.set_request("align-op-00000009")
        req["command"]["paragraph_ids"] = [P1, P1]
        req["command"]["expected_state_ids"] = {P1: self.expected()[P1]}
        with self.assertRaisesRegex(ValueError, "unique"):
            self.kernel.commit_paragraph_alignment(req, self.executor)

        req = self.set_request("align-op-00000010")
        req["command"]["base_alignment"] = "left"
        with self.assertRaisesRegex(ValueError, "non-intent"):
            self.kernel.commit_paragraph_alignment(req, self.executor)

    def test_redundant_clear_is_noop_and_does_not_advance_revision(self):
        req = {
            "protocol_version": "chaptera.paragraph-alignment-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": self.baseline.revision_id,
            "client_operation_id": "align-op-00000011",
            "command": {
                "kind": "clear_paragraph_alignment_override",
                "paragraph_ids": [P1],
                "expected_state_ids": {P1: self.expected()[P1]},
            },
        }
        with self.assertRaisesRegex(ValueError, "noop"):
            self.kernel.commit_paragraph_alignment(req, self.executor)
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_executor_cannot_rewrite_base_or_wrong_normalized_override(self):
        def bad_executor(base_project, command):
            op, project, consequences = self.executor(base_project, command)
            op["changes"][0]["after"]["base_alignment"] = "right"
            return op, project, consequences

        with self.assertRaisesRegex(ValueError, "base alignment"):
            self.kernel.commit_paragraph_alignment(
                self.set_request("align-op-00000012"),
                bad_executor,
            )
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_undo_redo_restore_exact_layered_alignment_state(self):
        accepted = self.kernel.commit_paragraph_alignment(
            self.set_request("align-op-00000013", value="right"),
            self.executor,
        )
        edited_project = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(self.project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(edited_project), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported history transition")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "align-history-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertIsNone(self.kernel.current_revision(DOCUMENT_ID).project["paragraphs"][P1]["alignment_override"])
        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "align-history-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual("right", self.kernel.current_revision(DOCUMENT_ID).project["paragraphs"][P1]["alignment_override"])


if __name__ == "__main__":
    unittest.main()
