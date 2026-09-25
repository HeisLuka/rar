#!/usr/bin/env python3
"""Source-neutral resolved render-effect IR validation/canonicalization."""

from __future__ import annotations

import copy
import json

SAFE_I64 = 9_007_199_254_740_991
FORBIDDEN_KEYS = {
    "officeart_property",
    "officeart_property_id",
    "fopte",
    "pub_property",
    "pub_property_id",
    "parser_record",
    "raw_officeart",
    "raw_pub_bytes",
}
BLEND_MODES = {"normal", "multiply", "screen"}
COMPOSITE_MODES = {"source_over", "source_in", "destination_in"}
FIDELITY_STATES = {"exact", "partial", "unsupported"}
EFFECT_KINDS = {"mask", "blur", "shadow", "filter_chain"}


class RenderEffectIrV1Error(ValueError):
    pass


def _fail(message: str):
    raise RenderEffectIrV1Error(message)


def _safe_int(value, label, *, minimum=None, maximum=None):
    if not isinstance(value, int) or isinstance(value, bool):
        _fail(f"{label} must be an integer")
    if abs(value) > SAFE_I64:
        _fail(f"{label} exceeds safe integer fence")
    if minimum is not None and value < minimum:
        _fail(f"{label} is below minimum")
    if maximum is not None and value > maximum:
        _fail(f"{label} exceeds maximum")
    return value


def _scan_forbidden(value, at="$"):
    if isinstance(value, list):
        for index, child in enumerate(value):
            _scan_forbidden(child, f"{at}[{index}]")
        return
    if not isinstance(value, dict):
        return
    for key, child in value.items():
        lowered = key.lower()
        if lowered in FORBIDDEN_KEYS or "officeart" in lowered or lowered.startswith("raw_pub"):
            _fail(f"raw/source-specific effect key forbidden at {at}.{key}")
        if "backend" in lowered or "texture" in lowered or "surface" in lowered or "shader" in lowered or "pass_id" == lowered:
            _fail(f"backend-local effect key forbidden at {at}.{key}")
        _scan_forbidden(child, f"{at}.{key}")


def _rect(value, label):
    if not isinstance(value, dict) or set(value) != {"x", "y", "width", "height"}:
        _fail(f"{label} must be exact RectEMU")
    out = {}
    for key in ("x", "y", "width", "height"):
        minimum = 0 if key in {"width", "height"} else None
        out[key] = _safe_int(value[key], f"{label}.{key}", minimum=minimum)
    return out


def _rgba(value, label):
    if not isinstance(value, dict) or set(value) != {"r", "g", "b", "a"}:
        _fail(f"{label} must be exact RGBA8")
    return {
        key: _safe_int(value[key], f"{label}.{key}", minimum=0, maximum=255)
        for key in ("r", "g", "b", "a")
    }


def _fidelity(effect):
    fidelity = effect.get("fidelity")
    if not isinstance(fidelity, dict) or set(fidelity) - {"state", "reason"}:
        _fail("effect fidelity must contain state and optional reason")
    state = fidelity.get("state")
    if state not in FIDELITY_STATES:
        _fail("effect fidelity state is invalid")
    reason = fidelity.get("reason")
    if state != "exact" and (not isinstance(reason, str) or not reason):
        _fail("partial/unsupported effect requires fidelity reason")
    if state == "exact" and reason not in (None, ""):
        _fail("exact effect must not carry a loss reason")
    source_format = effect.get("source_format", "synthetic")
    producer_receipt = effect.get("producer_receipt")
    if source_format == "publisher" and state == "exact":
        if not isinstance(producer_receipt, str) or not producer_receipt:
            _fail("Publisher effect cannot be exact without upstream producer receipt")
    return {"state": state, **({"reason": reason} if reason else {})}


