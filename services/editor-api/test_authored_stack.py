#!/usr/bin/env python3
import copy
import unittest

from authored_stack_v1 import (
    append_authored_member,
    reorder_authored_lane,
    validate_authored_lane,
)
from revision_store import RevisionKernel

DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64
PAGE_ID = "page:1"
A = "node:authored:a"
B = "node:authored:b"
C = "node:authored:c"


class AuthoredStackModelTests(unittest.TestCase):
    def test_lane_is_back_to_front_and_append_is_front(self):
        self.assertEqual([A, B, C], append_authored_member([A, B], C))
        self.assertEqual([A, C, B], reorder_authored_lane([A, B, C], B, "step_forward"))
        self.assertEqual([B, A, C], reorder_authored_lane([A, B, C], B, "step_backward"))
        self.assertEqual([A, C, B], reorder_authored_lane([A, B, C], B, "to_front"))
        self.assertEqual([B, A, C], reorder_authored_lane([A, B, C], B, "to_back"))

    def test_invalid_duplicate_unknown_and_noop_fail_closed(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            validate_authored_lane([A, A])
        with self.assertRaisesRegex(ValueError, "not a lane member"):
            reorder_authored_lane([A, B], C, "to_front")
        with self.assertRaisesRegex(ValueError, "no-op"):
            reorder_authored_lane([A, B], B, "to_front")
        with self.assertRaisesRegex(ValueError, "unsupported"):
            reorder_authored_lane([A, B], A, "sideways")


class FakeAuthoredStackExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        page_id = command["page_id"]
        lane = base_project["authored_stacks"].get(page_id)
        if lane is None:
            raise ValueError("unknown_authored_stack")
        if lane != command["expected_before"]:
            raise ValueError("stale_authored_stack")
        node = base_project["nodes"].get(command["node_id"])
        if node is None:
            raise ValueError("missing_authored_stack_node")
        if (
            not node.get("author_created")
            or node.get("page_id") != page_id
            or node.get("group_parent") is not None
        ):
            raise ValueError("unsupported_authored_stack_member")

        after = reorder_authored_lane(lane, command["node_id"], command["mode"])
        operation = {
            "kind": "reorder_authored_stack",
            "page_id": page_id,
            "node_id": command["node_id"],
            "mode": command["mode"],
            "before": list(lane),
            "after": after,
        }
        project = copy.deepcopy(base_project)
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        project["authored_stacks"] = copy.deepcopy(project["authored_stacks"])
        project["authored_stacks"][page_id] = list(after)
        return operation, project, [
            {"key": "authored_stack.order", "state": "supported", "note": None},
            {"key": "scene.paint_order", "state": "invalidated", "note": None},
        ]


class AuthoredStackCommitTests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "pages": {
                PAGE_ID: {
                    "children": ["node:source:1", A, B, C],
                    "imported_base_stack": ["node:source:1"],
                }
            },
            "nodes": {
                "node:source:1": {
                    "author_created": False,
                    "page_id": PAGE_ID,
                    "group_parent": None,
                },
                A: {"author_created": True, "page_id": PAGE_ID, "group_parent": None},
                B: {"author_created": True, "page_id": PAGE_ID, "group_parent": None},
                C: {"author_created": True, "page_id": PAGE_ID, "group_parent": None},
            },
            "authored_stacks": {PAGE_ID: [A, B, C]},
        }
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = FakeAuthoredStackExecutor()

    def request(self, op_id, *, node_id=B, mode="to_front", before=None, base=None, page_id=PAGE_ID):
        return {
            "protocol_version": "chaptera.authored-stack-reorder-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "reorder_authored_stack",
                "page_id": page_id,
                "node_id": node_id,
                "expected_before": list(before or [A, B, C]),
                "mode": mode,
            },
        }

    def test_reorder_changes_only_authored_lane_not_base_or_page_children(self):
        result = self.kernel.commit_reorder_authored_stack(
            self.request("stack-op-00000001"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual([A, C, B], current["authored_stacks"][PAGE_ID])
        self.assertEqual(
            self.project["pages"][PAGE_ID]["imported_base_stack"],
            current["pages"][PAGE_ID]["imported_base_stack"],
        )
        self.assertEqual(
            self.project["pages"][PAGE_ID]["children"],
            current["pages"][PAGE_ID]["children"],
        )
        self.assertEqual([A, B, C], result["canonical_operation"]["before"])
        self.assertEqual([A, C, B], result["canonical_operation"]["after"])

    def test_all_four_modes_have_exact_deterministic_results(self):
        cases = [
            (B, "step_forward", [A, C, B]),
            (B, "step_backward", [B, A, C]),
            (A, "to_front", [B, C, A]),
            (C, "to_back", [C, A, B]),
        ]
        for i, (node_id, mode, expected) in enumerate(cases):
            kernel = RevisionKernel()
            baseline = kernel.register_baseline(
                document_id=DOCUMENT_ID,
                source_hash=SOURCE_HASH,
                project=self.project,
            )
            result = kernel.commit_reorder_authored_stack(
                self.request(
                    f"stack-mode-{i:08d}",
                    node_id=node_id,
                    mode=mode,
                    base=baseline.revision_id,
                ),
                FakeAuthoredStackExecutor(),
            )
            self.assertEqual(expected, result["canonical_operation"]["after"])

    def test_exact_retry_and_stale_revision_fail_closed(self):
        req = self.request("stack-op-00000002")
        first = self.kernel.commit_reorder_authored_stack(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_reorder_authored_stack(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        calls = self.executor.calls
        stale = self.kernel.commit_reorder_authored_stack(
            self.request("stack-op-00000003", node_id=A, mode="to_front"),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(calls, self.executor.calls)

    def test_lane_precondition_and_capability_gate_reject(self):
        with self.assertRaisesRegex(ValueError, "stale_authored_stack"):
            self.kernel.commit_reorder_authored_stack(
                self.request(
                    "stack-op-00000004",
                    before=[A, C, B],
                    node_id=A,
                    mode="to_front",
                ),
                self.executor,
            )

        for node_id in ("node:source:1",):
            req = self.request(
                "stack-op-source-0001",
                node_id=node_id,
                before=[A, B, C, node_id],
                mode="to_back",
            )
            project = copy.deepcopy(self.project)
            project["authored_stacks"][PAGE_ID] = [A, B, C, node_id]
            kernel = RevisionKernel()
            baseline = kernel.register_baseline(
                document_id=DOCUMENT_ID,
                source_hash=SOURCE_HASH,
                project=project,
            )
            req["base_revision_id"] = baseline.revision_id
            with self.assertRaisesRegex(ValueError, "unsupported_authored_stack_member"):
                kernel.commit_reorder_authored_stack(req, FakeAuthoredStackExecutor())

    def test_noop_rejected_before_executor(self):
        with self.assertRaisesRegex(ValueError, "no-op"):
            self.kernel.commit_reorder_authored_stack(
                self.request("stack-op-00000005", node_id=C, mode="to_front"),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)

    def test_executor_cannot_invent_different_lane_transition(self):
        def bad_executor(base_project, command):
            operation, project, consequences = self.executor(base_project, command)
            operation["after"] = [B, A, C]
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "violates V1"):
            self.kernel.commit_reorder_authored_stack(
                self.request("stack-op-00000006"),
                bad_executor,
            )
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_undo_redo_restore_exact_lane_and_project_replay_state(self):
        accepted = self.kernel.commit_reorder_authored_stack(
            self.request("stack-op-00000007", node_id=A, mode="to_front"),
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
                "client_operation_id": "stack-history-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertEqual([A, B, C], self.kernel.current_revision(DOCUMENT_ID).project["authored_stacks"][PAGE_ID])

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "stack-history-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual([B, C, A], self.kernel.current_revision(DOCUMENT_ID).project["authored_stacks"][PAGE_ID])
        self.assertEqual(edited_project, self.kernel.current_revision(DOCUMENT_ID).project)


if __name__ == "__main__":
    unittest.main()
