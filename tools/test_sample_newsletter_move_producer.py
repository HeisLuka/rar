#!/usr/bin/env python3
import copy
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_revision_producer_receipt import build_receipt
from sample_newsletter_move_producer import (
    ProducerError,
    SampleNewsletterMoveSession,
    handle,
    load_baseline,
)
from validate_revision_producer_receipt import validate_schema, validate_semantics

SOURCE_HASH = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
NODE_ID = "007d9898-568b-5125-b519-8d88243aabfb"
BEFORE = {"x": 526710, "y": 1191292, "width": 4436165, "height": 587274}
AFTER = {"x": 653710, "y": 1445292, "width": 4436165, "height": 587274}


class SampleNewsletterMoveProducerTests(unittest.TestCase):
    def test_real_baseline_is_minimal_and_move_eligible(self):
        baseline = load_baseline()
        self.assertEqual(SOURCE_HASH, baseline["source"]["sha256"])
        self.assertEqual(291840, baseline["source"]["byte_len"])
        self.assertEqual(NODE_ID, baseline["move_candidate"]["node_id"])
        self.assertEqual(BEFORE, baseline["move_candidate"]["before"])
        self.assertIn(
            baseline["move_candidate"]["parent_page_id"],
            baseline["page_ids"],
        )
        self.assertNotIn("text", str(baseline).lower())
        self.assertNotIn("source_refs", baseline["move_candidate"])

    def test_commit_derives_before_and_replays_exact_project(self):
        baseline_output = handle({"action": "baseline", "source_hash": SOURCE_HASH})
        self.assertEqual(
            {
                "schema_version": "pub-editor-v0.2",
                "source_hash": SOURCE_HASH,
                "operations": [],
            },
            baseline_output["baseline_project"],
        )

        result = handle({
            "action": "commit",
            "source_hash": SOURCE_HASH,
            "base_project": baseline_output["baseline_project"],
            "command": {
                "kind": "move_node_to",
                "node_id": NODE_ID,
                "x_emu": AFTER["x"],
                "y_emu": AFTER["y"],
            },
        })
        self.assertEqual(
            {
                "kind": "move_node",
                "node_id": NODE_ID,
                "before": BEFORE,
                "after": AFTER,
            },
            result["canonical_operation"],
        )
        self.assertEqual("pub-editor-v0.4", result["resulting_project"]["schema_version"])
        self.assertEqual(result["resulting_project"], result["replayed_project"])
        self.assertEqual(SOURCE_HASH, result["source_hash_after"])
        self.assertEqual(SOURCE_HASH, result["source_hash_replay"])

    def test_replay_rejects_hidden_resize(self):
        baseline = load_baseline()
        session = SampleNewsletterMoveSession(baseline)
        operation = session.move_node_to(NODE_ID, AFTER["x"], AFTER["y"])
        project = session.project()
        project["operations"][0]["after"]["width"] += 1

        fresh = SampleNewsletterMoveSession(baseline)
        with self.assertRaisesRegex(ProducerError, "canonical operation mismatch"):
            fresh.apply_project(project)

        self.assertEqual([], fresh.operations)
        self.assertEqual(BEFORE, fresh.current_rect)

    def test_wrong_source_and_wrong_node_fail_closed(self):
        with self.assertRaisesRegex(ProducerError, "source hash mismatch"):
            handle({"action": "baseline", "source_hash": "0" * 64})

        baseline = handle({"action": "baseline", "source_hash": SOURCE_HASH})
        with self.assertRaisesRegex(ProducerError, "node_move_unsupported"):
            handle({
                "action": "commit",
                "source_hash": SOURCE_HASH,
                "base_project": baseline["baseline_project"],
                "command": {
                    "kind": "move_node_to",
                    "node_id": "00000000-0000-4000-8000-000000000000",
                    "x_emu": AFTER["x"],
                    "y_emu": AFTER["y"],
                },
            })

    def test_existing_rar_builder_accepts_real_fixture_slice(self):
        receipt = build_receipt(
            [sys.executable, str(TOOLS / "sample_newsletter_move_producer.py")],
            implementation="chaptera-rar-samplenewsletter-move-slice",
            commit_or_build="test-real-fixture-slice",
        )
        validate_schema(receipt)
        summary = validate_semantics(receipt)

        self.assertEqual(NODE_ID, receipt["request"]["command"]["node_id"])
        self.assertEqual(BEFORE, receipt["accepted"]["canonical_operation"]["before"])
        self.assertEqual(AFTER, receipt["accepted"]["canonical_operation"]["after"])
        self.assertEqual(
            receipt["resulting_project"],
            receipt["replayed_project"],
        )
        self.assertTrue(summary["replay_equal"])
        self.assertTrue(summary["idempotent_retry_single_execution"])


if __name__ == "__main__":
    unittest.main()
