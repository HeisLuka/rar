#!/usr/bin/env python3
"""Source-neutral end-to-end byte ownership/copy ledger contract."""

from __future__ import annotations

import collections
import copy
import hashlib
import json
from typing import Any

RECEIPT_VERSION = "chaptera.copy-ledger.v1"

COPY_CLASSES = {
    "required_transform",
    "required_serialization",
    "required_lifetime_detach",
    "avoidable_duplicate",
}

PAYLOAD_CLASSES = {
    "source_stream_bytes",
    "story_text",
    "authoring_graph_state",
    "page_node_state",
    "resource_image_bytes",
    "resource_font_bytes",
    "shaped_glyph_arrays",
    "layout_line_arrays",
    "render_scene_arrays",
    "serialized_wire_bytes",
    "other",
}

MEASUREMENT_CLASSES = {
    "synthetic_contract_fixture",
    "real_pub_source_free",
}

OPTIONAL_COUNT_FIELDS = {
    "allocation_count",
    "live_bytes_before",
    "live_bytes_after",
    "peak_live_bytes",
    "retained_bytes_after",
}

RECEIPT_FIELDS = {
    "receipt_version",
    "measurement_class",
    "producer",
    "equivalence",
    "evidence_authority",
    "events",
    "summary",
    "limitations",
}
PRODUCER_FIELDS = {"build_sha", "workload_id", "runtime_identity"}
RUNTIME_FIELDS = {
    "runtime",
    "platform",
    "machine",
    "runner_os",
    "runner_arch",
    "cpu",
    "allocator",
}
EQUIVALENCE_FIELDS = {"semantic_equal", "baseline_identity", "candidate_identity"}
AUTHORITY_FIELDS = {"real_pub_runtime", "technology_decision_allowed", "blocker"}
EVENT_FIELDS = {
    "event_id",
    "stage_id",
    "operation",
    "payload_class",
    "payload_identity",
    "copy_class",
    "reason",
    "logical_bytes",
    "materialized_bytes",
    "shared_bytes",
    "instances",
    "semantic_identity_equal",
    "source_identity",
    "target_identity",
    *OPTIONAL_COUNT_FIELDS,
}


class CopyLedgerInvalid(ValueError):
    pass


