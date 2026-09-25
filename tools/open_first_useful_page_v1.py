#!/usr/bin/env python3
"""Source-neutral Reader open -> first useful page timing receipt."""

from __future__ import annotations

import json
import math
from typing import Any

RECEIPT_VERSION = "chaptera.open-first-useful-page.v1"
MEASUREMENT_CLASSES = {"synthetic_contract_fixture", "hosted_public_fixture", "real_pub_source_free"}
CACHE_STATES = {"cold", "warm", "unknown"}
SCOPES = {
    "first_page_only",
    "bounded_dependencies",
    "document_global",
    "background",
}
PHASE_IDS = [
    "source_container_open",
    "profile_classification",
    "logical_stream_read",
    "parse_model_projection",
    "first_page_dependency_resolution",
    "first_page_layout_scene",
    "visible_resource_decode",
    "first_paint",
    "background_remaining_document",
    "search_index_ready",
]


class OpenReceiptInvalid(ValueError):
    pass


def _non_negative_number(value: Any, path: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise OpenReceiptInvalid(f"{path} must be numeric")
    value = float(value)
    if not math.isfinite(value) or value < 0:
        raise OpenReceiptInvalid(f"{path} must be finite and >= 0")
    return value


def _non_negative_int(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise OpenReceiptInvalid(f"{path} must be a non-negative integer")
    return value


def _validate_phase(phase: dict[str, Any], *, path: str) -> None:
    if phase.get("phase_id") not in PHASE_IDS:
        raise OpenReceiptInvalid(f"{path}.phase_id unsupported")
    if phase.get("scope") not in SCOPES:
        raise OpenReceiptInvalid(f"{path}.scope unsupported")
    start = _non_negative_number(phase.get("start_ms"), f"{path}.start_ms")
    end = _non_negative_number(phase.get("end_ms"), f"{path}.end_ms")
    if end < start:
        raise OpenReceiptInvalid(f"{path}.end_ms precedes start_ms")
    cpu = phase.get("cpu_ms")
    if cpu is not None:
        _non_negative_number(cpu, f"{path}.cpu_ms")
    for key in (
        "bytes_read",
        "bytes_materialized",
        "pages_touched",
        "stories_touched",
        "resources_touched",
    ):
        value = phase.get(key)
        if value is not None:
            _non_negative_int(value, f"{path}.{key}")
    note = phase.get("note")
    if note is not None and not isinstance(note, str):
        raise OpenReceiptInvalid(f"{path}.note must be string")


def _validate_run(run: dict[str, Any], *, path: str) -> None:
    if run.get("cache_state") not in CACHE_STATES:
        raise OpenReceiptInvalid(f"{path}.cache_state unsupported")
    first = _non_negative_number(
        run.get("first_useful_page_ms"), f"{path}.first_useful_page_ms"
    )
    full = _non_negative_number(run.get("fully_ready_ms"), f"{path}.fully_ready_ms")
    if full < first:
        raise OpenReceiptInvalid(f"{path}.fully_ready_ms < first_useful_page_ms")

    useful = run.get("first_useful_page_definition")
    if not isinstance(useful, dict):
        raise OpenReceiptInvalid(f"{path}.first_useful_page_definition required")
    required_flags = (
        "page_geometry_present",
        "fidelity_diagnostics_present",
        "visible_resources_ready",
        "current_text_layout_present",
        "non_empty_visible_content",
    )
    for flag in required_flags:
        if useful.get(flag) is not True:
            raise OpenReceiptInvalid(f"{path}.{flag} must be true")

    phases = run.get("phases")
    if not isinstance(phases, list) or not phases:
        raise OpenReceiptInvalid(f"{path}.phases required")
    seen = set()
    for i, phase in enumerate(phases):
        _validate_phase(phase, path=f"{path}.phases[{i}]")
        phase_id = phase["phase_id"]
        if phase_id in seen:
            raise OpenReceiptInvalid(f"{path} duplicate phase_id {phase_id}")
        seen.add(phase_id)

    if "first_paint" not in seen:
        raise OpenReceiptInvalid(f"{path} must include first_paint phase")

    global_pre_first = [
        p["phase_id"]
        for p in phases
        if p["scope"] == "document_global" and p["start_ms"] < first
    ]
    declared = run.get("document_global_before_first_useful_page")
    if declared != global_pre_first:
        raise OpenReceiptInvalid(
            f"{path}.document_global_before_first_useful_page does not match phases"
        )


def _nearest_rank(values: list[float], percentile: float) -> float:
    if not values:
        raise ValueError("values required")
    ordered = sorted(values)
    rank = max(1, math.ceil(percentile * len(ordered)))
    return ordered[rank - 1]


def summarize_runs(runs: list[dict[str, Any]]) -> dict[str, Any]:
    groups: dict[str, list[dict[str, Any]]] = {}
    for run in runs:
        groups.setdefault(run["cache_state"], []).append(run)
    out = {}
    for cache_state in sorted(groups):
        rows = groups[cache_state]
        first = [float(r["first_useful_page_ms"]) for r in rows]
        full = [float(r["fully_ready_ms"]) for r in rows]
        out[cache_state] = {
            "sample_count": len(rows),
            "first_useful_page_p50_ms": _nearest_rank(first, 0.50),
            "first_useful_page_p95_ms": _nearest_rank(first, 0.95),
            "fully_ready_p50_ms": _nearest_rank(full, 0.50),
            "fully_ready_p95_ms": _nearest_rank(full, 0.95),
            "runs_with_document_global_pre_first": sum(
                1 for r in rows if r["document_global_before_first_useful_page"]
            ),
        }
    return out


def validate_receipt(receipt: dict[str, Any]) -> None:
    if not isinstance(receipt, dict):
        raise OpenReceiptInvalid("receipt must be object")
    if receipt.get("receipt_version") != RECEIPT_VERSION:
        raise OpenReceiptInvalid("receipt version mismatch")
    measurement_class = receipt.get("measurement_class")
    if measurement_class not in MEASUREMENT_CLASSES:
        raise OpenReceiptInvalid("measurement_class mismatch")
    real_runtime = measurement_class != "synthetic_contract_fixture"
    architecture_allowed = measurement_class == "real_pub_source_free"

    producer = receipt.get("producer")
    if not isinstance(producer, dict):
        raise OpenReceiptInvalid("producer required")
    for key in ("build_sha", "workload_id", "document_class"):
        if not isinstance(producer.get(key), str) or not producer[key]:
            raise OpenReceiptInvalid(f"producer.{key} required")
    for key in ("source_bytes", "page_count"):
        _non_negative_int(producer.get(key), f"producer.{key}")
    runtime = producer.get("runtime_identity")
    if not isinstance(runtime, dict):
        raise OpenReceiptInvalid("runtime_identity required")
    for key in ("runtime", "platform", "arch"):
        if not isinstance(runtime.get(key), str) or not runtime[key]:
            raise OpenReceiptInvalid(f"runtime_identity.{key} required")

    runs = receipt.get("runs")
    if not isinstance(runs, list) or not runs:
        raise OpenReceiptInvalid("runs required")
    for i, run in enumerate(runs):
        _validate_run(run, path=f"runs[{i}]")

    summary = receipt.get("summary")
    expected = summarize_runs(runs)
    if summary != expected:
        raise OpenReceiptInvalid("summary does not match run data")

    eq = receipt.get("final_equivalence")
    if not isinstance(eq, dict):
        raise OpenReceiptInvalid("final_equivalence required")
    for flag in (
        "canonical_document_equal",
        "final_scene_equal",
        "final_search_projection_equal",
    ):
        if eq.get(flag) is not True:
            raise OpenReceiptInvalid(f"final_equivalence.{flag} must be true")

    authority = receipt.get("evidence_authority")
    if not isinstance(authority, dict):
        raise OpenReceiptInvalid("evidence_authority required")
    if authority.get("real_pub_runtime") is not real_runtime:
        raise OpenReceiptInvalid("real_pub_runtime mismatches measurement_class")
    if authority.get("architecture_decision_allowed") is not architecture_allowed:
        raise OpenReceiptInvalid(
            "architecture_decision_allowed must be true only for representative real_pub_source_free receipts"
        )

    if real_runtime:
        corpus = receipt.get("corpus_identity")
        if not isinstance(corpus, dict):
            raise OpenReceiptInvalid("real receipt requires corpus_identity")
        if not isinstance(corpus.get("fixture_hash"), str) or not corpus["fixture_hash"]:
            raise OpenReceiptInvalid("real receipt requires fixture_hash")
        if corpus.get("raw_path") is not None:
            raise OpenReceiptInvalid("raw_path must not cross into public receipt")


def synthetic_contract_fixture() -> dict[str, Any]:
    def phase(pid, start, end, scope, **counts):
        row = {
            "phase_id": pid,
            "start_ms": float(start),
            "end_ms": float(end),
            "cpu_ms": float(max(0, end - start) * 0.6),
            "scope": scope,
            "bytes_read": counts.get("bytes_read", 0),
            "bytes_materialized": counts.get("bytes_materialized", 0),
            "pages_touched": counts.get("pages_touched", 0),
            "stories_touched": counts.get("stories_touched", 0),
            "resources_touched": counts.get("resources_touched", 0),
            "note": "synthetic contract fixture",
        }
        return row

    def run(cache_state: str, scale: float):
        phases = [
            phase("source_container_open", 0, 8*scale, "bounded_dependencies", bytes_read=4096),
            phase("profile_classification", 8*scale, 12*scale, "bounded_dependencies"),
            phase("logical_stream_read", 12*scale, 28*scale, "document_global", bytes_read=600000, bytes_materialized=600000),
            phase("parse_model_projection", 28*scale, 55*scale, "document_global", pages_touched=20, stories_touched=18),
            phase("first_page_dependency_resolution", 55*scale, 65*scale, "bounded_dependencies", pages_touched=1, stories_touched=2, resources_touched=4),
            phase("first_page_layout_scene", 65*scale, 78*scale, "first_page_only", pages_touched=1, stories_touched=2),
            phase("visible_resource_decode", 68*scale, 82*scale, "bounded_dependencies", resources_touched=4),
            phase("first_paint", 82*scale, 85*scale, "first_page_only", pages_touched=1),
            phase("background_remaining_document", 85*scale, 125*scale, "background", pages_touched=19, stories_touched=16),
            phase("search_index_ready", 90*scale, 135*scale, "background", stories_touched=18),
        ]
        first = 85*scale
        return {
            "cache_state": cache_state,
            "first_useful_page_ms": first,
            "fully_ready_ms": 135*scale,
            "first_useful_page_definition": {
                "page_geometry_present": True,
                "fidelity_diagnostics_present": True,
                "visible_resources_ready": True,
                "current_text_layout_present": True,
                "non_empty_visible_content": True,
            },
            "phases": phases,
            "document_global_before_first_useful_page": [
                "logical_stream_read",
                "parse_model_projection",
            ],
        }

    runs = [
        run("cold", 1.00),
        run("cold", 1.05),
        run("cold", 0.95),
        run("warm", 0.72),
        run("warm", 0.70),
        run("warm", 0.75),
    ]
    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "measurement_class": "synthetic_contract_fixture",
        "producer": {
            "build_sha": "synthetic-open-contract",
            "workload_id": "synthetic-open-contract-v1",
            "document_class": "synthetic-20-page",
            "source_bytes": 1_000_000,
            "page_count": 20,
            "runtime_identity": {
                "runtime": "contract-fixture",
                "platform": "source-neutral",
                "arch": "none",
            },
        },
        "runs": runs,
        "summary": summarize_runs(runs),
        "final_equivalence": {
            "canonical_document_equal": True,
            "final_scene_equal": True,
            "final_search_projection_equal": True,
        },
        "evidence_authority": {
            "real_pub_runtime": False,
            "architecture_decision_allowed": False,
            "blocker": "synthetic contract fixture only",
        },
        "limitations": [
            "Synthetic phase values validate the contract only and are not Chaptera product latency.",
            "Document-global work before first useful page is a discriminator, not proof that laziness is correct or safe.",
        ],
    }
    validate_receipt(receipt)
    return receipt
