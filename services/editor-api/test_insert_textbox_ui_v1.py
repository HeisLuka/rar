#!/usr/bin/env python3
import copy
import unittest

from canvas_box_draw_v1 import PointEmu
from canvas_tool_state_v1 import SELECT_TOOL_V1, TEXTBOX_CREATE_TOOL_V1, default_canvas_tool_state_v1
from create_textbox_v1 import apply_create_textbox_v1
from editor_tool_routing_v1 import build_editor_tool_routing_state_v1, pointer_owner_v1
from group_member_selection_v1 import empty_top_level_scope_v1
from insert_textbox_ui_v1 import (
    InsertTextBoxUiError,
    activate_textbox_tool_v1,
    textbox_cancel_v1,
    textbox_commit_accepted_v1,
    textbox_pointer_down_v1,
    textbox_pointer_move_v1,
    textbox_pointer_up_v1,
)
from revision_store import RevisionKernel


DOCUMENT_ID = "doc:insert-textbox"
SOURCE_HASH = "cd" * 32
NODE_ID = "01900000-0000-7000-8000-000000001001"
STORY_ID = "01900000-0000-7000-8000-000000001002"
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


def baseline_project():
    return {
        "schema_version": "pub-editor-v0.6",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": "ef" * 32,
        "operations": [],
        "pages": {"page:1": {"authoring_enabled": True, "children": []}},
        "shapes": {},
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
        "stories": {},
        "story_models": {},
        "text_presets": {},
    }


def routing_state():
    return build_editor_tool_routing_state_v1(
        canvas=default_canvas_tool_state_v1(),
        selection_scope=empty_top_level_scope_v1(),
    )


def activate():
    result = activate_textbox_tool_v1(routing_state())
    if result.action == "composition_resolution_required":
        raise AssertionError("unexpected composition blocker")
    return result.session


def release_request(*, kernel=None, token="textbox-draw-1"):
    session = activate()
    down = textbox_pointer_down_v1(
        session,
        page_id="page:1",
        point=PointEmu(100, 100),
        gesture_token=token,
    )
    base = "rev:base"
    if kernel is not None:
        base = kernel.current_revision(DOCUMENT_ID).revision_id
    release = textbox_pointer_up_v1(
        down.session,
        point=PointEmu(20, 40),
        document_id=DOCUMENT_ID,
        source_hash=SOURCE_HASH,
        base_revision_id=base,
        client_operation_id=f"create-{token}",
        node_id=NODE_ID,
        story_id=STORY_ID,
        text_preset=copy.deepcopy(PRESET),
    )
    return release


