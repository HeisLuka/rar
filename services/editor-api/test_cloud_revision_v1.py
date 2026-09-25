import copy
import unittest

from cloud_revision_v1 import (
    AUTHORING_REVISION_SCHEMA_V1,
    CloudRevisionKernelV1,
    DerivedArtifactConflict,
    DerivedArtifactFenceV1,
    DerivedArtifactStoreV1,
    SemanticDiffContractError,
    normalize_semantic_diff_v1,
)


DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
NODE_ID = "30000000-0000-4000-8000-000000000001"
STORY_ID = "40000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64
CANONICAL_BASE = "1" * 64
CANONICAL_CHILD = "2" * 64
ENV = "sha256:" + "3" * 64
INPUT = "sha256:" + "4" * 64
CONTENT = "sha256:" + "5" * 64


def baseline_project():
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "operations": [],
        "nodes": {
            NODE_ID: {"bounds": {"x": 10, "y": 20, "width": 300, "height": 200}}
        },
        "stories": {STORY_ID: "abc"},
    }


def node_op(before_x=10, after_x=30):
    return {
        "kind": "update_node_bounds",
        "node_id": NODE_ID,
        "before": {"x": before_x, "y": 20, "width": 300, "height": 200},
        "after": {"x": after_x, "y": 40, "width": 300, "height": 200},
    }


def story_op(before="abc", after="xyz"):
    return {
        "kind": "update_story_text",
        "story_id": STORY_ID,
        "before": before,
        "after": after,
    }


def semantic_diff(*ops, base=CANONICAL_BASE):
    return {
        "schema_version": AUTHORING_REVISION_SCHEMA_V1,
        "base_revision_id": base,
        "operations": list(ops),
    }


def request(service_base, client_id, diff):
    return {
        "protocol_version": "chaptera.semantic-diff-commit.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": service_base,
        "client_operation_id": client_id,
        "command": {
            "kind": "apply_semantic_diff_v1",
            "semantic_diff": diff,
        },
    }


class FakeCanonicalExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, diff):
        self.calls += 1
        project = copy.deepcopy(base_project)
        for operation in diff["operations"]:
            if operation["kind"] == "update_node_bounds":
                current = project["nodes"][operation["node_id"]]["bounds"]
                if current != operation["before"]:
                    raise ValueError("node before-state mismatch")
                project["nodes"][operation["node_id"]]["bounds"] = copy.deepcopy(
                    operation["after"]
                )
            elif operation["kind"] == "update_story_text":
                if project["stories"][operation["story_id"]] != operation["before"]:
                    raise ValueError("story before-state mismatch")
                project["stories"][operation["story_id"]] = operation["after"]
            else:
                raise ValueError("unsupported")
        project["operations"] = list(project["operations"]) + copy.deepcopy(diff["operations"])
        receipt = {
            "schema_version": AUTHORING_REVISION_SCHEMA_V1,
            "revision_id": CANONICAL_CHILD,
            "parent_revision_id": diff["base_revision_id"],
        }
        return receipt, project, [{"key": "canonical.diff", "state": "supported", "note": None}]


