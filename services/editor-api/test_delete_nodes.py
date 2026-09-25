import copy
import json
import unittest

from delete_nodes_v1 import (
    DeleteNodesV1Error,
    execute_delete_nodes_v1,
    restore_delete_nodes_v1,
)
from revision_store import RevisionKernel, hash_id

DOCUMENT_ID = "delete-nodes-doc"
SOURCE_HASH = "e" * 64
PAGE_ID = "page:1"
KEEP_LEFT = "node:keep-left"
NODE_A = "node:a"
NODE_B = "node:b"
KEEP_RIGHT = "node:keep-right"


def safe_node(node_id, x):
    return {
        "node_id": node_id,
        "parent_id": PAGE_ID,
        "node_class": "ordinary_leaf",
        "author_created": True,
        "bounds": {"x": x, "y": 20, "width": 100, "height": 80},
        "paint": {"fill": "solid"},
        "dependencies": [],
        "story_id": None,
        "text_frame_id": None,
        "group_id": None,
    }


class CountingExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        return execute_delete_nodes_v1(base_project, command)


class DeleteNodesRevisionTests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.9",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "pages": {
                PAGE_ID: {
                    "children": [KEEP_LEFT, NODE_B, NODE_A, KEEP_RIGHT],
                }
            },
            "nodes": {
                KEEP_LEFT: safe_node(KEEP_LEFT, 0),
                NODE_A: safe_node(NODE_A, 200),
                NODE_B: safe_node(NODE_B, 100),
                KEEP_RIGHT: safe_node(KEEP_RIGHT, 300),
            },
            "resources": {"resource:keep": {"kind": "image"}},
        }
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = CountingExecutor()

    def entries_unsorted(self):
        return [
            {
                "node_id": NODE_B,
                "expected_state_id": hash_id(self.project["nodes"][NODE_B]),
                "expected_child_index": 1,
            },
            {
                "node_id": NODE_A,
                "expected_state_id": hash_id(self.project["nodes"][NODE_A]),
                "expected_child_index": 2,
            },
        ]

    def request(self, op_id, entries=None, base_revision_id=None):
        return {
            "protocol_version": "chaptera.delete-nodes-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base_revision_id or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "delete_nodes",
                "page_id": PAGE_ID,
                "entries": copy.deepcopy(
                    self.entries_unsorted() if entries is None else entries
                ),
            },
        }

    def test_batch_normalizes_by_node_id_but_deletes_against_base_order(self):
        result = self.kernel.commit_delete_nodes(
            self.request("delete-nodes-op-0001"),
            self.executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual(1, self.executor.calls)

        operation = result["canonical_operation"]
        self.assertEqual([NODE_A, NODE_B], [row["node_id"] for row in operation["entries"]])
        self.assertEqual(
            [KEEP_LEFT, NODE_B, NODE_A, KEEP_RIGHT],
            operation["base_child_order"],
        )
        self.assertEqual(
            [2, 1],
            [row["child_index"] for row in operation["entries"]],
        )

        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual([KEEP_LEFT, KEEP_RIGHT], current["pages"][PAGE_ID]["children"])
        self.assertNotIn(NODE_A, current["nodes"])
        self.assertNotIn(NODE_B, current["nodes"])
        self.assertEqual(self.project["nodes"][KEEP_LEFT], current["nodes"][KEEP_LEFT])
        self.assertEqual(self.project["nodes"][KEEP_RIGHT], current["nodes"][KEEP_RIGHT])
        self.assertEqual(self.project["resources"], current["resources"])
        self.assertEqual(SOURCE_HASH, current["source_hash"])
        self.assertEqual(1, len(current["operations"]))
        self.assertEqual("delete_nodes", current["operations"][0]["kind"])

    def test_entry_order_is_not_semantic_for_idempotency(self):
        client_id = "delete-nodes-op-0002"
        first = self.kernel.commit_delete_nodes(
            self.request(client_id),
            self.executor,
        )
        reversed_entries = list(reversed(self.entries_unsorted()))
        second = self.kernel.commit_delete_nodes(
            self.request(client_id, entries=reversed_entries),
            self.executor,
        )
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

    def test_empty_duplicate_or_duplicate_index_rejected_before_executor(self):
        bad_cases = []

        bad_cases.append(self.request("delete-nodes-empty", entries=[]))

        duplicate_node = self.entries_unsorted()
        duplicate_node[1]["node_id"] = NODE_B
        duplicate_node[1]["expected_state_id"] = duplicate_node[0]["expected_state_id"]
        bad_cases.append(self.request("delete-nodes-dup-node", entries=duplicate_node))

        duplicate_index = self.entries_unsorted()
        duplicate_index[1]["expected_child_index"] = 1
        bad_cases.append(self.request("delete-nodes-dup-index", entries=duplicate_index))

        for req in bad_cases:
            with self.subTest(op=req["client_operation_id"]):
                with self.assertRaises(ValueError):
                    self.kernel.commit_delete_nodes(req, self.executor)
        self.assertEqual(0, self.executor.calls)
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_stale_second_member_rejects_whole_batch_without_partial_delete(self):
        entries = self.entries_unsorted()
        for entry in entries:
            if entry["node_id"] == NODE_B:
                entry["expected_state_id"] = "sha256:" + "0" * 64

        with self.assertRaisesRegex(DeleteNodesV1Error, "stale entity state"):
            self.kernel.commit_delete_nodes(
                self.request("delete-nodes-stale", entries=entries),
                self.executor,
            )

        self.assertEqual(1, self.executor.calls)
        current = self.kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(self.baseline.revision_id, current.revision_id)
        self.assertEqual(self.project, current.project)

    def test_unsafe_member_rejects_whole_batch(self):
        project = copy.deepcopy(self.project)
        project["nodes"][NODE_B]["author_created"] = False
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        entries = self.entries_unsorted()
        for entry in entries:
            entry["expected_state_id"] = hash_id(project["nodes"][entry["node_id"]])
        executor = CountingExecutor()

        with self.assertRaisesRegex(DeleteNodesV1Error, "not canonically author-created"):
            kernel.commit_delete_nodes(
                self.request(
                    "delete-nodes-unsafe",
                    entries=entries,
                    base_revision_id=baseline.revision_id,
                ),
                executor,
            )
        self.assertEqual(1, executor.calls)
        self.assertEqual(project, kernel.current_revision(DOCUMENT_ID).project)

    def test_browser_cannot_supply_inverse_or_cascade_state(self):
        req = self.request("delete-nodes-extra-command")
        req["command"]["base_child_order"] = list(
            self.project["pages"][PAGE_ID]["children"]
        )
        with self.assertRaisesRegex(ValueError, "non-intent"):
            self.kernel.commit_delete_nodes(req, self.executor)

        req = self.request("delete-nodes-extra-entry")
        req["command"]["entries"][0]["before_entity"] = copy.deepcopy(
            self.project["nodes"][NODE_B]
        )
        with self.assertRaisesRegex(ValueError, "non-intent"):
            self.kernel.commit_delete_nodes(req, self.executor)
        self.assertEqual(0, self.executor.calls)

    def test_executor_cannot_smuggle_resource_or_other_project_state_changes(self):
        def bad_resources(base_project, command):
            operation, project, consequences = execute_delete_nodes_v1(
                base_project, command
            )
            project["resources"] = {}
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "non-node project state"):
            self.kernel.commit_delete_nodes(
                self.request("delete-nodes-forged-resources"),
                bad_resources,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )
        self.assertEqual(
            self.project["resources"],
            self.kernel.current_revision(DOCUMENT_ID).project["resources"],
        )

    def test_forged_canonical_member_or_base_order_cannot_advance_revision(self):
        def forged_member(base_project, command):
            operation, project, consequences = execute_delete_nodes_v1(
                base_project, command
            )
            operation["entries"][0]["before_entity"]["paint"] = {"fill": "forged"}
            project["operations"][-1] = copy.deepcopy(operation)
            return operation, project, consequences

        with self.assertRaises(ValueError):
            self.kernel.commit_delete_nodes(
                self.request("delete-nodes-forged-member"),
                forged_member,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

        def forged_order(base_project, command):
            operation, project, consequences = execute_delete_nodes_v1(
                base_project, command
            )
            operation["base_child_order"] = list(reversed(operation["base_child_order"]))
            project["operations"][-1] = copy.deepcopy(operation)
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "base_child_order"):
            self.kernel.commit_delete_nodes(
                self.request("delete-nodes-forged-order"),
                forged_order,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_undo_restores_exact_entities_and_original_child_order_then_redo_replays(self):
        accepted = self.kernel.commit_delete_nodes(
            self.request("delete-nodes-history-commit"),
            self.executor,
        )
        operation = copy.deepcopy(accepted["canonical_operation"])
        deleted_project = copy.deepcopy(
            self.kernel.current_revision(DOCUMENT_ID).project
        )
        canonical_command = copy.deepcopy(self.request("unused")["command"])
        canonical_command["entries"] = sorted(
            canonical_command["entries"], key=lambda row: row["node_id"]
        )

        def history_executor(base_project, kind):
            if kind == "undo":
                restored = restore_delete_nodes_v1(base_project, operation)
                return restored, [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                replayed_operation, replayed, _ = execute_delete_nodes_v1(
                    base_project,
                    canonical_command,
                )
                self.assertEqual(operation, replayed_operation)
                return replayed, [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported history transition")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "delete-nodes-history-undo",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        restored = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(self.project, restored)

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "delete-nodes-history-redo",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            deleted_project,
            self.kernel.current_revision(DOCUMENT_ID).project,
        )

    def test_save_reopen_preserves_batch_result(self):
        self.kernel.commit_delete_nodes(
            self.request("delete-nodes-save-reopen"),
            self.executor,
        )
        saved = json.loads(
            json.dumps(
                self.kernel.current_revision(DOCUMENT_ID).project,
                sort_keys=True,
            )
        )

        reopened = RevisionKernel()
        reopened.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(saved, reopened.current_revision(DOCUMENT_ID).project)
        self.assertEqual(
            [KEEP_LEFT, KEEP_RIGHT],
            reopened.current_revision(DOCUMENT_ID).project["pages"][PAGE_ID]["children"],
        )


if __name__ == "__main__":
    unittest.main()
