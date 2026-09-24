#!/usr/bin/env python3
import copy
import unittest

from revision_store import RevisionKernel

DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64
FRAME_ID = "frame:1"
STORY_ID = "story:1"


class FakeTextFrameColumnsExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        frame = base_project["text_frames"][command["node_id"]]
        if not frame["supported"]:
            raise ValueError("unsupported_text_frame_class")
        before = copy.deepcopy(frame["columns"])
        if before != command["expected_before"]:
            raise ValueError("stale_text_frame_columns")
        after = copy.deepcopy(command["after"])
        operation = {
            "kind": "set_text_frame_columns",
            "node_id": command["node_id"],
            "before": before,
            "after": after,
        }
        project = copy.deepcopy(base_project)
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        project["text_frames"] = copy.deepcopy(project["text_frames"])
        project["text_frames"][command["node_id"]]["columns"] = copy.deepcopy(after)
        consequences = [
            {"key": "text_frame.columns", "state": "supported", "note": None},
            {"key": "layout.reflow", "state": "invalidated", "note": None},
            {"key": "story.overset", "state": "invalidated", "note": None},
        ]
        return operation, project, consequences


class TextFrameColumnsCommitTests(unittest.TestCase):
    BEFORE = {"column_count": 1, "gutter_emu": 0}
    AFTER = {"column_count": 3, "gutter_emu": 91440}

    def setUp(self):
        self.kernel = RevisionKernel()
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "stories": {STORY_ID: "full story text"},
            "text_frames": {
                FRAME_ID: {
                    "story_id": STORY_ID,
                    "bounds": {"x": 10, "y": 20, "width": 3000000, "height": 1800000},
                    "columns": copy.deepcopy(self.BEFORE),
                    "supported": True,
                }
            },
        }
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = FakeTextFrameColumnsExecutor()

    def request(self, op_id, *, before=None, after=None, base=None):
        return {
            "protocol_version": "chaptera.text-frame-columns-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "set_text_frame_columns",
                "node_id": FRAME_ID,
                "expected_before": copy.deepcopy(before or self.BEFORE),
                "after": copy.deepcopy(after or self.AFTER),
            },
        }

    def test_commit_changes_only_column_axis_and_invalidates_layout(self):
        result = self.kernel.commit_text_frame_columns(
            self.request("columns-op-00000001"),
            self.executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual("set_text_frame_columns", result["canonical_operation"]["kind"])
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual("full story text", current["stories"][STORY_ID])
        self.assertEqual(
            self.project["text_frames"][FRAME_ID]["story_id"],
            current["text_frames"][FRAME_ID]["story_id"],
        )
        self.assertEqual(
            self.project["text_frames"][FRAME_ID]["bounds"],
            current["text_frames"][FRAME_ID]["bounds"],
        )
        self.assertEqual(self.AFTER, current["text_frames"][FRAME_ID]["columns"])
        self.assertEqual(
            ["text_frame.columns", "layout.reflow", "story.overset"],
            [item["key"] for item in result["consequences"]],
        )
        self.assertEqual("invalidated", result["consequences"][1]["state"])
        self.assertEqual("invalidated", result["consequences"][2]["state"])

    def test_exact_retry_is_idempotent(self):
        req = self.request("columns-op-00000002")
        first = self.kernel.commit_text_frame_columns(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_text_frame_columns(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

    def test_revision_stale_base_rejected_before_executor(self):
        first = self.kernel.commit_text_frame_columns(
            self.request("columns-op-00000003"),
            self.executor,
        )
        calls = self.executor.calls
        stale = self.kernel.commit_text_frame_columns(
            self.request(
                "columns-op-00000004",
                before=self.AFTER,
                after={"column_count": 2, "gutter_emu": 45720},
            ),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertEqual(calls, self.executor.calls)

    def test_stale_expected_before_fails_without_revision_move(self):
        with self.assertRaisesRegex(ValueError, "stale_text_frame_columns"):
            self.kernel.commit_text_frame_columns(
                self.request(
                    "columns-op-00000005",
                    before={"column_count": 2, "gutter_emu": 0},
                ),
                self.executor,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_noop_and_invalid_state_rejected_before_executor(self):
        with self.assertRaisesRegex(ValueError, "no-op"):
            self.kernel.commit_text_frame_columns(
                self.request("columns-op-00000006", after=self.BEFORE),
                self.executor,
            )
        bad = self.request("columns-op-00000007")
        bad["command"]["after"]["column_count"] = 0
        with self.assertRaisesRegex(ValueError, "column_count"):
            self.kernel.commit_text_frame_columns(bad, self.executor)
        self.assertEqual(0, self.executor.calls)

    def test_unsupported_class_can_fail_before_executor(self):
        def reject(_command):
            raise ValueError("unsupported_text_frame_class")

        with self.assertRaisesRegex(ValueError, "unsupported_text_frame_class"):
            self.kernel.commit_text_frame_columns(
                self.request("columns-op-00000008"),
                self.executor,
                pre_execute_validator=reject,
            )
        self.assertEqual(0, self.executor.calls)

    def test_browser_cannot_supply_story_bounds_or_layout_authority(self):
        for field, value in (
            ("story_id", STORY_ID),
            ("bounds", {"x": 0}),
            ("layout_state", "fits"),
        ):
            req = self.request("columns-op-" + field + "-0001")
            req["command"][field] = value
            with self.subTest(field=field):
                with self.assertRaisesRegex(ValueError, "non-intent"):
                    self.kernel.commit_text_frame_columns(req, self.executor)

    def test_executor_cannot_change_canonical_precondition_or_after(self):
        def bad_executor(base_project, command):
            operation, project, consequences = self.executor(base_project, command)
            operation["after"]["gutter_emu"] += 1
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "after-state"):
            self.kernel.commit_text_frame_columns(
                self.request("columns-op-00000009"),
                bad_executor,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_generic_history_can_restore_and_redo_exact_project_state(self):
        accepted = self.kernel.commit_text_frame_columns(
            self.request("columns-op-00000010"),
            self.executor,
        )
        accepted_project = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)

        def history_executor(_base_project, kind):
            if kind == "undo":
                return copy.deepcopy(self.project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(accepted_project), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported history transition")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "columns-history-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertEqual(self.BEFORE, self.kernel.current_revision(DOCUMENT_ID).project["text_frames"][FRAME_ID]["columns"])

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "columns-history-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(self.AFTER, self.kernel.current_revision(DOCUMENT_ID).project["text_frames"][FRAME_ID]["columns"])
        self.assertEqual("full story text", self.kernel.current_revision(DOCUMENT_ID).project["stories"][STORY_ID])


if __name__ == "__main__":
    unittest.main()
