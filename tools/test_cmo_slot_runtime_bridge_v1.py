#!/usr/bin/env python3
import json
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from cmo_slot_runtime_bridge_v1 import (
    CmoSlotRuntimeError,
    build_cmo_slot_runtime_v1,
    merge_cmo_runtime_into_scene_v1,
)
from resolved_graph_scene_bridge_v1 import project_resolved_graph_scene

SOURCE_HASH = "a" * 64
PAGE = "20000000-0000-4000-8000-000000000001"
CARRIER_PAGE = "20000000-0000-4000-8000-000000000002"
FRAME = "10000000-0000-4000-8000-000000000001"
CARRIER_A = "10000000-0000-4000-8000-000000000002"
CARRIER_B = "10000000-0000-4000-8000-000000000003"
STORY = "30000000-0000-4000-8000-000000000001"
CARRIER_STORY = "30000000-0000-4000-8000-000000000002"


def node(node_id, parent, bounds, payload=None):
    return {
        "header": {
            "id": node_id,
            "parent_id": parent,
            "bounds": dict(bounds),
            "transform": {
                "a": "1",
                "b": "0",
                "c": "0",
                "d": "1",
                "tx": 0,
                "ty": 0,
            },
            "source_refs": [{"private": "must-not-cross"}],
        },
        "payload": payload or {},
    }


def graph(text="\uFFFC"):
    return {
        "document": {
            "source_hash": SOURCE_HASH,
            "pages": [PAGE, CARRIER_PAGE],
        },
        "pages": {
            PAGE: {
                "id": PAGE,
                "size": {"width": 1000, "height": 1000},
                "bleed": None,
                "margins": None,
                "children": [FRAME],
            },
            CARRIER_PAGE: {
                "id": CARRIER_PAGE,
                "size": {"width": 1000, "height": 1000},
                "bleed": None,
                "margins": None,
                "children": [CARRIER_A, CARRIER_B],
            },
        },
        "nodes": {
            FRAME: node(
                FRAME,
                PAGE,
                {"x": 100, "y": 200, "width": 100, "height": 100},
                {
                    "story_frame": {
                        "story_id": STORY,
                        "ordinal": 0,
                        "previous_frame": None,
                        "next_frame": None,
                    }
                },
            ),
            CARRIER_A: node(
                CARRIER_A,
                CARRIER_PAGE,
                {"x": 0, "y": 0, "width": 80, "height": 60},
                {
                    "story_frame": {
                        "story_id": CARRIER_STORY,
                        "ordinal": 0,
                        "previous_frame": None,
                        "next_frame": None,
                    }
                },
            ),
            CARRIER_B: node(
                CARRIER_B,
                CARRIER_PAGE,
                {"x": 0, "y": 0, "width": 80, "height": 50},
            ),
        },
        "stories": {
            STORY: {
                "id": STORY,
                "text": text,
                "source_refs": [{"carrier": "Quill"}],
            },
            CARRIER_STORY: {
                "id": CARRIER_STORY,
                "text": "PRIVATE CARRIER TEXT",
                "source_refs": [{"carrier": "Quill"}],
            },
        },
    }


def relation(order, cmo_id, carrier, carrier_story=CARRIER_STORY):
    return {
        "source_order": order,
        "cmo_id": cmo_id,
        "carrier_ohpo": 300 + order,
        "carrier_cmo_id": cmo_id,
        "target_qsid": 49,
        "carrier_node_id": carrier,
        "carrier_story_id": carrier_story,
        "target_story_id": STORY,
        "target_frame_node_id": FRAME,
    }


def context(relations):
    return {
        "schema_version": "chaptera.pub-projection-context.v1",
        "master_relations": [],
        "cmo_relations": relations,
    }


def shaped_flow(lines=None):
    return {
        "schema_version": "chaptera.shaped-flow-bridge-input.v1",
        "source_hash": SOURCE_HASH,
        "flow_id": "sha256:" + "d" * 64,
        "environment": {
            "font_size_emu": 1000,
            "line_height_emu": 10,
        },
        "lines": lines or [],
        "diagnostics": [],
    }


def build(g, ctx, flow=None):
    return build_cmo_slot_runtime_v1(
        resolved_graph=g,
        projection_context=ctx,
        shaped_flow=flow or shaped_flow(),
    )


