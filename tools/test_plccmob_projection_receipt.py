#!/usr/bin/env python3
import copy
import unittest

from validate_plccmob_projection_receipt import validate_schema, validate_semantics

SOURCE_HASH = "a" * 64

def probe(code):
    return {
        "code": code,
        "diagnostic_emitted": True,
        "projection_emitted": False,
    }

def valid_receipt():
    return {
        "receipt_version": "chaptera.plccmob-projection-receipt.v1",
        "producer": {
            "implementation": "chaptera-private-pub-adapter",
            "commit_or_build": "deadbeef",
            "core_integration": True,
        },
        "source_hash": SOURCE_HASH,
        "projection_context_version": "chaptera.pub-projection-context.v1",
        "plc_cmob": {
            "declared_count": 2,
            "row_count": 2,
            "raw_size": 64,
        },
        "relations": [
            {
                "source_order": 0,
                "cmo_id": 7,
                "carrier_ohpo": 441,
                "carrier_cmo_id": 7,
                "target_qsid": 49,
                "carrier_node_id": "10000000-0000-4000-8000-000000000001",
                "carrier_story_id": "20000000-0000-4000-8000-000000000001",
                "target_story_id": "30000000-0000-4000-8000-000000000001",
                "target_frame_node_id": "40000000-0000-4000-8000-000000000001",
            },
            {
                "source_order": 1,
                "cmo_id": 9,
                "carrier_ohpo": 446,
                "carrier_cmo_id": 9,
                "target_qsid": 49,
                "carrier_node_id": "10000000-0000-4000-8000-000000000002",
                "carrier_story_id": None,
                "target_story_id": "30000000-0000-4000-8000-000000000001",
                "target_frame_node_id": "40000000-0000-4000-8000-000000000001",
            },
        ],
        "targets": [
            {
                "target_qsid": 49,
                "relation_count": 2,
                "object_marker_count": 2,
            }
        ],
        "invariants": {
            "source_parentage_preserved": True,
            "carrier_reparent_count": 0,
            "raw_text_emitted": False,
            "ordered_relation": True,
        },
        "fail_closed_probes": {
            "count_mismatch": probe("plccmob.count_mismatch"),
            "malformed_row": probe("plccmob.malformed_row"),
            "unresolved_ohpo": probe("plccmob.unresolved_ohpo"),
            "carrier_cmo_id_mismatch": probe("plccmob.carrier_cmo_id_mismatch"),
            "unresolved_target_qsid": probe("plccmob.unresolved_target_qsid"),
            "marker_count_mismatch": probe("plccmob.marker_count_mismatch"),
        },
    }

class PlcCmobReceiptTests(unittest.TestCase):
    def test_valid_receipt_is_admitted(self):
        receipt = valid_receipt()
        validate_schema(receipt)
        validate_semantics(receipt)

    def test_unknown_private_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["private_checkout_path"] = "/home/private/core"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_raw_text_field_fails_closed(self):
        receipt = valid_receipt()
        receipt["relations"][0]["carrier_text"] = "secret customer text"
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_declared_count_mismatch_fails(self):
        receipt = valid_receipt()
        receipt["plc_cmob"]["declared_count"] = 3
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_raw_size_mismatch_fails(self):
        receipt = valid_receipt()
        receipt["plc_cmob"]["raw_size"] += 1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_source_order_gap_fails(self):
        receipt = valid_receipt()
        receipt["relations"][1]["source_order"] = 2
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_carrier_cmo_id_mismatch_fails(self):
        receipt = valid_receipt()
        receipt["relations"][1]["carrier_cmo_id"] = 99
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_marker_cardinality_mismatch_fails(self):
        receipt = valid_receipt()
        receipt["targets"][0]["object_marker_count"] = 1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_missing_target_summary_fails(self):
        receipt = valid_receipt()
        receipt["targets"] = []
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_reparenting_fails_schema(self):
        receipt = valid_receipt()
        receipt["invariants"]["carrier_reparent_count"] = 1
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_fail_closed_probe_must_not_emit_projection(self):
        receipt = valid_receipt()
        receipt["fail_closed_probes"]["unresolved_ohpo"]["projection_emitted"] = True
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

if __name__ == "__main__":
    unittest.main()
