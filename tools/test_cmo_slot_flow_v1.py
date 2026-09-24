#!/usr/bin/env python3
import copy
import unittest

from cmo_slot_flow_v1 import CmoSlotFlowError, build_receipt
from validate_cmo_slot_flow_receipt import validate_schema, validate_semantics

STORY = "20000000-0000-4000-8000-000000000001"
FRAME = "30000000-0000-4000-8000-000000000001"


def slot(index, scalar, source_order, *, width=90, height=40):
    return {
        "kind": "object_slot",
        "slot_index": index,
        "scalar_index": scalar,
        "source_order": source_order,
        "cmo_id": index + 1,
        "carrier_node_id": f"40000000-0000-4000-8000-{index + 1:012x}",
        "carrier_story_id": f"50000000-0000-4000-8000-{index + 1:012x}",
        "intrinsic_width_emu": width,
        "intrinsic_height_emu": height,
    }


def shaped_line(start, end, consumed, *, height=20):
    return {
        "kind": "shaped_line",
        "scalar_start": start,
        "scalar_end": end,
        "consumed_scalar_end": consumed,
        "height_emu": height,
    }


def fixture(items=None, *, width=100, height=100):
    items = list(items or [slot(0, 0, 7)])
    slot_count = sum(item["kind"] == "object_slot" for item in items)
    return {
        "schema_version": "chaptera.cmo-slot-flow-input.v1",
        "producer": {
            "implementation": "rar-cmo-slot-flow",
            "commit_or_build": "unit-test",
            "core_integration": True,
        },
        "source_hash": "a" * 64,
        "target_story_id": STORY,
        "target_frame_node_id": FRAME,
        "target_frame_count": 1,
        "story_marker_count": slot_count,
        "host": {"width_emu": width, "height_emu": height},
        "items": items,
    }


