#!/usr/bin/env python3
import copy
import unittest

from optimization_receipt_v1 import (
    MEASUREMENT_SCHEMA,
    OptimizationIncompatible,
    compare_measurements,
    copy_ledger_measurement,
    observed,
    unknown,
)

def snapshot(*,sha="a",runtime_tag="linux",workload="w1",synthetic=True,latency=10.0,memory=100):
    workload_identity={"fixture":workload,"nodes":100}
    from optimization_receipt_v1 import identity_hash
    return {
        "schema":MEASUREMENT_SCHEMA,
        "producer":{"receipt_version":"test.bench.v1","measurement_class":"test"},
        "build_identity":{"repository":"HeisLuka/rar","sha":sha},
        "runtime_identity":{"python":"3.12","platform":runtime_tag,"machine":"x86_64","runner_os":"Linux","runner_arch":"X64"},
        "workload_identity":workload_identity,
        "workload_identity_hash":identity_hash(workload_identity),
        "correctness":{"equivalent":True},
        "evidence_authority":{
            "real_product_corpus":not synthetic,
            "synthetic_or_product_grounded_public":synthetic,
            "technology_decision_allowed":not synthetic,
            "blocker":"synthetic" if synthetic else None,
        },
        "metrics":{
            "latency":observed(latency,"ms","lower_is_better",samples=5),
            "memory":observed(memory,"bytes","lower_is_better"),
            "gpu":unknown("not measured","ms","lower_is_better"),
        },
        "limitations":[],
    }

class OptimizationReceiptTests(unittest.TestCase):
    def test_copy_ledger_normalizes_into_shared_measurement_spine(self):
        from copy_ledger_v1 import synthetic_contract_fixture

        receipt = synthetic_contract_fixture()
        snap = copy_ledger_measurement(
            receipt,
            {"repository": "HeisLuka/rar", "sha": receipt["producer"]["build_sha"]},
        )
        self.assertEqual(MEASUREMENT_SCHEMA, snap["schema"])
        self.assertEqual("chaptera.copy-ledger.v1", snap["producer"]["receipt_version"])
        self.assertEqual(
            2_000_000,
            snap["metrics"]["copy.avoidable_duplicate_bytes"]["value"],
        )
        self.assertFalse(snap["evidence_authority"]["technology_decision_allowed"])

    def test_copy_ledger_candidate_compares_without_a_second_scoring_system(self):
        from copy_ledger_v1 import synthetic_contract_fixture, with_summary

        baseline_receipt = synthetic_contract_fixture()
        candidate_receipt = synthetic_contract_fixture()
        candidate_receipt["events"][1]["materialized_bytes"] = 200_000
        candidate_receipt.pop("summary", None)
        candidate_receipt = with_summary(candidate_receipt)

        baseline = copy_ledger_measurement(
            baseline_receipt,
            {"repository": "HeisLuka/rar", "sha": "copy-base"},
        )
        candidate = copy_ledger_measurement(
            candidate_receipt,
            {"repository": "HeisLuka/rar", "sha": "copy-candidate"},
        )
        result = compare_measurements(
            optimization_id="ENGINE-COPY-LEDGER-01",
            hot_path="story-history",
            baseline=baseline,
            candidate=candidate,
            budgets={"copy.avoidable_duplicate_bytes": {"max_regression_pct": 0}},
            correctness_equivalent=True,
            fidelity_equivalent=True,
        )
        self.assertEqual("keep", result["decision"])
        self.assertLess(
            result["metric_comparisons"]["copy.avoidable_duplicate_bytes"]["delta"],
            0,
        )


    def test_metric_specific_budget_keeps_improvement_without_global_score(self):
        result=compare_measurements(
            optimization_id="OPT-1",
            hot_path="patch",
            baseline=snapshot(sha="base",latency=10,memory=100),
            candidate=snapshot(sha="cand",latency=7,memory=104),
            budgets={
                "latency":{"max_regression_pct":5},
                "memory":{"max_regression_pct":5},
            },
            correctness_equivalent=True,
            fidelity_equivalent=True,
            trade_offs=["memory +4%"],
            new_bottleneck="hashing",
        )
        self.assertEqual("keep",result["decision"])
        self.assertEqual([],result["budget_violations"])
        self.assertLess(result["metric_comparisons"]["latency"]["regression_pct"],0)
        self.assertNotIn("score",result)
        self.assertNotIn("performance_score",result)

    def test_regression_budget_violation_reverts(self):
        result=compare_measurements(
            optimization_id="OPT-2",
            hot_path="patch",
            baseline=snapshot(sha="base",latency=10),
            candidate=snapshot(sha="cand",latency=12),
            budgets={"latency":{"max_regression_pct":5}},
            correctness_equivalent=True,
            fidelity_equivalent=True,
        )
        self.assertEqual("revert",result["decision"])
        self.assertEqual(["latency"],result["budget_violations"])

    def test_correctness_failure_overrides_performance_win(self):
        result=compare_measurements(
            optimization_id="OPT-3",
            hot_path="patch",
            baseline=snapshot(sha="base",latency=10),
            candidate=snapshot(sha="cand",latency=1),
            budgets={"latency":{"max_regression_pct":5}},
            correctness_equivalent=False,
            fidelity_equivalent=True,
        )
        self.assertEqual("revert",result["decision"])
        self.assertFalse(result["correctness_fidelity_fence"]["passed"])

    def test_synthetic_evidence_cannot_authorize_product_technology_decision(self):
        result=compare_measurements(
            optimization_id="OPT-4",
            hot_path="renderer-backend",
            baseline=snapshot(sha="base",latency=10,synthetic=True),
            candidate=snapshot(sha="cand",latency=5,synthetic=True),
            budgets={"latency":{"max_regression_pct":5}},
            correctness_equivalent=True,
            fidelity_equivalent=True,
            decision_scope="product_technology",
        )
        self.assertEqual("needs_real_corpus_validation",result["decision"])
        self.assertFalse(result["evidence_authority"]["technology_decision_allowed"])

    def test_unknown_budget_metric_stays_unknown_not_zero(self):
        baseline=snapshot(sha="base")
        candidate=snapshot(sha="cand")
        result=compare_measurements(
            optimization_id="OPT-5",
            hot_path="gpu",
            baseline=baseline,
            candidate=candidate,
            budgets={"gpu":{"max_regression_pct":5}},
            correctness_equivalent=True,
            fidelity_equivalent=True,
        )
        self.assertEqual("unknown",result["metric_comparisons"]["gpu"]["state"])
        self.assertEqual("needs_real_corpus_validation",result["decision"])
        self.assertIn("gpu",result["unevaluable_budgets"])

    def test_incompatible_runtime_fails_closed(self):
        with self.assertRaisesRegex(OptimizationIncompatible,"runtime_identity_mismatch"):
            compare_measurements(
                optimization_id="OPT-6",
                hot_path="patch",
                baseline=snapshot(sha="base",runtime_tag="linux-a"),
                candidate=snapshot(sha="cand",runtime_tag="linux-b"),
                correctness_equivalent=True,
                fidelity_equivalent=True,
            )

    def test_incompatible_workload_fails_closed(self):
        with self.assertRaisesRegex(OptimizationIncompatible,"workload_identity_mismatch"):
            compare_measurements(
                optimization_id="OPT-7",
                hot_path="patch",
                baseline=snapshot(sha="base",workload="w1"),
                candidate=snapshot(sha="cand",workload="w2"),
                correctness_equivalent=True,
                fidelity_equivalent=True,
            )

if __name__=="__main__":
    unittest.main()
