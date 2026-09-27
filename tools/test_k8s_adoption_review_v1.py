#!/usr/bin/env python3
"""Deterministic tests for chaptera.k8s-adoption-review.v1."""

from __future__ import annotations

import unittest
from copy import deepcopy
from datetime import datetime, timedelta, timezone

from k8s_adoption_review_v1 import (
    CRITICAL_TRIGGERED,
    NO_TRIGGER_EVIDENCE,
    TRIGGERED,
    UNKNOWN,
    AdoptionEvidenceInvalid,
    reduce_review,
)

NOW = datetime(2026, 9, 27, 9, 0, tzinfo=timezone.utc)


def ts(delta_days: int = 0) -> str:
    return (NOW + timedelta(days=delta_days)).isoformat().replace("+00:00", "Z")


def evidence(
    source_id: str,
    values: dict,
    *,
    authority_class: str = "real_operational",
    fresh_until_days: int = 7,
) -> dict:
    return {
        "source_id": source_id,
        "source_kind": "test-receipt",
        "receipt_version": "fixture.v1",
        "authority_class": authority_class,
        "observed_at": ts(-1),
        "fresh_until": ts(fresh_until_days),
        "values": values,
    }


def prerequisites(complete: bool = True) -> dict:
    names = (
        "semantic_truth_outside_process",
        "separate_startup_readiness_liveness",
        "graceful_drain",
        "durable_job_fencing_and_idempotency",
        "authority_epoch_fencing",
        "schema_migration_compatibility",
        "backup_restore_drill",
        "capacity_metrics",
        "k8s_owner_or_managed_service",
        "cost_comparison",
    )
    return {
        name: {
            "complete": complete,
            "source_id": f"prereq:{name}",
        }
        for name in names
    }


def payload(rows: list[dict], *, prereqs: dict | None = None) -> dict:
    return {
        "input_version": "chaptera.k8s-adoption-evidence-input.v1",
        "reviewed_at": ts(),
        "evidence": rows,
        "prerequisites": prerequisites() if prereqs is None else prereqs,
    }


