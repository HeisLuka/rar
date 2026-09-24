#!/usr/bin/env python3
"""Source-neutral copy/materialization ledger adapter for Chaptera optimization receipts."""

from __future__ import annotations

import copy
from typing import Any

from optimization_receipt_v1 import (
    MEASUREMENT_SCHEMA,
    OptimizationIncompatible,
    identity_hash,
    observed,
    unknown,
)

COPY_LEDGER_SCHEMA = "chaptera.copy-ledger.v1"
COPY_CLASSES = {
    "REQUIRED_TRANSFORM",
    "REQUIRED_SERIALIZATION",
    "REQUIRED_LIFETIME_DETACH",
    "AVOIDABLE_DUPLICATE",
}
FREQUENCY_CLASSES = {"per_open", "per_edit", "per_frame", "per_export", "per_replay", "other"}
FORBIDDEN_PUBLIC_KEYS = {
    "source_path",
    "local_path",
    "filename",
    "file_name",
    "document_text",
    "story_text",
    "raw_bytes",
    "source_bytes",
    "customer_id",
    "tenant_id",
}


def _require_nonnegative_int(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise OptimizationIncompatible(f"{label} must be a non-negative integer")
    return value


def _walk_public_keys(value: Any, path: str = "receipt") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_PUBLIC_KEYS:
                raise OptimizationIncompatible(f"public copy ledger contains forbidden key: {path}.{key}")
            _walk_public_keys(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            _walk_public_keys(child, f"{path}[{index}]")


def _runtime_identity(receipt: dict[str, Any]) -> dict[str, Any]:
    runtime = receipt.get("runtime") or {}
    keys = ("runtime", "platform", "machine", "runner_os", "runner_arch")
    identity = {key: runtime.get(key) for key in keys}
    if any(value is None for value in identity.values()):
        raise OptimizationIncompatible("copy ledger runtime identity is incomplete")
    return identity


def _validate_site(site: dict[str, Any], scenario_name: str, index: int) -> dict[str, Any]:
    if not isinstance(site, dict):
        raise OptimizationIncompatible(f"copy site must be an object: {scenario_name}[{index}]")
    site_id = site.get("site_id")
    payload_class = site.get("payload_class")
    classification = site.get("classification")
    if not isinstance(site_id, str) or not site_id:
        raise OptimizationIncompatible(f"copy site id missing: {scenario_name}[{index}]")
    if not isinstance(payload_class, str) or not payload_class:
        raise OptimizationIncompatible(f"payload class missing: {scenario_name}.{site_id}")
    if classification not in COPY_CLASSES:
        raise OptimizationIncompatible(f"unsupported copy classification: {classification}")
    bytes_total = _require_nonnegative_int(site.get("bytes_total"), f"{scenario_name}.{site_id}.bytes_total")
    events = _require_nonnegative_int(site.get("events"), f"{scenario_name}.{site_id}.events")
    if bytes_total > 0 and events == 0:
        raise OptimizationIncompatible(f"non-zero bytes require at least one event: {scenario_name}.{site_id}")
    method = site.get("measurement_method")
    if not isinstance(method, str) or not method:
        raise OptimizationIncompatible(f"measurement method missing: {scenario_name}.{site_id}")
    return {
        "site_id": site_id,
        "payload_class": payload_class,
        "classification": classification,
        "bytes_total": bytes_total,
        "events": events,
        "measurement_method": method,
        "from_stage": site.get("from_stage"),
        "to_stage": site.get("to_stage"),
    }


def copy_ledger_measurement(receipt: dict[str, Any], build_identity: dict[str, Any]) -> dict[str, Any]:
    if receipt.get("receipt_version") != COPY_LEDGER_SCHEMA:
        raise OptimizationIncompatible("expected chaptera.copy-ledger.v1 producer receipt")
    _walk_public_keys(receipt)
    if not isinstance(build_identity, dict) or not isinstance(build_identity.get("sha"), str) or not build_identity["sha"]:
        raise OptimizationIncompatible("build identity sha is required")

    workload = receipt.get("workload")
    if not isinstance(workload, dict) or not workload:
        raise OptimizationIncompatible("copy ledger workload identity is required")
    fixture_binding_id = workload.get("fixture_binding_id")
    if not isinstance(fixture_binding_id, str) or not fixture_binding_id:
        raise OptimizationIncompatible("opaque fixture_binding_id is required")

    scenarios = receipt.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        raise OptimizationIncompatible("copy ledger requires at least one scenario")

    metrics: dict[str, dict[str, Any]] = {}
    correctness: dict[str, bool] = {}
    normalized_scenarios: list[dict[str, Any]] = []
    payload_totals: dict[str, dict[str, int]] = {}
    seen_names: set[str] = set()

    for row in scenarios:
        if not isinstance(row, dict):
            raise OptimizationIncompatible("scenario must be an object")
        name = row.get("name")
        if not isinstance(name, str) or not name:
            raise OptimizationIncompatible("scenario name is required")
        if name in seen_names:
            raise OptimizationIncompatible(f"duplicate scenario name: {name}")
        seen_names.add(name)
        frequency_class = row.get("frequency_class")
        if frequency_class not in FREQUENCY_CLASSES:
            raise OptimizationIncompatible(f"unsupported frequency class: {frequency_class}")

        allocations = _require_nonnegative_int(row.get("allocations"), f"{name}.allocations")
        allocated_bytes = _require_nonnegative_int(row.get("allocated_bytes"), f"{name}.allocated_bytes")
        peak_live_bytes = _require_nonnegative_int(row.get("peak_live_bytes"), f"{name}.peak_live_bytes")
        serialized_bytes = _require_nonnegative_int(row.get("serialized_bytes"), f"{name}.serialized_bytes")

        sites_raw = row.get("copy_sites")
        if not isinstance(sites_raw, list):
            raise OptimizationIncompatible(f"copy_sites must be a list: {name}")
        sites = [_validate_site(site, name, index) for index, site in enumerate(sites_raw)]
        ids = [site["site_id"] for site in sites]
        if len(ids) != len(set(ids)):
            raise OptimizationIncompatible(f"duplicate copy site id in scenario: {name}")

        total_materialized = sum(site["bytes_total"] for site in sites)
        avoidable = sum(
            site["bytes_total"] for site in sites if site["classification"] == "AVOIDABLE_DUPLICATE"
        )
        required_transform = sum(
            site["bytes_total"] for site in sites if site["classification"] == "REQUIRED_TRANSFORM"
        )
        required_serialization = sum(
            site["bytes_total"] for site in sites if site["classification"] == "REQUIRED_SERIALIZATION"
        )
        lifetime_detach = sum(
            site["bytes_total"] for site in sites if site["classification"] == "REQUIRED_LIFETIME_DETACH"
        )

        prefix = f"scenario.{name}"
        metrics[f"{prefix}.allocations"] = observed(allocations, "count", "lower_is_better")
        metrics[f"{prefix}.allocated_bytes"] = observed(allocated_bytes, "bytes", "lower_is_better")
        metrics[f"{prefix}.peak_live_bytes"] = observed(peak_live_bytes, "bytes", "lower_is_better")
        metrics[f"{prefix}.serialized_bytes"] = observed(serialized_bytes, "bytes", "lower_is_better")
        metrics[f"{prefix}.materialized_bytes"] = observed(total_materialized, "bytes", "lower_is_better")
        metrics[f"{prefix}.avoidable_duplicate_bytes"] = observed(avoidable, "bytes", "lower_is_better")
        metrics[f"{prefix}.required_transform_bytes"] = observed(required_transform, "bytes", "neutral")
        metrics[f"{prefix}.required_serialization_bytes"] = observed(required_serialization, "bytes", "neutral")
        metrics[f"{prefix}.required_lifetime_detach_bytes"] = observed(lifetime_detach, "bytes", "neutral")

        tracked_unique_bytes = row.get("tracked_unique_payload_bytes")
        if tracked_unique_bytes is None:
            metrics[f"{prefix}.copy_amplification_ratio"] = unknown(
                "producer did not provide a defensible unique-payload denominator",
                "ratio",
                "lower_is_better",
            )
        else:
            tracked_unique_bytes = _require_nonnegative_int(
                tracked_unique_bytes, f"{name}.tracked_unique_payload_bytes"
            )
            if tracked_unique_bytes == 0:
                metrics[f"{prefix}.copy_amplification_ratio"] = unknown(
                    "unique-payload denominator is zero",
                    "ratio",
                    "lower_is_better",
                )
            else:
                metrics[f"{prefix}.copy_amplification_ratio"] = observed(
                    total_materialized / tracked_unique_bytes,
                    "ratio",
                    "lower_is_better",
                )

        row_correctness = row.get("correctness") or {}
        semantic_equivalent = row_correctness.get("semantic_equivalent") is True
        output_equivalent = row_correctness.get("output_equivalent") is True
        correctness[f"{name}.semantic_equivalent"] = semantic_equivalent
        correctness[f"{name}.output_equivalent"] = output_equivalent

        for site in sites:
            payload = payload_totals.setdefault(
                site["payload_class"],
                {copy_class: 0 for copy_class in sorted(COPY_CLASSES)},
            )
            payload[site["classification"]] += site["bytes_total"]

        normalized_scenarios.append(
            {
                "name": name,
                "frequency_class": frequency_class,
                "copy_sites": sites,
                "copy_site_count": len(sites),
            }
        )

    for payload_class, totals in sorted(payload_totals.items()):
        key = payload_class.replace(" ", "_")
        metrics[f"payload.{key}.avoidable_duplicate_bytes"] = observed(
            totals["AVOIDABLE_DUPLICATE"], "bytes", "lower_is_better"
        )
        metrics[f"payload.{key}.materialized_bytes"] = observed(
            sum(totals.values()), "bytes", "lower_is_better"
        )

    ranked_avoidable = sorted(
        (
            {
                "scenario": row["name"],
                **site,
            }
            for row in normalized_scenarios
            for site in row["copy_sites"]
            if site["classification"] == "AVOIDABLE_DUPLICATE"
        ),
        key=lambda item: (-item["bytes_total"], item["scenario"], item["site_id"]),
    )

    evidence = receipt.get("evidence_authority") or {}
    real_source_free = evidence.get("real_product_source_free") is True
    public_synthetic = evidence.get("synthetic_or_public_fixture") is True
    if real_source_free and public_synthetic:
        raise OptimizationIncompatible("evidence authority cannot be both real and synthetic")

    workload_identity = {
        "producer_receipt_version": COPY_LEDGER_SCHEMA,
        "workload": copy.deepcopy(workload),
        "scenarios": [
            {"name": row["name"], "frequency_class": row["frequency_class"]}
            for row in normalized_scenarios
        ],
    }

    return {
        "schema": MEASUREMENT_SCHEMA,
        "producer": {
            "receipt_version": COPY_LEDGER_SCHEMA,
            "measurement_class": "copy_materialization_ledger",
        },
        "build_identity": copy.deepcopy(build_identity),
        "runtime_identity": _runtime_identity(receipt),
        "workload_identity": workload_identity,
        "workload_identity_hash": identity_hash(workload_identity),
        "correctness": correctness,
        "evidence_authority": {
            "real_product_corpus": real_source_free,
            "synthetic_or_product_grounded_public": public_synthetic,
            "technology_decision_allowed": real_source_free,
            "blocker": None if real_source_free else "real source-free producer receipt required",
        },
        "metrics": metrics,
        "copy_ledger": {
            "ranked_avoidable_sites": ranked_avoidable,
            "payload_class_totals": payload_totals,
            "scenarios": normalized_scenarios,
        },
        "limitations": copy.deepcopy(receipt.get("limitations", [])),
    }
