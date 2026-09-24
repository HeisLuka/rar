#!/usr/bin/env python3
import copy
import json
import unittest

from authoring_fragment_v1 import (
    AuthoringFragmentError,
    apply_paste_fragment_v1,
    capture_rectangle_fragment_v1,
)
from revision_store import RevisionKernel, canonical_json, project_hash


SOURCE_HASH = "11" * 32
DEST_HASH = "22" * 32
SOURCE_NODE = "01890f47-0c00-7abc-8def-0123456789ab"
PASTE_NODE = "01890f47-0c01-7abc-8def-0123456789ab"
SECOND_PASTE_NODE = "01890f47-0c02-7abc-8def-0123456789ab"
PAGE = "page:destination"

SOURCE_BOUNDS = {"x": -100, "y": 200, "width": 300, "height": 400}
SOURCE_PAINT = {
    "fill": {"visible": True, "color": {"r": 17, "g": 34, "b": 51}},
    "stroke": {
        "visible": True,
        "color": {"r": 68, "g": 85, "b": 102},
        "width_emu": 12_700,
    },
}


def source_project():
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "operations": [],
        "pages": {"page:source": {"authoring_enabled": True, "children": ["source:base"]}},
        "shapes": {
            SOURCE_NODE: {
                "node_id": SOURCE_NODE,
                "kind": "shape",
                "shape_kind": "rectangle",
                "page_id": "page:source",
                "parent_id": "page:source",
                "bounds": copy.deepcopy(SOURCE_BOUNDS),
                "transform": {"kind": "identity"},
                "paint": {
                    "fill": copy.deepcopy(SOURCE_PAINT["fill"]),
                    "stroke": copy.deepcopy(SOURCE_PAINT["stroke"]),
                    "provenance": {"kind": "author_created"},
                },
                "provenance": {"kind": "author_created"},
            }
        },
    }


def destination_project(*, authoring_enabled=True, existing_shapes=None):
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": DEST_HASH,
        "operations": [],
        "pages": {
            PAGE: {
                "authoring_enabled": authoring_enabled,
                "children": ["source:base"],
            }
        },
        "shapes": copy.deepcopy(existing_shapes or {}),
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
    }


def paste_request(fragment, base_revision_id, op_id, *, node_id=PASTE_NODE, dx=1000, dy=-50):
    return {
        "protocol_version": "chaptera.paste-fragment-intent.v1",
        "document_id": "doc:destination",
        "source_hash": DEST_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "paste_fragment",
            "fragment": copy.deepcopy(fragment),
            "identity_map": {
                "fragment_entity_id": "entity:0",
                "destination_node_id": node_id,
            },
            "destination": {"kind": "page", "page_id": PAGE},
            "placement": {"kind": "translate", "dx_emu": dx, "dy_emu": dy},
        },
    }


class CountingPasteExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, project, command):
        self.calls += 1
        return apply_paste_fragment_v1(project, command)


class AuthoringFragmentTests(unittest.TestCase):
    def setUp(self):
        self.fragment = capture_rectangle_fragment_v1(source_project(), SOURCE_NODE)

    def test_capture_uses_local_identity_and_source_only_as_provenance(self):
        rectangle = self.fragment["rectangle"]
        self.assertEqual("chaptera.authoring-fragment.v1", self.fragment["schema_version"])
        self.assertEqual("entity:0", rectangle["fragment_entity_id"])
        self.assertEqual(SOURCE_BOUNDS, rectangle["bounds"])
        self.assertEqual(SOURCE_PAINT["fill"], rectangle["fill"])
        self.assertEqual(SOURCE_PAINT["stroke"], rectangle["stroke"])
        self.assertEqual(
            {"source_node_id": SOURCE_NODE},
            rectangle["source_provenance"],
        )
        self.assertNotEqual(SOURCE_NODE, rectangle["fragment_entity_id"])

    def test_capture_rejects_non_author_created_or_transformed_shape(self):
        project = source_project()
        project["shapes"][SOURCE_NODE]["provenance"] = {"kind": "source_backed"}
        with self.assertRaisesRegex(AuthoringFragmentError, "author-created"):
            capture_rectangle_fragment_v1(project, SOURCE_NODE)

        project = source_project()
        project["shapes"][SOURCE_NODE]["transform"] = {"kind": "rotate"}
        with self.assertRaisesRegex(AuthoringFragmentError, "identity-transform"):
            capture_rectangle_fragment_v1(project, SOURCE_NODE)


