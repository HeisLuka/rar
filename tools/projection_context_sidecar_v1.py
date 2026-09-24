#!/usr/bin/env python3
"""Immutable PUB projection-context sidecar for current Editor state.

The sidecar binds one PubProjectionContextV1 to the immutable source hash but
keeps it outside EditorProject. Consumers receive only relation classes they
explicitly support; unsupported Cmo layout remains deferred rather than being
silently dropped or guessed.
"""

from __future__ import annotations

import copy
import hashlib
import json
import re
from typing import Any

PROJECTION_CONTEXT_SCHEMA_V1 = "chaptera.pub-projection-context.v1"
PROJECTION_CONTEXT_SIDECAR_SCHEMA_V1 = (
    "chaptera.editor-projection-context-sidecar.v1"
)
UUID_RE = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)
SHA_RE = re.compile(r"^[0-9a-f]{64}$")


class ProjectionContextSidecarError(ValueError):
    pass


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def projection_context_hash(context: dict[str, Any]) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(context)).hexdigest()


def empty_projection_context() -> dict[str, Any]:
    return {
        "schema_version": PROJECTION_CONTEXT_SCHEMA_V1,
        "master_relations": [],
        "cmo_relations": [],
    }


def empty_sidecar(source_hash: str) -> dict[str, Any]:
    _require_source_hash(source_hash, "source_hash")
    return {
        "schema_version": PROJECTION_CONTEXT_SIDECAR_SCHEMA_V1,
        "source_hash": source_hash,
        "context": empty_projection_context(),
    }


def normalize_sidecar(
    value: Any,
    *,
    expected_source_hash: str,
) -> dict[str, Any]:
    _require_source_hash(expected_source_hash, "expected_source_hash")
    value = _require_exact_keys(
        value,
        {"schema_version", "source_hash", "context"},
        "projection context sidecar",
    )
    if value["schema_version"] != PROJECTION_CONTEXT_SIDECAR_SCHEMA_V1:
        raise ProjectionContextSidecarError(
            "projection context sidecar schema_version mismatch"
        )
    _require_source_hash(value["source_hash"], "projection context sidecar source_hash")
    if value["source_hash"] != expected_source_hash:
        raise ProjectionContextSidecarError(
            "projection context sidecar source identity mismatch"
        )

    context = normalize_projection_context(value["context"])
    return {
        "schema_version": PROJECTION_CONTEXT_SIDECAR_SCHEMA_V1,
        "source_hash": value["source_hash"],
        "context": context,
    }


def normalize_projection_context(value: Any) -> dict[str, Any]:
    value = _require_exact_keys(
        value,
        {"schema_version", "master_relations", "cmo_relations"},
        "projection context",
    )
    if value["schema_version"] != PROJECTION_CONTEXT_SCHEMA_V1:
        raise ProjectionContextSidecarError(
            "projection context schema_version mismatch"
        )
    masters = value["master_relations"]
    cmos = value["cmo_relations"]
    if not isinstance(masters, list) or not isinstance(cmos, list):
        raise ProjectionContextSidecarError(
            "projection context relation collections must be arrays"
        )

    seen_master_sources: set[str] = set()
    for index, relation in enumerate(masters):
        relation = _require_exact_keys(
            relation,
            {
                "source_page_id",
                "source_page_seq_num",
                "master_page_id",
                "master_page_seq_num",
            },
            f"master_relations[{index}]",
        )
        source_page_id = _require_uuid(
            relation["source_page_id"],
            f"master_relations[{index}].source_page_id",
        )
        master_page_id = _require_uuid(
            relation["master_page_id"],
            f"master_relations[{index}].master_page_id",
        )
        _require_u32(
            relation["source_page_seq_num"],
            f"master_relations[{index}].source_page_seq_num",
        )
        _require_u32(
            relation["master_page_seq_num"],
            f"master_relations[{index}].master_page_seq_num",
        )
        if source_page_id == master_page_id:
            raise ProjectionContextSidecarError(
                f"master_relations[{index}] cannot self-reference"
            )
        if source_page_id in seen_master_sources:
            raise ProjectionContextSidecarError(
                f"duplicate master relation for source page {source_page_id}"
            )
        seen_master_sources.add(source_page_id)

    expected_source_order = 0
    for index, relation in enumerate(cmos):
        relation = _require_exact_keys(
            relation,
            {
                "source_order",
                "cmo_id",
                "carrier_ohpo",
                "carrier_cmo_id",
                "target_qsid",
                "carrier_node_id",
                "carrier_story_id",
                "target_story_id",
                "target_frame_node_id",
            },
            f"cmo_relations[{index}]",
        )
        source_order = _require_non_negative_int(
            relation["source_order"],
            f"cmo_relations[{index}].source_order",
        )
        if source_order != expected_source_order:
            raise ProjectionContextSidecarError(
                "cmo_relations must preserve contiguous source_order"
            )
        expected_source_order += 1
        for field in ("cmo_id", "carrier_ohpo", "carrier_cmo_id", "target_qsid"):
            _require_u32(
                relation[field],
                f"cmo_relations[{index}].{field}",
            )
        _require_uuid(
            relation["carrier_node_id"],
            f"cmo_relations[{index}].carrier_node_id",
        )
        _require_optional_uuid(
            relation["carrier_story_id"],
            f"cmo_relations[{index}].carrier_story_id",
        )
        _require_uuid(
            relation["target_story_id"],
            f"cmo_relations[{index}].target_story_id",
        )
        _require_optional_uuid(
            relation["target_frame_node_id"],
            f"cmo_relations[{index}].target_frame_node_id",
        )

    return copy.deepcopy(value)


def scene_supported_context(context: dict[str, Any]) -> dict[str, Any]:
    """Return only relation classes already implemented by geometry Scene V1."""
    normalized = normalize_projection_context(context)
    return {
        "schema_version": normalized["schema_version"],
        "master_relations": copy.deepcopy(normalized["master_relations"]),
        "cmo_relations": [],
    }


def sidecar_state(sidecar: dict[str, Any]) -> dict[str, Any]:
    context = sidecar["context"]
    return {
        "projection_context_hash": projection_context_hash(context),
        "master_relation_count": len(context["master_relations"]),
        "cmo_relation_count": len(context["cmo_relations"]),
        "carried_outside_editor_project": True,
        "cmo_layout_consumed": False,
    }


def _require_exact_keys(
    value: Any,
    expected: set[str],
    label: str,
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ProjectionContextSidecarError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise ProjectionContextSidecarError(
            f"{label} fields mismatch: "
            f"missing={sorted(expected - actual)} extra={sorted(actual - expected)}"
        )
    return value


def _require_source_hash(value: Any, label: str) -> str:
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        raise ProjectionContextSidecarError(
            f"{label} must be 64 lowercase hexadecimal characters"
        )
    return value


def _require_uuid(value: Any, label: str) -> str:
    if not isinstance(value, str) or not UUID_RE.fullmatch(value):
        raise ProjectionContextSidecarError(
            f"{label} must be canonical lowercase UUID"
        )
    return value


def _require_optional_uuid(value: Any, label: str) -> str | None:
    if value is None:
        return None
    return _require_uuid(value, label)


def _require_non_negative_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ProjectionContextSidecarError(
            f"{label} must be a non-negative integer"
        )
    return value


def _require_u32(value: Any, label: str) -> int:
    value = _require_non_negative_int(value, label)
    if value > 0xFFFF_FFFF:
        raise ProjectionContextSidecarError(f"{label} exceeds u32")
    return value
