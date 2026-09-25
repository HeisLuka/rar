#!/usr/bin/env python3
import copy
import json
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
EDITOR_API = ROOT / "services" / "editor-api"
for path in (str(TOOLS), str(EDITOR_API)):
    if path not in sys.path:
        sys.path.insert(0, path)

from editor_fixed_output_current_state_v1 import (
    EditorFixedOutputError,
    build_current_fixed_output,
)
from story_range_v1 import replace_story_range_v1

SOURCE_HASH = "a" * 64
PAGE_ID = "20000000-0000-4000-8000-000000000001"
NODE_ID = "10000000-0000-4000-8000-000000000001"
STORY_ID = "30000000-0000-4000-8000-000000000001"
BEFORE = {"x": 1000, "y": 2000, "width": 3000, "height": 4000}
AFTER = {"x": 128000, "y": 256000, "width": 3000, "height": 4000}


def graph():
    return {
        "document": {
            "source_hash": SOURCE_HASH,
            "pages": [PAGE_ID],
        },
        "pages": {
            PAGE_ID: {
                "id": PAGE_ID,
                "size": {"width": 914400, "height": 1828800},
                "bleed": None,
                "margins": None,
                "children": [NODE_ID],
                "source_refs": [{"private": "must-not-cross"}],
            }
        },
        "nodes": {
            NODE_ID: {
                "header": {
                    "id": NODE_ID,
                    "parent_id": PAGE_ID,
                    "bounds": dict(BEFORE),
                    "transform": {
                        "a": "1",
                        "b": "0",
                        "c": "0",
                        "d": "1",
                        "tx": 0,
                        "ty": 0,
                    },
                    "source_refs": [{"carrier": "Escher"}],
                },
                "payload": {
                    "story_frame": {
                        "story_id": STORY_ID,
                        "ordinal": 0,
                        "previous_frame": None,
                        "next_frame": None,
                    }
                },
            }
        },
        "stories": {
            STORY_ID: {
                "id": STORY_ID,
                "text": "AB",
                "source_refs": [{"carrier": "Quill"}],
            }
        },
    }


def move_operation():
    return {
        "kind": "move_node",
        "node_id": NODE_ID,
        "before": dict(BEFORE),
        "after": dict(AFTER),
    }


def text_operation():
    return replace_story_range_v1(
        story_id=STORY_ID,
        story_text="AB",
        start_scalar=1,
        end_scalar=2,
        expected_before="B",
        replacement_text="C",
    ).operation


def project(operations):
    return {
        "schema_version": "pub-editor-v0.4" if operations else "pub-editor-v0.2",
        "source_hash": SOURCE_HASH,
        "operations": copy.deepcopy(operations),
    }


def glyph(glyph_id, cluster):
    return {
        "glyph_id": glyph_id,
        "cluster": cluster,
        "x_advance": 500,
        "y_advance": 0,
        "x_offset": 0,
        "y_offset": 0,
    }


def shaped_flow(text):
    return {
        "schema_version": "chaptera.shaped-flow-bridge-input.v1",
        "source_hash": SOURCE_HASH,
        "flow_id": "sha256:" + "d" * 64,
        "environment": {
            "font_size_emu": 1000,
            "line_height_emu": 1400,
        },
        "lines": [{
            "frame_node_id": NODE_ID,
            "story_id": STORY_ID,
            "frame_line_index": 0,
            "scalar_start": 0,
            "scalar_end": 2,
            "consumed_scalar_end": 2,
            "text": text,
            "units_per_em": 1000,
            "measured_width": 1000,
            "glyphs": [glyph(11, 0), glyph(12, 1)],
        }],
        "diagnostics": [],
    }


def build(p, flow):
    return build_current_fixed_output(
        baseline_graph=graph(),
        editor_project=p,
        projection_context=None,
        shaped_flow=flow,
        implementation="rar-editor-fixed-output-current-state-v1",
        commit_or_build="deadbeef",
    )