class PasteFragmentCommitTests(unittest.TestCase):
    def setUp(self):
        self.fragment = capture_rectangle_fragment_v1(source_project(), SOURCE_NODE)
        self.base_project = destination_project()
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id="doc:destination",
            source_hash=DEST_HASH,
            project=self.base_project,
        )
        self.executor = CountingPasteExecutor()

    def test_cross_document_paste_materializes_exact_translated_rectangle(self):
        result = self.kernel.commit_paste_fragment(
            paste_request(self.fragment, self.baseline.revision_id, "paste-op-00000001"),
            self.executor,
        )
        current = self.kernel.current_revision("doc:destination").project
        entity = current["shapes"][PASTE_NODE]

        self.assertEqual(
            {"x": 900, "y": 150, "width": 300, "height": 400},
            entity["bounds"],
        )
        self.assertEqual(SOURCE_PAINT["fill"], entity["paint"]["fill"])
        self.assertEqual(SOURCE_PAINT["stroke"], entity["paint"]["stroke"])
        self.assertEqual({"kind": "author_created"}, entity["paint"]["provenance"])
        self.assertEqual({"kind": "author_created"}, entity["provenance"])
        self.assertEqual(PAGE, entity["parent_id"])
        self.assertEqual(["source:base"], current["pages"][PAGE]["children"])
        self.assertEqual(DEST_HASH, current["source_hash"])
        self.assertNotIn("source_provenance", entity)
        self.assertEqual(
            {"source_node_id": SOURCE_NODE},
            result["canonical_operation"]["fragment"]["rectangle"]["source_provenance"],
        )
        self.assertEqual(PASTE_NODE, result["canonical_operation"]["created_entity"]["node_id"])

    def test_exact_retry_is_idempotent(self):
        request = paste_request(
            self.fragment,
            self.baseline.revision_id,
            "paste-op-00000002",
        )
        first = self.kernel.commit_paste_fragment(copy.deepcopy(request), self.executor)
        second = self.kernel.commit_paste_fragment(copy.deepcopy(request), self.executor)
        self.assertEqual(first, second)
        self.assertEqual(1, self.executor.calls)
        self.assertEqual(
            1,
            len(self.kernel.current_revision("doc:destination").project["operations"]),
        )

    def test_second_user_paste_gets_distinct_persisted_identity(self):
        first = self.kernel.commit_paste_fragment(
            paste_request(self.fragment, self.baseline.revision_id, "paste-op-00000003"),
            self.executor,
        )
        second = self.kernel.commit_paste_fragment(
            paste_request(
                self.fragment,
                first["revision_id"],
                "paste-op-00000004",
                node_id=SECOND_PASTE_NODE,
            ),
            self.executor,
        )
        current = self.kernel.current_revision("doc:destination").project
        self.assertIn(PASTE_NODE, current["shapes"])
        self.assertIn(SECOND_PASTE_NODE, current["shapes"])
        self.assertNotEqual(PASTE_NODE, SECOND_PASTE_NODE)
        self.assertEqual(
            current["shapes"][PASTE_NODE]["bounds"],
            current["shapes"][SECOND_PASTE_NODE]["bounds"],
        )
        self.assertEqual(second["revision_id"], self.kernel.current_revision("doc:destination").revision_id)

    def test_stale_base_rejected_before_second_execution(self):
        first = self.kernel.commit_paste_fragment(
            paste_request(self.fragment, self.baseline.revision_id, "paste-op-00000005"),
            self.executor,
        )
        calls = self.executor.calls
        stale = self.kernel.commit_paste_fragment(
            paste_request(
                self.fragment,
                self.baseline.revision_id,
                "paste-op-00000006",
                node_id=SECOND_PASTE_NODE,
            ),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(first["revision_id"], stale["current_revision_id"])
        self.assertEqual(calls, self.executor.calls)

    def test_source_identity_reuse_fails_before_execution(self):
        with self.assertRaisesRegex(AuthoringFragmentError, "reuses source identity"):
            self.kernel.commit_paste_fragment(
                paste_request(
                    self.fragment,
                    self.baseline.revision_id,
                    "paste-op-source-reuse",
                    node_id=SOURCE_NODE,
                ),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision("doc:destination").revision_id,
        )

    def test_destination_collision_invalid_page_and_disabled_page_fail_atomically(self):
        project = destination_project(existing_shapes={PASTE_NODE: {"kind": "existing"}})
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:destination",
            source_hash=DEST_HASH,
            project=project,
        )
        with self.assertRaisesRegex(AuthoringFragmentError, "node_id_collision"):
            kernel.commit_paste_fragment(
                paste_request(self.fragment, baseline.revision_id, "paste-collision"),
                apply_paste_fragment_v1,
            )
        self.assertEqual(baseline.revision_id, kernel.current_revision("doc:destination").revision_id)

        bad = paste_request(self.fragment, self.baseline.revision_id, "paste-bad-page")
        bad["command"]["destination"]["page_id"] = "page:missing"
        with self.assertRaisesRegex(AuthoringFragmentError, "invalid_paste_fragment_page"):
            self.kernel.commit_paste_fragment(bad, self.executor)

        disabled = destination_project(authoring_enabled=False)
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:destination",
            source_hash=DEST_HASH,
            project=disabled,
        )
        with self.assertRaisesRegex(AuthoringFragmentError, "not_authorable"):
            kernel.commit_paste_fragment(
                paste_request(self.fragment, baseline.revision_id, "paste-disabled"),
                apply_paste_fragment_v1,
            )

    def test_unsafe_translation_and_forged_executor_fail_without_revision_move(self):
        unsafe = paste_request(
            self.fragment,
            self.baseline.revision_id,
            "paste-unsafe",
            dx=9_007_199_254_740_991,
        )
        with self.assertRaisesRegex(AuthoringFragmentError, "translated bounds are unsafe"):
            self.kernel.commit_paste_fragment(unsafe, self.executor)
        self.assertEqual(0, self.executor.calls)

        def forged(project, command):
            operation, result, consequences = apply_paste_fragment_v1(project, command)
            operation["created_entity"]["bounds"]["x"] += 1
            return operation, result, consequences

        with self.assertRaisesRegex(ValueError, "non-canonical PasteFragment"):
            self.kernel.commit_paste_fragment(
                paste_request(self.fragment, self.baseline.revision_id, "paste-forged"),
                forged,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision("doc:destination").revision_id,
        )

    def test_undo_redo_replay_and_save_reopen_preserve_identity_and_state(self):
        accepted = self.kernel.commit_paste_fragment(
            paste_request(self.fragment, self.baseline.revision_id, "paste-op-00000007"),
            self.executor,
        )
        pasted_project = copy.deepcopy(self.kernel.current_revision("doc:destination").project)

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(self.base_project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(pasted_project), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported history transition")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": "doc:destination",
                "source_hash": DEST_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "paste-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertNotIn(PASTE_NODE, self.kernel.current_revision("doc:destination").project["shapes"])

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": "doc:destination",
                "source_hash": DEST_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "paste-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            pasted_project["shapes"][PASTE_NODE],
            self.kernel.current_revision("doc:destination").project["shapes"][PASTE_NODE],
        )

        canonical = accepted["canonical_operation"]
        replay_command = {
            "kind": "paste_fragment",
            "fragment": copy.deepcopy(canonical["fragment"]),
            "identity_map": copy.deepcopy(canonical["identity_map"]),
            "destination": copy.deepcopy(canonical["destination"]),
            "placement": copy.deepcopy(canonical["placement"]),
        }
        replay_operation, replayed, _ = apply_paste_fragment_v1(
            copy.deepcopy(self.base_project),
            replay_command,
        )
        self.assertEqual(canonical, replay_operation)
        self.assertEqual(
            pasted_project["shapes"][PASTE_NODE],
            replayed["shapes"][PASTE_NODE],
        )

        saved = json.loads(canonical_json(pasted_project).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="doc:reopened",
            source_hash=DEST_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(pasted_project), reopened_baseline.project_hash)
        self.assertEqual(PASTE_NODE, reopened_baseline.project["shapes"][PASTE_NODE]["node_id"])


if __name__ == "__main__":
    unittest.main()
