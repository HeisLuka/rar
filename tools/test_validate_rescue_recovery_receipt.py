import copy
import hashlib
import json
import pathlib
import tempfile
import unittest

from tools.validate_rescue_recovery_receipt import (
    PINNED_FIXTURES,
    build_product_validation,
    validate_producer_receipt,
)


def artifact(artifact_id="thumb-1", kind="thumbnail"):
    return {
        "artifact_id": artifact_id,
        "kind": kind,
        "sha256": "a" * 64,
        "exact": True,
        "source_ranges": [
            {"offset": 4096, "length": 512, "sha256": "b" * 64}
        ],
    }


def base_receipt(kind="natural_partial_cfb"):
    source_sha = PINNED_FIXTURES[kind]
    return {
        "receipt_version": "chaptera.rescue-recovery-producer-receipt.v1",
        "fixture": {"kind": kind, "source_sha256": source_sha},
        "source_immutability": {
            "before_sha256": source_sha,
            "after_sha256": source_sha,
            "unchanged": True,
        },
        "recovery": {
            "recovery_class": "bounded_exact_salvage",
            "repair_plan_id": "partial-cfb.v1",
            "action_ids": ["recover-surviving-streams"],
            "fabricated_bytes": 0,
            "silent_drops": 0,
        },
        "artifacts": [artifact()],
        "loss": {"known_loss": True, "items": [{"code": "cfb.truncated", "severity": "warning", "count": 1}]},
        "native_validation": {"state": "inconclusive"},
        "proposed_route": "bounded_recovered",
        "privacy": {
            "raw_pub_bytes": False,
            "document_text": False,
            "local_paths": False,
            "customer_identity": False,
            "credentials": False,
        },
    }


class RescueRecoveryReceiptTests(unittest.TestCase):
    def test_natural_bounded_recovery_maps_to_product_outcome(self):
        receipt = base_receipt()
        summary = validate_producer_receipt(receipt)
        self.assertEqual(summary["product_outcome"], "bounded_recovered")
        product = build_product_validation(receipt, producer_receipt_sha256="c" * 64)
        self.assertEqual(product["product"], "Chaptera Rescue")
        self.assertEqual(product["outcome"], "bounded_recovered")
        self.assertFalse(product["native_pub_delivery_allowed"])

    def test_source_mutation_is_rejected(self):
        receipt = base_receipt()
        receipt["source_immutability"]["after_sha256"] = "d" * 64
        receipt["source_immutability"]["unchanged"] = False
        with self.assertRaisesRegex(AssertionError, "source"):
            validate_producer_receipt(receipt)

    def test_fabricated_bytes_are_rejected(self):
        receipt = base_receipt()
        receipt["recovery"]["fabricated_bytes"] = 1
        with self.assertRaisesRegex(AssertionError, "fabricated_bytes"):
            validate_producer_receipt(receipt)

    def test_silent_drops_are_rejected(self):
        receipt = base_receipt()
        receipt["recovery"]["silent_drops"] = 1
        with self.assertRaisesRegex(AssertionError, "silent_drops"):
            validate_producer_receipt(receipt)

    def test_partial_recovery_requires_explicit_loss(self):
        receipt = base_receipt()
        receipt["recovery"]["recovery_class"] = "partial_salvage"
        receipt["proposed_route"] = "partially_recovered"
        receipt["loss"] = {"known_loss": False, "items": []}
        with self.assertRaisesRegex(AssertionError, "known loss"):
            validate_producer_receipt(receipt)

    def test_healthy_control_cannot_be_promoted_to_recovered(self):
        receipt = base_receipt("healthy_control")
        with self.assertRaisesRegex(AssertionError, "healthy control"):
            validate_producer_receipt(receipt)

    def test_healthy_control_diagnostic_only_maps_fail_closed(self):
        receipt = base_receipt("healthy_control")
        receipt["recovery"]["recovery_class"] = "diagnostic_only"
        receipt["recovery"]["repair_plan_id"] = "diagnostic-only.v1"
        receipt["recovery"]["action_ids"] = []
        receipt["artifacts"] = []
        receipt["loss"] = {"known_loss": False, "items": []}
        receipt["proposed_route"] = "diagnostic_only"
        summary = validate_producer_receipt(receipt)
        self.assertEqual(summary["product_outcome"], "unsupported/no_safe_recovery")

    def test_partial_control_cannot_be_complete(self):
        receipt = base_receipt("partial_control")
        with self.assertRaisesRegex(AssertionError, "partial control"):
            validate_producer_receipt(receipt)

    def test_native_pub_requires_valid_separate_native_receipt(self):
        receipt = base_receipt()
        receipt["artifacts"] = [artifact(kind="native_pub")]
        with self.assertRaisesRegex(AssertionError, "native PUB artifact"):
            validate_producer_receipt(receipt)

        receipt["native_validation"] = {"state": "valid", "receipt_sha256": "e" * 64}
        summary = validate_producer_receipt(receipt)
        self.assertTrue(summary["native_pub_delivery_allowed"])

    def test_wrong_pinned_fixture_hash_is_rejected(self):
        receipt = base_receipt()
        receipt["fixture"]["source_sha256"] = "f" * 64
        receipt["source_immutability"]["before_sha256"] = "f" * 64
        receipt["source_immutability"]["after_sha256"] = "f" * 64
        with self.assertRaisesRegex(AssertionError, "pinned acceptance witness"):
            validate_producer_receipt(receipt)

    def test_schema_rejects_private_field(self):
        receipt = base_receipt()
        receipt["local_path"] = "C:/private/file.pub"
        with self.assertRaisesRegex(AssertionError, "schema validation"):
            validate_producer_receipt(receipt)


if __name__ == "__main__":
    unittest.main()
