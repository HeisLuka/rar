#!/usr/bin/env python3
import copy
import unittest

from compare_state_transition_receipts import canonical_json, compare_receipts, validate_receipt


def known(value):
    return {"status": "known", "value": value}


def unknown(reason="authority not proven"):
    return {"status": "unknown", "reason": reason}


def receipt(case, before_fields, after_fields, operation_kind, operation_inputs, required_fields):
    value = {
        "receipt_version": "chaptera.state-transition-receipt.v1",
        "contract": {"id": f"chaptera.synthetic.{case}", "version": "v1"},
        "system": {"kind": "synthetic_model", "runtime": "python", "build": "test"},
        "fixture": {
            "kind": "synthetic",
            "identity": case,
            "provenance": "source-free transition comparator control",
        },
        "comparison": {
            "required_state_fields": required_fields,
            "required_invariants": ["identity_preserved"],
        },
        "before": {"fields": before_fields},
        "operation": {"kind": operation_kind, "inputs": operation_inputs},
        "after": {"fields": after_fields},
        "invariants": {"identity_preserved": known(True)},
    }
    validate_receipt(value)
    return value


class StateTransitionComparatorTests(unittest.TestCase):
    def assert_divergent_after(self, left, right, field):
        result = compare_receipts(left, right)
        self.assertEqual(result["status"], "divergent")
        self.assertTrue(
            any(
                r.get("phase") == "after" and r.get("field") == field
                for r in result["reasons"]
            ),
            result,
        )

    def test_equivalent_is_order_independent_for_json_objects(self):
        left = receipt(
            "equivalent-order",
            {"geometry": known({"x": 1, "y": 2})},
            {"geometry": known({"x": 3, "y": 4})},
            "MoveNode",
            {"dx": 2, "dy": 2},
            ["geometry"],
        )
        right = copy.deepcopy(left)
        right["before"]["fields"]["geometry"]["value"] = {"y": 2, "x": 1}
        right["after"]["fields"]["geometry"]["value"] = {"y": 4, "x": 3}
        self.assertEqual(compare_receipts(left, right)["status"], "equivalent")
        self.assertEqual(
            canonical_json(left["operation"]["inputs"]),
            canonical_json(right["operation"]["inputs"]),
        )

    def test_aspect_lock_same_rect_next_resize_diverges(self):
        left = receipt(
            "aspect-lock",
            {"geometry": known({"w": 100, "h": 200})},
            {"geometry": known({"w": 150, "h": 300})},
            "ResizeWidth",
            {"width": 150},
            ["geometry"],
        )
        right = copy.deepcopy(left)
        right["after"]["fields"]["geometry"] = known({"w": 150, "h": 200})
        self.assert_divergent_after(left, right, "geometry")

    def test_inline_ownership_same_placement_next_text_edit_diverges(self):
        left = receipt(
            "inline-ownership",
            {"placement": known({"x": 50, "y": 80})},
            {"placement": known({"story_offset": 12})},
            "InsertText",
            {"at": 10, "text_class": "marker"},
            ["placement"],
        )
        right = copy.deepcopy(left)
        right["after"]["fields"]["placement"] = known({"x": 50, "y": 80})
        self.assert_divergent_after(left, right, "placement")

    def test_connector_same_pixels_next_endpoint_move_diverges(self):
        left = receipt(
            "connector-relation",
            {"path": known([[0, 0], [100, 0]])},
            {"path": known([[0, 0], [140, 20]])},
            "MoveEndpointOwner",
            {"dx": 40, "dy": 20},
            ["path"],
        )
        right = copy.deepcopy(left)
        right["after"]["fields"]["path"] = known([[0, 0], [100, 0]])
        self.assert_divergent_after(left, right, "path")

    def test_picture_same_frame_relative_original_scaling_diverges(self):
        left = receipt(
            "picture-original-size",
            {"frame": known({"w": 400, "h": 300})},
            {"frame": known({"w": 800, "h": 600})},
            "ScaleRelativeToOriginal",
            {"percent": 200},
            ["frame"],
        )
        right = copy.deepcopy(left)
        right["after"]["fields"]["frame"] = known({"w": 640, "h": 480})
        self.assert_divergent_after(left, right, "frame")

    def test_unknown_required_field_is_not_comparable_even_when_other_values_match(self):
        left = receipt(
            "unknown-authority",
            {"geometry": known({"w": 100, "h": 100}), "policy": unknown()},
            {"geometry": known({"w": 120, "h": 120}), "policy": unknown()},
            "ResizeWidth",
            {"width": 120},
            ["geometry", "policy"],
        )
        right = copy.deepcopy(left)
        result = compare_receipts(left, right)
        self.assertEqual(result["status"], "not_comparable")
        self.assertTrue(any(r["kind"] == "missing_authority" for r in result["reasons"]))

    def test_contract_version_mismatch_is_not_comparable(self):
        left = receipt(
            "version-fence",
            {"geometry": known({"w": 1, "h": 1})},
            {"geometry": known({"w": 2, "h": 2})},
            "Resize",
            {"w": 2, "h": 2},
            ["geometry"],
        )
        right = copy.deepcopy(left)
        right["contract"]["version"] = "v2"
        result = compare_receipts(left, right)
        self.assertEqual(result["status"], "not_comparable")
        self.assertTrue(any(r["field"] == "contract.version" for r in result["reasons"]))


if __name__ == "__main__":
    unittest.main()