class EditorFixedOutputCurrentStateTests(unittest.TestCase):
    def test_move_and_story_edit_feed_one_current_output_packet(self):
        packet, receipt = build(
            project([move_operation(), text_operation()]),
            shaped_flow("AC"),
        )
        node = packet["scene"]["nodes"][0]
        self.assertEqual(AFTER, node["bounds"])
        self.assertEqual("AC", packet["fixed_text_runs"][0]["logical_text"])
        self.assertEqual(0, packet["fixed_text_runs"][0]["scalar_base"])
        self.assertEqual(1, receipt["fixed_run_count"])
        self.assertTrue(receipt["invariants"]["current_editor_project_authoritative"])
        self.assertTrue(receipt["invariants"]["rust_fixed_flow_adapter_authoritative"])
        self.assertEqual(
            0,
            receipt["invariants"]["source_reparse_after_edit_count"],
        )

    def test_receipt_is_public_safe_but_packet_remains_renderer_input(self):
        packet, receipt = build(
            project([move_operation(), text_operation()]),
            shaped_flow("AC"),
        )
        self.assertEqual("AC", packet["fixed_text_runs"][0]["logical_text"])
        encoded = json.dumps(receipt, sort_keys=True)
        self.assertNotIn("AC", encoded)
        self.assertNotIn("logical_text", encoded)
        self.assertNotIn("expected_before", encoded)
        self.assertNotIn("replacement_text", encoded)
        self.assertNotIn("source_refs", encoded)

    def test_stale_shaped_flow_is_rejected_after_story_edit(self):
        with self.assertRaisesRegex(EditorFixedOutputError, "stale"):
            build(
                project([text_operation()]),
                shaped_flow("AB"),
            )

    def test_baseline_undo_and_redo_replay_are_state_deterministic(self):
        baseline_packet, baseline_receipt = build(project([]), shaped_flow("AB"))
        edited_project = project([move_operation(), text_operation()])
        edited_packet, edited_receipt = build(edited_project, shaped_flow("AC"))
        replay_packet, replay_receipt = build(
            copy.deepcopy(edited_project),
            copy.deepcopy(shaped_flow("AC")),
        )
        undone_packet, undone_receipt = build(project([]), shaped_flow("AB"))

        self.assertEqual(BEFORE, baseline_packet["scene"]["nodes"][0]["bounds"])
        self.assertEqual(AFTER, edited_packet["scene"]["nodes"][0]["bounds"])
        self.assertNotEqual(
            baseline_receipt["scene_snapshot_id"],
            edited_receipt["scene_snapshot_id"],
        )
        self.assertEqual(
            edited_receipt["packet_id"],
            replay_receipt["packet_id"],
        )
        self.assertEqual(
            edited_receipt["current_story_states"],
            replay_receipt["current_story_states"],
        )
        self.assertEqual(
            baseline_receipt["packet_id"],
            undone_receipt["packet_id"],
        )
        self.assertEqual(
            baseline_packet["fixed_text_runs"],
            undone_packet["fixed_text_runs"],
        )

    def test_source_identity_mismatch_fails_closed(self):
        bad = project([])
        bad["source_hash"] = "b" * 64
        with self.assertRaisesRegex(EditorFixedOutputError, "source identity"):
            build(bad, shaped_flow("AB"))

    def test_unknown_operation_cannot_enter_fixed_output_v1(self):
        bad = project([{"kind": "delete_node", "node_id": NODE_ID}])
        with self.assertRaisesRegex(EditorFixedOutputError, "outside current fixed-output"):
            build(bad, shaped_flow("AB"))

    def test_shaped_flow_frame_must_exist_in_current_scene(self):
        flow = shaped_flow("AB")
        flow["lines"][0]["frame_node_id"] = "40000000-0000-4000-8000-000000000001"
        with self.assertRaisesRegex(EditorFixedOutputError, "absent from current Scene"):
            build(project([]), flow)

    def test_story_state_receipt_changes_after_edit_without_text_leak(self):
        _, baseline = build(project([]), shaped_flow("AB"))
        _, edited = build(project([text_operation()]), shaped_flow("AC"))
        self.assertNotEqual(
            baseline["current_story_states"][0]["story_state_id"],
            edited["current_story_states"][0]["story_state_id"],
        )
        self.assertEqual(
            baseline["current_story_states"][0]["scalar_count"],
            edited["current_story_states"][0]["scalar_count"],
        )


if __name__ == "__main__":
    unittest.main()
