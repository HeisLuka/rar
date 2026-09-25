import copy
import json
import unittest

from authoring_fragment_set_v1 import (
    AuthoringFragmentSetError,
    apply_paste_fragment_set_v1,
    capture_rectangle_fragment_set_v1,
)
from revision_store import RevisionKernel, canonical_json, project_hash


DOCUMENT_ID = "document:fragment-set"
SOURCE_HASH = "a" * 64
SOURCE_A = "01890f47-0c00-7abc-8def-0123456789ab"
SOURCE_B = "01890f47-0c03-7abc-8def-0123456789ab"
DEST_A = "01890f47-0c01-7abc-8def-0123456789ab"
DEST_B = "01890f47-0c02-7abc-8def-0123456789ab"


def shape(node_id, x, y, page_id="page:source"):
    return {
        "node_id": node_id,
        "kind": "shape",
        "shape_kind": "rectangle",
        "page_id": page_id,
        "parent_id": page_id,
        "bounds": {"x": x, "y": y, "width": 300, "height": 400},
        "transform": {"kind": "identity"},
        "paint": {
            "fill": {
                "visible": True,
                "color": {"r": 1, "g": 2, "b": 3},
            },
            "stroke": {
                "visible": True,
                "color": {"r": 4, "g": 5, "b": 6},
                "width_emu": 12700,
            },
            "provenance": {"kind": "author_created"},
        },
        "provenance": {"kind": "author_created"},
    }


def project():
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "operations": [],
        "pages": {
            "page:source": {"authoring_enabled": True},
            "page:destination": {"authoring_enabled": True},
        },
        "shapes": {
            SOURCE_A: shape(SOURCE_A, 500, 100),
            SOURCE_B: shape(SOURCE_B, -200, 900),
        },
    }


def paste_command(fragment_set, *, ids=(DEST_A, DEST_B)):
    return {
        "kind": "paste_fragment_set",
        "fragment_set": copy.deepcopy(fragment_set),
        "identity_map": [
            {
                "member_id": "member:0",
                "destination_node_id": ids[0],
            },
            {
                "member_id": "member:1",
                "destination_node_id": ids[1],
            },
        ],
        "destination": {"kind": "page", "page_id": "page:destination"},
        "placement": {"kind": "translate", "dx_emu": 1000, "dy_emu": -50},
    }


def request(base_revision_id, op_id, command):
    return {
        "protocol_version": "chaptera.paste-fragment-set-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": copy.deepcopy(command),
    }


class CountingExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, command):
        self.calls += 1
        return apply_paste_fragment_set_v1(base_project, command)


class FragmentSetContractTests(unittest.TestCase):
    def test_capture_normalizes_input_order_and_origin(self):
        base = project()
        first = capture_rectangle_fragment_set_v1(base, [SOURCE_A, SOURCE_B])
        second = capture_rectangle_fragment_set_v1(base, [SOURCE_B, SOURCE_A])
        self.assertEqual(first, second)
        self.assertEqual(["member:0", "member:1"], [m["member_id"] for m in first["members"]])
        self.assertEqual({"x": -200, "y": 100}, first["origin"])

    def test_capture_rejects_duplicate_and_mixed_page_atomically(self):
        base = project()
        with self.assertRaisesRegex(AuthoringFragmentSetError, "unique"):
            capture_rectangle_fragment_set_v1(base, [SOURCE_A, SOURCE_A])

        other = project()
        other["shapes"][SOURCE_B]["page_id"] = "page:other"
        other["shapes"][SOURCE_B]["parent_id"] = "page:other"
        with self.assertRaisesRegex(AuthoringFragmentSetError, "one source page"):
            capture_rectangle_fragment_set_v1(other, [SOURCE_A, SOURCE_B])

    def test_apply_preserves_pairwise_offsets_with_one_shared_translation(self):
        base = project()
        fragment_set = capture_rectangle_fragment_set_v1(base, [SOURCE_B, SOURCE_A])
        operation, result, _ = apply_paste_fragment_set_v1(
            base, paste_command(fragment_set)
        )
        self.assertEqual("paste_fragment_set", operation["kind"])
        self.assertEqual(2, len(operation["created_entities"]))
        created = operation["created_entities"]
        source = fragment_set["members"]
        self.assertEqual(
            created[1]["bounds"]["x"] - created[0]["bounds"]["x"],
            source[1]["fragment"]["rectangle"]["bounds"]["x"]
            - source[0]["fragment"]["rectangle"]["bounds"]["x"],
        )
        self.assertEqual(
            created[1]["bounds"]["y"] - created[0]["bounds"]["y"],
            source[1]["fragment"]["rectangle"]["bounds"]["y"]
            - source[0]["fragment"]["rectangle"]["bounds"]["y"],
        )
        self.assertEqual(1, len(result["operations"]))

    def test_destination_collision_rejects_entire_set_before_mutation(self):
        base = project()
        fragment_set = capture_rectangle_fragment_set_v1(base, [SOURCE_A, SOURCE_B])
        base["shapes"][DEST_B] = shape(DEST_B, 0, 0, "page:destination")
        before = copy.deepcopy(base)
        with self.assertRaisesRegex(AuthoringFragmentSetError, "collision"):
            apply_paste_fragment_set_v1(base, paste_command(fragment_set))
        self.assertEqual(before, base)

    def test_identity_map_must_be_complete_unique_and_normalized(self):
        base = project()
        fragment_set = capture_rectangle_fragment_set_v1(base, [SOURCE_A, SOURCE_B])

        duplicate = paste_command(fragment_set, ids=(DEST_A, DEST_A))
        with self.assertRaisesRegex(AuthoringFragmentSetError, "unique"):
            apply_paste_fragment_set_v1(base, duplicate)

        reversed_map = paste_command(fragment_set)
        reversed_map["identity_map"].reverse()
        with self.assertRaisesRegex(AuthoringFragmentSetError, "normalized"):
            apply_paste_fragment_set_v1(base, reversed_map)


