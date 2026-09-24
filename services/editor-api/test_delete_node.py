#!/usr/bin/env python3
import copy
import unittest

from revision_store import RevisionKernel, hash_id

DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64
PAGE_ID = "page:1"
NODE_ID = "node:authored:1"
OTHER_NODE_ID = "node:authored:2"


class FakeDeleteNodeExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        node_id = command["node_id"]
        node = base_project["nodes"].get(node_id)
        if node is None:
            raise ValueError("already_deleted_or_missing")
        if not node.get("author_created") or node.get("node_class") != "ordinary_leaf":
            raise ValueError("unsupported_delete_node_class")
        if node.get("dependencies"):
            raise ValueError("delete_node_has_dependencies")

        parent_id = node["parent_id"]
        children = base_project["pages"][parent_id]["children"]
        try:
            child_index = children.index(node_id)
        except ValueError as exc:
            raise ValueError("delete_node_parent_order_mismatch") from exc

        before_entity = copy.deepcopy(node)
        before_state_id = hash_id(before_entity)
        if before_state_id != command["expected_state_id"]:
            raise ValueError("stale_delete_node_state")
        if parent_id != command["expected_parent_id"]:
            raise ValueError("stale_delete_node_parent")
        if child_index != command["expected_child_index"]:
            raise ValueError("stale_delete_node_order")

        operation = {
            "kind": "delete_node",
            "node_id": node_id,
            "before_entity": before_entity,
            "before_state_id": before_state_id,
            "parent_id": parent_id,
            "child_index": child_index,
        }

        project = copy.deepcopy(base_project)
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        project["nodes"] = copy.deepcopy(project["nodes"])
        del project["nodes"][node_id]
        project["pages"] = copy.deepcopy(project["pages"])
        del project["pages"][parent_id]["children"][child_index]

        return operation, project, [
            {"key": "node.delete", "state": "supported", "note": "intentional_effective_deletion"},
            {"key": "layout.scene", "state": "invalidated", "note": None},
        ]


