import copy
import unittest

from revision_store import RevisionKernel


DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
NODE_ID = "30000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64


class FakeAuthoritativeExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        before = {"x": 0, "y": 0, "width": 1828800, "height": 914400}
        operation = {
            "kind": "move_node",
            "node_id": command["node_id"],
            "before": before,
            "after": {
                "x": command["x_emu"],
                "y": command["y_emu"],
                "width": before["width"],
                "height": before["height"],
            },
        }
        project = copy.deepcopy(base_project)
        project["schema_version"] = "pub-editor-v0.4"
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        consequences = [
            {"key": "node.geometry.position", "state": "supported", "note": None}
        ]
        return operation, project, consequences


def request(base_revision_id, client_operation_id, *, x=-12700, source_hash=SOURCE_HASH):
    return {
        "protocol_version": "chaptera.commit-request.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": source_hash,
        "base_revision_id": base_revision_id,
        "client_operation_id": client_operation_id,
        "command": {
            "kind": "move_node_to",
            "node_id": NODE_ID,
            "x_emu": x,
            "y_emu": 25400,
        },
    }


class RevisionKernelTests(unittest.TestCase):
    def setUp(self):
        self.kernel = RevisionKernel()
        self.baseline_project = {
            "schema_version": "pub-editor-v0.2",
            "source_hash": SOURCE_HASH,
            "operations": [],
        }
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.baseline_project,
        )
        self.executor = FakeAuthoritativeExecutor()

    def test_baseline_is_deterministic(self):
        other = RevisionKernel().register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=copy.deepcopy(self.baseline_project),
        )
        self.assertEqual(self.baseline.state_id, other.state_id)
        self.assertEqual(self.baseline.revision_id, other.revision_id)

    def test_accept_creates_one_revision_and_server_before(self):
        result = self.kernel.commit_move(
            request(self.baseline.revision_id, "90000000-0000-4000-8000-000000000001"),
            self.executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual(1, self.executor.calls)
        self.assertEqual({"x": 0, "y": 0, "width": 1828800, "height": 914400},
                         result["canonical_operation"]["before"])
        self.assertNotEqual(self.baseline.revision_id, result["revision_id"])
        self.assertEqual(result["revision_id"], self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_exact_retry_is_idempotent_and_does_not_execute_twice(self):
        req = request(self.baseline.revision_id, "90000000-0000-4000-8000-000000000002")
        first = self.kernel.commit_move(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_move(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

    def test_same_id_different_payload_is_conflict_without_mutation(self):
        client_id = "90000000-0000-4000-8000-000000000003"
        first = self.kernel.commit_move(
            request(self.baseline.revision_id, client_id),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).revision_id
        conflict = self.kernel.commit_move(
            request(self.baseline.revision_id, client_id, x=-12699),
            self.executor,
        )
        self.assertEqual("idempotency_conflict", conflict["code"])
        self.assertEqual(current, self.kernel.current_revision(DOCUMENT_ID).revision_id)
        self.assertEqual(1, self.executor.calls)
        self.assertEqual(first["revision_id"], current)

    def test_stale_base_rejected_before_executor(self):
        first = self.kernel.commit_move(
            request(self.baseline.revision_id, "90000000-0000-4000-8000-000000000004"),
            self.executor,
        )
        calls = self.executor.calls
        stale = self.kernel.commit_move(
            request(self.baseline.revision_id, "90000000-0000-4000-8000-000000000005"),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertEqual(calls, self.executor.calls)
        self.assertEqual(first["revision_id"], self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_source_mismatch_rejected_before_executor(self):
        bad = self.kernel.commit_move(
            request(
                self.baseline.revision_id,
                "90000000-0000-4000-8000-000000000006",
                source_hash="b" * 64,
            ),
            self.executor,
        )
        self.assertEqual("source_hash_mismatch", bad["code"])
        self.assertEqual(0, self.executor.calls)
        self.assertEqual(self.baseline.revision_id,
                         self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_executor_cannot_change_source_identity(self):
        def bad_executor(base_project, command):
            op, project, consequences = self.executor(base_project, command)
            project["source_hash"] = "c" * 64
            return op, project, consequences

        with self.assertRaisesRegex(ValueError, "source hash changed"):
            self.kernel.commit_move(
                request(self.baseline.revision_id, "90000000-0000-4000-8000-000000000007"),
                bad_executor,
            )
        self.assertEqual(self.baseline.revision_id,
                         self.kernel.current_revision(DOCUMENT_ID).revision_id)


    def test_unsafe_browser_emu_rejected_before_executor(self):
        unsafe = 9_007_199_254_740_992
        with self.assertRaisesRegex(ValueError, "JavaScript-safe"):
            self.kernel.commit_move(
                request(
                    self.baseline.revision_id,
                    "90000000-0000-4000-8000-000000000008",
                    x=unsafe,
                ),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_unsafe_canonical_emu_cannot_advance_revision(self):
        unsafe = 9_007_199_254_740_992

        def bad_executor(base_project, command):
            op, project, consequences = self.executor(base_project, command)
            op["before"]["x"] = unsafe
            project["operations"][-1] = copy.deepcopy(op)
            return op, project, consequences

        with self.assertRaisesRegex(ValueError, "outside the V1 JavaScript-safe EMU range"):
            self.kernel.commit_move(
                request(
                    self.baseline.revision_id,
                    "90000000-0000-4000-8000-000000000009",
                ),
                bad_executor,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

class FakeStoryExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        import hashlib
        self.calls += 1
        story = base_project["stories"][command["story_id"]]
        chars = list(story)
        start = command["start_scalar"]
        end = command["end_scalar"]
        if end > len(chars):
            raise ValueError("Story range outside canonical text")
        replacement = command["replacement_text"]
        after = "".join(chars[:start]) + replacement + "".join(chars[end:])
        operation = {
            "kind": "replace_story_range",
            "story_id": command["story_id"],
            "start_scalar": start,
            "end_scalar": end,
            "replacement_text": replacement,
            "before_text_hash": hashlib.sha256(story.encode("utf-8")).hexdigest(),
            "after_text_hash": hashlib.sha256(after.encode("utf-8")).hexdigest(),
        }
        project = copy.deepcopy(base_project)
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        project["stories"] = dict(project["stories"])
        project["stories"][command["story_id"]] = after
        return operation, project, [{"key": "story.text", "state": "supported", "note": None}]


class StoryRangeCommitTests(unittest.TestCase):
    def setUp(self):
        self.kernel = RevisionKernel()
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "stories": {"story:1": "A😀B"},
        }
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = FakeStoryExecutor()

    def story_request(self, op_id, start, end, replacement, base=None, depends=None):
        return {
            "protocol_version": "chaptera.story-range-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "depends_on_client_operation_id": depends,
            "command": {
                "kind": "replace_story_range",
                "story_id": "story:1",
                "start_scalar": start,
                "end_scalar": end,
                "replacement_text": replacement,
            },
        }

    def test_story_range_commit_uses_scalar_range_and_server_hashes(self):
        result = self.kernel.commit_story_range(
            self.story_request("text-op-00000001", 1, 2, "漢"),
            self.executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual("replace_story_range", result["canonical_operation"]["kind"])
        self.assertEqual(1, result["canonical_operation"]["start_scalar"])
        self.assertEqual(2, result["canonical_operation"]["end_scalar"])
        self.assertEqual(
            "A漢B",
            self.kernel.current_revision(DOCUMENT_ID).project["stories"]["story:1"],
        )
        self.assertEqual(1, self.executor.calls)

    def test_story_range_exact_retry_is_idempotent(self):
        req = self.story_request("text-op-00000002", 1, 2, "漢")
        first = self.kernel.commit_story_range(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_story_range(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

    def test_story_range_stale_base_rejected_without_execution(self):
        first = self.kernel.commit_story_range(
            self.story_request("text-op-00000003", 1, 2, "漢"),
            self.executor,
        )
        calls = self.executor.calls
        stale = self.kernel.commit_story_range(
            self.story_request("text-op-00000004", 0, 1, "X"),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertEqual(calls, self.executor.calls)

    def test_story_range_same_id_different_payload_conflicts(self):
        op_id = "text-op-00000005"
        self.kernel.commit_story_range(
            self.story_request(op_id, 1, 2, "漢"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).revision_id
        conflict = self.kernel.commit_story_range(
            self.story_request(op_id, 1, 2, "X"),
            self.executor,
        )
        self.assertEqual("idempotency_conflict", conflict["code"])
        self.assertEqual(current, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_story_range_invalid_range_fails_before_execution(self):
        with self.assertRaisesRegex(ValueError, "range"):
            self.kernel.commit_story_range(
                self.story_request("text-op-00000006", 3, 2, "X"),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)

    def test_browser_cannot_supply_authoritative_text_hashes(self):
        req = self.story_request("text-op-00000007", 1, 2, "漢")
        req["command"]["before_text_hash"] = "evil"
        with self.assertRaisesRegex(ValueError, "replace_story_range"):
            self.kernel.commit_story_range(req, self.executor)


class FakeHistoryExecutor:
    def __init__(self, *, baseline_project, moved_project):
        self.baseline_project = copy.deepcopy(baseline_project)
        self.moved_project = copy.deepcopy(moved_project)
        self.calls = []

    def __call__(self, base_project, transition_kind):
        self.calls.append(transition_kind)
        if transition_kind == "undo":
            return copy.deepcopy(self.baseline_project), [
                {"key": "history.undo", "state": "supported", "note": None}
            ]
        if transition_kind == "redo":
            return copy.deepcopy(self.moved_project), [
                {"key": "history.redo", "state": "supported", "note": None}
            ]
        raise ValueError("unsupported history transition")


class HistoryTransitionTests(unittest.TestCase):
    def setUp(self):
        self.kernel = RevisionKernel()
        self.baseline_project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
        }
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.baseline_project,
        )
        self.move_executor = FakeAuthoritativeExecutor()
        self.move = self.kernel.commit_move(
            request(
                self.baseline.revision_id,
                "90000000-0000-4000-8000-000000000101",
            ),
            self.move_executor,
        )
        self.moved_record = self.kernel.current_revision(DOCUMENT_ID)
        self.history_executor = FakeHistoryExecutor(
            baseline_project=self.baseline_project,
            moved_project=self.moved_record.project,
        )

    def history_request(
        self,
        kind,
        op_id,
        *,
        base=None,
        source_hash=SOURCE_HASH,
    ):
        return {
            "protocol_version": "chaptera.history-transition-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": source_hash,
            "base_revision_id": base or self.kernel.current_revision(DOCUMENT_ID).revision_id,
            "client_operation_id": op_id,
            "command": {"kind": kind},
        }

    def test_undo_creates_new_revision_that_reuses_baseline_state(self):
        undo = self.kernel.commit_history_transition(
            self.history_request("undo", "history-op-00000001"),
            self.history_executor,
        )
        self.assertEqual(
            "chaptera.history-transition-accepted.v1",
            undo["protocol_version"],
        )
        self.assertEqual("undo", undo["transition_kind"])
        self.assertEqual(self.baseline.state_id, undo["state_id"])
        self.assertNotEqual(self.baseline.revision_id, undo["revision_id"])
        self.assertNotEqual(self.move["revision_id"], undo["revision_id"])
        record = self.kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(self.move["revision_id"], record.parent_revision_id)
        self.assertEqual("undo", record.transition_kind)
        self.assertEqual(self.baseline_project, record.project)
        self.assertTrue(
            self.kernel.has_revision(
                document_id=DOCUMENT_ID,
                revision_id=self.baseline.revision_id,
            )
        )
        self.assertTrue(
            self.kernel.has_revision(
                document_id=DOCUMENT_ID,
                revision_id=self.move["revision_id"],
            )
        )

    def test_redo_creates_fresh_revision_that_reuses_moved_state(self):
        undo = self.kernel.commit_history_transition(
            self.history_request("undo", "history-op-00000002"),
            self.history_executor,
        )
        redo = self.kernel.commit_history_transition(
            self.history_request(
                "redo",
                "history-op-00000003",
                base=undo["revision_id"],
            ),
            self.history_executor,
        )
        self.assertEqual("redo", redo["transition_kind"])
        self.assertEqual(self.moved_record.state_id, redo["state_id"])
        self.assertNotEqual(self.move["revision_id"], redo["revision_id"])
        self.assertNotEqual(undo["revision_id"], redo["revision_id"])
        record = self.kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(undo["revision_id"], record.parent_revision_id)
        self.assertEqual(self.moved_record.project, record.project)
        self.assertEqual(["undo", "redo"], self.history_executor.calls)

    def test_history_exact_retry_is_idempotent_without_second_executor_call(self):
        req = self.history_request("undo", "history-op-00000004")
        first = self.kernel.commit_history_transition(
            copy.deepcopy(req),
            self.history_executor,
        )
        second = self.kernel.commit_history_transition(
            copy.deepcopy(req),
            self.history_executor,
        )
        self.assertEqual(first, second)
        self.assertEqual(["undo"], self.history_executor.calls)

    def test_history_stale_base_rejected_before_executor(self):
        stale = self.kernel.commit_history_transition(
            self.history_request(
                "undo",
                "history-op-00000005",
                base=self.baseline.revision_id,
            ),
            self.history_executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(self.move["revision_id"], stale["current_revision_id"])
        self.assertEqual([], self.history_executor.calls)
        self.assertEqual(
            self.move["revision_id"],
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_history_same_id_different_intent_conflicts_without_execution(self):
        op_id = "history-op-00000006"
        first = self.kernel.commit_history_transition(
            self.history_request("undo", op_id),
            self.history_executor,
        )
        calls = list(self.history_executor.calls)
        conflict = self.kernel.commit_history_transition(
            self.history_request(
                "redo",
                op_id,
                base=first["revision_id"],
            ),
            self.history_executor,
        )
        self.assertEqual("idempotency_conflict", conflict["code"])
        self.assertEqual(calls, self.history_executor.calls)
        self.assertEqual(
            first["revision_id"],
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_history_source_mismatch_rejected_before_executor(self):
        result = self.kernel.commit_history_transition(
            self.history_request(
                "undo",
                "history-op-00000007",
                source_hash="b" * 64,
            ),
            self.history_executor,
        )
        self.assertEqual("source_hash_mismatch", result["code"])
        self.assertEqual([], self.history_executor.calls)

    def test_browser_cannot_supply_history_target_or_project(self):
        req = self.history_request("undo", "history-op-00000008")
        req["command"]["target_revision_id"] = self.baseline.revision_id
        with self.assertRaisesRegex(ValueError, "non-intent"):
            self.kernel.commit_history_transition(req, self.history_executor)
        self.assertEqual([], self.history_executor.calls)

    def test_history_intent_rejects_unknown_transition_kind(self):
        req = self.history_request("rewind", "history-op-00000009")
        with self.assertRaisesRegex(ValueError, "undo or redo"):
            self.kernel.commit_history_transition(req, self.history_executor)
        self.assertEqual([], self.history_executor.calls)


if __name__ == "__main__":
    unittest.main()
