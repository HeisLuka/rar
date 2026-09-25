#!/usr/bin/env python3
import copy
import json
import unittest

from create_textbox_v1 import (
    CreateTextBoxError,
    apply_create_textbox_v1,
)
from revision_store import RevisionKernel, canonical_json, project_hash


DOCUMENT_ID = "doc:create-textbox"
SOURCE_HASH = "ab" * 32
SOURCE_BLOB_SHA256 = "cd" * 32
NODE_ID = "01890f47-0c00-7abc-8def-0123456789ab"
STORY_ID = "01890f47-0c01-7abc-8def-0123456789ab"
SECOND_NODE_ID = "01890f47-0c02-7abc-8def-0123456789ab"
SECOND_STORY_ID = "01890f47-0c03-7abc-8def-0123456789ab"
BOUNDS = {"x": 100, "y": 200, "width": 3_000_000, "height": 1_000_000}
PRESET = {
    "preset_version": "chaptera.authoring-text-preset.v1",
    "font_fingerprint": "12" * 32,
    "face_index": 0,
    "font_size_emu": 152_400,
    "paragraph_defaults": {
        "alignment": "left",
        "space_before_emu": 0,
        "space_after_emu": 0,
    },
    "character_defaults": {
        "bold": False,
        "italic": False,
    },
}