class CmoSlotRuntimeBridgeTests(unittest.TestCase):
    def test_typed_relation_becomes_visible_target_page_instance(self):
        g = graph()
        ctx = context([relation(3, 7, CARRIER_A)])
        result = build(g, ctx)

        self.assertEqual(1, len(result["native_outputs"]))
        self.assertEqual(1, len(result["scene_instances"]))
        output = result["native_outputs"][0]
        self.assertEqual("chaptera.cmo-slot-flow.native.v1", output["schema_version"])
        instance = result["scene_instances"][0]
        self.assertEqual(CARRIER_A, instance["origin"])
        self.assertEqual(PAGE, instance["parent_origin"])
        self.assertEqual("cmo_story_slot", instance["projection_kind"])
        self.assertEqual(STORY, instance["target_story_origin"])
        self.assertEqual(FRAME, instance["target_frame_origin"])
        self.assertEqual(
            {"x": 100, "y": 200, "width": 80, "height": 60},
            instance["bounds"],
        )
        self.assertFalse(result["story_overset"])

        encoded = json.dumps(result, sort_keys=True)
        self.assertNotIn("PRIVATE CARRIER TEXT", encoded)
        self.assertNotIn("source_refs", encoded)

    def test_carrier_origin_is_withheld_from_base_scene_until_slot_flow(self):
        g = graph()
        ctx = context([relation(3, 7, CARRIER_A)])
        scene = project_resolved_graph_scene(g, context=ctx)
        origins = {item["origin"] for item in scene["nodes"]}
        self.assertIn(FRAME, origins)
        self.assertNotIn(CARRIER_A, origins)
        self.assertTrue(
            any(
                diagnostic["code"] == "cmo_slot_flow_not_materialized"
                for diagnostic in scene["diagnostics"]
            )
        )

    def test_runtime_merge_replaces_pending_diagnostic_with_projected_instance(self):
        g = graph()
        ctx = context([relation(3, 7, CARRIER_A)])
        scene = project_resolved_graph_scene(g, context=ctx)
        runtime = build(g, ctx)
        merged = merge_cmo_runtime_into_scene_v1(scene, runtime)

        projected = [
            item
            for item in merged["nodes"]
            if item.get("projection_kind") == "cmo_story_slot"
        ]
        self.assertEqual(1, len(projected))
        self.assertEqual(CARRIER_A, projected[0]["origin"])
        self.assertEqual(PAGE, projected[0]["parent_origin"])
        self.assertFalse(
            any(
                item["code"] == "cmo_slot_flow_not_materialized"
                and item["origin"] == STORY
                for item in merged["diagnostics"]
            )
        )
        mapping = [
            item
            for item in merged["origin_mapping"]
            if item.get("projection_kind") == "cmo_story_slot"
        ]
        self.assertEqual(projected[0]["instance_id"], mapping[0]["resolved_instance_id"])

    def test_first_nonfit_slot_blocks_later_smaller_slot(self):
        g = graph("\uFFFC\uFFFC")
        g["nodes"][CARRIER_A]["header"]["bounds"]["height"] = 80
        g["nodes"][CARRIER_B]["header"]["bounds"]["height"] = 30
        ctx = context([
            relation(3, 7, CARRIER_A),
            relation(4, 9, CARRIER_B, None),
        ])
        result = build(g, ctx)

        self.assertEqual(
            [7],
            [slot["cmo_id"] for slot in result["native_outputs"][0]["visible_slots"]],
        )
        overset = result["native_outputs"][0]["overset"]
        self.assertTrue(overset["story_overset"])
        self.assertEqual(1, overset["first_nonfitting_slot_index"])
        self.assertEqual(1, overset["remaining_slot_count"])
        self.assertEqual(1, len(result["scene_instances"]))

    def test_marker_relation_cardinality_mismatch_fails_closed(self):
        with self.assertRaisesRegex(CmoSlotRuntimeError, "marker count"):
            build(
                graph("\uFFFC\uFFFC"),
                context([relation(3, 7, CARRIER_A)]),
            )

    def test_non_marker_scalars_require_resolved_line_coverage(self):
        g = graph("A\uFFFCB")
        ctx = context([relation(3, 7, CARRIER_A)])
        with self.assertRaisesRegex(CmoSlotRuntimeError, "coverage incomplete"):
            build(g, ctx)

        lines = [
            {
                "frame_node_id": FRAME,
                "story_id": STORY,
                "frame_line_index": 0,
                "scalar_start": 0,
                "scalar_end": 1,
                "consumed_scalar_end": 1,
                "text": "A",
                "units_per_em": 1000,
                "measured_width": 10,
                "glyphs": [],
            },
            {
                "frame_node_id": FRAME,
                "story_id": STORY,
                "frame_line_index": 1,
                "scalar_start": 2,
                "scalar_end": 3,
                "consumed_scalar_end": 3,
                "text": "B",
                "units_per_em": 1000,
                "measured_width": 10,
                "glyphs": [],
            },
        ]
        result = build(g, ctx, shaped_flow(lines))
        slot = result["native_outputs"][0]["visible_slots"][0]
        self.assertEqual(10, slot["preceding_text_height_emu"])
        self.assertEqual(10, slot["used_height_before_emu"])
        self.assertEqual(70, slot["used_height_after_emu"])
        self.assertFalse(result["story_overset"])
        self.assertEqual(210, result["scene_instances"][0]["bounds"]["y"])

    def test_shaped_line_cannot_cross_object_marker(self):
        g = graph("A\uFFFC")
        ctx = context([relation(3, 7, CARRIER_A)])
        lines = [{
            "frame_node_id": FRAME,
            "story_id": STORY,
            "frame_line_index": 0,
            "scalar_start": 0,
            "scalar_end": 2,
            "consumed_scalar_end": 2,
            "text": "A\uFFFC",
            "units_per_em": 1000,
            "measured_width": 10,
            "glyphs": [],
        }]
        with self.assertRaisesRegex(CmoSlotRuntimeError, "crosses a U\+FFFC"):
            build(g, ctx, shaped_flow(lines))


if __name__ == "__main__":
    unittest.main()
