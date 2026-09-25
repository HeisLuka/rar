#!/usr/bin/env python3
import copy
import unittest

from create_textbox_v1 import apply_create_textbox_v1
from delete_textbox_v1 import DeleteTextBoxError, apply_delete_textbox_v1
from revision_store import RevisionKernel


DOCUMENT_ID = "doc:delete-textbox"
SOURCE_HASH = "ab" * 32
NODE_ID = "01900000-0000-7000-8000-000000002001"
STORY_ID = "01900000-0000-7000-8000-000000002002"
SECOND_NODE_ID = "01900000-0000-7000-8000-000000002003"
PAGE_ID = "page:1"
PRESET = {
    "preset_version": "chaptera.authoring-text-preset.v1",
    "font_fingerprint": "34" * 32,
    "face_index": 0,
    "font_size_emu": 152_400,
    "paragraph_defaults": {
        "alignment": "left",
        "space_before_emu": 0,
        "space_after_emu": 0,
    },
    "character_defaults": {"bold": False, "italic": False},
}


def empty_project():
    return {
        "schema_version": "pub-editor-v0.6",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": "cd" * 32,
        "operations": [],
        "pages": {PAGE_ID: {"authoring_enabled": True, "children": []}},
        "shapes": {},
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
        "stories": {},
        "story_models": {},
        "text_presets": {},
    }


def create_request(base_revision_id: str):
    return {
        "protocol_version": "chaptera.create-textbox-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": "create-textbox-for-delete",
        "command": {
            "kind": "create_textbox",
            "node_id": NODE_ID,
            "story_id": STORY_ID,
            "page_id": PAGE_ID,
            "bounds": {
                "x": 100,
                "y": 200,
                "width": 800000,
                "height": 400000,
            },
            "text_preset": copy.deepcopy(PRESET),
            "initial_text": "Keep this exact Story",
        },
    }


def delete_request(project: dict, base_revision_id: str, op_id="delete-textbox-1"):
    frame = copy.deepcopy(project["text_frames"][NODE_ID])
    story = project["stories"][STORY_ID]
    model = copy.deepcopy(project["story_models"][STORY_ID])
    preset = copy.deepcopy(project["text_presets"][frame["text_preset_id"]])
    children = copy.deepcopy(project["pages"][PAGE_ID]["children"])
    return {
        "protocol_version": "chaptera.delete-textbox-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "delete_textbox",
            "node_id": NODE_ID,
            "story_id": STORY_ID,
            "expected_frame": frame,
            "expected_story_text": story,
            "expected_story_model": model,
            "expected_text_preset_record": preset,
            "expected_page_children": children,
            "expected_page_child_index": children.index(NODE_ID),
        },
    }


