import copy
import json
import unittest
from pathlib import Path

from tools.validate_resize_node_producer_receipt import validate_receipt

FIXTURE = Path("packages/protocol/editor-resize/v1/fixtures/producer-receipt.synthetic.json")


class ResizeNodeProducerReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.receipt = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_synthetic_receipt_passes(self):
        summary = validate_receipt(copy.deepcopy(self.receipt))
        self.assertTrue(summary["one_durable_operation"])
        self.assertTrue(summary["undo_redo_exact"])
        self.assertTrue(summary["idml_odg_geometry_persistence"])
        self.assertTrue(summary["source_free_receipt"])

    def test_commit_must_append_exactly_one_operation(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["commit"]["operation_count_after"] += 1
        with self.assertRaises(AssertionError):
            validate_receipt(receipt)

    def test_resize_cannot_smuggle_pure_move(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["commit"]["pure_move"] = True
        with self.assertRaises(AssertionError):
            validate_receipt(receipt)

    def test_resize_requires_size_change(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["commit"]["size_changed"] = False
        with self.assertRaises(AssertionError):
            validate_receipt(receipt)

    def test_native_pub_writer_cannot_be_promoted_by_resize_receipt(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["persistence"]["native_pub_writer_state"] = "supported"
        with self.assertRaises(AssertionError):
            validate_receipt(receipt)

    def test_every_negative_probe_is_required(self):
        for key in self.receipt["negative_probes"]:
            receipt = copy.deepcopy(self.receipt)
            receipt["negative_probes"][key] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(receipt)

    def test_private_identity_fields_are_rejected(self):
        for key in self.receipt["privacy"]:
            receipt = copy.deepcopy(self.receipt)
            receipt["privacy"][key] = True
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(receipt)

    def test_unknown_producer_integration_is_rejected_by_schema(self):
        receipt = copy.deepcopy(self.receipt)
        receipt["producer"]["integration"] = "browser"
        with self.assertRaises(AssertionError):
            validate_receipt(receipt)

    def test_extra_document_identity_fields_are_rejected_by_schema(self):
        for key, value in [
            ("node_id", "node:123"),
            ("source_hash", "a" * 64),
            ("filename", "newsletter.pub"),
            ("path", "C:/private/newsletter.pub"),
        ]:
            receipt = copy.deepcopy(self.receipt)
            receipt[key] = value
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(receipt)


if __name__ == "__main__":
    unittest.main()
