#!/usr/bin/env python3
import copy
import unittest

from canvas_box_draw_v1 import PointEmu
from canvas_tool_state_v1 import RECTANGLE_CREATE_TOOL_V1, SELECT_TOOL_V1, default_canvas_tool_state_v1
from create_shape_v1 import apply_create_shape_v1
from insert_shape_ui_v1 import (
    activate_rectangle_tool_v1,
    rectangle_cancel_v1,
    rectangle_commit_accepted_v1,
    rectangle_pointer_down_v1,
    rectangle_pointer_move_v1,
    rectangle_pointer_up_v1,
)
from revision_store import RevisionKernel

DOCUMENT_ID = "doc:insert-shape"
SOURCE_HASH = "cd" * 32
NODE_ID = "01890f47-0c00-7abc-8def-0123456789ab"
PAINT = {
    "fill": {"visible": True, "color": {"r": 10, "g": 20, "b": 30}},
    "stroke": {"visible": True, "color": {"r": 40, "g": 50, "b": 60}, "width_emu": 12700},
}


def baseline_project():
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": "ef" * 32,
        "operations": [],
        "pages": {"page:1": {"authoring_enabled": True, "children": []}},
        "shapes": {},
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
    }


class InsertShapeUiV1Tests(unittest.TestCase):
    def test_pointer_motion_is_preview_only_and_release_builds_exactly_one_request(self):
        session = activate_rectangle_tool_v1(default_canvas_tool_state_v1())
        self.assertEqual(RECTANGLE_CREATE_TOOL_V1, session.tool_state.active_tool)

        down = rectangle_pointer_down_v1(
            session,
            page_id="page:1",
            point=PointEmu(100, 100),
            gesture_token="draw-1",
        )
        moved = rectangle_pointer_move_v1(down.session, point=PointEmu(20, 40))
        self.assertEqual({"x": 20, "y": 40, "width": 80, "height": 60}, moved.preview_bounds)
        self.assertIsNone(moved.create_shape_request)

        release = rectangle_pointer_up_v1(
            moved.session,
            point=PointEmu(20, 40),
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            base_revision_id="rev:base",
            client_operation_id="insert-shape-1",
            node_id=NODE_ID,
            paint=copy.deepcopy(PAINT),
        )
        self.assertEqual("submit_create_shape", release.action)
        self.assertEqual(
            {"x": 20, "y": 40, "width": 80, "height": 60},
            release.create_shape_request["command"]["bounds"],
        )
        self.assertEqual(NODE_ID, release.create_shape_request["command"]["node_id"])
        self.assertIsNone(release.session.gesture_token)
        self.assertIsNone(release.session.draw)

    def test_zero_size_and_cancel_submit_nothing(self):
        session = activate_rectangle_tool_v1(default_canvas_tool_state_v1())
        down = rectangle_pointer_down_v1(
            session,
            page_id="page:1",
            point=PointEmu(10, 10),
            gesture_token="draw-zero",
        )
        zero = rectangle_pointer_up_v1(
            down.session,
            point=PointEmu(10, 20),
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            base_revision_id="rev:base",
            client_operation_id="insert-zero",
            node_id=NODE_ID,
            paint=copy.deepcopy(PAINT),
        )
        self.assertEqual("no_change", zero.action)
        self.assertIsNone(zero.create_shape_request)

        down2 = rectangle_pointer_down_v1(
            session,
            page_id="page:1",
            point=PointEmu(1, 1),
            gesture_token="draw-cancel",
        )
        cancelled = rectangle_cancel_v1(down2.session)
        self.assertEqual("cancelled", cancelled.action)
        self.assertIsNone(cancelled.create_shape_request)

    def test_accept_selects_only_after_commit_and_returns_tool_to_select(self):
        project = baseline_project()
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        session = activate_rectangle_tool_v1(default_canvas_tool_state_v1())
        down = rectangle_pointer_down_v1(
            session,
            page_id="page:1",
            point=PointEmu(0, 0),
            gesture_token="draw-accept",
        )
        release = rectangle_pointer_up_v1(
            down.session,
            point=PointEmu(100, 50),
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,
            client_operation_id="insert-shape-accepted",
            node_id=NODE_ID,
            paint=copy.deepcopy(PAINT),
        )
        self.assertIsNone(release.selected_node_id)

        accepted = kernel.commit_create_shape(release.create_shape_request, apply_create_shape_v1)
        self.assertEqual(NODE_ID, accepted["canonical_operation"]["node_id"])
        post = rectangle_commit_accepted_v1(release.session, accepted_node_id=NODE_ID)
        self.assertEqual(NODE_ID, post.selected_node_id)
        self.assertEqual(SELECT_TOOL_V1, post.session.tool_state.active_tool)

    def test_undo_redo_restores_same_identity(self):
        project = baseline_project()
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(document_id=DOCUMENT_ID, source_hash=SOURCE_HASH, project=project)
        session = activate_rectangle_tool_v1(default_canvas_tool_state_v1())
        down = rectangle_pointer_down_v1(session, page_id="page:1", point=PointEmu(0, 0), gesture_token="draw-history")
        release = rectangle_pointer_up_v1(
            down.session,
            point=PointEmu(100, 50),
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,
            client_operation_id="insert-history",
            node_id=NODE_ID,
            paint=copy.deepcopy(PAINT),
        )
        accepted = kernel.commit_create_shape(release.create_shape_request, apply_create_shape_v1)
        accepted_project = copy.deepcopy(kernel.current_revision(DOCUMENT_ID).project)

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(project), []
            if kind == "redo":
                return copy.deepcopy(accepted_project), []
            raise ValueError(kind)

        undo = kernel.commit_history_transition({
            "protocol_version":"chaptera.history-transition-intent.v1",
            "document_id":DOCUMENT_ID,
            "source_hash":SOURCE_HASH,
            "base_revision_id":accepted["revision_id"],
            "client_operation_id":"insert-undo",
            "command":{"kind":"undo"},
        }, history_executor)
        self.assertNotIn(NODE_ID, kernel.current_revision(DOCUMENT_ID).project["shapes"])

        kernel.commit_history_transition({
            "protocol_version":"chaptera.history-transition-intent.v1",
            "document_id":DOCUMENT_ID,
            "source_hash":SOURCE_HASH,
            "base_revision_id":undo["revision_id"],
            "client_operation_id":"insert-redo",
            "command":{"kind":"redo"},
        }, history_executor)
        self.assertEqual(NODE_ID, kernel.current_revision(DOCUMENT_ID).project["shapes"][NODE_ID]["node_id"])


if __name__ == "__main__":
    unittest.main()
