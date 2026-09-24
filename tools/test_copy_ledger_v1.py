#!/usr/bin/env python3
import copy
import unittest

from copy_ledger_v1 import COPY_LEDGER_SCHEMA, copy_ledger_measurement
from optimization_receipt_v1 import OptimizationIncompatible, compare_measurements


def receipt(*, avoidable=4096, allocated=12000, peak=8000, real=False):
    return {
        "receipt_version": COPY_LEDGER_SCHEMA,
        "runtime": {
            "runtime": "rust-1.85",
            "platform": "linux",
            "machine": "x86_64",
            "runner_os": "Linux",
            "runner_arch": "X64",
        },
        "workload": {
            "fixture_binding_id": "opaque-fixture-1",
            "document_class": "medium",
            "pages": 12,
            "stories": 30,
            "resources": 8,
        },
        "evidence_authority": {
            "real_product_source_free": real,
            "synthetic_or_public_fixture": not real,
        },
        "scenarios": [
            {
                "name": "small_story_edit",
                "frequency_class": "per_edit",
                "allocations": 40,
                "allocated_bytes": allocated,
                "peak_live_bytes": peak,
                "serialized_bytes": 256,
                "tracked_unique_payload_bytes": 2048,
                "correctness": {
                    "semantic_equivalent": True,
                    "output_equivalent": True,
                },
                "copy_sites": [
                    {
                        "site_id": "editor.history.before_after",
                        "payload_class": "story_text",
                        "classification": "AVOIDABLE_DUPLICATE",
                        "bytes_total": avoidable,
                        "events": 2 if avoidable else 0,
                        "measurement_method": "explicit_copy_counter",
                        "from_stage": "authoring.story",
                        "to_stage": "editor.history",
                    },
                    {
                        "site_id": "project.serialize",
                        "payload_class": "story_text",
                        "classification": "REQUIRED_SERIALIZATION",
                        "bytes_total": 256,
                        "events": 1,
                        "measurement_method": "serializer_count",
                        "from_stage": "editor.history",
                        "to_stage": "project.bytes",
                    },
                ],
            }
        ],
        "limitations": ["synthetic contract fixture"],
    }


class CopyLedgerTests(unittest.TestCase):
    def test_recomputes_copy_totals_and_ranks_avoidable_sites(self):
        snap = copy_ledger_measurement(receipt(), {"repository": "HeisLuka/rar", "sha": "a"})
        self.assertEqual(4352, snap["metrics"]["scenario.small_story_edit.materialized_bytes"]["value"])
        self.assertEqual(4096, snap["metrics"]["scenario.small_story_edit.avoidable_duplicate_bytes"]["value"])
        self.assertEqual(4096, snap["metrics"]["payload.story_text.avoidable_duplicate_bytes"]["value"])
        self.assertEqual("editor.history.before_after", snap["copy_ledger"]["ranked_avoidable_sites"][0]["site_id"])
        self.assertFalse(snap["evidence_authority"]["technology_decision_allowed"])

    def test_baseline_candidate_uses_existing_optimization_comparator(self):
        baseline = copy_ledger_measurement(receipt(avoidable=4096), {"repository": "HeisLuka/rar", "sha": "base"})
        candidate = copy_ledger_measurement(
            receipt(avoidable=256, allocated=7000, peak=5000),
            {"repository": "HeisLuka/rar", "sha": "cand"},
        )
        result = compare_measurements(
            optimization_id="ENGINE-COPY-LEDGER-01",
            hot_path="story-edit",
            baseline=baseline,
            candidate=candidate,
            budgets={
                "scenario.small_story_edit.avoidable_duplicate_bytes": {"max_regression_pct": 0},
                "scenario.small_story_edit.peak_live_bytes": {"max_regression_pct": 5},
            },
            correctness_equivalent=all(candidate["correctness"].values()),
            fidelity_equivalent=True,
        )
        self.assertEqual("keep", result["decision"])
        self.assertLess(
            result["metric_comparisons"]["scenario.small_story_edit.avoidable_duplicate_bytes"]["delta"],
            0,
        )

    def test_real_source_free_receipt_can_authorize_technology_scope(self):
        snap = copy_ledger_measurement(receipt(real=True), {"repository": "HeisLuka/rar", "sha": "a"})
        self.assertTrue(snap["evidence_authority"]["technology_decision_allowed"])

    def test_rejects_sensitive_public_fields(self):
        bad = receipt()
        bad["workload"]["source_path"] = "/home/user/private.pub"
        with self.assertRaisesRegex(OptimizationIncompatible, "forbidden key"):
            copy_ledger_measurement(bad, {"repository": "HeisLuka/rar", "sha": "a"})

    def test_rejects_unclassified_copy_site(self):
        bad = receipt()
        bad["scenarios"][0]["copy_sites"][0]["classification"] = "MAYBE"
        with self.assertRaisesRegex(OptimizationIncompatible, "unsupported copy classification"):
            copy_ledger_measurement(bad, {"repository": "HeisLuka/rar", "sha": "a"})

    def test_missing_denominator_is_unknown_not_fabricated(self):
        data = receipt()
        del data["scenarios"][0]["tracked_unique_payload_bytes"]
        snap = copy_ledger_measurement(data, {"repository": "HeisLuka/rar", "sha": "a"})
        self.assertEqual(
            "unknown",
            snap["metrics"]["scenario.small_story_edit.copy_amplification_ratio"]["state"],
        )

    def test_duplicate_scenario_names_fail_closed(self):
        bad = receipt()
        bad["scenarios"].append(copy.deepcopy(bad["scenarios"][0]))
        with self.assertRaisesRegex(OptimizationIncompatible, "duplicate scenario name"):
            copy_ledger_measurement(bad, {"repository": "HeisLuka/rar", "sha": "a"})


if __name__ == "__main__":
    unittest.main()