def _require_allowed_fields(row: dict[str, Any], allowed: set[str], label: str) -> None:
    unknown = sorted(set(row) - allowed)
    if unknown:
        raise CopyLedgerInvalid(f"{label} contains unsupported field(s): {', '.join(unknown)}")


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def identity_hash(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


def _require_non_negative_int(row: dict[str, Any], key: str) -> int:
    value = row.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise CopyLedgerInvalid(f"{key} must be a non-negative integer")
    return value


def _validate_optional_count(row: dict[str, Any], key: str) -> None:
    value = row.get(key)
    if value is None:
        return
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise CopyLedgerInvalid(f"{key} must be null or a non-negative integer")


def validate_copy_event(row: dict[str, Any]) -> None:
    if not isinstance(row, dict):
        raise CopyLedgerInvalid("copy event must be an object")
    _require_allowed_fields(row, EVENT_FIELDS, "copy event")

    for key in (
        "event_id",
        "stage_id",
        "operation",
        "payload_class",
        "payload_identity",
        "copy_class",
        "reason",
    ):
        if not isinstance(row.get(key), str) or not row[key]:
            raise CopyLedgerInvalid(f"{key} is required")

    if row["payload_class"] not in PAYLOAD_CLASSES:
        raise CopyLedgerInvalid(f"unsupported payload_class: {row['payload_class']}")
    if row["copy_class"] not in COPY_CLASSES:
        raise CopyLedgerInvalid(f"unsupported copy_class: {row['copy_class']}")

    _require_non_negative_int(row, "logical_bytes")
    materialized = _require_non_negative_int(row, "materialized_bytes")
    _require_non_negative_int(row, "shared_bytes")
    _require_non_negative_int(row, "instances")
    for key in OPTIONAL_COUNT_FIELDS:
        _validate_optional_count(row, key)

    equal = row.get("semantic_identity_equal")
    if not isinstance(equal, bool):
        raise CopyLedgerInvalid("semantic_identity_equal must be boolean")

    if row["copy_class"] == "avoidable_duplicate":
        if not equal:
            raise CopyLedgerInvalid(
                "avoidable_duplicate requires semantic_identity_equal=true"
            )
        if materialized <= 0:
            raise CopyLedgerInvalid(
                "avoidable_duplicate requires materialized_bytes > 0"
            )

    source_identity = row.get("source_identity")
    target_identity = row.get("target_identity")
    if source_identity is not None and not isinstance(source_identity, str):
        raise CopyLedgerInvalid("source_identity must be string or null")
    if target_identity is not None and not isinstance(target_identity, str):
        raise CopyLedgerInvalid("target_identity must be string or null")


def _validate_runtime_identity(runtime: dict[str, Any]) -> None:
    if not isinstance(runtime, dict) or not runtime:
        raise CopyLedgerInvalid("runtime_identity is required")
    _require_allowed_fields(runtime, RUNTIME_FIELDS, "runtime_identity")
    if not isinstance(runtime.get("runtime"), str) or not runtime["runtime"]:
        raise CopyLedgerInvalid("runtime_identity.runtime is required")
    if not isinstance(runtime.get("platform"), str) or not runtime["platform"]:
        raise CopyLedgerInvalid("runtime_identity.platform is required")


def validate_receipt(receipt: dict[str, Any]) -> None:
    if not isinstance(receipt, dict):
        raise CopyLedgerInvalid("receipt must be an object")
    _require_allowed_fields(receipt, RECEIPT_FIELDS, "receipt")
    if receipt.get("receipt_version") != RECEIPT_VERSION:
        raise CopyLedgerInvalid("receipt_version mismatch")
    if receipt.get("measurement_class") not in MEASUREMENT_CLASSES:
        raise CopyLedgerInvalid("measurement_class mismatch")

    producer = receipt.get("producer")
    if not isinstance(producer, dict):
        raise CopyLedgerInvalid("producer is required")
    _require_allowed_fields(producer, PRODUCER_FIELDS, "producer")
    if not isinstance(producer.get("build_sha"), str) or not producer["build_sha"]:
        raise CopyLedgerInvalid("producer.build_sha is required")
    if not isinstance(producer.get("workload_id"), str) or not producer["workload_id"]:
        raise CopyLedgerInvalid("producer.workload_id is required")
    _validate_runtime_identity(producer.get("runtime_identity"))

    equivalence = receipt.get("equivalence")
    if not isinstance(equivalence, dict):
        raise CopyLedgerInvalid("equivalence is required")
    _require_allowed_fields(equivalence, EQUIVALENCE_FIELDS, "equivalence")
    if equivalence.get("semantic_equal") is not True:
        raise CopyLedgerInvalid("semantic/canonical equivalence must be true")
    for key in ("baseline_identity", "candidate_identity"):
        if not isinstance(equivalence.get(key), str) or not equivalence[key]:
            raise CopyLedgerInvalid(f"equivalence.{key} is required")

    events = receipt.get("events")
    if not isinstance(events, list) or not events:
        raise CopyLedgerInvalid("events must be a non-empty array")

    seen: set[str] = set()
    for row in events:
        validate_copy_event(row)
        if row["event_id"] in seen:
            raise CopyLedgerInvalid(f"duplicate event_id: {row['event_id']}")
        seen.add(row["event_id"])

    declared_summary = receipt.get("summary")
    if declared_summary is not None:
        if declared_summary != summarize_events(events):
            raise CopyLedgerInvalid("summary does not match event data")

    real_pub = receipt["measurement_class"] == "real_pub_source_free"
    authority = receipt.get("evidence_authority")
    if not isinstance(authority, dict):
        raise CopyLedgerInvalid("evidence_authority is required")
    _require_allowed_fields(authority, AUTHORITY_FIELDS, "evidence_authority")
    if authority.get("real_pub_runtime") is not real_pub:
        raise CopyLedgerInvalid("real_pub_runtime authority mismatches measurement_class")
    if authority.get("technology_decision_allowed") is not real_pub:
        raise CopyLedgerInvalid(
            "technology_decision_allowed must be true only for real_pub_source_free"
        )

    limitations = receipt.get("limitations", [])
    if not isinstance(limitations, list) or not all(isinstance(item, str) for item in limitations):
        raise CopyLedgerInvalid("limitations must be an array of strings")


def summarize_events(events: list[dict[str, Any]]) -> dict[str, Any]:
    by_class = collections.defaultdict(lambda: {
        "events": 0,
        "materialized_bytes": 0,
        "shared_bytes": 0,
        "logical_unique_bytes": 0,
        "avoidable_duplicate_bytes": 0,
        "copy_amplification_ratio": None,
    })
    identities: dict[str, dict[str, int]] = collections.defaultdict(dict)

    top = []
    total_materialized = 0
    total_avoidable = 0

    for row in events:
        payload_class = row["payload_class"]
        bucket = by_class[payload_class]
        bucket["events"] += 1
        bucket["materialized_bytes"] += row["materialized_bytes"]
        bucket["shared_bytes"] += row["shared_bytes"]
        if row["copy_class"] == "avoidable_duplicate":
            bucket["avoidable_duplicate_bytes"] += row["materialized_bytes"]
            total_avoidable += row["materialized_bytes"]
        total_materialized += row["materialized_bytes"]

        prior = identities[payload_class].get(row["payload_identity"])
        logical = row["logical_bytes"]
        if prior is None:
            identities[payload_class][row["payload_identity"]] = logical
        elif prior != logical:
            raise CopyLedgerInvalid(
                "same payload_identity has conflicting logical_bytes: "
                f"{payload_class}:{row['payload_identity']}"
            )

        top.append({
            "event_id": row["event_id"],
            "stage_id": row["stage_id"],
            "operation": row["operation"],
            "payload_class": payload_class,
            "copy_class": row["copy_class"],
            "materialized_bytes": row["materialized_bytes"],
            "instances": row["instances"],
        })

    for payload_class, bucket in by_class.items():
        unique = sum(identities[payload_class].values())
        bucket["logical_unique_bytes"] = unique
        if unique > 0:
            bucket["copy_amplification_ratio"] = bucket["materialized_bytes"] / unique

    top.sort(
        key=lambda row: (
            -row["materialized_bytes"],
            -row["instances"],
            row["event_id"],
        )
    )

    return {
        "total_materialized_bytes": total_materialized,
        "avoidable_duplicate_bytes": total_avoidable,
        "by_payload_class": {
            key: by_class[key] for key in sorted(by_class)
        },
        "top_materialization_sites": top[:20],
    }


def with_summary(receipt: dict[str, Any]) -> dict[str, Any]:
    out = copy.deepcopy(receipt)
    out["summary"] = summarize_events(out["events"])
    validate_receipt(out)
    return out


def synthetic_contract_fixture() -> dict[str, Any]:
    base = {
        "receipt_version": RECEIPT_VERSION,
        "measurement_class": "synthetic_contract_fixture",
        "producer": {
            "build_sha": "synthetic-copy-ledger-contract",
            "workload_id": "synthetic-copy-ledger-contract-v1",
            "runtime_identity": {
                "runtime": "python-contract-fixture",
                "platform": "source-neutral",
            },
        },
        "equivalence": {
            "semantic_equal": True,
            "baseline_identity": "sha256:" + "a" * 64,
            "candidate_identity": "sha256:" + "a" * 64,
        },
        "evidence_authority": {
            "real_pub_runtime": False,
            "technology_decision_allowed": False,
            "blocker": "synthetic contract fixture only",
        },
        "events": [
            {
                "event_id": "cfb-stream-materialize",
                "stage_id": "cfb_to_raw",
                "operation": "open",
                "payload_class": "source_stream_bytes",
                "payload_identity": "sha256:" + "1" * 64,
                "copy_class": "required_lifetime_detach",
                "reason": "synthetic logical stream must outlive reader in fixture",
                "logical_bytes": 1_000_000,
                "materialized_bytes": 1_000_000,
                "shared_bytes": 1_000_000,
                "instances": 1,
                "allocation_count": 1,
                "live_bytes_before": 0,
                "live_bytes_after": 1_000_000,
                "peak_live_bytes": 1_000_000,
                "retained_bytes_after": 1_000_000,
                "semantic_identity_equal": True,
                "source_identity": "stream:Contents",
                "target_identity": "raw-backing:Contents",
            },
            {
                "event_id": "story-history-clone",
                "stage_id": "editor_history",
                "operation": "replace_story_range",
                "payload_class": "story_text",
                "payload_identity": "story:synthetic-long",
                "copy_class": "avoidable_duplicate",
                "reason": "synthetic whole-Story before snapshot for a one-scalar edit",
                "logical_bytes": 2_000_000,
                "materialized_bytes": 2_000_000,
                "shared_bytes": 0,
                "instances": 1,
                "allocation_count": 1,
                "live_bytes_before": 2_000_000,
                "live_bytes_after": 4_000_000,
                "peak_live_bytes": 4_000_000,
                "retained_bytes_after": 2_000_000,
                "semantic_identity_equal": True,
                "source_identity": "story:synthetic-long@before",
                "target_identity": "history:synthetic-whole-before",
            },
            {
                "event_id": "shaping-output",
                "stage_id": "layout_shape",
                "operation": "shape_paragraph",
                "payload_class": "shaped_glyph_arrays",
                "payload_identity": "shape:synthetic-paragraph",
                "copy_class": "required_transform",
                "reason": "text-to-glyph transformation creates a new semantic representation",
                "logical_bytes": 80_000,
                "materialized_bytes": 48_000,
                "shared_bytes": 48_000,
                "instances": 1,
                "allocation_count": 3,
                "live_bytes_before": 0,
                "live_bytes_after": 48_000,
                "peak_live_bytes": 48_000,
                "retained_bytes_after": 48_000,
                "semantic_identity_equal": False,
                "source_identity": "story-range:synthetic-paragraph",
                "target_identity": "glyphs:synthetic-paragraph",
            },
            {
                "event_id": "browser-scene-json",
                "stage_id": "scene_transport",
                "operation": "serialize_page_window",
                "payload_class": "serialized_wire_bytes",
                "payload_identity": "scene-window:synthetic",
                "copy_class": "required_serialization",
                "reason": "process/network boundary requires a serialized representation",
                "logical_bytes": 100_000,
                "materialized_bytes": 115_000,
                "shared_bytes": 0,
                "instances": 1,
                "allocation_count": 1,
                "live_bytes_before": 100_000,
                "live_bytes_after": 215_000,
                "peak_live_bytes": 215_000,
                "retained_bytes_after": 0,
                "semantic_identity_equal": True,
                "source_identity": "scene-window:synthetic",
                "target_identity": "json:scene-window:synthetic",
            },
        ],
        "limitations": [
            "Synthetic fixture exercises the contract only; it is not evidence about Chaptera product copy amplification.",
            "Unknown/unobservable counters must be null in real receipts, never fabricated as zero.",
        ],
    }
    return with_summary(base)
