#!/usr/bin/env python3
import copy
import unittest

from fixed_pdf_shaped_flow_bridge_v1 import (
    FixedPdfShapedFlowBridgeError,
    build_receipt,
    materialize_fixed_runs,
)
from validate_fixed_pdf_shaped_flow_receipt import validate_schema, validate_semantics

SOURCE_HASH = "b" * 64
FLOW_ID = "sha256:" + "c" * 64
FRAME = "10000000-0000-4000-8000-000000000001"
STORY = "20000000-0000-4000-8000-000000000001"


def glyph(glyph_id, cluster, advance=500):
    return {
        "glyph_id": glyph_id,
        "cluster": cluster,
        "x_advance": advance,
        "y_advance": 0,
        "x_offset": 0,
        "y_offset": 0,
    }


def scene():
    return {
        "schema_version": "chaptera.shaped-flow-bridge-input.v1",
        "source_hash": SOURCE_HASH,
        "flow_id": FLOW_ID,
        "environment": {
            "font_size_emu": 1000,
            "line_height_emu": 1400,
        },
        "lines": [
            {
                "frame_node_id": FRAME,
                "story_id": STORY,
                "frame_line_index": 0,
                "scalar_start": 0,
                "scalar_end": 1,
                "consumed_scalar_end": 2,
                "text": "A",
                "units_per_em": 1000,
                "measured_width": 500,
                "glyphs": [glyph(11, 0)],
            },
            {
                "frame_node_id": FRAME,
                "story_id": STORY,
                "frame_line_index": 1,
                "scalar_start": 2,
                "scalar_end": 4,
                "consumed_scalar_end": 4,
                "text": "БC",
                "units_per_em": 1000,
                "measured_width": 1000,
                "glyphs": [glyph(12, 2), glyph(13, 3)],
            },
        ],
        "diagnostics": [{"code": "story_overset"}],
    }


class FixedPdfShapedFlowBridgeTests(unittest.TestCase):
    def test_materializes_current_resolved_lines_without_reshaping(self):
        runs = materialize_fixed_runs(scene())
        self.assertEqual(len(runs), 2)
        self.assertEqual(runs[0]["scalar_base"], 0)
        self.assertEqual(runs[1]["scalar_base"], 2)
        self.assertEqual(runs[0]["baseline_y"], 1000)
        self.assertEqual(runs[1]["baseline_y"], 2400)
        self.assertEqual([g["cluster"] for g in runs[1]["glyphs"]], [2, 3])
        self.assertEqual(runs[1]["logical_text"], "БC")

    def test_receipt_is_sanitized_and_admitted(self):
        receipt = build_receipt(
            scene(),
            implementation="rar-fixed-pdf-shaped-flow-bridge-v1",
            commit_or_build="deadbeef",
        )
        validate_schema(receipt)
        summary = validate_semantics(receipt)
        self.assertEqual(summary["visible_line_count"], 2)
        self.assertTrue(summary["story_overset"])
        encoded = str(receipt)
        self.assertNotIn('"text"', encoded)
        self.assertNotIn("БC", encoded)

    def test_nonzero_story_global_cluster_maps_to_local_text(self):
        value = scene()
        runs = materialize_fixed_runs(value)
        self.assertEqual(runs[1]["scalar_base"], 2)
        self.assertEqual(runs[1]["scalar_end"], 4)

    def test_cluster_before_scalar_base_fails_closed(self):
        value = scene()
        value["lines"][1]["glyphs"][0]["cluster"] = 1
        with self.assertRaises(FixedPdfShapedFlowBridgeError):
            materialize_fixed_runs(value)

    def test_duplicate_cluster_fails_bounded_pdf_mapping(self):
        value = scene()
        value["lines"][1]["glyphs"][1]["cluster"] = 2
        with self.assertRaises(FixedPdfShapedFlowBridgeError):
            materialize_fixed_runs(value)

    def test_bridge_does_not_infer_overset_from_hidden_tail(self):
        value = scene()
        value["diagnostics"] = []
        receipt = build_receipt(
            value,
            implementation="rar-fixed-pdf-shaped-flow-bridge-v1",
            commit_or_build="deadbeef",
        )
        self.assertFalse(receipt["story_overset"])

    def test_canonical_line_order_required(self):
        value = scene()
        value["lines"].reverse()
        with self.assertRaises(FixedPdfShapedFlowBridgeError):
            materialize_fixed_runs(value)

    def test_baseline_overflow_fails_closed(self):
        value = scene()
        value["environment"]["line_height_emu"] = 2**63 - 1
        value["lines"][1]["frame_line_index"] = 2
        with self.assertRaises(FixedPdfShapedFlowBridgeError):
            materialize_fixed_runs(value)

    def test_raw_text_never_enters_public_receipt(self):
        value = scene()
        value["lines"][0]["text"] = "Z"
        receipt = build_receipt(
            value,
            implementation="rar-fixed-pdf-shaped-flow-bridge-v1",
            commit_or_build="deadbeef",
        )
        self.assertNotIn("logical_text", receipt["runs"][0])
        self.assertNotIn("text", receipt["lines"][0])


if __name__ == "__main__":
    unittest.main()