class DeleteTextBoxV1Tests(unittest.TestCase):
    def setUp(self):
        self.kernel = RevisionKernel()
        baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=empty_project(),
        )
        self.created = self.kernel.commit_create_textbox(
            create_request(baseline.revision_id),
            apply_create_textbox_v1,
        )
        self.created_project = copy.deepcopy(
            self.kernel.current_revision(DOCUMENT_ID).project
        )

    def test_delete_removes_exclusive_frame_story_model_and_page_edge_atomically(self):
        request = delete_request(self.created_project, self.created["revision_id"])
        accepted = self.kernel.commit_delete_textbox(request, apply_delete_textbox_v1)
        self.assertEqual("chaptera.commit-accepted.v1", accepted["protocol_version"])
        operation = accepted["canonical_operation"]
        self.assertEqual("delete_textbox", operation["kind"])
        self.assertEqual(NODE_ID, operation["node_id"])
        self.assertEqual(STORY_ID, operation["story_id"])
        self.assertEqual(0, operation["page_child_index"])
        self.assertEqual(
            self.created_project["text_frames"][NODE_ID],
            operation["deleted_frame"],
        )
        self.assertEqual(
            self.created_project["story_models"][STORY_ID],
            operation["deleted_story_model"],
        )

        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(NODE_ID, current["text_frames"])
        self.assertNotIn(STORY_ID, current["stories"])
        self.assertNotIn(STORY_ID, current["story_models"])
        self.assertNotIn(NODE_ID, current["pages"][PAGE_ID]["children"])
        preset_id = operation["text_preset_id"]
        self.assertEqual(
            operation["text_preset_record"],
            current["text_presets"][preset_id],
        )
        self.assertEqual(
            self.created_project["immutable_source_blob_sha256"],
            current["immutable_source_blob_sha256"],
        )

    def test_duplicate_frame_reference_to_story_rejects_before_mutation(self):
        project = copy.deepcopy(self.created_project)
        duplicate = copy.deepcopy(project["text_frames"][NODE_ID])
        duplicate["node_id"] = SECOND_NODE_ID
        project["text_frames"][SECOND_NODE_ID] = duplicate
        project["pages"][PAGE_ID]["children"].append(SECOND_NODE_ID)

        kernel = RevisionKernel()
        base = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        with self.assertRaisesRegex(
            DeleteTextBoxError,
            "story_not_exclusively_owned",
        ):
            kernel.commit_delete_textbox(
                delete_request(project, base.revision_id),
                apply_delete_textbox_v1,
            )
        self.assertEqual(base.project, kernel.current_revision(DOCUMENT_ID).project)

    def test_source_backed_frame_rejects(self):
        project = copy.deepcopy(self.created_project)
        project["text_frames"][NODE_ID]["provenance"] = {"kind": "source_backed"}
        kernel = RevisionKernel()
        base = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        with self.assertRaisesRegex(
            DeleteTextBoxError,
            "target_not_author_created",
        ):
            kernel.commit_delete_textbox(
                delete_request(project, base.revision_id),
                apply_delete_textbox_v1,
            )

    def test_stale_frame_precondition_rejects(self):
        request = delete_request(self.created_project, self.created["revision_id"])
        request["command"]["expected_frame"]["bounds"]["width"] += 1
        with self.assertRaisesRegex(DeleteTextBoxError, "stale_frame"):
            self.kernel.commit_delete_textbox(request, apply_delete_textbox_v1)
        self.assertEqual(
            self.created_project,
            self.kernel.current_revision(DOCUMENT_ID).project,
        )

    def test_stale_page_order_precondition_rejects(self):
        project = copy.deepcopy(self.created_project)
        project["pages"][PAGE_ID]["children"] = ["other", NODE_ID]
        kernel = RevisionKernel()
        base = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        request = delete_request(project, base.revision_id)
        request["command"]["expected_page_child_index"] = 0
        with self.assertRaisesRegex(
            DeleteTextBoxError,
            "expected page child position",
        ):
            kernel.commit_delete_textbox(request, apply_delete_textbox_v1)

    def test_replay_from_same_created_graph_is_deterministic(self):
        command = delete_request(
            self.created_project,
            self.created["revision_id"],
        )["command"]
        op1, project1, consequences1 = apply_delete_textbox_v1(
            self.created_project,
            copy.deepcopy(command),
        )
        op2, project2, consequences2 = apply_delete_textbox_v1(
            self.created_project,
            copy.deepcopy(command),
        )
        self.assertEqual(op1, op2)
        self.assertEqual(project1, project2)
        self.assertEqual(consequences1, consequences2)

    def test_undo_redo_restores_exact_frame_story_identity_and_order(self):
        request = delete_request(self.created_project, self.created["revision_id"])
        accepted = self.kernel.commit_delete_textbox(request, apply_delete_textbox_v1)
        deleted_project = copy.deepcopy(
            self.kernel.current_revision(DOCUMENT_ID).project
        )
        created_project = copy.deepcopy(self.created_project)

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(created_project), []
            if kind == "redo":
                return copy.deepcopy(deleted_project), []
            raise ValueError(kind)

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "delete-textbox-undo",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        restored = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(
            created_project["text_frames"][NODE_ID],
            restored["text_frames"][NODE_ID],
        )
        self.assertEqual(
            created_project["stories"][STORY_ID],
            restored["stories"][STORY_ID],
        )
        self.assertEqual(
            created_project["story_models"][STORY_ID],
            restored["story_models"][STORY_ID],
        )
        self.assertEqual(
            created_project["pages"][PAGE_ID]["children"],
            restored["pages"][PAGE_ID]["children"],
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "delete-textbox-redo",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            deleted_project,
            self.kernel.current_revision(DOCUMENT_ID).project,
        )


if __name__ == "__main__":
    unittest.main()