class FragmentSetRevisionTests(unittest.TestCase):
    def setUp(self):
        self.base_project = project()
        self.fragment_set = capture_rectangle_fragment_set_v1(
            self.base_project, [SOURCE_B, SOURCE_A]
        )
        self.command = paste_command(self.fragment_set)
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.base_project,
        )
        self.executor = CountingExecutor()

    def test_set_commit_is_one_operation_and_one_revision(self):
        accepted = self.kernel.commit_paste_fragment_set(
            request(self.baseline.revision_id, "fragment-set-op-1", self.command),
            self.executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", accepted["protocol_version"])
        self.assertEqual("paste_fragment_set", accepted["canonical_operation"]["kind"])
        self.assertEqual(1, self.executor.calls)
        current = self.kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(1, len(current.project["operations"]))
        self.assertIn(DEST_A, current.project["shapes"])
        self.assertIn(DEST_B, current.project["shapes"])
        self.assertEqual(self.baseline.revision_id, current.parent_revision_id)

    def test_exact_retry_is_idempotent_and_stale_base_never_reexecutes(self):
        req = request(self.baseline.revision_id, "fragment-set-op-2", self.command)
        first = self.kernel.commit_paste_fragment_set(copy.deepcopy(req), self.executor)
        retry = self.kernel.commit_paste_fragment_set(copy.deepcopy(req), self.executor)
        self.assertEqual(first, retry)
        self.assertEqual(1, self.executor.calls)

        stale = self.kernel.commit_paste_fragment_set(
            request(self.baseline.revision_id, "fragment-set-op-3", self.command),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(1, self.executor.calls)

    def test_replay_and_save_reopen_preserve_complete_identity_map(self):
        accepted = self.kernel.commit_paste_fragment_set(
            request(self.baseline.revision_id, "fragment-set-op-replay", self.command),
            self.executor,
        )
        pasted = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)
        canonical = accepted["canonical_operation"]
        replay_command = {
            "kind": "paste_fragment_set",
            "fragment_set": copy.deepcopy(canonical["fragment_set"]),
            "identity_map": copy.deepcopy(canonical["identity_map"]),
            "destination": copy.deepcopy(canonical["destination"]),
            "placement": copy.deepcopy(canonical["placement"]),
        }
        replay_operation, replayed, _ = apply_paste_fragment_set_v1(
            copy.deepcopy(self.base_project),
            replay_command,
        )
        self.assertEqual(canonical, replay_operation)
        self.assertEqual(pasted["shapes"][DEST_A], replayed["shapes"][DEST_A])
        self.assertEqual(pasted["shapes"][DEST_B], replayed["shapes"][DEST_B])

        saved = json.loads(canonical_json(pasted).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="document:fragment-set-reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(pasted), reopened_baseline.project_hash)
        self.assertEqual(DEST_A, reopened_baseline.project["shapes"][DEST_A]["node_id"])
        self.assertEqual(DEST_B, reopened_baseline.project["shapes"][DEST_B]["node_id"])

    def test_undo_redo_restore_whole_set_with_same_destination_ids(self):
        accepted = self.kernel.commit_paste_fragment_set(
            request(self.baseline.revision_id, "fragment-set-op-4", self.command),
            self.executor,
        )
        pasted = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(self.base_project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(pasted), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unexpected history kind")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "fragment-set-undo",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        after_undo = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(DEST_A, after_undo["shapes"])
        self.assertNotIn(DEST_B, after_undo["shapes"])

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "fragment-set-redo",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        after_redo = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(pasted, after_redo)
        self.assertEqual({DEST_A, DEST_B}, {node for node in (DEST_A, DEST_B) if node in after_redo["shapes"]})


if __name__ == "__main__":
    unittest.main()
