#!/usr/bin/env python3
import copy
import json
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
BASE = ROOT / "packages" / "protocol" / "editor-image-replace" / "v1"
sys.path.insert(0, str(TOOLS))

from build_replace_image_export_receipt import build_export_receipt
from validate_replace_image_receipt import validate_pair_receipts

UI = json.loads(
    (BASE / "fixtures" / "ui-producer-receipt.synthetic.json").read_text(encoding="utf-8")
)
SOURCE_HASH = "1" * 64

FAKE_PRODUCER = r"""#!/usr/bin/env python3
import json
import sys

request = json.load(sys.stdin)
source_asset = "2" * 64
replacement = "3" * 64
response = {
    "replacement_binding_id": request["replacement_binding_id"],
    "source_hash_after": request["source_hash"],
    "source_asset_sha256": source_asset,
    "committed_asset_sha256": replacement,
    "effective_asset_sha256": replacement,
    "idml": {
        "can_serialize": True,
        "embedded_asset_sha256": replacement,
        "frame_geometry": "preserved",
        "content_transform": "approximated",
        "z_order": "approximated",
    },
    "odg": {
        "can_serialize": True,
        "embedded_asset_sha256": replacement,
        "frame_geometry": "preserved",
        "content_transform": "approximated",
        "z_order": "preserved",
    },
    "unsupported_target": {
        "blocked": True,
        "explicit_loss": True,
        "silent_drop": False,
        "silent_source_fallback": False,
    },
    "native_pub_writer_promoted": False,
}
json.dump(response, sys.stdout)
"""


class ReplaceImageExportBuilderTests(unittest.TestCase):
    def producer(self, root, source=FAKE_PRODUCER):
        path = pathlib.Path(root) / "producer.py"
        path.write_text(source, encoding="utf-8")
        return [sys.executable, str(path)]

    def test_builder_proves_private_sha_chain_and_emits_valid_pair(self):
        with tempfile.TemporaryDirectory() as tmp:
            receipt = build_export_receipt(
                copy.deepcopy(UI),
                self.producer(tmp),
                source_hash=SOURCE_HASH,
            )
        result = validate_pair_receipts(copy.deepcopy(UI), receipt)
        self.assertTrue(result["replacement_identity_bound_end_to_end"])
        self.assertEqual(
            UI["replacement_binding"]["binding_id"],
            receipt["replacement_binding"]["binding_id"],
        )
        self.assertFalse(any(receipt["privacy"].values()))

    def test_binding_mismatch_fails(self):
        broken = FAKE_PRODUCER.replace(
            'request["replacement_binding_id"]',
            '"rb_ffffffffffffffffffffffffffffffff"',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "binding"):
                build_export_receipt(
                    copy.deepcopy(UI),
                    self.producer(tmp, broken),
                    source_hash=SOURCE_HASH,
                )

    def test_effective_asset_must_equal_committed_replacement(self):
        broken = FAKE_PRODUCER.replace(
            '"effective_asset_sha256": replacement,',
            '"effective_asset_sha256": "4" * 64,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "effective export asset"):
                build_export_receipt(
                    copy.deepcopy(UI),
                    self.producer(tmp, broken),
                    source_hash=SOURCE_HASH,
                )

    def test_idml_bytes_must_equal_effective_replacement(self):
        broken = FAKE_PRODUCER.replace(
            '"embedded_asset_sha256": replacement,',
            '"embedded_asset_sha256": "4" * 64,',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "idml embedded bytes"):
                build_export_receipt(
                    copy.deepcopy(UI),
                    self.producer(tmp, broken),
                    source_hash=SOURCE_HASH,
                )

    def test_source_asset_fallback_cannot_masquerade_as_replacement(self):
        broken = FAKE_PRODUCER.replace(
            '"committed_asset_sha256": replacement,',
            '"committed_asset_sha256": source_asset,',
        ).replace(
            '"effective_asset_sha256": replacement,',
            '"effective_asset_sha256": source_asset,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "did not change the source asset"):
                build_export_receipt(
                    copy.deepcopy(UI),
                    self.producer(tmp, broken),
                    source_hash=SOURCE_HASH,
                )

    def test_unsupported_target_cannot_silently_fallback(self):
        broken = FAKE_PRODUCER.replace(
            '"silent_source_fallback": False,',
            '"silent_source_fallback": True,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "explicit loss"):
                build_export_receipt(
                    copy.deepcopy(UI),
                    self.producer(tmp, broken),
                    source_hash=SOURCE_HASH,
                )

    def test_source_pub_identity_must_remain_immutable(self):
        broken = FAKE_PRODUCER.replace(
            '"source_hash_after": request["source_hash"],',
            '"source_hash_after": "9" * 64,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "source PUB identity"):
                build_export_receipt(
                    copy.deepcopy(UI),
                    self.producer(tmp, broken),
                    source_hash=SOURCE_HASH,
                )

    def test_real_pub_requires_projection_instance_admission(self):
        ui = copy.deepcopy(UI)
        ui["fixture_kind"] = "real_pub_sanitized"
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "projection_instance_gate_unresolved"):
                build_export_receipt(
                    ui,
                    self.producer(tmp),
                    source_hash=SOURCE_HASH,
                    projection_instance_admitted=False,
                )

    def test_real_pub_after_typed_admission_keeps_same_public_contract(self):
        ui = copy.deepcopy(UI)
        ui["fixture_kind"] = "real_pub_sanitized"
        with tempfile.TemporaryDirectory() as tmp:
            receipt = build_export_receipt(
                ui,
                self.producer(tmp),
                source_hash=SOURCE_HASH,
                projection_instance_admitted=True,
            )
        self.assertEqual("real_pub_sanitized", receipt["fixture_kind"])
        self.assertEqual(ui["replacement_binding"]["binding_id"], receipt["replacement_binding"]["binding_id"])


if __name__ == "__main__":
    unittest.main()