class DeleteNodeCommitTests(unittest.TestCase):
    def setUp(self):
        self.node = {
            "node_id": NODE_ID,
            "parent_id": PAGE_ID,
            "node_class": "ordinary_leaf",
            "author_created": True,
            "bounds": {"x": 10, "y": 20, "width": 300, "height": 200},
            "paint": {"fill": "solid"},
            "dependencies": [],
        }
        self.other_node = {
            "node_id": OTHER_NODE_ID,
            "parent_id": PAGE_ID,
            "node_class": "ordinary_leaf",
            "author_created": True,
            "bounds": {"x": 400, "y": 20, "width": 100, "height": 100},
            "paint": {"fill": "none"},
            "dependencies": [],
        }
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "pages": {
                PAGE_ID: {"children": [OTHER_NODE_ID, NODE_ID]},
            },
            "nodes": {
                OTHER_NODE_ID: copy.deepcopy(self.other_node),
                NODE_ID: copy.deepcopy(self.node),
            },
            "resources": {"resource:keep": {"kind": "image"}},
        }
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = FakeDeleteNodeExecutor()

    def request(self, op_id, *, state_id=None, parent_id=PAGE_ID, child_index=1, base=None):
        return {
            "protocol_version": "chaptera.delete-node-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "delete_node",
                "node_id": NODE_ID,
                "expected_state_id": state_id or hash_id(self.node),
                "expected_parent_id": parent_id,
                "expected_child_index": child_index,
            },
        }

    def test_delete_removes_only_target_and_preserves_source_and_resources(self):
        result = self.kernel.commit_delete_node(
            self.request("delete-op-00000001"),
            self.executor,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(NODE_ID, current["nodes"])
        self.assertEqual([OTHER_NODE_ID], current["pages"][PAGE_ID]["children"])
        self.assertEqual(self.other_node, current["nodes"][OTHER_NODE_ID])
        self.assertEqual(self.project["resources"], current["resources"])
        self.assertEqual(SOURCE_HASH, current["source_hash"])
        operation = result["canonical_operation"]
        self.assertEqual(self.node, operation["before_entity"])
        self.assertEqual(hash_id(self.node), operation["before_state_id"])
        self.assertEqual(PAGE_ID, operation["parent_id"])
        self.assertEqual(1, operation["child_index"])
        self.assertEqual("intentional_effective_deletion", result["consequences"][0]["note"])

    def test_exact_retry_is_idempotent_and_does_not_delete_twice(self):
        req = self.request("delete-op-00000002")
        first = self.kernel.commit_delete_node(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_delete_node(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

    def test_revision_stale_base_rejected_before_executor(self):
        first = self.kernel.commit_delete_node(
            self.request("delete-op-00000003"),
            self.executor,
        )
        calls = self.executor.calls
        stale = self.kernel.commit_delete_node(
            self.request("delete-op-00000004"),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertEqual(calls, self.executor.calls)

    def test_entity_parent_and_order_preconditions_fail_closed(self):
        cases = [
            ("state", self.request("delete-op-00000005", state_id="sha256:" + "b" * 64), "stale_delete_node_state"),
            ("parent", self.request("delete-op-00000006", parent_id="page:other"), "stale_delete_node_parent"),
            ("order", self.request("delete-op-00000007", child_index=0), "stale_delete_node_order"),
        ]
        for _label, req, message in cases:
            kernel = RevisionKernel()
            kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=self.project)
            executor = FakeDeleteNodeExecutor()
            with self.subTest(message=message):
                with self.assertRaisesRegex(ValueError, message):
                    kernel.commit_delete_node(req, executor)

    def test_capability_gate_rejects_source_backed_or_dependent_node(self):
        for mutation in (
            {"author_created": False},
            {"dependencies": ["connector:1"]},
            {"node_class": "text_frame"},
        ):
            project = copy.deepcopy(self.project)
            project["nodes"][NODE_ID].update(mutation)
            kernel = RevisionKernel()
            baseline = kernel.register_baseline(
                document_id=DOCUMENT_ID,
                source_hash=SOURCE_HASH,
                project=project,
            )
            req = self.request("delete-capability-" + str(len(str(mutation))), base=baseline.revision_id)
            req["command"]["expected_state_id"] = hash_id(project["nodes"][NODE_ID])
            with self.subTest(mutation=mutation):
                with self.assertRaises(ValueError):
                    kernel.commit_delete_node(req, FakeDeleteNodeExecutor())

    def test_browser_cannot_supply_before_entity_or_cascade_policy(self):
        for field, value in (
            ("before_entity", copy.deepcopy(self.node)),
            ("cascade", True),
            ("delete_story", True),
        ):
            req = self.request("delete-extra-" + field)
            req["command"][field] = value
            with self.subTest(field=field):
                with self.assertRaisesRegex(ValueError, "non-intent"):
                    self.kernel.commit_delete_node(req, self.executor)

    def test_executor_cannot_forge_entity_parent_or_order(self):
        mutations = [
            lambda op: op["before_entity"].update({"paint": {"fill": "forged"}}),
            lambda op: op.update({"parent_id": "page:other"}),
            lambda op: op.update({"child_index": 0}),
        ]
        for i, mutate in enumerate(mutations):
            kernel = RevisionKernel()
            kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=self.project)
            executor = FakeDeleteNodeExecutor()

            def bad_executor(base_project, command, mutate=mutate, executor=executor):
                op, project, consequences = executor(base_project, command)
                mutate(op)
                return op, project, consequences

            with self.subTest(i=i):
                with self.assertRaises(ValueError):
                    kernel.commit_delete_node(self.request(f"delete-forge-{i:08d}"), bad_executor)
                self.assertEqual(self.baseline.state_id, kernel.current_revision(DOCUMENT_ID).state_id)

    def test_undo_redo_restore_exact_node_and_child_order(self):
        accepted = self.kernel.commit_delete_node(
            self.request("delete-op-00000008"),
            self.executor,
        )
        deleted_project = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(self.project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(deleted_project), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported history transition")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "delete-history-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        restored = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(self.node, restored["nodes"][NODE_ID])
        self.assertEqual([OTHER_NODE_ID, NODE_ID], restored["pages"][PAGE_ID]["children"])

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "delete-history-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        redone = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(NODE_ID, redone["nodes"])
        self.assertEqual([OTHER_NODE_ID], redone["pages"][PAGE_ID]["children"])


if __name__ == "__main__":
    unittest.main()