class CmoSlotFlowTests(unittest.TestCase):
    def assert_admitted(self, value):
        receipt = build_receipt(value)
        validate_schema(receipt)
        validate_semantics(receipt)
        return receipt

    def test_one_slot_preserves_identity_and_u_fffc_scalar(self):
        receipt = self.assert_admitted(fixture())
        self.assertEqual(receipt["slot_count"], 1)
        self.assertEqual(len(receipt["visible_slots"]), 1)
        visible = receipt["visible_slots"][0]
        self.assertEqual(visible["scalar_index"], 0)
        self.assertEqual(visible["source_order"], 7)
        self.assertEqual(visible["carrier_node_id"], slot(0, 0, 7)["carrier_node_id"])
        self.assertFalse(receipt["overset"]["story_overset"])
        self.assertEqual(receipt["invariants"]["carrier_reparent_count"], 0)

    def test_object_exactly_fills_remaining_height_but_plus_one_oversets(self):
        exact = self.assert_admitted(
            fixture([slot(0, 0, 7, height=60)], height=60)
        )
        self.assertEqual(exact["visible_slots"][0]["used_height_after_emu"], 60)
        self.assertFalse(exact["overset"]["story_overset"])

        too_tall = self.assert_admitted(
            fixture([slot(0, 0, 7, height=61)], height=60)
        )
        self.assertEqual(too_tall["visible_slots"], [])
        self.assertTrue(too_tall["overset"]["story_overset"])
        self.assertEqual(too_tall["overset"]["failure_reason"], "height")
        self.assertEqual(too_tall["overset"]["first_nonfitting_slot_index"], 0)

    def test_object_width_equal_host_fits_but_plus_one_never_scales(self):
        exact = self.assert_admitted(
            fixture([slot(0, 0, 7, width=100)], width=100)
        )
        visible = exact["visible_slots"][0]
        self.assertEqual(visible["resolved_width_emu"], 100)
        self.assertEqual(visible["intrinsic_width_emu"], 100)

        too_wide = self.assert_admitted(
            fixture([slot(0, 0, 7, width=101)], width=100)
        )
        self.assertEqual(too_wide["visible_slots"], [])
        self.assertEqual(too_wide["overset"]["failure_reason"], "width")
        self.assertFalse(too_wide["invariants"]["scaling_applied"])

    def test_first_nonfitting_slot_stops_flow_even_if_later_slot_would_fit(self):
        value = fixture(
            [
                slot(0, 0, 7, height=40),
                slot(1, 2, 9, height=70),
                slot(2, 4, 12, height=20),
            ],
            height=100,
        )
        receipt = self.assert_admitted(value)
        self.assertEqual(
            [item["slot_index"] for item in receipt["visible_slots"]],
            [0],
        )
        self.assertEqual(receipt["overset"]["first_nonfitting_slot_index"], 1)
        self.assertEqual(receipt["overset"]["remaining_slot_count"], 2)
        self.assertFalse(receipt["invariants"]["skip_to_fit"])

    def test_shaped_line_and_object_share_one_vertical_cursor(self):
        value = fixture(
            [
                shaped_line(0, 1, 2, height=30),
                slot(0, 2, 7, height=80),
            ],
            height=100,
        )
        receipt = self.assert_admitted(value)
        self.assertEqual(receipt["visible_slots"], [])
        self.assertTrue(receipt["overset"]["story_overset"])
        self.assertEqual(receipt["overset"]["first_nonfitting_kind"], "object_slot")
        self.assertEqual(receipt["overset"]["first_nonfitting_scalar_index"], 2)
        self.assertEqual(receipt["overset"]["failure_reason"], "height")

    def test_failing_text_line_also_establishes_authoritative_overset(self):
        value = fixture(
            [
                shaped_line(0, 3, 4, height=101),
                slot(0, 4, 7, height=10),
            ],
            height=100,
        )
        receipt = self.assert_admitted(value)
        self.assertEqual(receipt["visible_slots"], [])
        self.assertEqual(receipt["overset"]["first_nonfitting_kind"], "shaped_line")
        self.assertIsNone(receipt["overset"]["first_nonfitting_slot_index"])
        self.assertEqual(receipt["overset"]["remaining_slot_count"], 1)

    def test_marker_count_mismatch_fails_closed(self):
        value = fixture()
        value["story_marker_count"] = 2
        with self.assertRaises(CmoSlotFlowError):
            build_receipt(value)

    def test_multiframe_target_is_outside_v1_admission(self):
        value = fixture()
        value["target_frame_count"] = 2
        with self.assertRaises(CmoSlotFlowError):
            build_receipt(value)

    def test_slot_ordinals_scalar_order_and_source_order_are_fail_closed(self):
        bad_ordinal = fixture([slot(1, 0, 7)])
        with self.assertRaises(CmoSlotFlowError):
            build_receipt(bad_ordinal)

        bad_scalar = fixture([slot(0, 3, 7), slot(1, 2, 9)])
        with self.assertRaises(CmoSlotFlowError):
            build_receipt(bad_scalar)

        bad_source_order = fixture([slot(0, 0, 9), slot(1, 2, 7)])
        with self.assertRaises(CmoSlotFlowError):
            build_receipt(bad_source_order)

    def test_input_rejects_raw_text_escape_hatch(self):
        value = fixture()
        value["carrier_text"] = "must never cross this boundary"
        with self.assertRaises(CmoSlotFlowError):
            build_receipt(value)

    def test_validator_rejects_receipt_tampering(self):
        receipt = self.assert_admitted(fixture())
        scaled = copy.deepcopy(receipt)
        scaled["visible_slots"][0]["resolved_width_emu"] -= 1
        with self.assertRaises(AssertionError):
            validate_semantics(scaled)

        raw_text = copy.deepcopy(receipt)
        raw_text["visible_slots"][0]["text"] = "forbidden"
        with self.assertRaises(AssertionError):
            validate_semantics(raw_text)


if __name__ == "__main__":
    unittest.main()
