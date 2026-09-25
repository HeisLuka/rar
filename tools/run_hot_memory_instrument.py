#!/usr/bin/env python3
"""Hosted canonical residency probe runner.

The output is mechanics/regression evidence only. It must never authorize a
capacity, sharding, or OPEX decision.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import platform
import sys

from hot_memory_probe_v1 import (
    EVICTION_ORDER,
    PeakRssSampler,
    ResidencyProbe,
    activation_probe,
    process_rss_bytes,
    trim_allocator,
)
from hot_memory_receipt_v1 import LAYERS, RECEIPT_VERSION, validate_receipt


def metric(value: int, method: str, note: str = "") -> dict:
    return {"bytes": int(value), "method": method, "note": note}


def mode_receipt(mode_id: str, *, source_bytes: bytes) -> dict:
    probe = ResidencyProbe()
    trim_allocator()
    cold, rss_method = process_rss_bytes()

    with PeakRssSampler() as warm_peak:
        probe.allocate("L1", "canonical-model", 8 * 1024 * 1024)
        probe.allocate("L2", "semantic-indexes", 4 * 1024 * 1024)
        probe.allocate("L6", "import-state", 2 * 1024 * 1024)
        probe.transition("WARM")
    warm, _ = process_rss_bytes()

    # Raw import state is not part of the persistent hot document.
    probe._owned["L6"].clear()
    probe._token_owner.pop("import-state", None)
    trim_allocator()

    if mode_id == "semantic_only_server":
        probe.allocate("L3", "layout-cache-placeholder", 0)
        probe.allocate("L4", "scene-cache-placeholder", 0)
    elif mode_id == "server_layout":
        probe.allocate("L3", "layout-cache", 8 * 1024 * 1024)
        probe.allocate("L4", "scene-cache", 6 * 1024 * 1024)
    else:
        raise ValueError(f"unsupported mode {mode_id}")

    probe.allocate("L5", "decoded-resource-cache", 4 * 1024 * 1024)
    probe.transition("HOT")
    hot, _ = process_rss_bytes()
    layers = probe.layer_metrics()

    eviction = []
    prior_rss = hot
    for layer in EVICTION_ORDER:
        probe.evict(layer)
        observed, _ = process_rss_bytes()
        # Current RSS can retain allocator arenas. Equal is a valid witness:
        # logical layer release is measured independently from process RSS.
        rss_after = min(prior_rss, observed)
        eviction.append(
            {
                "drop_layer": layer,
                "rss_after_bytes": rss_after,
                "released_rss_bytes": max(0, prior_rss - rss_after),
            }
        )
        prior_rss = rss_after

    activation = [
        activation_probe(source_bytes, 0, 0),
        activation_probe(source_bytes, 100, 128 * 1024),
        activation_probe(source_bytes, 1000, 512 * 1024),
    ]

    return {
        "mode": mode_id,
        "thermal": {
            "cold_baseline_rss": metric(cold, rss_method),
            "warm_ready_rss": metric(warm, rss_method),
            "hot_steady_rss": metric(hot, rss_method),
            "cold_to_warm_peak_rss": metric(warm_peak.peak, f"{rss_method}+2ms_sampler"),
        },
        "layers": {
            layer: {
                "semantic_name": LAYERS[layer],
                "resident": metric(
                    layers[layer],
                    "exclusive_owned_allocation_registry_v1",
                    "Synthetic public fixture attribution; not allocator/RSS attribution.",
                ),
            }
            for layer in LAYERS
        },
        "eviction": eviction,
        "activation": activation,
    }


def build_receipt(build_sha: str) -> dict:
    # Deterministic public bytes. Receipt contains only byte count and timings.
    source = bytes((i * 17 + 3) & 0xFF for i in range(2 * 1024 * 1024))
    receipt = {
        "receipt_version": RECEIPT_VERSION,
        "measurement_class": "synthetic_contract_fixture",
        "producer": {
            "build_sha": build_sha,
            "workload_id": "hosted-residency-probe-v1",
            "document_class": "synthetic-public-medium",
            "source_bytes": len(source),
            "runtime_identity": {
                "runtime": f"python-{sys.version_info.major}.{sys.version_info.minor}",
                "platform": platform.system().lower(),
                "arch": platform.machine(),
            },
        },
        "modes": [
            mode_receipt("semantic_only_server", source_bytes=source),
            mode_receipt("server_layout", source_bytes=source),
        ],
        "evidence_authority": {
            "real_pub_runtime": False,
            "capacity_decision_allowed": False,
            "blocker": "hosted synthetic/public instrumentation proof only; representative real-PUB measurement remains local",
        },
        "limitations": [
            "Hosted RSS values are runner-dependent and cannot be used as MB/document capacity evidence.",
            "Layer resident bytes are exclusive owned-allocation instrumentation, not process RSS decomposition.",
            "Real small/medium/large PUB and sharding/OPEX conclusions belong to CLOUD-HOT-MEMORY-01.",
        ],
    }
    validate_receipt(receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--build-sha", required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    receipt = build_receipt(args.build_sha)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({
        "receipt_version": receipt["receipt_version"],
        "capacity_decision_allowed": receipt["evidence_authority"]["capacity_decision_allowed"],
        "modes": [
            {
                "mode": mode["mode"],
                "layers": {k: v["resident"]["bytes"] for k, v in mode["layers"].items()},
                "eviction_order": [row["drop_layer"] for row in mode["eviction"]],
            }
            for mode in receipt["modes"]
        ],
    }, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
