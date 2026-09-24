#!/usr/bin/env python3
import copy
import json
import pathlib
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_revision_producer_receipt import (
    PINNED_SOURCE_HASH,
    build_receipt,
)
from validate_revision_producer_receipt import validate_schema, validate_semantics


FAKE_PRODUCER = r"""#!/usr/bin/env python3
import copy
import json
import sys

payload = json.load(sys.stdin)
source_hash = payload["source_hash"]
node_id = "30000000-0000-4000-8000-000000000001"

if payload["action"] == "baseline":
    json.dump({
        "source_hash": source_hash,
        "baseline_project": {
            "schema_version": "pub-editor-v0.2",
            "source_hash": source_hash,
            "operations": [],
        },
        "move_candidate": {
            "node_id": node_id,
            "before": {"x": 0, "y": 0, "width": 1828800, "height": 914400},
        },
    }, sys.stdout)
    raise SystemExit(0)

if payload["action"] != "commit":
    raise SystemExit(2)

command = payload["command"]
before = {"x": 0, "y": 0, "width": 1828800, "height": 914400}
after = {
    "x": command["x_emu"],
    "y": command["y_emu"],
    "width": before["width"],
    "height": before["height"],
}
operation = {
    "kind": "move_node",
    "node_id": command["node_id"],
    "before": before,
    "after": after,
}
project = copy.deepcopy(payload["base_project"])
project["schema_version"] = "pub-editor-v0.4"
project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]

json.dump({
    "canonical_operation": operation,
    "resulting_project": project,
    "replayed_project": copy.deepcopy(project),
    "consequences": [
        {"key": "node.geometry.position", "state": "supported", "note": None}
    ],
    "source_hash_after": source_hash,
    "source_hash_replay": source_hash,
}, sys.stdout)
"""


class RevisionProducerBuilderTests(unittest.TestCase):
    def test_builder_reuses_revision_kernel_and_emits_valid_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = pathlib.Path(tmp) / "fake_producer.py"
            producer.write_text(FAKE_PRODUCER, encoding="utf-8")
            receipt = build_receipt(
                [sys.executable, str(producer)],
                implementation="fake-canonical-editor",
                commit_or_build="test-build",
            )

        validate_schema(receipt)
        summary = validate_semantics(receipt)

        self.assertEqual(PINNED_SOURCE_HASH, receipt["source_hash"])
        self.assertEqual("pub-editor-v0.2", receipt["baseline"]["project"]["schema_version"])
        self.assertEqual("pub-editor-v0.4", receipt["resulting_project"]["schema_version"])
        self.assertEqual(
            receipt["resulting_project"],
            receipt["replayed_project"],
        )
        self.assertEqual(1, receipt["probes"]["exact_retry"]["executor_calls_total"])
        self.assertEqual(0, receipt["probes"]["stale_base"]["executor_calls_delta"])
        self.assertEqual(
            0,
            receipt["probes"]["idempotency_conflict"]["executor_calls_delta"],
        )
        self.assertTrue(summary["idempotent_retry_single_execution"])


if __name__ == "__main__":
    unittest.main()