class CloudRevisionContractTests(unittest.TestCase):
    def setUp(self):
        self.kernel = CloudRevisionKernelV1()
        self.baseline = self.kernel.register_canonical_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=baseline_project(),
            canonical_revision_id=CANONICAL_BASE,
        )
        self.executor = FakeCanonicalExecutor()

    def test_operation_enumeration_order_normalizes(self):
        left = normalize_semantic_diff_v1(semantic_diff(story_op(), node_op()))
        right = normalize_semantic_diff_v1(semantic_diff(node_op(), story_op()))
        self.assertEqual(left, right)
        self.assertEqual(
            ["update_node_bounds", "update_story_text"],
            [item["kind"] for item in left["operations"]],
        )

    def test_duplicate_same_target_fails_closed(self):
        with self.assertRaisesRegex(SemanticDiffContractError, "duplicate_target"):
            normalize_semantic_diff_v1(semantic_diff(node_op(), node_op(after_x=40)))

    def test_add_remove_rebind_are_serializable_but_consumer_rejects(self):
        diff = semantic_diff(
            {
                "kind": "add_entity",
                "target_id": "50000000-0000-4000-8000-000000000001",
            }
        )
        normalized = normalize_semantic_diff_v1(diff)
        self.assertEqual("add_entity", normalized["operations"][0]["kind"])
        with self.assertRaisesRegex(SemanticDiffContractError, "unsupported_operation"):
            self.kernel.commit_semantic_diff(
                request(
                    self.baseline.revision_id,
                    "90000000-0000-4000-8000-000000000001",
                    diff,
                ),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)

    def test_semantic_diff_base_must_match_canonical_binding(self):
        wrong = semantic_diff(node_op(), base="e" * 64)
        with self.assertRaisesRegex(SemanticDiffContractError, "wrong_base_revision"):
            self.kernel.commit_semantic_diff(
                request(
                    self.baseline.revision_id,
                    "90000000-0000-4000-8000-000000000002",
                    wrong,
                ),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)

    def test_commit_binds_service_child_to_canonical_child(self):
        result = self.kernel.commit_semantic_diff(
            request(
                self.baseline.revision_id,
                "90000000-0000-4000-8000-000000000003",
                semantic_diff(story_op(), node_op()),
            ),
            self.executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual(CANONICAL_CHILD, result["canonical_revision_id"])
        self.assertEqual(CANONICAL_BASE, result["canonical_parent_revision_id"])
        self.assertEqual(
            CANONICAL_CHILD,
            self.kernel.canonical_revision_for(
                document_id=DOCUMENT_ID,
                service_revision_id=result["revision_id"],
            ),
        )
        self.assertNotEqual(result["revision_id"], result["canonical_revision_id"])

    def test_exact_retry_is_idempotent_and_keeps_binding(self):
        req = request(
            self.baseline.revision_id,
            "90000000-0000-4000-8000-000000000004",
            semantic_diff(node_op()),
        )
        first = self.kernel.commit_semantic_diff(copy.deepcopy(req), self.executor)
        second = self.kernel.commit_semantic_diff(copy.deepcopy(req), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)
        self.assertEqual(CANONICAL_CHILD, second["canonical_revision_id"])

    def test_stale_service_base_rejected_before_second_executor_call(self):
        first = self.kernel.commit_semantic_diff(
            request(
                self.baseline.revision_id,
                "90000000-0000-4000-8000-000000000005",
                semantic_diff(node_op()),
            ),
            self.executor,
        )
        stale = self.kernel.commit_semantic_diff(
            request(
                self.baseline.revision_id,
                "90000000-0000-4000-8000-000000000006",
                semantic_diff(node_op()),
            ),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertEqual(1, self.executor.calls)

    def test_canonical_child_parent_mismatch_cannot_advance_service_head(self):
        def bad_executor(base_project, diff):
            return (
                {
                    "schema_version": AUTHORING_REVISION_SCHEMA_V1,
                    "revision_id": CANONICAL_CHILD,
                    "parent_revision_id": "f" * 64,
                },
                copy.deepcopy(base_project),
                [],
            )

        with self.assertRaisesRegex(SemanticDiffContractError, "wrong_parent_revision"):
            self.kernel.commit_semantic_diff(
                request(
                    self.baseline.revision_id,
                    "90000000-0000-4000-8000-000000000007",
                    semantic_diff(node_op()),
                ),
                bad_executor,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )


class DerivedArtifactFenceTests(unittest.TestCase):
    def fence(self, *, revision=CANONICAL_BASE, stage="scene", env=ENV, input_fp=INPUT):
        return DerivedArtifactFenceV1(
            document_id=DOCUMENT_ID,
            service_revision_id="sha256:" + "6" * 64,
            canonical_revision_id=revision,
            stage=stage,
            stage_version="v1",
            environment_fingerprint=env,
            input_fingerprint=input_fp,
        )

    def test_exact_fence_hit_and_identical_retry(self):
        store = DerivedArtifactStoreV1()
        fence = self.fence()
        key = store.publish(fence, CONTENT)
        self.assertEqual(CONTENT, store.resolve(fence))
        self.assertEqual(key, store.publish(fence, CONTENT))
        self.assertFalse(store.rebuild_required(fence))

    def test_revision_environment_stage_or_input_change_is_exact_miss(self):
        store = DerivedArtifactStoreV1()
        store.publish(self.fence(), CONTENT)
        self.assertTrue(store.rebuild_required(self.fence(revision=CANONICAL_CHILD)))
        self.assertTrue(
            store.rebuild_required(
                self.fence(env="sha256:" + "7" * 64)
            )
        )
        self.assertTrue(store.rebuild_required(self.fence(stage="preview")))
        self.assertTrue(
            store.rebuild_required(
                self.fence(input_fp="sha256:" + "8" * 64)
            )
        )

    def test_same_exact_fence_cannot_publish_different_content(self):
        store = DerivedArtifactStoreV1()
        fence = self.fence()
        store.publish(fence, CONTENT)
        with self.assertRaisesRegex(DerivedArtifactConflict, "different content"):
            store.publish(fence, "sha256:" + "9" * 64)


if __name__ == "__main__":
    unittest.main()
