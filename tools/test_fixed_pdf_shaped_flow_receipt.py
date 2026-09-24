#!/usr/bin/env python3
import copy
import unittest

from validate_fixed_pdf_shaped_flow_receipt import validate_schema, validate_semantics

HASH = "sha256:" + "a" * 64
SOURCE_HASH = "b" * 64
FRAME = "10000000-0000-4000-8000-000000000001"
STORY = "20000000-0000-4000-8000-000000000001"

def valid_receipt():
    return {
        "receipt_version": "chaptera.fixed-pdf-shaped-flow-receipt.v1",
        "producer": {
            "implementation": "chaptera-private-fixed-output",
            "commit_or_build": "deadbeef",
            "core_integration": True,
        },
        "source_hash": SOURCE_HASH,
        "flow_id": HASH,
        "lines": [
            {
                "line_index": 0,
                "frame_node_id": FRAME,
                "story_id": STORY,
                "scalar_start": 0,
                "scalar_end": 1,
                "glyph_count": 1,
                "glyph_sequence_hash": HASH,
                "units_per_em": 1000,
                "measured_width": 500,
            },
            {
                "line_index": 1,
                "frame_node_id": FRAME,
                "story_id": STORY,
                "scalar_start": 2,
                "scalar_end": 3,
                "glyph_count": 1,
                "glyph_sequence_hash": "sha256:" + "c" * 64,
                "units_per_em": 1000,
                "measured_width": 500,
            },
        ],
        "runs": [
            {
                "run_index": 0,
                "frame_node_id": FRAME,
                "story_id": STORY,
                "scalar_base": 0,
                "scalar_end": 1,
                "glyph_count": 1,
                "glyph_sequence_hash": HASH,
                "baseline_x": 0,
                "baseline_y": 1000,
            },
            {
                "run_index": 1,
                "frame_node_id": FRAME,
                "story_id": STORY,
                "scalar_base": 2,
                "scalar_end": 3,
                "glyph_count": 1,
                "glyph_sequence_hash": "sha256:" + "c" * 64,
                "baseline_x": 0,
                "baseline_y": 2000,
            },
        ],
        "story_overset": True,
        "invariants": {
            "reshaping_calls": 0,
            "raw_text_emitted": False,
            "ascii_gate_applied": False,
            "overset_tail_painted": False,
            "line_order_preserved": True,
            "story_global_clusters_preserved": True,
        },
    }

class FixedPdfShapedFlowReceiptTests(unittest.TestCase):
    def test_valid_receipt_is_admitted(self):
        receipt = valid_receipt()
        validate_schema(receipt)
        validate_semantics(receipt)

    def test_private_raw_text_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["lines"][0]["logical_text"] = "secret"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_line_run_count_mismatch_fails(self):
        receipt = valid_receipt()
        receipt["runs"].pop()
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_scalar_base_must_preserve_story_global_start(self):
        receipt = valid_receipt()
        receipt["runs"][1]["scalar_base"] = 0
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_glyph_sequence_must_be_identical(self):
        receipt = valid_receipt()
        receipt["runs"][1]["glyph_sequence_hash"] = HASH
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_reshaping_is_forbidden(self):
        receipt = valid_receipt()
        receipt["invariants"]["reshaping_calls"] = 1
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_ascii_gate_is_forbidden(self):
        receipt = valid_receipt()
        receipt["invariants"]["ascii_gate_applied"] = True
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_overset_tail_paint_is_forbidden(self):
        receipt = valid_receipt()
        receipt["invariants"]["overset_tail_painted"] = True
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

if __name__ == "__main__":
    unittest.main()
