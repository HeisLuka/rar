#!/usr/bin/env python3
"""Machine-readable Chaptera Kubernetes adoption review reducer.

This module intentionally decides only whether an architecture review is
required. It never recommends or selects Kubernetes and never emits manifests.
"""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

INPUT_VERSION = "chaptera.k8s-adoption-evidence-input.v1"
RECEIPT_VERSION = "chaptera.k8s-adoption-review.v1"

UNKNOWN = "UNKNOWN"
NO_TRIGGER_EVIDENCE = "NO_TRIGGER_EVIDENCE"
TRIGGERED = "TRIGGERED"
CRITICAL_TRIGGERED = "CRITICAL_TRIGGERED"

TRIGGERS = tuple("ABCDEFGH")
ORDINARY_TRIGGERS = ("A", "B", "C", "D", "F", "G", "H")
AUTHORITY_CLASSES = {"real_operational", "release_contract", "synthetic"}

PREREQUISITES = (
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

TRIGGER_KEYS = {
    "A": {
        "production_nodes",
        "manual_placement_operations",
        "recurring_manual_reschedule",
        "custom_placement_controller",
    },
    "B": {"independent_scalable_workload_pools"},
    "C": {
        "steady_worker_count",
        "peak_worker_count",
        "recurring_manual_capacity_changes",
        "specialized_worker_pools",
    },
    "D": {
        "recurring_manual_deploy_coordination",
        "rollout_incident_class",
        "recurring_progressive_delivery_need",
    },
    "E": {
        "automatic_node_loss_survival_required",
        "current_topology_satisfies_node_loss_slo_without_operator",
    },
    "F": {"concurrent_environments", "recurring_customer_fleet_ops"},
    "G": {
        "persistent_specialized_node_classes",
        "hardware_class_placement_problem",
        "specialized_node_classes_required",
    },
    "H": {"explicit_multi_region_requirement", "region_count"},
}


class AdoptionEvidenceInvalid(ValueError):
    pass


def _parse_time(value: Any, path: str) -> datetime:
    if not isinstance(value, str) or not value:
        raise AdoptionEvidenceInvalid(f"{path} must be a non-empty RFC3339 timestamp")
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError as exc:
        raise AdoptionEvidenceInvalid(f"{path} must be RFC3339") from exc
    if parsed.tzinfo is None:
        raise AdoptionEvidenceInvalid(f"{path} must include timezone")
    return parsed.astimezone(timezone.utc)


def _require_non_negative_int(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise AdoptionEvidenceInvalid(f"{path} must be a non-negative integer")
    return value


def _require_bool(value: Any, path: str) -> bool:
    if not isinstance(value, bool):
        raise AdoptionEvidenceInvalid(f"{path} must be boolean")
    return value


def _validate_input(payload: Any) -> tuple[dict[str, Any], datetime]:
    if not isinstance(payload, dict):
        raise AdoptionEvidenceInvalid("input must be an object")
    if payload.get("input_version") != INPUT_VERSION:
        raise AdoptionEvidenceInvalid("input_version mismatch")

    reviewed_at = _parse_time(payload.get("reviewed_at"), "reviewed_at")

    evidence = payload.get("evidence")
    if not isinstance(evidence, list):
        raise AdoptionEvidenceInvalid("evidence must be an array")

    seen_ids: set[str] = set()
    for i, row in enumerate(evidence):
        path = f"evidence[{i}]"
        if not isinstance(row, dict):
            raise AdoptionEvidenceInvalid(f"{path} must be an object")
        source_id = row.get("source_id")
        if not isinstance(source_id, str) or not source_id:
            raise AdoptionEvidenceInvalid(f"{path}.source_id is required")
        if source_id in seen_ids:
            raise AdoptionEvidenceInvalid(f"duplicate source_id: {source_id}")
        seen_ids.add(source_id)

        source_kind = row.get("source_kind")
        if not isinstance(source_kind, str) or not source_kind:
            raise AdoptionEvidenceInvalid(f"{path}.source_kind is required")

        receipt_version = row.get("receipt_version")
        if receipt_version is not None and (
            not isinstance(receipt_version, str) or not receipt_version
        ):
            raise AdoptionEvidenceInvalid(f"{path}.receipt_version must be a string")

        authority = row.get("authority_class")
        if authority not in AUTHORITY_CLASSES:
            raise AdoptionEvidenceInvalid(
                f"{path}.authority_class must be one of {sorted(AUTHORITY_CLASSES)}"
            )

        observed_at = _parse_time(row.get("observed_at"), f"{path}.observed_at")
        fresh_until = _parse_time(row.get("fresh_until"), f"{path}.fresh_until")
        if fresh_until < observed_at:
            raise AdoptionEvidenceInvalid(
                f"{path}.fresh_until must not precede observed_at"
            )

        values = row.get("values")
        if not isinstance(values, dict):
            raise AdoptionEvidenceInvalid(f"{path}.values must be an object")

    prerequisites = payload.get("prerequisites", {})
    if not isinstance(prerequisites, dict):
        raise AdoptionEvidenceInvalid("prerequisites must be an object")
    for name, row in prerequisites.items():
        if name not in PREREQUISITES:
            raise AdoptionEvidenceInvalid(f"unsupported prerequisite: {name}")
        if not isinstance(row, dict):
            raise AdoptionEvidenceInvalid(f"prerequisites.{name} must be an object")
        _require_bool(row.get("complete"), f"prerequisites.{name}.complete")
        source_id = row.get("source_id")
        if source_id is not None and (
            not isinstance(source_id, str) or not source_id
        ):
            raise AdoptionEvidenceInvalid(
                f"prerequisites.{name}.source_id must be a string"
            )

    return payload, reviewed_at


def _source_summary(row: dict[str, Any], reviewed_at: datetime) -> dict[str, Any]:
    observed_at = _parse_time(row["observed_at"], "observed_at")
    fresh_until = _parse_time(row["fresh_until"], "fresh_until")
    freshness_state = "FRESH" if reviewed_at <= fresh_until else "STALE"
    if row["authority_class"] == "synthetic":
        authority_state = "SYNTHETIC_ONLY"
    else:
        authority_state = "AUTHORITATIVE"
    return {
        "source_id": row["source_id"],
        "source_kind": row["source_kind"],
        "receipt_version": row.get("receipt_version"),
        "authority_class": row["authority_class"],
        "observed_at": observed_at.isoformat().replace("+00:00", "Z"),
        "fresh_until": fresh_until.isoformat().replace("+00:00", "Z"),
        "freshness_state": freshness_state,
        "authority_state": authority_state,
    }


def _eligible_rows(
    evidence: Iterable[dict[str, Any]],
    reviewed_at: datetime,
    trigger: str,
) -> list[dict[str, Any]]:
    keys = TRIGGER_KEYS[trigger]
    eligible = []
    for row in evidence:
        if row["authority_class"] == "synthetic":
            continue
        if reviewed_at > _parse_time(row["fresh_until"], "fresh_until"):
            continue
        if keys.intersection(row["values"]):
            eligible.append(row)
    return eligible


def _values_for_key(
    rows: Iterable[dict[str, Any]],
    key: str,
) -> tuple[Any | None, list[str], bool]:
    found: list[tuple[str, Any]] = []
    for row in rows:
        if key in row["values"]:
            found.append((row["source_id"], row["values"][key]))
    if not found:
        return None, [], False

    first = found[0][1]
    conflicting = any(value != first for _, value in found[1:])
    return first, [source_id for source_id, _ in found], conflicting


def _result(
    state: str,
    rows: Iterable[dict[str, Any]],
    values: dict[str, Any],
    reason: str,
) -> dict[str, Any]:
    return {
        "state": state,
        "source_ids": sorted({row["source_id"] for row in rows}),
        "measured_values": values,
        "reason": reason,
    }


def _conflict_or_value(
    rows: list[dict[str, Any]],
    key: str,
    validator,
) -> tuple[Any | None, list[str], str | None]:
    value, source_ids, conflict = _values_for_key(rows, key)
    if conflict:
        return None, source_ids, f"contradictory fresh authoritative values for {key}"
    if value is None:
        return None, source_ids, None
    try:
        return validator(value, key), source_ids, None
    except AdoptionEvidenceInvalid as exc:
        return None, source_ids, str(exc)


def _evaluate_a(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, "no fresh authoritative placement evidence")
    nodes, _, err = _conflict_or_value(rows, "production_nodes", _require_non_negative_int)
    if err:
        return _result(UNKNOWN, rows, {}, err)
    manual, _, err2 = _conflict_or_value(
        rows, "manual_placement_operations", _require_non_negative_int
    )
    recurring, _, err3 = _conflict_or_value(
        rows, "recurring_manual_reschedule", _require_bool
    )
    custom, _, err4 = _conflict_or_value(
        rows, "custom_placement_controller", _require_bool
    )
    if err2 or err3 or err4:
        return _result(UNKNOWN, rows, {}, err2 or err3 or err4 or "")
    if nodes is None:
        return _result(UNKNOWN, rows, {}, "production_nodes is missing")

    pressure_known = any(v is not None for v in (manual, recurring, custom))
    if not pressure_known:
        return _result(
            UNKNOWN,
            rows,
            {"production_nodes": nodes},
            "placement pressure evidence is missing",
        )
    manual = 0 if manual is None else manual
    recurring = False if recurring is None else recurring
    custom = False if custom is None else custom
    measured = {
        "production_nodes": nodes,
        "manual_placement_operations": manual,
        "recurring_manual_reschedule": recurring,
        "custom_placement_controller": custom,
    }
    triggered = nodes >= 3 and (manual > 0 or recurring or custom)
    return _result(
        TRIGGERED if triggered else NO_TRIGGER_EVIDENCE,
        rows,
        measured,
        "multi-host placement is an active operational problem"
        if triggered
        else "no qualifying multi-host placement problem is evidenced",
    )


def _evaluate_b(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, "no fresh authoritative workload-pool evidence")
    pools, _, err = _conflict_or_value(
        rows, "independent_scalable_workload_pools", _require_non_negative_int
    )
    if err:
        return _result(UNKNOWN, rows, {}, err)
    if pools is None:
        return _result(UNKNOWN, rows, {}, "independent_scalable_workload_pools is missing")
    measured = {"independent_scalable_workload_pools": pools}
    return _result(
        TRIGGERED if pools >= 4 else NO_TRIGGER_EVIDENCE,
        rows,
        measured,
        "four or more independently scalable workload pools"
        if pools >= 4
        else "fewer than four independently scalable workload pools",
    )


def _evaluate_c(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, "no fresh authoritative worker-elasticity evidence")

    steady, _, err1 = _conflict_or_value(
        rows, "steady_worker_count", _require_non_negative_int
    )
    peak, _, err2 = _conflict_or_value(
        rows, "peak_worker_count", _require_non_negative_int
    )
    manual, _, err3 = _conflict_or_value(
        rows, "recurring_manual_capacity_changes", _require_bool
    )
    specialized, _, err4 = _conflict_or_value(
        rows, "specialized_worker_pools", _require_non_negative_int
    )
    if err1 or err2 or err3 or err4:
        return _result(UNKNOWN, rows, {}, err1 or err2 or err3 or err4 or "")

    ratio = None
    if steady is not None and peak is not None:
        if steady == 0:
            ratio = None if peak == 0 else float("inf")
        else:
            ratio = peak / steady

    if ratio is None and manual is None and specialized is None:
        return _result(UNKNOWN, rows, {}, "worker elasticity inputs are incomplete")

    manual = False if manual is None else manual
    specialized = 0 if specialized is None else specialized
    ratio_trigger = ratio is not None and ratio >= 5.0
    triggered = ratio_trigger or manual or specialized >= 2
    measured = {
        "steady_worker_count": steady,
        "peak_worker_count": peak,
        "peak_to_steady_ratio": ratio,
        "recurring_manual_capacity_changes": manual,
        "specialized_worker_pools": specialized,
    }
    return _result(
        TRIGGERED if triggered else NO_TRIGGER_EVIDENCE,
        rows,
        measured,
        "worker elasticity/manual capacity/specialized pools reached a review trigger"
        if triggered
        else "worker elasticity evidence is below review triggers",
    )


def _evaluate_boolean_any(
    rows: list[dict[str, Any]],
    keys: tuple[str, ...],
    label: str,
) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, f"no fresh authoritative {label} evidence")
    measured: dict[str, Any] = {}
    seen = False
    for key in keys:
        value, _, err = _conflict_or_value(rows, key, _require_bool)
        if err:
            return _result(UNKNOWN, rows, {}, err)
        if value is not None:
            measured[key] = value
            seen = True
    if not seen:
        return _result(UNKNOWN, rows, {}, f"{label} inputs are missing")
    triggered = any(measured.values())
    return _result(
        TRIGGERED if triggered else NO_TRIGGER_EVIDENCE,
        rows,
        measured,
        f"{label} is a recurring operational problem"
        if triggered
        else f"no recurring {label} problem is evidenced",
    )


def _evaluate_e(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, "no fresh authoritative node-failure SLO evidence")
    required, _, err1 = _conflict_or_value(
        rows, "automatic_node_loss_survival_required", _require_bool
    )
    satisfies, _, err2 = _conflict_or_value(
        rows,
        "current_topology_satisfies_node_loss_slo_without_operator",
        _require_bool,
    )
    if err1 or err2:
        return _result(UNKNOWN, rows, {}, err1 or err2 or "")
    if required is None:
        return _result(
            UNKNOWN,
            rows,
            {},
            "automatic_node_loss_survival_required is missing",
        )
    measured = {
        "automatic_node_loss_survival_required": required,
        "current_topology_satisfies_node_loss_slo_without_operator": satisfies,
    }
    if not required:
        return _result(
            NO_TRIGGER_EVIDENCE,
            rows,
            measured,
            "current product/SLO contract does not require automatic node-loss survival",
        )
    if satisfies is None:
        return _result(
            UNKNOWN,
            rows,
            measured,
            "node-loss SLO is required but current topology capability is unknown",
        )
    if satisfies:
        return _result(
            NO_TRIGGER_EVIDENCE,
            rows,
            measured,
            "current topology already satisfies the explicit node-loss SLO",
        )
    return _result(
        CRITICAL_TRIGGERED,
        rows,
        measured,
        "explicit node-loss SLO requires automatic survival and current topology cannot satisfy it",
    )


def _evaluate_f(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, "no fresh authoritative environment-fleet evidence")
    count, _, err1 = _conflict_or_value(
        rows, "concurrent_environments", _require_non_negative_int
    )
    recurring, _, err2 = _conflict_or_value(
        rows, "recurring_customer_fleet_ops", _require_bool
    )
    if err1 or err2:
        return _result(UNKNOWN, rows, {}, err1 or err2 or "")
    if count is None and recurring is None:
        return _result(UNKNOWN, rows, {}, "environment-fleet inputs are missing")
    count = 0 if count is None else count
    recurring = False if recurring is None else recurring
    triggered = count >= 5 or recurring
    measured = {
        "concurrent_environments": count,
        "recurring_customer_fleet_ops": recurring,
    }
    return _result(
        TRIGGERED if triggered else NO_TRIGGER_EVIDENCE,
        rows,
        measured,
        "environment/customer fleet lifecycle reached a review trigger"
        if triggered
        else "environment/customer fleet evidence is below review triggers",
    )


def _evaluate_g(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, "no fresh authoritative heterogeneous-compute evidence")
    required, _, err1 = _conflict_or_value(
        rows, "specialized_node_classes_required", _require_bool
    )
    classes, _, err2 = _conflict_or_value(
        rows, "persistent_specialized_node_classes", _require_non_negative_int
    )
    placement, _, err3 = _conflict_or_value(
        rows, "hardware_class_placement_problem", _require_bool
    )
    if err1 or err2 or err3:
        return _result(UNKNOWN, rows, {}, err1 or err2 or err3 or "")

    if required is False:
        return _result(
            NO_TRIGGER_EVIDENCE,
            rows,
            {"specialized_node_classes_required": False},
            "current authoritative requirement does not need specialized node classes",
        )
    if required is None and classes is None and placement is None:
        return _result(UNKNOWN, rows, {}, "heterogeneous-compute inputs are missing")

    classes = 0 if classes is None else classes
    placement = False if placement is None else placement
    triggered = (required is True and classes >= 2 and placement) or placement
    measured = {
        "specialized_node_classes_required": required,
        "persistent_specialized_node_classes": classes,
        "hardware_class_placement_problem": placement,
    }
    return _result(
        TRIGGERED if triggered else NO_TRIGGER_EVIDENCE,
        rows,
        measured,
        "persistent hardware-class placement is an active scheduler problem"
        if triggered
        else "no persistent heterogeneous-compute scheduler problem is evidenced",
    )


def _evaluate_h(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return _result(UNKNOWN, rows, {}, "no fresh authoritative multi-region requirement evidence")
    required, _, err1 = _conflict_or_value(
        rows, "explicit_multi_region_requirement", _require_bool
    )
    regions, _, err2 = _conflict_or_value(
        rows, "region_count", _require_non_negative_int
    )
    if err1 or err2:
        return _result(UNKNOWN, rows, {}, err1 or err2 or "")
    if required is None:
        return _result(UNKNOWN, rows, {}, "explicit_multi_region_requirement is missing")
    measured = {
        "explicit_multi_region_requirement": required,
        "region_count": regions,
    }
    return _result(
        TRIGGERED if required else NO_TRIGGER_EVIDENCE,
        rows,
        measured,
        "explicit latency/residency/business requirement requires multiple regions"
        if required
        else "current authoritative requirement does not require multi-region",
    )


def _evaluate_trigger(
    trigger: str,
    evidence: list[dict[str, Any]],
    reviewed_at: datetime,
) -> dict[str, Any]:
    rows = _eligible_rows(evidence, reviewed_at, trigger)
    if trigger == "A":
        return _evaluate_a(rows)
    if trigger == "B":
        return _evaluate_b(rows)
    if trigger == "C":
        return _evaluate_c(rows)
    if trigger == "D":
        return _evaluate_boolean_any(
            rows,
            (
                "recurring_manual_deploy_coordination",
                "rollout_incident_class",
                "recurring_progressive_delivery_need",
            ),
            "rollout/deploy coordination",
        )
    if trigger == "E":
        return _evaluate_e(rows)
    if trigger == "F":
        return _evaluate_f(rows)
    if trigger == "G":
        return _evaluate_g(rows)
    if trigger == "H":
        return _evaluate_h(rows)
    raise AssertionError(trigger)


def _prerequisite_summary(payload: dict[str, Any]) -> dict[str, Any]:
    supplied = payload.get("prerequisites", {})
    items: dict[str, Any] = {}
    for name in PREREQUISITES:
        row = supplied.get(name)
        if row is None:
            items[name] = {"complete": False, "source_id": None, "state": "UNKNOWN"}
        else:
            complete = bool(row["complete"])
            items[name] = {
                "complete": complete,
                "source_id": row.get("source_id"),
                "state": "COMPLETE" if complete else "INCOMPLETE",
            }
    missing = [name for name, row in items.items() if row["state"] == "UNKNOWN"]
    incomplete = [name for name, row in items.items() if row["state"] == "INCOMPLETE"]
    return {
        "all_complete": not missing and not incomplete,
        "missing": missing,
        "incomplete": incomplete,
        "items": items,
    }


def reduce_review(payload: dict[str, Any]) -> dict[str, Any]:
    payload, reviewed_at = _validate_input(payload)
    evidence = payload["evidence"]

    trigger_results = {
        trigger: _evaluate_trigger(trigger, evidence, reviewed_at)
        for trigger in TRIGGERS
    }
    ordinary_trigger_count = sum(
        trigger_results[trigger]["state"] == TRIGGERED
        for trigger in ORDINARY_TRIGGERS
    )
    critical_slo_triggered = (
        trigger_results["E"]["state"] == CRITICAL_TRIGGERED
    )
    review_required = critical_slo_triggered or ordinary_trigger_count >= 2

    return {
        "receipt_version": RECEIPT_VERSION,
        "reviewed_at": reviewed_at.isoformat().replace("+00:00", "Z"),
        "sources": [_source_summary(row, reviewed_at) for row in evidence],
        "triggers": trigger_results,
        "prerequisites": _prerequisite_summary(payload),
        "ordinary_trigger_count": ordinary_trigger_count,
        "critical_slo_triggered": critical_slo_triggered,
        "review_required": review_required,
        "decision_boundary": (
            "review_required opens a human architecture review only; "
            "this receipt never selects Kubernetes"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    payload = json.loads(args.input.read_text(encoding="utf-8"))
    receipt = reduce_review(payload)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "receipt_version": receipt["receipt_version"],
                "ordinary_trigger_count": receipt["ordinary_trigger_count"],
                "critical_slo_triggered": receipt["critical_slo_triggered"],
                "review_required": receipt["review_required"],
                "trigger_states": {
                    key: value["state"] for key, value in receipt["triggers"].items()
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
