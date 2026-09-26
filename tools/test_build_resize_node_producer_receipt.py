#!/usr/bin/env python3
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_resize_node_producer_receipt import build_receipt
from validate_resize_node_producer_receipt import validate_receipt

SOURCE_HASH = "a" * 64
DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
BINARY_SHA = "b" * 64

FAKE_PRODUCER = r"""#!/usr/bin/env python3
import copy
import json
import sys

payload = json.load(sys.stdin)
source_hash = payload["source_hash"]
node_id = "30000000-0000-4000-8000-000000000001"
before = {"x": -12700, "y": 25400, "width": 1828800, "height": 914400}
after = {"x": -25400, "y": 0, "width": 1900000, "height": 1000000}

baseline_project = {
    "schema_version": "pub-editor-v0.4",
    "source_hash": source_hash,
    "operations": [],
    "nodes": {
        node_id: {
            "bounds": copy.deepcopy(before),
            "direct_page_owned": True,
            "identity_transform": True,
        }
    },
}
operation = {
    "kind": "resize_node",
    "node_id": node_id,
    "before": copy.deepcopy(before),
    "after": copy.deepcopy(after),
}
accepted_project = copy.deepcopy(baseline_project)
accepted_project["schema_version"] = "pub-editor-v0.5"
accepted_project["operations"] = [copy.deepcopy(operation)]
accepted_project["nodes"][node_id]["bounds"] = copy.deepcopy(after)

action = payload["action"]
if action == "baseline":
    json.dump({
        "source_hash": source_hash,
        "baseline_project": baseline_project,
        "resize_candidate": {
            "node_id": node_id,
            "before": before,
            "after": after,
            "direct_page_owned": True,
            "identity_transform": True,
            "original_bounds_valid": True,
        },
        "signed_origin_probe_passed": True,
    }, sys.stdout)
    raise SystemExit(0)

if action == "commit":
    command = payload["command"]
    if command["node_id"] != node_id:
        raise SystemExit(3)
    json.dump({
        "canonical_operation": operation,
        "resulting_project": accepted_project,
        "consequences": [
            {"key": "node.geometry.bounds", "state": "supported", "note": None}
        ],
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "history":
    if payload["kind"] == "undo":
        project = baseline_project
    elif payload["kind"] == "redo":
        project = accepted_project
    else:
        raise SystemExit(4)
    json.dump({
        "resulting_project": project,
        "consequences": [
            {"key": "history." + payload["kind"], "state": "supported", "note": None}
        ],
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "replay":
    json.dump({
        "replayed_project": accepted_project,
        "legacy_v0_4_rejected": True,
        "stale_before_rejected_transactionally": True,
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "export":
    json.dump({
        "idml_reflects_resized_bounds": True,
        "odg_reflects_resized_bounds": True,
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "probe":
    json.dump({
        "rejected_no_mutation": True,
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

raise SystemExit(2)
"""


class ResizeNodeProducerBuilderTests(unittest.TestCase):
    def make_producer(self, root, content=FAKE_PRODUCER):
        producer = pathlib.Path(root) / "fake_resize_producer.py"
        producer.write_text(content, encoding="utf-8")
        return producer

    def test_builder_reuses_revision_kernel_and_emits_valid_source_free_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = self.make_producer(tmp)
            receipt = build_receipt(
                [sys.executable, str(producer)],
                source_hash=SOURCE_HASH,
                document_id=DOCUMENT_ID,
                chaptera_version="0.1.0-test",
                platform="windows",
                binary_sha256=BINARY_SHA,
                fixture_kind="synthetic_geometry",
            )

        summary = validate_receipt(receipt)
        self.assertEqual("local_private", receipt["producer"]["integration"])
        self.assertTrue(summary["one_durable_operation"])
        self.assertTrue(summary["undo_redo_exact"])
        self.assertTrue(summary["transactional_replay"])
        self.assertTrue(summary["idml_odg_geometry_persistence"])
        self.assertNotIn("source_hash", receipt)
        self.assertNotIn("node_id", receipt)

    def test_real_pub_requires_projection_instance_admission(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = self.make_producer(tmp)
            with self.assertRaisesRegex(RuntimeError, "projection_instance_gate_unresolved"):
                build_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    document_id=DOCUMENT_ID,
                    chaptera_version="0.1.0-test",
                    platform="windows",
                    binary_sha256=BINARY_SHA,
                    fixture_kind="real_pub_sanitized",
                    projection_instance_admitted=False,
                )

    def test_real_pub_can_enter_builder_only_after_typed_admission(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = self.make_producer(tmp)
            receipt = build_receipt(
                [sys.executable, str(producer)],
                source_hash=SOURCE_HASH,
                document_id=DOCUMENT_ID,
                chaptera_version="0.1.0-test",
                platform="windows",
                binary_sha256=BINARY_SHA,
                fixture_kind="real_pub_sanitized",
                projection_instance_admitted=True,
            )
        self.assertEqual("real_pub_sanitized", receipt["fixture_kind"])

    def test_hosted_native_receipt_uses_same_real_pub_contract(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = self.make_producer(tmp)
            receipt = build_receipt(
                [sys.executable, str(producer)],
                source_hash=SOURCE_HASH,
                document_id=DOCUMENT_ID,
                chaptera_version="0.1.0-ci",
                platform="windows",
                binary_sha256=BINARY_SHA,
                fixture_kind="real_pub_sanitized",
                projection_instance_admitted=True,
                integration="hosted_native",
            )
        summary = validate_receipt(receipt)
        self.assertEqual("hosted_native", receipt["producer"]["integration"])
        self.assertTrue(summary["one_durable_operation"])
        self.assertTrue(summary["transactional_replay"])

    def test_unknown_integration_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = self.make_producer(tmp)
            with self.assertRaisesRegex(RuntimeError, "unsupported producer integration"):
                build_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    document_id=DOCUMENT_ID,
                    chaptera_version="0.1.0-test",
                    platform="windows",
                    binary_sha256=BINARY_SHA,
                    fixture_kind="synthetic_geometry",
                    integration="browser",
                )

    def test_builder_fails_closed_if_producer_changes_source_identity(self):
        broken = FAKE_PRODUCER.replace(
            '"source_hash_after": source_hash,',
            '"source_hash_after": "c" * 64,',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            producer = self.make_producer(tmp, broken)
            with self.assertRaisesRegex(RuntimeError, "changed immutable source identity"):
                build_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    document_id=DOCUMENT_ID,
                    chaptera_version="0.1.0-test",
                    platform="windows",
                    binary_sha256=BINARY_SHA,
                    fixture_kind="synthetic_geometry",
                )

    def test_builder_rejects_failed_negative_probe(self):
        broken = FAKE_PRODUCER.replace(
            '"rejected_no_mutation": True,',
            '"rejected_no_mutation": False,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            producer = self.make_producer(tmp, broken)
            with self.assertRaisesRegex(RuntimeError, "negative probe failed"):
                build_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    document_id=DOCUMENT_ID,
                    chaptera_version="0.1.0-test",
                    platform="windows",
                    binary_sha256=BINARY_SHA,
                    fixture_kind="synthetic_geometry",
                )


if __name__ == "__main__":
    unittest.main()