def canonical_effect_v1(effect):
    if not isinstance(effect, dict):
        _fail("effect must be an object")
    _scan_forbidden(effect)
    effect_id = effect.get("effect_id")
    if not isinstance(effect_id, str) or not effect_id:
        _fail("effect_id is required")
    kind = effect.get("kind")
    if kind not in EFFECT_KINDS:
        _fail("effect kind is unsupported")
    region = _rect(effect.get("region"), "effect.region")
    fidelity = _fidelity(effect)
    out = {
        "effect_id": effect_id,
        "kind": kind,
        "region": region,
        "fidelity": fidelity,
        "source_format": effect.get("source_format", "synthetic"),
    }
    if effect.get("producer_receipt"):
        out["producer_receipt"] = effect["producer_receipt"]

    if kind == "mask":
        mode = effect.get("coverage_mode")
        if mode not in {"alpha", "luminance"}:
            _fail("mask coverage_mode must be alpha or luminance")
        source_ref = effect.get("coverage_source_ref")
        if not isinstance(source_ref, str) or not source_ref:
            _fail("mask coverage_source_ref is required")
        out.update({
            "coverage_mode": mode,
            "coverage_source_ref": source_ref,
        })
    elif kind == "blur":
        out["radius_emu"] = _safe_int(
            effect.get("radius_emu"), "blur.radius_emu", minimum=0
        )
    elif kind == "shadow":
        out.update({
            "offset_x_emu": _safe_int(effect.get("offset_x_emu"), "shadow.offset_x_emu"),
            "offset_y_emu": _safe_int(effect.get("offset_y_emu"), "shadow.offset_y_emu"),
            "blur_radius_emu": _safe_int(
                effect.get("blur_radius_emu", 0), "shadow.blur_radius_emu", minimum=0
            ),
            "color": _rgba(effect.get("color"), "shadow.color"),
        })
        if "spread_emu" in effect:
            _fail("shadow spread is not admitted by RenderEffectV1")
    else:
        filters = effect.get("filters")
        if not isinstance(filters, list) or not filters:
            _fail("filter_chain requires at least one filter")
        normalized = []
        for index, item in enumerate(filters):
            if not isinstance(item, dict):
                _fail(f"filter[{index}] must be an object")
            fkind = item.get("kind")
            if fkind == "opacity":
                normalized.append({
                    "kind": "opacity",
                    "amount_milli": _safe_int(
                        item.get("amount_milli"),
                        f"filter[{index}].amount_milli",
                        minimum=0,
                        maximum=1000,
                    ),
                })
            elif fkind == "color_matrix":
                values = item.get("values_milli")
                if not isinstance(values, list) or len(values) != 20:
                    _fail("color_matrix must contain exactly 20 values_milli")
                normalized.append({
                    "kind": "color_matrix",
                    "values_milli": [
                        _safe_int(v, f"filter[{index}].values_milli")
                        for v in values
                    ],
                })
            else:
                _fail(f"filter[{index}] kind is not admitted by V1")
        out["filters"] = normalized
    return out


def canonical_effect_group_v1(group, known_effect_ids):
    if not isinstance(group, dict):
        _fail("effect group must be an object")
    _scan_forbidden(group)
    group_id = group.get("effect_group_id")
    if not isinstance(group_id, str) or not group_id:
        _fail("effect_group_id is required")
    effect_ids = group.get("effect_ids", [])
    if not isinstance(effect_ids, list) or any(not isinstance(v, str) for v in effect_ids):
        _fail("effect_ids must be strings")
    if len(effect_ids) != len(set(effect_ids)):
        _fail("effect_ids must be unique within group")
    unknown = sorted(set(effect_ids) - set(known_effect_ids))
    if unknown:
        _fail("effect group references unknown effect ids: " + ",".join(unknown))
    blend = group.get("blend_mode", "normal")
    composite = group.get("composite_mode", "source_over")
    if blend not in BLEND_MODES:
        _fail("blend_mode not admitted by V1")
    if composite not in COMPOSITE_MODES:
        _fail("composite_mode not admitted by V1")
    parent_group_id = group.get("parent_effect_group_id")
    if parent_group_id is not None and (not isinstance(parent_group_id, str) or not parent_group_id):
        _fail("parent_effect_group_id must be a non-empty string or null")
    return {
        "effect_group_id": group_id,
        "parent_effect_group_id": parent_group_id,
        "effect_ids": list(effect_ids),
        "opacity_milli": _safe_int(
            group.get("opacity_milli", 1000),
            "effect_group.opacity_milli",
            minimum=0,
            maximum=1000,
        ),
        "isolation": bool(group.get("isolation", False)),
        "blend_mode": blend,
        "composite_mode": composite,
    }


def canonical_effect_tables_v1(source):
    effects = [canonical_effect_v1(value) for value in source.get("effects", [])]
    effects.sort(key=lambda value: value["effect_id"])
    ids = [value["effect_id"] for value in effects]
    if len(ids) != len(set(ids)):
        _fail("effect_id must be unique")
    groups = [
        canonical_effect_group_v1(value, ids)
        for value in source.get("effect_groups", [])
    ]
    groups.sort(key=lambda value: value["effect_group_id"])
    group_ids = [value["effect_group_id"] for value in groups]
    if len(group_ids) != len(set(group_ids)):
        _fail("effect_group_id must be unique")
    known_groups = set(group_ids)
    parent_by_group = {}
    for group in groups:
        group_id = group["effect_group_id"]
        parent_id = group["parent_effect_group_id"]
        if parent_id is None:
            continue
        if parent_id == group_id:
            _fail("effect group cannot parent itself")
        if parent_id not in known_groups:
            _fail("effect group references unknown parent")
        parent_by_group[group_id] = parent_id
    for group_id in group_ids:
        seen = set()
        cursor = group_id
        while cursor in parent_by_group:
            if cursor in seen:
                _fail("effect group parent cycle")
            seen.add(cursor)
            cursor = parent_by_group[cursor]
    return effects, groups


def effect_diagnostics_v1(effects):
    out = []
    for effect in effects:
        state = effect["fidelity"]["state"]
        if state == "exact":
            continue
        out.append({
            "code": "render.effect_" + state,
            "severity": "warning",
            "origin_node_id": None,
            "detail": f'{effect["effect_id"]}:{effect["fidelity"]["reason"]}',
        })
    return out


def canonical_json_v1(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def effect_tables_digest_v1(effects, groups):
    import hashlib
    payload = {"effects": copy.deepcopy(effects), "effect_groups": copy.deepcopy(groups)}
    return "sha256:" + hashlib.sha256(canonical_json_v1(payload).encode("utf-8")).hexdigest()