class K8sAdoptionReviewTests(unittest.TestCase):
    def test_zero_triggers(self) -> None:
        rows = [
            evidence("a", {
                "production_nodes": 1,
                "manual_placement_operations": 0,
                "recurring_manual_reschedule": False,
                "custom_placement_controller": False,
            }),
            evidence("b", {"independent_scalable_workload_pools": 2}),
            evidence("c", {
                "steady_worker_count": 4,
                "peak_worker_count": 8,
                "recurring_manual_capacity_changes": False,
                "specialized_worker_pools": 1,
            }),
            evidence("d", {
                "recurring_manual_deploy_coordination": False,
                "rollout_incident_class": False,
                "recurring_progressive_delivery_need": False,
            }),
            evidence("e", {"automatic_node_loss_survival_required": False}, authority_class="release_contract"),
            evidence("f", {"concurrent_environments": 2, "recurring_customer_fleet_ops": False}),
            evidence("g", {"specialized_node_classes_required": False}, authority_class="release_contract"),
            evidence("h", {"explicit_multi_region_requirement": False, "region_count": 1}, authority_class="release_contract"),
        ]
        result = reduce_review(payload(rows))
        self.assertFalse(result["review_required"])
        self.assertEqual(result["ordinary_trigger_count"], 0)
        self.assertFalse(result["critical_slo_triggered"])
        self.assertTrue(
            all(
                row["state"] == NO_TRIGGER_EVIDENCE
                for row in result["triggers"].values()
            )
        )

    def test_one_ordinary_trigger_does_not_open_review(self) -> None:
        rows = [
            evidence("b", {"independent_scalable_workload_pools": 4}),
            evidence("e", {"automatic_node_loss_survival_required": False}, authority_class="release_contract"),
        ]
        result = reduce_review(payload(rows))
        self.assertEqual(result["triggers"]["B"]["state"], TRIGGERED)
        self.assertEqual(result["ordinary_trigger_count"], 1)
        self.assertFalse(result["review_required"])

    def test_two_independent_triggers_open_review(self) -> None:
        rows = [
            evidence("b", {"independent_scalable_workload_pools": 4}),
            evidence("f", {"concurrent_environments": 5, "recurring_customer_fleet_ops": False}),
        ]
        result = reduce_review(payload(rows))
        self.assertEqual(result["triggers"]["B"]["state"], TRIGGERED)
        self.assertEqual(result["triggers"]["F"]["state"], TRIGGERED)
        self.assertEqual(result["ordinary_trigger_count"], 2)
        self.assertTrue(result["review_required"])

    def test_critical_e_opens_review_alone(self) -> None:
        rows = [
            evidence(
                "e",
                {
                    "automatic_node_loss_survival_required": True,
                    "current_topology_satisfies_node_loss_slo_without_operator": False,
                },
                authority_class="release_contract",
            )
        ]
        result = reduce_review(payload(rows))
        self.assertEqual(result["triggers"]["E"]["state"], CRITICAL_TRIGGERED)
        self.assertTrue(result["critical_slo_triggered"])
        self.assertTrue(result["review_required"])

    def test_stale_evidence_fails_closed_to_unknown(self) -> None:
        rows = [
            evidence(
                "b-stale",
                {"independent_scalable_workload_pools": 8},
                fresh_until_days=-1,
            )
        ]
        result = reduce_review(payload(rows))
        self.assertEqual(result["triggers"]["B"]["state"], UNKNOWN)
        self.assertEqual(result["ordinary_trigger_count"], 0)
        self.assertFalse(result["review_required"])
        self.assertEqual(result["sources"][0]["freshness_state"], "STALE")

    def test_synthetic_evidence_cannot_trigger(self) -> None:
        rows = [
            evidence(
                "synthetic-b",
                {"independent_scalable_workload_pools": 20},
                authority_class="synthetic",
            )
        ]
        result = reduce_review(payload(rows))
        self.assertEqual(result["triggers"]["B"]["state"], UNKNOWN)
        self.assertFalse(result["review_required"])
        self.assertEqual(result["sources"][0]["authority_state"], "SYNTHETIC_ONLY")

    def test_contradictory_authoritative_inputs_fail_closed(self) -> None:
        rows = [
            evidence("b1", {"independent_scalable_workload_pools": 3}),
            evidence("b2", {"independent_scalable_workload_pools": 4}),
        ]
        result = reduce_review(payload(rows))
        self.assertEqual(result["triggers"]["B"]["state"], UNKNOWN)
        self.assertIn("contradictory", result["triggers"]["B"]["reason"])
        self.assertFalse(result["review_required"])

    def test_high_cpu_without_placement_evidence_does_not_trigger_a(self) -> None:
        rows = [
            evidence("capacity", {"cpu_percent": 99, "rss_bytes": 2_000_000_000})
        ]
        result = reduce_review(payload(rows))
        self.assertEqual(result["triggers"]["A"]["state"], UNKNOWN)
        self.assertFalse(result["review_required"])

    def test_missing_prerequisites_are_reported_but_do_not_change_gate_math(self) -> None:
        rows = [
            evidence("b", {"independent_scalable_workload_pools": 4}),
            evidence("f", {"concurrent_environments": 5}),
        ]
        result = reduce_review(payload(rows, prereqs={}))
        self.assertTrue(result["review_required"])
        self.assertFalse(result["prerequisites"]["all_complete"])
        self.assertEqual(
            len(result["prerequisites"]["missing"]),
            len(result["prerequisites"]["items"]),
        )

    def test_invalid_input_version_is_rejected(self) -> None:
        data = payload([])
        data["input_version"] = "wrong"
        with self.assertRaises(AdoptionEvidenceInvalid):
            reduce_review(data)

    def test_duplicate_source_identity_is_rejected(self) -> None:
        row = evidence("same", {"independent_scalable_workload_pools": 4})
        with self.assertRaises(AdoptionEvidenceInvalid):
            reduce_review(payload([row, deepcopy(row)]))


if __name__ == "__main__":
    unittest.main()
