#!/usr/bin/env python3
"""Source-neutral hot-document residency receipt contract."""

from __future__ import annotations

import copy
import json
from typing import Any

RECEIPT_VERSION = "chaptera.hot-memory.v1"
MEASUREMENT_CLASSES = {"synthetic_contract_fixture", "real_pub_source_free"}
MODES = {"semantic_only_server", "server_layout"}
LAYERS = {
    "L1": "canonical_hot_core",
    "L2": "semantic_acceleration",
    "L3": "layout_caches",
    "L4": "scene_render_caches",
    "L5": "decoded_resources",
    "L6": "raw_parser_state",
}
EVICTION_ORDER = ["L5", "L4", "L3", "L2"]


class HotMemoryInvalid(ValueError):
    pass


def _metric(value: int | None, method: str, note: str = "") -> dict[str, Any]:
    if value is not None and (isinstance(value, bool) or not isinstance(value, int) or value < 0):
        raise ValueError("memory metric must be null or non-negative integer")
    if not method:
        raise ValueError("measurement method is required")
    return {"bytes": value, "method": method, "note": note}


def _require_count(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise HotMemoryInvalid(f"{path} must be a non-negative integer")
    return value


def _validate_metric(metric: Any, path: str, *, real: bool, required_observed: bool = False) -> None:
    if not isinstance(metric, dict):
        raise HotMemoryInvalid(f"{path} must be an object")
    value = metric.get("bytes")
    if value is not None:
        _require_count(value, f"{path}.bytes")
    elif real and required_observed:
        raise HotMemoryInvalid(f"{path}.bytes must be observed for real receipt")
    if not isinstance(metric.get("method"), str) or not metric["method"]:
        raise HotMemoryInvalid(f"{path}.method is required")
    if metric.get("note") is not None and not isinstance(metric.get("note"), str):
        raise HotMemoryInvalid(f"{path}.note must be string")


def _validate_mode(mode: dict[str, Any], *, real: bool) -> None:
    mode_id = mode.get("mode")
    if mode_id not in MODES:
        raise HotMemoryInvalid(f"unsupported mode: {mode_id}")

    thermal = mode.get("thermal")
    if not isinstance(thermal, dict):
        raise HotMemoryInvalid("thermal metrics are required")
    for key in (
        "cold_baseline_rss",
        "warm_ready_rss",
        "hot_steady_rss",
        "cold_to_warm_peak_rss",
    ):
        _validate_metric(
            thermal.get(key),
            f"{mode_id}.thermal.{key}",
            real=real,
            required_observed=True,
        )

    layers = mode.get("layers")
    if not isinstance(layers, dict) or set(layers) != set(LAYERS):
        raise HotMemoryInvalid(f"{mode_id}.layers must contain exactly L1..L6")
    for layer_id, semantic_name in LAYERS.items():
        row = layers[layer_id]
        if row.get("semantic_name") != semantic_name:
            raise HotMemoryInvalid(f"{mode_id}.{layer_id} semantic_name mismatch")
        _validate_metric(
            row.get("resident"),
            f"{mode_id}.{layer_id}.resident",
            real=real,
            required_observed=(layer_id in {"L1", "L2", "L3", "L4", "L5"}),
        )

    eviction = mode.get("eviction")
    if not isinstance(eviction, list) or len(eviction) != len(EVICTION_ORDER):
        raise HotMemoryInvalid(f"{mode_id}.eviction must contain L5→L4→L3→L2")
    if [row.get("drop_layer") for row in eviction] != EVICTION_ORDER:
        raise HotMemoryInvalid(f"{mode_id}.eviction order must be L5→L4→L3→L2")
    prior_rss = None
    for i, row in enumerate(eviction):
        after = row.get("rss_after_bytes")
        _require_count(after, f"{mode_id}.eviction[{i}].rss_after_bytes")
        if prior_rss is not None and after > prior_rss:
            raise HotMemoryInvalid(
                f"{mode_id}.eviction RSS increased after dropping {row['drop_layer']}"
            )
        prior_rss = after
        delta = row.get("released_rss_bytes")
        _require_count(delta, f"{mode_id}.eviction[{i}].released_rss_bytes")

    activation = mode.get("activation")
    if not isinstance(activation, list) or not activation:
        raise HotMemoryInvalid(f"{mode_id}.activation scenarios are required")
    for i, row in enumerate(activation):
        for key in ("snapshot_lag_edges", "tail_bytes", "bytes_read", "peak_rss_bytes"):
            _require_count(row.get(key), f"{mode_id}.activation[{i}].{key}")
        for key in ("activation_wall_ms", "replay_cpu_ms"):
            value = row.get(key)
            if not isinstance(value, (int, float)) or isinstance(value, bool) or value < 0:
                raise HotMemoryInvalid(f"{mode_id}.activation[{i}].{key} must be >= 0")


def validate_receipt(receipt: dict[str, Any]) -> None:
    if not isinstance(receipt, dict):
        raise HotMemoryInvalid("receipt must be object")
    if receipt.get("receipt_version") != RECEIPT_VERSION:
        raise HotMemoryInvalid("receipt version mismatch")
    measurement_class = receipt.get("measurement_class")
    if measurement_class not in MEASUREMENT_CLASSES:
        raise HotMemoryInvalid("measurement_class mismatch")
    real = measurement_class == "real_pub_source_free"

    producer = receipt.get("producer")
    if not isinstance(producer, dict):
        raise HotMemoryInvalid("producer is required")
    for key in ("build_sha", "workload_id", "document_class"):
        if not isinstance(producer.get(key), str) or not producer[key]:
            raise HotMemoryInvalid(f"producer.{key} is required")
    runtime = producer.get("runtime_identity")
    if not isinstance(runtime, dict):
        raise HotMemoryInvalid("runtime_identity is required")
    for key in ("runtime", "platform", "arch"):
        if not isinstance(runtime.get(key), str) or not runtime[key]:
            raise HotMemoryInvalid(f"runtime_identity.{key} is required")
    _require_count(producer.get("source_bytes"), "producer.source_bytes")

    modes = receipt.get("modes")
    if not isinstance(modes, list) or {m.get("mode") for m in modes} != MODES:
        raise HotMemoryInvalid("receipt must contain semantic_only_server and server_layout")
    for mode in modes:
        _validate_mode(mode, real=real)

    authority = receipt.get("evidence_authority")
    if not isinstance(authority, dict):
        raise HotMemoryInvalid("evidence_authority is required")
    if authority.get("real_pub_runtime") is not real:
        raise HotMemoryInvalid("real_pub_runtime mismatches measurement_class")
    if authority.get("capacity_decision_allowed") is not real:
        raise HotMemoryInvalid("capacity_decision_allowed must be true only for real receipt")

    if real:
        corpus = receipt.get("corpus_identity")
        if not isinstance(corpus, dict):
            raise HotMemoryInvalid("real receipt requires corpus_identity")
        if not isinstance(corpus.get("fixture_hash"), str) or not corpus["fixture_hash"]:
            raise HotMemoryInvalid("real receipt requires source-free fixture_hash")
        if corpus.get("raw_path") is not None:
            raise HotMemoryInvalid("raw fixture path must not cross into public receipt")


def synthetic_contract_fixture() -> dict[str, Any]:
    def mode(mode_id: str, layout: bool) -> dict[str, Any]:
        cold = 40_000_000
        warm = 70_000_000
        hot = 100_000_000 if layout else 82_000_000
        peak = 112_000_000 if layout else 92_000_000
        l3 = 9_000_000 if layout else 2_000_000
        l4 = 6_000_000 if layout else 1_000_000
        l5 = 5_000_000
        layers = {
            "L1": {"semantic_name": LAYERS["L1"], "resident": _metric(25_000_000, "synthetic_attribution")},
            "L2": {"semantic_name": LAYERS["L2"], "resident": _metric(5_000_000, "synthetic_attribution")},
            "L3": {"semantic_name": LAYERS["L3"], "resident": _metric(l3, "synthetic_attribution")},
            "L4": {"semantic_name": LAYERS["L4"], "resident": _metric(l4, "synthetic_attribution")},
            "L5": {"semantic_name": LAYERS["L5"], "resident": _metric(l5, "synthetic_attribution")},
            "L6": {"semantic_name": LAYERS["L6"], "resident": _metric(0, "synthetic_attribution", "fixture assumes parser state released")},
        }
        eviction = []
        rss = hot
        for layer_id, released in [
            ("L5", l5),
            ("L4", l4),
            ("L3", l3),
            ("L2", 5_000_000),
        ]:
            rss -= released
            eviction.append({
                "drop_layer": layer_id,
                "rss_after_bytes": rss,
                "released_rss_bytes": released,
            })
        return {
            "mode": mode_id,
            "thermal": {
                "cold_baseline_rss": _metric(cold, "synthetic_rss"),
                "warm_ready_rss": _metric(warm, "synthetic_rss"),
                "hot_steady_rss": _metric(hot, "synthetic_rss"),
                "cold_to_warm_peak_rss": _metric(peak, "synthetic_rss"),
            },
            "layers": layers,
            "eviction": eviction,
            "activation": [
                {
                    "snapshot_lag_edges": 0,
                    "tail_bytes": 0,
                    "activation_wall_ms": 12.0,
                    "replay_cpu_ms": 0.0,
                    "bytes_read": 2_000_000,
                    "peak_rss_bytes": peak,
                },
                {
                    "snapshot_lag_edges": 100,
                    "tail_bytes": 120_000,
                    "activation_wall_ms": 28.0,
                    "replay_cpu_ms": 11.0,
                    "bytes_read": 2_120_000,
                    "peak_rss_bytes": peak + 2_000_000,
                },
            ],
        }

    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "measurement_class": "synthetic_contract_fixture",
        "producer": {
            "build_sha": "synthetic-hot-memory-contract",
            "workload_id": "synthetic-hot-memory-contract-v1",
            "document_class": "synthetic-medium",
            "source_bytes": 2_000_000,
            "runtime_identity": {
                "runtime": "contract-fixture",
                "platform": "source-neutral",
                "arch": "none",
            },
        },
        "modes": [
            mode("semantic_only_server", False),
            mode("server_layout", True),
        ],
        "evidence_authority": {
            "real_pub_runtime": False,
            "capacity_decision_allowed": False,
            "blocker": "synthetic contract fixture only",
        },
        "limitations": [
            "Synthetic values validate schema/invariants only and are not Chaptera memory measurements.",
            "Layer attribution method must be stated because RSS deltas and heap attribution are not interchangeable.",
        ],
    }
    validate_receipt(receipt)
    return receipt