class CreateTextBoxV1Tests(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema_version": "pub-editor-v0.6",
            "source_hash": SOURCE_HASH,
            "immutable_source_blob_sha256": SOURCE_BLOB_SHA256,
            "operations": [],
            "pages": {
                "page:1": {
                    "authoring_enabled": True,
                    "children": ["source:existing"],
                }
            },
            "shapes": {},
            "text_frames": {},
            "picture_frames": {},
            "groups": {},
            "stories": {},
            "story_models": {},
            "text_presets": {},
        }
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )

    def request(
        self,
        op_id,
        *,
        node_id=NODE_ID,
        story_id=STORY_ID,
        page_id="page:1",
        initial_text=None,
        preset=None,
        base=None,
    ):
        return {
            "protocol_version": "chaptera.create-textbox-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "create_textbox",
                "node_id": node_id,
                "story_id": story_id,
                "page_id": page_id,
                "bounds": copy.deepcopy(BOUNDS),
                "text_preset": copy.deepcopy(preset or PRESET),
                "initial_text": initial_text,
            },
        }

    def test_empty_create_is_one_atomic_frame_story_page_edge_and_preset(self):
        result = self.kernel.commit_create_textbox(
            self.request("create-textbox-0001"),
            apply_create_textbox_v1,
        )
        project = self.kernel.current_revision(DOCUMENT_ID).project
        frame = project["text_frames"][NODE_ID]
        self.assertEqual("text_frame", frame["kind"])
        self.assertEqual(STORY_ID, frame["story_id"])
        self.assertEqual("page:1", frame["page_id"])
        self.assertEqual({"kind": "identity"}, frame["transform"])
        self.assertEqual({"kind": "author_created"}, frame["provenance"])

        self.assertEqual("", project["stories"][STORY_ID])
        model = project["story_models"][STORY_ID]
        self.assertEqual("chaptera_created", model["provenance"])
        self.assertEqual("", model["paragraph_state"]["story_text"])
        self.assertFalse(model["paragraph_state"]["protected_terminal_cr"])
        self.assertEqual(1, len(model["paragraph_state"]["paragraphs"]))
        self.assertIsNotNone(model["empty_story_preset_format"])
        self.assertFalse(project["stories"][STORY_ID].endswith("\r"))

        preset_id = frame["text_preset_id"]
        self.assertEqual(PRESET, project["text_presets"][preset_id]["preset"])
        self.assertEqual(
            ["source:existing", NODE_ID],
            project["pages"]["page:1"]["children"],
        )
        self.assertEqual("create_textbox", result["canonical_operation"]["kind"])
        self.assertEqual(1, len(project["operations"]))

    def test_initial_external_text_is_normalized_once_without_terminal_cr(self):
        self.kernel.commit_create_textbox(
            self.request("create-textbox-0002", initial_text="A\r\nB\nC"),
            apply_create_textbox_v1,
        )
        project = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual("A\rB\rC", project["stories"][STORY_ID])
        self.assertFalse(project["stories"][STORY_ID].endswith("\r"))
        model = project["story_models"][STORY_ID]
        self.assertEqual(3, len(model["paragraph_state"]["paragraphs"]))
        self.assertEqual(5, model["format_state"]["story_scalar_len"])
        self.assertEqual(
            [{"start_scalar": 0, "end_scalar": 5, "format": model["format_state"]["base_runs"][0]["format"]}],
            model["format_state"]["base_runs"],
        )

    def test_exact_retry_is_idempotent_and_stale_second_create_rejects(self):
        request = self.request("create-textbox-0003")
        first = self.kernel.commit_create_textbox(
            copy.deepcopy(request),
            apply_create_textbox_v1,
        )
        second = self.kernel.commit_create_textbox(
            copy.deepcopy(request),
            apply_create_textbox_v1,
        )
        self.assertEqual(first, second)
        self.assertEqual(
            1,
            len(self.kernel.current_revision(DOCUMENT_ID).project["operations"]),
        )

        stale = self.kernel.commit_create_textbox(
            self.request(
                "create-textbox-0004",
                node_id=SECOND_NODE_ID,
                story_id=SECOND_STORY_ID,
                base=self.baseline.revision_id,
            ),
            apply_create_textbox_v1,
        )
        self.assertEqual("stale_revision", stale["code"])
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(SECOND_NODE_ID, current["text_frames"])
        self.assertNotIn(SECOND_STORY_ID, current["stories"])

    def test_collision_or_invalid_page_rolls_back_whole_transaction(self):
        collision = copy.deepcopy(self.project)
        collision["stories"][STORY_ID] = "existing"
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:textbox-collision",
            source_hash=SOURCE_HASH,
            project=collision,
        )
        request = self.request("create-textbox-collision")
        request["document_id"] = "doc:textbox-collision"
        request["base_revision_id"] = baseline.revision_id
        with self.assertRaisesRegex(CreateTextBoxError, "story_id_collision"):
            kernel.commit_create_textbox(request, apply_create_textbox_v1)
        current = kernel.current_revision("doc:textbox-collision")
        self.assertEqual(baseline.revision_id, current.revision_id)
        self.assertNotIn(NODE_ID, current.project["text_frames"])
        self.assertEqual(["source:existing"], current.project["pages"]["page:1"]["children"])

        with self.assertRaisesRegex(CreateTextBoxError, "invalid_create_textbox_page"):
            self.kernel.commit_create_textbox(
                self.request("create-textbox-bad-page", page_id="page:missing"),
                apply_create_textbox_v1,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_browser_cannot_supply_canonical_or_native_publisher_fields(self):
        for field, value in (
            ("parent_id", "page:forged"),
            ("transform", {"kind": "rotate"}),
            ("provenance", {"kind": "source_backed"}),
            ("publisher_story_id", 12),
            ("syid", 99),
            ("terminal_cr", True),
        ):
            request = self.request(f"create-textbox-extra-{field}")
            request["command"][field] = value
            with self.subTest(field=field):
                with self.assertRaisesRegex(CreateTextBoxError, "non-intent"):
                    self.kernel.commit_create_textbox(request, apply_create_textbox_v1)

    def test_cross_registry_graph_collisions_fail_before_mutation(self):
        same_id = self.request("create-textbox-same-id")
        same_id["command"]["story_id"] = same_id["command"]["node_id"]
        with self.assertRaisesRegex(CreateTextBoxError, "must be distinct"):
            self.kernel.commit_create_textbox(same_id, apply_create_textbox_v1)

        child_collision = copy.deepcopy(self.project)
        child_collision["pages"]["page:1"]["children"].append(NODE_ID)
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:textbox-child-collision",
            source_hash=SOURCE_HASH,
            project=child_collision,
        )
        request = self.request("create-textbox-child-collision")
        request["document_id"] = "doc:textbox-child-collision"
        request["base_revision_id"] = baseline.revision_id
        with self.assertRaisesRegex(CreateTextBoxError, "node_id_collision"):
            kernel.commit_create_textbox(request, apply_create_textbox_v1)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:textbox-child-collision").revision_id,
        )

        story_ref_collision = copy.deepcopy(self.project)
        story_ref_collision["text_frames"]["frame:orphan"] = {
            "kind": "text_frame",
            "story_id": STORY_ID,
        }
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:textbox-story-ref-collision",
            source_hash=SOURCE_HASH,
            project=story_ref_collision,
        )
        request = self.request("create-textbox-story-ref-collision")
        request["document_id"] = "doc:textbox-story-ref-collision"
        request["base_revision_id"] = baseline.revision_id
        with self.assertRaisesRegex(CreateTextBoxError, "story_id_collision"):
            kernel.commit_create_textbox(request, apply_create_textbox_v1)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:textbox-story-ref-collision").revision_id,
        )

        preset_collision = copy.deepcopy(self.project)
        from create_textbox_v1 import _preset_id

        preset_id = _preset_id(PRESET)
        preset_collision["text_presets"][preset_id] = {
            "preset_id": preset_id,
            "preset": {"forged": True},
        }
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:textbox-preset-collision",
            source_hash=SOURCE_HASH,
            project=preset_collision,
        )
        request = self.request("create-textbox-preset-collision")
        request["document_id"] = "doc:textbox-preset-collision"
        request["base_revision_id"] = baseline.revision_id
        with self.assertRaisesRegex(CreateTextBoxError, "preset_id_collision"):
            kernel.commit_create_textbox(request, apply_create_textbox_v1)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:textbox-preset-collision").revision_id,
        )

    def test_invalid_preset_or_uuid_fails_before_revision_advance(self):
        bad = copy.deepcopy(PRESET)
        bad["font_size_emu"] = 0
        with self.assertRaisesRegex(CreateTextBoxError, "positive safe EMU"):
            self.kernel.commit_create_textbox(
                self.request("create-textbox-bad-preset", preset=bad),
                apply_create_textbox_v1,
            )

        request = self.request("create-textbox-bad-story")
        request["command"]["story_id"] = "not-a-uuid"
        with self.assertRaises(ValueError):
            self.kernel.commit_create_textbox(request, apply_create_textbox_v1)
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_executor_cannot_return_canonical_operation_with_forged_project(self):
        def forged(base_project, command):
            operation, project, consequences = apply_create_textbox_v1(
                base_project, command
            )
            project["stories"][command["story_id"]] = "forged"
            return operation, project, consequences

        with self.assertRaisesRegex(ValueError, "canonical Story text"):
            self.kernel.commit_create_textbox(
                self.request("create-textbox-forged-project"),
                forged,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )
        self.assertNotIn(
            NODE_ID,
            self.kernel.current_revision(DOCUMENT_ID).project["text_frames"],
        )

    def test_undo_redo_and_save_reopen_preserve_same_ids_and_state(self):
        accepted = self.kernel.commit_create_textbox(
            self.request("create-textbox-0005", initial_text="hello"),
            apply_create_textbox_v1,
        )
        accepted_project = copy.deepcopy(
            self.kernel.current_revision(DOCUMENT_ID).project
        )

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
                "client_operation_id": "textbox-undo",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertNotIn(
            NODE_ID,
            self.kernel.current_revision(DOCUMENT_ID).project["text_frames"],
        )
        self.assertNotIn(
            STORY_ID,
            self.kernel.current_revision(DOCUMENT_ID).project["stories"],
        )

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "textbox-redo",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(
            accepted_project,
            self.kernel.current_revision(DOCUMENT_ID).project,
        )

        saved = json.loads(canonical_json(accepted_project).decode("utf-8"))
        reopened = RevisionKernel()
        reopened_baseline = reopened.register_baseline(
            document_id="doc:create-textbox-reopened",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(accepted_project), reopened_baseline.project_hash)
        self.assertEqual(NODE_ID, reopened_baseline.project["text_frames"][NODE_ID]["node_id"])
        self.assertEqual(STORY_ID, reopened_baseline.project["text_frames"][NODE_ID]["story_id"])


if __name__ == "__main__":
    unittest.main()
