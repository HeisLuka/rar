#!/usr/bin/env python3
import json
import pathlib
import unittest

from cmo_slot_flow_v1 import build_receipt
from validate_cmo_slot_flow_receipt import validate_schema, validate_semantics

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE_DIR = ROOT / "packages" / "protocol" / "pub-projection" / "v1"
SOURCE_HASH = "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3"


def load(name):
    value = json.loads((FIXTURE_DIR / name).read_text(encoding="utf-8"))
    receipt = build_receipt(value)
    validate_schema(receipt)
    validate_semantics(receipt)
    return value, receipt


class CarltonMarchCmoSlotFlowWitnesses(unittest.TestCase):
    def test_single_slot_targets_pass_exact_axis_fit_witnesses(self):
        # These are slot0 axis-fit witnesses only. They intentionally do not
        # claim whole-Story non-overset because trailing CR/paragraph advance
        # belongs to the native shaped-flow consumer that is still gated.
        cases = [
            (
                "cmo-slot-flow-carlton-march-q218-fit.json",
                1,
                1_947_077,
                1_710_156,
                4_411_989,
                2_170_073,
            ),
            (
                "cmo-slot-flow-carlton-march-q216-fit.json",
                5,
                2_473_777,
                987_695,
                2_518_890,
                1_099_091,
            ),
            (
                "cmo-slot-flow-carlton-march-q120-fit.json",
                6,
                3_041_690,
                1_281_563,
                3_199_116,
                1_430_873,
            ),
        ]
        for name, cmo_id, width, height, host_width, host_height in cases:
            with self.subTest(name=name):
                value, receipt = load(name)
                self.assertEqual(value["source_hash"], SOURCE_HASH)
                self.assertEqual(value["host"]["width_emu"], host_width)
                self.assertEqual(value["host"]["height_emu"], host_height)
                self.assertEqual(len(receipt["visible_slots"]), 1)
                slot = receipt["visible_slots"][0]
                self.assertEqual(slot["cmo_id"], cmo_id)
                self.assertEqual(slot["intrinsic_width_emu"], width)
                self.assertEqual(slot["intrinsic_height_emu"], height)
                self.assertEqual(slot["resolved_width_emu"], width)
                self.assertEqual(slot["resolved_height_emu"], height)
                self.assertLessEqual(width, host_width)
                self.assertLessEqual(height, host_height)
                self.assertFalse(receipt["invariants"]["scaling_applied"])

    def test_q49_zero_text_height_lower_bound_already_oversets_at_cmo9(self):
        # This fixture deliberately omits all inter-slot shaped-line height.
        # That makes it a lower bound on vertical consumption: if Cmo9 already
        # fails here, real CR/paragraph advance can only reduce the residual.
        value, receipt = load(
            "cmo-slot-flow-carlton-march-q49-lower-bound.json"
        )
        self.assertEqual(value["source_hash"], SOURCE_HASH)
        self.assertEqual(value["story_marker_count"], 6)
        self.assertEqual([item["cmo_id"] for item in value["items"]], [7, 9, 12, 13, 15, 16])
        self.assertEqual(
            [item["scalar_index"] for item in value["items"]],
            [0, 3, 5, 7, 9, 11],
        )
        self.assertEqual(
            [item["source_order"] for item in value["items"]],
            [3, 4, 5, 6, 7, 8],
        )

        self.assertEqual(
            [slot["cmo_id"] for slot in receipt["visible_slots"]],
            [7],
        )
        visible = receipt["visible_slots"][0]
        self.assertEqual(visible["preceding_text_height_emu"], 0)
        self.assertEqual(visible["intrinsic_width_emu"], 2_002_380)
        self.assertEqual(visible["intrinsic_height_emu"], 1_469_908)

        host_height = value["host"]["height_emu"]
        host_width = value["host"]["width_emu"]
        residual = host_height - visible["used_height_after_emu"]
        self.assertEqual(residual, 323_122)

        cmo9 = value["items"][1]
        self.assertEqual(cmo9["cmo_id"], 9)
        self.assertEqual(cmo9["intrinsic_height_emu"], 331_221)
        self.assertEqual(cmo9["intrinsic_height_emu"] - residual, 8_099)
        self.assertEqual(cmo9["intrinsic_width_emu"] - host_width, 5_481_479)

        overset = receipt["overset"]
        self.assertTrue(overset["story_overset"])
        self.assertEqual(overset["first_nonfitting_kind"], "object_slot")
        self.assertEqual(overset["first_nonfitting_slot_index"], 1)
        self.assertEqual(overset["first_nonfitting_scalar_index"], 3)
        self.assertEqual(overset["failure_reason"], "width_and_height")
        self.assertEqual(overset["remaining_slot_count"], 5)
        self.assertFalse(receipt["invariants"]["skip_to_fit"])
        self.assertFalse(receipt["invariants"]["raw_text_emitted"])


if __name__ == "__main__":
    unittest.main()
