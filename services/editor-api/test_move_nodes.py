import copy
import unittest

from revision_store import RevisionKernel


DOCUMENT_ID = "move-nodes-doc"
SOURCE_HASH = "d" * 64
PAGE_ID = "page:1"


def rect(x, y, width=100, height=80):
    return {"x": x, "y": y, "width": width, "height": height}


class MoveNodesExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        project = copy.deepcopy(base_project)
        entries = []
        for requested in command["entries"]:
            node_id = requested["node_id"]
            node = project["nodes"][node_id]
            if node["page_id"] != command["page_id"]:
                raise ValueError("node is not on requested page")
            if node["provenance"] != "chaptera-authored":
                raise ValueError("node is not canonically author-created")
            before = copy.deepcopy(node["bounds"])
            if before != requested["expected_before"]:
                raise ValueError("stale MoveNodes before-state")
            after = copy.deepcopy(requested["after"])
            node["bounds"] = copy.deepcopy(after)
            entries.append(
                {
                    "node_id": node_id,
                    "before": before,
                    "after": after,
                }
            )

        operation = {
            "kind": "move_nodes",
            "page_id": command["page_id"],
            "entries": entries,
        }
        project["schema_version"] = "pub-editor-v0.7"
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        return operation, project, [
            {"key": "node.geometry.position.batch", "state": "supported", "note": None}
        ]


def move_request(base_revision_id, op_id, entries):
    return {
        "protocol_version": "chaptera.move-nodes-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "move_nodes",
            "page_id": PAGE_ID,
            "entries": copy.deepcopy(entries),
        },
    }


class MoveNodesRevisionTests(unittest.TestCase):
    def setUp(self):
        self.kernel = RevisionKernel()
        self.project = {
            "schema_version": "pub-editor-v0.6",
            "source_hash": SOURCE_HASH,
            "operations": [],
            "nodes": {
                "node:a": {
                    "page_id": PAGE_ID,
                    "provenance": "chaptera-authored",
                    "bounds": rect(0, 0),
                },
                "node:b": {
                    "page_id": PAGE_ID,
                    "provenance": "chaptera-authored",
                    "bounds": rect(200, 10, 50, 60),
                },
                "node:source": {
                    "page_id": PAGE_ID,
                    "provenance": "source-backed",
                    "bounds": rect(500, 0),
                },
            },
        }
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = MoveNodesExecutor()

    def entries_unsorted(self):
        return [
            {
                "node_id": "node:b",
                "expected_before": rect(200, 10, 50, 60),
                "after": rect(180, 40, 50, 60),
            },
            {
                "node_id": "node:a",
                "expected_before": rect(0, 0),
                "after": rect(-20, 30),
            },
        ]

    def test_batch_normalizes_by_node_id_and_commits_one_revision(self):
        result = self.kernel.commit_move_nodes(
            move_request(self.baseline.revision_id, "op:move:1", self.entries_unsorted()),
            self.executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual(1, self.executor.calls)
        self.assertEqual(
            ["node:a", "node:b"],
            [entry["node_id"] for entry in result["canonical_operation"]["entries"]],
        )
        current = self.kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(result["revision_id"], current.revision_id)
        self.assertEqual(1, len(current.project["operations"]))
        self.assertEqual(rect(-20, 30), current.project["nodes"]["node:a"]["bounds"])
        self.assertEqual(rect(180, 40, 50, 60), current.project["nodes"]["node:b"]["bounds"])

    def test_entry_order_is_not_semantic_for_idempotency(self):
        client_id = "op:move:2"
        first = self.kernel.commit_move_nodes(
            move_request(self.baseline.revision_id, client_id, self.entries_unsorted()),
            self.executor,
        )
        same_entries_reordered = list(reversed(self.entries_unsorted()))
        second = self.kernel.commit_move_nodes(
            move_request(self.baseline.revision_id, client_id, same_entries_reordered),
            self.executor,
        )
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)

    def test_duplicate_node_resize_smuggling_and_noop_fail_before_executor(self):
        duplicate = self.entries_unsorted()
        duplicate[1]["node_id"] = "node:b"
        with self.assertRaisesRegex(ValueError, "unique"):
            self.kernel.commit_move_nodes(
                move_request(self.baseline.revision_id, "op:bad:dup", duplicate),
                self.executor,
            )

        resized = self.entries_unsorted()
        resized[0]["after"]["width"] += 1
        with self.assertRaisesRegex(ValueError, "translation only"):
            self.kernel.commit_move_nodes(
                move_request(self.baseline.revision_id, "op:bad:resize", resized),
                self.executor,
            )

        noop = self.entries_unsorted()
        noop[0]["after"] = copy.deepcopy(noop[0]["expected_before"])
        with self.assertRaisesRegex(ValueError, "no-ops"):
            self.kernel.commit_move_nodes(
                move_request(self.baseline.revision_id, "op:bad:noop", noop),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)

    def test_stale_member_rejects_whole_batch_without_revision_advance(self):
        stale = self.entries_unsorted()
        stale[1]["expected_before"]["x"] = 1
        with self.assertRaisesRegex(ValueError, "stale MoveNodes"):
            self.kernel.commit_move_nodes(
                move_request(self.baseline.revision_id, "op:bad:stale", stale),
                self.executor,
            )
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)
        self.assertEqual(self.project, self.kernel.current_revision(DOCUMENT_ID).project)

    def test_bad_canonical_second_member_cannot_partially_advance_kernel(self):
        def bad_executor(base_project, command):
            operation, project, consequences = self.executor(base_project, command)
            operation["entries"][1]["after"]["x"] += 1
            project["operations"][-1] = copy.deepcopy(operation)
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "after-state"):
            self.kernel.commit_move_nodes(
                move_request(self.baseline.revision_id, "op:bad:canonical", self.entries_unsorted()),
                bad_executor,
            )
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_pre_execute_gate_can_reject_source_backed_member_before_executor(self):
        entries = [
            {
                "node_id": "node:source",
                "expected_before": rect(500, 0),
                "after": rect(520, 20),
            }
        ]

        def gate(command):
            for entry in command["entries"]:
                node = self.project["nodes"][entry["node_id"]]
                if node["provenance"] != "chaptera-authored":
                    raise ValueError("unsupported projection/source-backed member")

        with self.assertRaisesRegex(ValueError, "source-backed"):
            self.kernel.commit_move_nodes(
                move_request(self.baseline.revision_id, "op:bad:source", entries),
                self.executor,
                pre_execute_validator=gate,
            )
        self.assertEqual(0, self.executor.calls)
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_unsafe_bounds_fail_closed_before_executor(self):
        entries = self.entries_unsorted()
        entries[0]["after"]["x"] = 9_007_199_254_740_992
        with self.assertRaisesRegex(ValueError, "JavaScript-safe"):
            self.kernel.commit_move_nodes(
                move_request(self.baseline.revision_id, "op:bad:overflow", entries),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)


if __name__ == "__main__":
    unittest.main()