class InsertTextBoxUiV1Tests(unittest.TestCase):
    def test_pointer_motion_is_transient_and_release_builds_one_empty_create_request(self):
        session = activate()
        self.assertEqual(TEXTBOX_CREATE_TOOL_V1, session.routing_state.canvas.active_tool)
        self.assertEqual("canvas_tool", pointer_owner_v1(session.routing_state))

        down = textbox_pointer_down_v1(
            session,
            page_id="page:1",
            point=PointEmu(100, 100),
            gesture_token="textbox-preview",
        )
        self.assertEqual("canvas_gesture", pointer_owner_v1(down.session.routing_state))
        moved = textbox_pointer_move_v1(down.session, point=PointEmu(20, 40))
        self.assertEqual(
            {"x": 20, "y": 40, "width": 80, "height": 60},
            moved.preview_bounds,
        )
        self.assertIsNone(moved.create_textbox_request)

        release = textbox_pointer_up_v1(
            moved.session,
            point=PointEmu(20, 40),
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            base_revision_id="rev:base",
            client_operation_id="textbox-preview-create",
            node_id=NODE_ID,
            story_id=STORY_ID,
            text_preset=copy.deepcopy(PRESET),
        )
        self.assertEqual("submit_create_textbox", release.action)
        command = release.create_textbox_request["command"]
        self.assertEqual(NODE_ID, command["node_id"])
        self.assertEqual(STORY_ID, command["story_id"])
        self.assertEqual("page:1", command["page_id"])
        self.assertEqual(
            {"x": 20, "y": 40, "width": 80, "height": 60},
            command["bounds"],
        )
        self.assertIsNone(command["initial_text"])
        self.assertIsNone(release.session.gesture_token)
        self.assertEqual(TEXTBOX_CREATE_TOOL_V1, release.session.routing_state.canvas.active_tool)

    def test_zero_size_and_cancel_commit_nothing_and_return_to_select(self):
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=baseline_project(),
        )

        session = activate()
        down = textbox_pointer_down_v1(
            session,
            page_id="page:1",
            point=PointEmu(10, 10),
            gesture_token="textbox-zero",
        )
        zero = textbox_pointer_up_v1(
            down.session,
            point=PointEmu(10, 20),
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,
            client_operation_id="textbox-zero-create",
            node_id=NODE_ID,
            story_id=STORY_ID,
            text_preset=copy.deepcopy(PRESET),
        )
        self.assertEqual("no_change", zero.action)
        self.assertIsNone(zero.create_textbox_request)
        self.assertEqual(SELECT_TOOL_V1, zero.session.routing_state.canvas.active_tool)
        self.assertEqual(baseline.revision_id, kernel.current_revision(DOCUMENT_ID).revision_id)

        session2 = activate()
        down2 = textbox_pointer_down_v1(
            session2,
            page_id="page:1",
            point=PointEmu(1, 1),
            gesture_token="textbox-cancel",
        )
        cancelled = textbox_cancel_v1(down2.session)
        self.assertEqual("cancelled", cancelled.action)
        self.assertEqual(SELECT_TOOL_V1, cancelled.session.routing_state.canvas.active_tool)
        self.assertEqual(baseline.revision_id, kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_accepted_create_enters_same_story_at_zero_then_types_via_canonical_story_transaction(self):
        kernel = RevisionKernel()
        kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=baseline_project(),
        )
        release = release_request(kernel=kernel, token="textbox-accepted")
        request = release.create_textbox_request
        accepted = kernel.commit_create_textbox(request, apply_create_textbox_v1)

        self.assertEqual("chaptera.commit-accepted.v1", accepted["protocol_version"])
        self.assertEqual("create_textbox", accepted["canonical_operation"]["kind"])
        current = kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(1, len(current.project["operations"]))
        self.assertEqual("", current.project["stories"][STORY_ID])

        handed = textbox_commit_accepted_v1(
            release.session,
            create_textbox_request=request,
            accepted_commit=accepted,
            current_project=current.project,
            text_session_id="text-session:new",
            layout_revision_id="layout:pending:new-textbox",
        )
        self.assertEqual("textbox_created_to_text_session", handed.action)
        self.assertEqual(SELECT_TOOL_V1, handed.session.routing_state.canvas.active_tool)
        self.assertEqual("story_text", handed.session.routing_state.focus_owner)
        self.assertEqual("story_text", pointer_owner_v1(handed.session.routing_state))
        self.assertIs(handed.text_session, handed.session.routing_state.active_text_session)
        self.assertEqual(STORY_ID, handed.text_session.story_id)
        self.assertEqual(NODE_ID, handed.text_session.current_frame_id)
        self.assertEqual(0, handed.text_session.selection.focus_scalar)
        self.assertEqual("layout_pending", handed.text_session.selection.projection_state)

        typed = kernel.commit_story_edit_transaction(
            {
                "protocol_version": "chaptera.story-edit-transaction-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "textbox-type-hello",
                "command": {
                    "kind": "story_edit_transaction",
                    "story_id": STORY_ID,
                    "start_scalar": 0,
                    "end_scalar": 0,
                    "expected_before": "",
                    "replacement_text": "Hello",
                    "paragraph_inserted_ids": [],
                    "paragraph_inserted_property_presets": [],
                    "typing_format": None,
                    "fragment_format_runs": [],
                    "incoming_semantic_kinds": [],
                },
            }
        )
        self.assertEqual("chaptera.commit-accepted.v1", typed["protocol_version"])
        self.assertEqual(
            "Hello",
            kernel.current_revision(DOCUMENT_ID).project["stories"][STORY_ID],
        )
        self.assertEqual(2, len(kernel.current_revision(DOCUMENT_ID).project["operations"]))

    def test_accepted_handoff_rejects_mismatched_durable_receipt(self):
        kernel = RevisionKernel()
        kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=baseline_project(),
        )
        release = release_request(kernel=kernel, token="textbox-mismatch")
        accepted = kernel.commit_create_textbox(
            release.create_textbox_request,
            apply_create_textbox_v1,
        )
        forged = copy.deepcopy(accepted)
        forged["canonical_operation"]["story_id"] = "01900000-0000-7000-8000-000000009999"
        with self.assertRaisesRegex(InsertTextBoxUiError, "story_id differs"):
            textbox_commit_accepted_v1(
                release.session,
                create_textbox_request=release.create_textbox_request,
                accepted_commit=forged,
                current_project=kernel.current_revision(DOCUMENT_ID).project,
                text_session_id="text-session:forged",
                layout_revision_id="layout:pending:forged",
            )

    def test_undo_redo_preserves_atomic_frame_story_identity_after_ui_create(self):
        project = baseline_project()
        kernel = RevisionKernel()
        kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        release = release_request(kernel=kernel, token="textbox-history")
        accepted = kernel.commit_create_textbox(
            release.create_textbox_request,
            apply_create_textbox_v1,
        )
        accepted_project = copy.deepcopy(kernel.current_revision(DOCUMENT_ID).project)

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(project), []
            if kind == "redo":
                return copy.deepcopy(accepted_project), []
            raise ValueError(kind)

        undo = kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "textbox-ui-undo",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        after_undo = kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(NODE_ID, after_undo["text_frames"])
        self.assertNotIn(STORY_ID, after_undo["stories"])

        kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "textbox-ui-redo",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        after_redo = kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(STORY_ID, after_redo["text_frames"][NODE_ID]["story_id"])
        self.assertEqual("", after_redo["stories"][STORY_ID])


if __name__ == "__main__":
    unittest.main()
