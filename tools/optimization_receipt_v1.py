#!/usr/bin/env python3
"""Source-neutral optimization measurement + regression comparison contract."""

from __future__ import annotations

import copy
import hashlib
import json
from typing import Any

MEASUREMENT_SCHEMA = "chaptera.optimization.measurement.v1"
RECEIPT_SCHEMA = "chaptera.optimization.receipt.v1"
RENDER_BENCH_SCHEMA = "chaptera.render-bench.v1"
COPY_LEDGER_SCHEMA = "chaptera.copy-ledger.v1"

DIRECTIONS = {"lower_is_better", "higher_is_better", "neutral"}


class OptimizationIncompatible(ValueError):
    pass


def _canonical(value: Any) -> Any:
    if value is None or isinstance(value, (str, bool, int)):
        return value
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            raise TypeError("non-finite optimization value")
        return value
    if isinstance(value, list):
        return [_canonical(item) for item in value]
    if isinstance(value, dict):
        return {key: _canonical(value[key]) for key in sorted(value)}
    raise TypeError(f"unsupported optimization value: {type(value).__name__}")


def canonical_json(value: Any) -> str:
    return json.dumps(_canonical(value), sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def identity_hash(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


def observed(value: float | int, unit: str, direction: str, *, samples: int | None = None) -> dict[str, Any]:
    if direction not in DIRECTIONS:
        raise ValueError(f"unsupported metric direction: {direction}")
    metric = {
        "state": "observed",
        "unit": unit,
        "direction": direction,
        "value": value,
    }
    if samples is not None:
        metric["samples"] = samples
    return metric


def unknown(reason: str, unit: str | None = None, direction: str | None = None) -> dict[str, Any]:
    if not reason:
        raise ValueError("unknown metric requires reason")
    metric: dict[str, Any] = {"state": "unknown", "reason": reason}
    if unit is not None:
        metric["unit"] = unit
    if direction is not None:
        if direction not in DIRECTIONS:
            raise ValueError(f"unsupported metric direction: {direction}")
        metric["direction"] = direction
    return metric


def _timing_metric(row: dict[str, Any], key: str = "median_ms") -> dict[str, Any]:
    samples = row.get("samples_ms")
    return observed(
        row[key],
        "ms",
        "lower_is_better",
        samples=len(samples) if isinstance(samples, list) else None,
    )


def _require_build_identity(build_identity: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(build_identity, dict) or not build_identity:
        raise TypeError("build identity is required")
    if not isinstance(build_identity.get("sha"), str) or not build_identity["sha"]:
        raise TypeError("build identity sha is required")
    return copy.deepcopy(build_identity)


def render_bench_measurement(receipt: dict[str, Any], build_identity: dict[str, Any]) -> dict[str, Any]:
    if receipt.get("receipt_version") != RENDER_BENCH_SCHEMA:
        raise OptimizationIncompatible("expected chaptera.render-bench.v1 producer receipt")

    runtime = receipt.get("runtime") or {}
    runtime_identity = {
        "python": runtime.get("python"),
        "platform": runtime.get("platform"),
        "machine": runtime.get("machine"),
        "runner_os": runtime.get("runner_os"),
        "runner_arch": runtime.get("runner_arch"),
    }
    if any(value is None for value in runtime_identity.values()):
        raise OptimizationIncompatible("render benchmark runtime identity is incomplete")

    product_workloads = [
        {
            "name": row["name"],
            "input_pages": row["input_pages"],
            "input_nodes": row["input_nodes"],
            "glyph_runs": row["glyph_runs"],
        }
        for row in receipt.get("product_grounded_workloads", [])
    ]
    stress_workloads = [
        {
            "name": row["name"],
            "input_pages": row["input_pages"],
            "input_nodes": row["input_nodes"],
            "glyph_runs": row["glyph_runs"],
        }
        for row in receipt.get("synthetic_stress", [])
    ]
    first_visible = receipt.get("first_visible_page") or {}
    workload_identity = {
        "producer_receipt_version": receipt["receipt_version"],
        "measurement_class": receipt.get("measurement_class"),
        "product_grounded_workloads": product_workloads,
        "synthetic_stress": stress_workloads,
        "incremental_patch_nodes": (receipt.get("incremental_patch") or {}).get("node_count"),
        "first_visible_page": {
            "name": first_visible.get("name"),
            "input_pages": first_visible.get("input_pages"),
            "input_nodes": first_visible.get("input_nodes"),
            "glyph_runs": first_visible.get("glyph_runs"),
        },
        "output_sheet_instance_count": (receipt.get("output_sheet_instancing") or {}).get("instance_count"),
    }

    metrics: dict[str, dict[str, Any]] = {}
    for row in receipt.get("product_grounded_workloads", []):
        prefix = f"product.{row['name']}"
        metrics[f"{prefix}.compile_latency"] = _timing_metric(row["compile"])
        metrics[f"{prefix}.compiled_json_bytes"] = observed(
            row["compiled_json_bytes"], "bytes", "lower_is_better"
        )
        metrics[f"{prefix}.peak_python_allocation"] = observed(
            row["peak_tracemalloc_bytes"], "bytes", "lower_is_better"
        )

    for row in receipt.get("synthetic_stress", []):
        prefix = f"stress.{row['input_nodes']}"
        metrics[f"{prefix}.compile_latency"] = _timing_metric(row["compile"])
        metrics[f"{prefix}.compiled_json_bytes"] = observed(
            row["compiled_json_bytes"], "bytes", "lower_is_better"
        )
        metrics[f"{prefix}.peak_python_allocation"] = observed(
            row["peak_tracemalloc_bytes"], "bytes", "lower_is_better"
        )

    patch = receipt.get("incremental_patch") or {}
    if patch:
        metrics["patch.generation_latency"] = _timing_metric(patch["generation"])
        metrics["patch.apply_latency"] = _timing_metric(patch["apply"])
        metrics["patch.payload_bytes"] = observed(
            patch["patch_json_bytes"], "bytes", "lower_is_better"
        )
        metrics["patch.full_scene_bytes"] = observed(
            patch["full_scene_json_bytes"], "bytes", "neutral"
        )

    frame = receipt.get("frame_prep") or {}
    if frame.get("timing"):
        metrics["frame_prep.latency"] = _timing_metric(frame["timing"])

    if first_visible.get("compile"):
        metrics["first_visible_page.compile_latency"] = _timing_metric(first_visible["compile"])

    overlay = receipt.get("preview_overlay_120hz_proxy") or {}
    timing = overlay.get("timing") or {}
    if timing.get("per_update_median_us") is not None:
        metrics["preview_overlay.per_update_latency"] = observed(
            timing["per_update_median_us"], "us", "lower_is_better",
            samples=len(timing.get("samples_ms", [])) or None,
        )

    # Explicitly unknown rather than fabricated zeroes.
    metrics["gpu.draw_latency"] = unknown(
        "GPU/backend phase is not measured by chaptera.render-bench.v1",
        "ms",
        "lower_is_better",
    )
    metrics["gpu.resident_memory"] = unknown(
        "GPU/backend phase is not measured by chaptera.render-bench.v1",
        "bytes",
        "lower_is_better",
    )
    metrics["cost.usd_per_1000_edits"] = unknown(
        "No cost model is attached to this public hosted benchmark",
        "usd",
        "lower_is_better",
    )

    real_pub = receipt.get("real_pub_scene_present") is True
    return {
        "schema": MEASUREMENT_SCHEMA,
        "producer": {
            "receipt_version": receipt["receipt_version"],
            "measurement_class": receipt.get("measurement_class"),
        },
        "build_identity": _require_build_identity(build_identity),
        "runtime_identity": runtime_identity,
        "workload_identity": workload_identity,
        "workload_identity_hash": identity_hash(workload_identity),
        "correctness": {
            "scene_patch_apply_equals_full_compile": patch.get("apply_equals_full_compile") is True,
            "preview_overlay_emits_no_durable_patch": (
                overlay.get("durable_patch_count") == 0
            ),
            "output_sheet_clones_no_authoring_nodes": (
                (receipt.get("output_sheet_instancing") or {}).get("cloned_authoring_nodes") == 0
            ),
        },
        "evidence_authority": {
            "real_product_corpus": real_pub,
            "synthetic_or_product_grounded_public": not real_pub,
            "technology_decision_allowed": real_pub,
            "blocker": receipt.get("closure_blocker"),
        },
        "metrics": metrics,
        "limitations": copy.deepcopy(receipt.get("limitations", [])),
    }


def copy_ledger_measurement(receipt: dict[str, Any], build_identity: dict[str, Any]) -> dict[str, Any]:
    """Normalize a validated chaptera.copy-ledger.v1 receipt into the shared optimization spine."""
    if receipt.get("receipt_version") != COPY_LEDGER_SCHEMA:
        raise OptimizationIncompatible("expected chaptera.copy-ledger.v1 producer receipt")

    from copy_ledger_v1 import validate_receipt

    validate_receipt(receipt)
    build = _require_build_identity(build_identity)
    producer = receipt["producer"]
    summary = receipt["summary"]
    runtime_identity = copy.deepcopy(producer["runtime_identity"])

    metrics: dict[str, dict[str, Any]] = {
        "copy.total_materialized_bytes": observed(
            summary["total_materialized_bytes"], "bytes", "lower_is_better"
        ),
        "copy.avoidable_duplicate_bytes": observed(
            summary["avoidable_duplicate_bytes"], "bytes", "lower_is_better"
        ),
    }

    allocation_values = [row.get("allocation_count") for row in receipt["events"]]
    if all(value is not None for value in allocation_values):
        metrics["copy.allocation_count"] = observed(
            sum(allocation_values), "count", "lower_is_better"
        )
    else:
        metrics["copy.allocation_count"] = unknown(
            "one or more copy-ledger events do not expose allocation_count",
            "count",
            "lower_is_better",
        )

    peak_values = [row.get("peak_live_bytes") for row in receipt["events"]]
    observed_peaks = [value for value in peak_values if value is not None]
    if len(observed_peaks) == len(peak_values):
        metrics["copy.max_event_peak_live_bytes"] = observed(
            max(observed_peaks), "bytes", "lower_is_better"
        )
    else:
        metrics["copy.max_event_peak_live_bytes"] = unknown(
            "one or more copy-ledger events do not expose peak_live_bytes",
            "bytes",
            "lower_is_better",
        )

    for payload_class, row in sorted(summary["by_payload_class"].items()):
        prefix = f"copy.payload.{payload_class}"
        metrics[f"{prefix}.materialized_bytes"] = observed(
            row["materialized_bytes"], "bytes", "lower_is_better"
        )
        metrics[f"{prefix}.avoidable_duplicate_bytes"] = observed(
            row["avoidable_duplicate_bytes"], "bytes", "lower_is_better"
        )
        metrics[f"{prefix}.shared_bytes"] = observed(
            row["shared_bytes"], "bytes", "neutral"
        )
        metrics[f"{prefix}.logical_unique_bytes"] = observed(
            row["logical_unique_bytes"], "bytes", "neutral"
        )
        ratio = row.get("copy_amplification_ratio")
        if ratio is None:
            metrics[f"{prefix}.amplification_ratio"] = unknown(
                "no non-zero unique logical byte denominator for this payload class",
                "ratio",
                "lower_is_better",
            )
        else:
            metrics[f"{prefix}.amplification_ratio"] = observed(
                ratio, "ratio", "lower_is_better"
            )

    workload_identity = {
        "producer_receipt_version": receipt["receipt_version"],
        "workload_id": producer["workload_id"],
    }
    real_pub = receipt["measurement_class"] == "real_pub_source_free"

    return {
        "schema": MEASUREMENT_SCHEMA,
        "producer": {
            "receipt_version": receipt["receipt_version"],
            "measurement_class": receipt["measurement_class"],
        },
        "build_identity": build,
        "runtime_identity": runtime_identity,
        "workload_identity": workload_identity,
        "workload_identity_hash": identity_hash(workload_identity),
        "correctness": {
            "semantic_equal": receipt["equivalence"]["semantic_equal"] is True,
        },
        "evidence_authority": {
            "real_product_corpus": real_pub,
            "synthetic_or_product_grounded_public": not real_pub,
            "technology_decision_allowed": (
                receipt["evidence_authority"]["technology_decision_allowed"] is True
            ),
            "blocker": receipt["evidence_authority"].get("blocker"),
        },
        "metrics": metrics,
        "diagnostics": {
            "top_materialization_sites": copy.deepcopy(
                summary["top_materialization_sites"]
            ),
        },
        "limitations": copy.deepcopy(receipt.get("limitations", [])),
    }


def _validate_measurement(snapshot: dict[str, Any]) -> None:
    if snapshot.get("schema") != MEASUREMENT_SCHEMA:
        raise OptimizationIncompatible("optimization measurement schema mismatch")
    if snapshot.get("workload_identity_hash") != identity_hash(snapshot.get("workload_identity")):
        raise OptimizationIncompatible("workload identity hash mismatch")
    if not isinstance(snapshot.get("runtime_identity"), dict):
        raise OptimizationIncompatible("runtime identity missing")
    if not isinstance(snapshot.get("metrics"), dict):
        raise OptimizationIncompatible("metric vector missing")
    _require_build_identity(snapshot.get("build_identity"))


def _compatibility_errors(baseline: dict[str, Any], candidate: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if baseline["workload_identity_hash"] != candidate["workload_identity_hash"]:
        errors.append("workload_identity_mismatch")
    if baseline["runtime_identity"] != candidate["runtime_identity"]:
        errors.append("runtime_identity_mismatch")
    if baseline["producer"]["receipt_version"] != candidate["producer"]["receipt_version"]:
        errors.append("producer_schema_mismatch")

    for key in sorted(set(baseline["metrics"]) & set(candidate["metrics"])):
        left = baseline["metrics"][key]
        right = candidate["metrics"][key]
        if left.get("unit") != right.get("unit"):
            errors.append(f"metric_unit_mismatch:{key}")
        if left.get("direction") != right.get("direction"):
            errors.append(f"metric_direction_mismatch:{key}")
    return errors


def _metric_comparison(left: dict[str, Any] | None, right: dict[str, Any] | None) -> dict[str, Any]:
    if left is None or right is None:
        return {
            "state": "unknown",
            "reason": "metric missing from baseline or candidate",
        }
    if left.get("state") != "observed" or right.get("state") != "observed":
        reasons = []
        if left.get("state") != "observed":
            reasons.append(f"baseline:{left.get('reason', 'unobserved')}")
        if right.get("state") != "observed":
            reasons.append(f"candidate:{right.get('reason', 'unobserved')}")
        return {
            "state": "unknown",
            "unit": left.get("unit") or right.get("unit"),
            "direction": left.get("direction") or right.get("direction"),
            "reason": "; ".join(reasons),
        }

    base = left["value"]
    cand = right["value"]
    direction = left["direction"]
    raw_delta = cand - base
    delta_pct = None if base == 0 else (raw_delta / abs(base)) * 100.0
    if direction == "lower_is_better":
        regression_abs = raw_delta
        regression_pct = delta_pct
    elif direction == "higher_is_better":
        regression_abs = base - cand
        regression_pct = None if base == 0 else (regression_abs / abs(base)) * 100.0
    else:
        regression_abs = None
        regression_pct = None

    return {
        "state": "observed",
        "unit": left["unit"],
        "direction": direction,
        "baseline": base,
        "candidate": cand,
        "delta": raw_delta,
        "delta_pct": delta_pct,
        "regression_abs": regression_abs,
        "regression_pct": regression_pct,
        "baseline_samples": left.get("samples"),
        "candidate_samples": right.get("samples"),
    }


def compare_measurements(
    *,
    optimization_id: str,
    hot_path: str,
    baseline: dict[str, Any],
    candidate: dict[str, Any],
    budgets: dict[str, dict[str, float]] | None = None,
    correctness_equivalent: bool,
    fidelity_equivalent: bool,
    decision_scope: str = "local_optimization",
    trade_offs: list[str] | None = None,
    new_bottleneck: str | None = None,
) -> dict[str, Any]:
    if not optimization_id or not hot_path:
        raise ValueError("optimization id and hot path are required")
    if decision_scope not in {"local_optimization", "product_technology"}:
        raise ValueError("unsupported decision scope")

    _validate_measurement(baseline)
    _validate_measurement(candidate)
    errors = _compatibility_errors(baseline, candidate)
    if errors:
        raise OptimizationIncompatible(",".join(errors))

    comparisons = {
        key: _metric_comparison(
            baseline["metrics"].get(key),
            candidate["metrics"].get(key),
        )
        for key in sorted(set(baseline["metrics"]) | set(candidate["metrics"]))
    }

    budget_defs = copy.deepcopy(budgets or {})
    budget_results: dict[str, Any] = {}
    budget_violations: list[str] = []
    unevaluable_budgets: list[str] = []
    for metric_key, budget in sorted(budget_defs.items()):
        comparison = comparisons.get(metric_key)
        if comparison is None or comparison.get("state") != "observed":
            budget_results[metric_key] = {
                "state": "unknown",
                "reason": "budget metric is unobserved",
                "budget": budget,
            }
            unevaluable_budgets.append(metric_key)
            continue

        violations = []
        max_pct = budget.get("max_regression_pct")
        if max_pct is not None:
            if comparison.get("regression_pct") is None:
                violations.append("percent_regression_unavailable")
                unevaluable_budgets.append(metric_key)
            elif comparison["regression_pct"] > max_pct:
                violations.append("max_regression_pct")

        max_abs = budget.get("max_regression_abs")
        if max_abs is not None:
            if comparison.get("regression_abs") is None:
                violations.append("absolute_regression_unavailable")
                unevaluable_budgets.append(metric_key)
            elif comparison["regression_abs"] > max_abs:
                violations.append("max_regression_abs")

        if any(v in {"max_regression_pct", "max_regression_abs"} for v in violations):
            budget_violations.append(metric_key)
        budget_results[metric_key] = {
            "state": "evaluated" if not any("unavailable" in v for v in violations) else "partial",
            "budget": budget,
            "violations": violations,
        }

    technology_allowed = (
        baseline["evidence_authority"].get("technology_decision_allowed") is True
        and candidate["evidence_authority"].get("technology_decision_allowed") is True
    )

    if not correctness_equivalent or not fidelity_equivalent:
        decision = "revert"
    elif budget_violations:
        decision = "revert"
    elif unevaluable_budgets:
        decision = "needs_real_corpus_validation"
    elif decision_scope == "product_technology" and not technology_allowed:
        decision = "needs_real_corpus_validation"
    else:
        decision = "keep"

    return {
        "schema": RECEIPT_SCHEMA,
        "optimization_id": optimization_id,
        "hot_path": hot_path,
        "baseline_build_identity": copy.deepcopy(baseline["build_identity"]),
        "candidate_build_identity": copy.deepcopy(candidate["build_identity"]),
        "workload_identity": copy.deepcopy(candidate["workload_identity"]),
        "workload_identity_hash": candidate["workload_identity_hash"],
        "runtime_identity": copy.deepcopy(candidate["runtime_identity"]),
        "correctness_fidelity_fence": {
            "correctness_equivalent": correctness_equivalent,
            "fidelity_equivalent": fidelity_equivalent,
            "passed": correctness_equivalent and fidelity_equivalent,
        },
        "metric_comparisons": comparisons,
        "regression_budgets": budget_results,
        "budget_violations": budget_violations,
        "unevaluable_budgets": sorted(set(unevaluable_budgets)),
        "trade_offs": copy.deepcopy(trade_offs or []),
        "new_bottleneck": new_bottleneck,
        "evidence_authority": {
            "baseline": copy.deepcopy(baseline["evidence_authority"]),
            "candidate": copy.deepcopy(candidate["evidence_authority"]),
            "technology_decision_allowed": technology_allowed,
        },
        "decision_scope": decision_scope,
        "decision": decision,
    }
