#!/usr/bin/env python3
"""Disposable source-neutral path materialization/cache runtime."""

from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass
from typing import Any

from render_path_ir_v1 import PATH_SCHEMA, geometry_digest

SCHEMA = "chaptera.render-path-runtime.v1"
ALLOWED_USAGES = {"fill", "stroke", "clip"}
ALLOWED_RENDER_MODES = {"vector_native", "reference_command_stream"}


def _canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _digest(value):
    return "sha256:" + hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


def _scan_for_source_specific(value, at="$"):
    if isinstance(value, list):
        for index, child in enumerate(value):
            _scan_for_source_specific(child, f"{at}[{index}]")
        return
    if not isinstance(value, dict):
        return
    for key, child in value.items():
        lowered = key.lower()
        if "officeart" in lowered or "fopt" in lowered or lowered.startswith("raw_pub"):
            raise ValueError(f"source-specific path state forbidden at {at}.{key}")
        _scan_for_source_specific(child, f"{at}.{key}")


def validate_path_geometry_v1(geometry):
    if not isinstance(geometry, dict) or geometry.get("schema") != PATH_SCHEMA:
        raise ValueError("PathRuntime requires normalized PathGeometryV1")
    _scan_for_source_specific(geometry)
    digest = geometry.get("path_digest")
    if not isinstance(digest, str) or not digest.startswith("sha256:"):
        raise ValueError("normalized path requires path_digest")
    payload = copy.deepcopy(geometry)
    payload.pop("path_digest", None)
    if geometry_digest(payload) != digest:
        raise ValueError("path_digest does not match normalized geometry")
    commands = geometry.get("commands")
    if not isinstance(commands, list) or not commands:
        raise ValueError("normalized path commands required")
    return geometry


def normalize_stroke_geometry_v1(stroke):
    if not isinstance(stroke, dict):
        raise ValueError("stroke geometry is required for stroke materialization")
    width = stroke.get("width_emu")
    miter = stroke.get("miter_limit_milli", 4000)
    if not isinstance(width, int) or isinstance(width, bool) or width <= 0:
        raise ValueError("stroke width_emu must be a positive integer")
    if not isinstance(miter, int) or isinstance(miter, bool) or miter <= 0:
        raise ValueError("stroke miter_limit_milli must be positive integer")
    join = stroke.get("join", "miter")
    cap = stroke.get("cap", "butt")
    if join not in {"miter", "round", "bevel"}:
        raise ValueError("unsupported stroke join")
    if cap not in {"butt", "round", "square"}:
        raise ValueError("unsupported stroke cap")
    return {
        "width_emu": width,
        "join": join,
        "cap": cap,
        "miter_limit_milli": miter,
    }


def path_material_key_v1(
    geometry,
    *,
    usage,
    render_mode="vector_native",
    quality_bucket="default",
    stroke_geometry=None,
):
    validate_path_geometry_v1(geometry)
    if usage not in ALLOWED_USAGES:
        raise ValueError("unsupported path usage")
    if render_mode not in ALLOWED_RENDER_MODES:
        raise ValueError("unsupported path render mode")
    if not isinstance(quality_bucket, str) or not quality_bucket:
        raise ValueError("quality_bucket must be a stable non-empty string")
    stroke = normalize_stroke_geometry_v1(stroke_geometry) if usage == "stroke" else None
    rule = geometry["clip_rule"] if usage == "clip" else geometry["fill_rule"]
    payload = {
        "schema": "chaptera.path-material-key.v1",
        "path_digest": geometry["path_digest"],
        "usage": usage,
        "geometry_rule": rule,
        "stroke_geometry": stroke,
        "render_mode": render_mode,
        "quality_bucket": quality_bucket,
    }
    return payload, _digest(payload)


@dataclass
class PathMaterialV1:
    key_payload: dict[str, Any]
    identity: str
    path_digest: str
    generation: int
    device_generation: int
    state: str
    resident_bytes: int
    draw_input: dict[str, Any]


class PathRuntimeV1:
    def __init__(self):
        self._device_generation = 1
        self._entries: dict[str, PathMaterialV1] = {}
        self.metrics = {
            "cache_hits": 0,
            "cache_misses": 0,
            "evictions": 0,
            "rebuilds": 0,
            "device_resets": 0,
            "stale_binding_rejections": 0,
            "path_invalidations": 0,
        }

    @property
    def device_generation(self):
        return self._device_generation

    def materialize(
        self,
        geometry,
        *,
        usage,
        render_mode="vector_native",
        quality_bucket="default",
        stroke_geometry=None,
    ):
        key_payload, identity = path_material_key_v1(
            geometry,
            usage=usage,
            render_mode=render_mode,
            quality_bucket=quality_bucket,
            stroke_geometry=stroke_geometry,
        )
        existing = self._entries.get(identity)
        if existing is not None and existing.state == "Resident":
            self.metrics["cache_hits"] += 1
            return self.binding(identity)

        generation = 1 if existing is None else existing.generation + 1
        draw_input = {
            "schema": "chaptera.path-draw-input.v1",
            "material_identity": identity,
            "path_digest": geometry["path_digest"],
            "usage": usage,
            "geometry_rule": key_payload["geometry_rule"],
            "stroke_geometry": copy.deepcopy(key_payload["stroke_geometry"]),
            "render_mode": render_mode,
            "quality_bucket": quality_bucket,
            "commands": copy.deepcopy(geometry["commands"]),
        }
        resident_bytes = len(_canonical(draw_input).encode("utf-8"))
        self._entries[identity] = PathMaterialV1(
            key_payload=copy.deepcopy(key_payload),
            identity=identity,
            path_digest=geometry["path_digest"],
            generation=generation,
            device_generation=self.device_generation,
            state="Resident",
            resident_bytes=resident_bytes,
            draw_input=draw_input,
        )
        self.metrics["cache_misses"] += 1
        if existing is not None:
            self.metrics["rebuilds"] += 1
        return self.binding(identity)

    def binding(self, identity):
        entry = self._entries[identity]
        return {
            "identity": entry.identity,
            "generation": entry.generation,
            "device_generation": entry.device_generation,
            "state": entry.state,
        }

    def validate_binding(self, binding):
        entry = self._entries.get(binding.get("identity"))
        ok = bool(
            entry
            and entry.state == "Resident"
            and binding.get("generation") == entry.generation
            and binding.get("device_generation") == self.device_generation
            and entry.device_generation == self.device_generation
        )
        if not ok:
            self.metrics["stale_binding_rejections"] += 1
        return ok

    def draw_input(self, identity):
        entry = self._entries[identity]
        if entry.state != "Resident" or entry.device_generation != self.device_generation:
            raise ValueError("path material is not resident in current device generation")
        return copy.deepcopy(entry.draw_input)

    def invalidate_path_digest(self, path_digest):
        count = 0
        for entry in self._entries.values():
            if entry.path_digest == path_digest and entry.state == "Resident":
                entry.state = "Evicted"
                entry.resident_bytes = 0
                count += 1
        self.metrics["path_invalidations"] += count
        return count

    def evict_identity(self, identity):
        entry = self._entries.get(identity)
        if entry is None or entry.state != "Resident":
            return {"bytes_reclaimed": 0, "identity": identity}
        reclaimed = entry.resident_bytes
        entry.state = "Evicted"
        entry.resident_bytes = 0
        self.metrics["evictions"] += 1
        return {"bytes_reclaimed": reclaimed, "identity": identity}

    def invalidate_identity(self, identity):
        out = self.evict_identity(identity)
        out["invalidated"] = identity
        return out

    def reset_device(self):
        self._device_generation += 1
        self.metrics["device_resets"] += 1
        for entry in self._entries.values():
            entry.state = "Evicted"
            entry.resident_bytes = 0

    def memory_entries(self):
        return [
            {
                "identity": identity,
                "resident_bytes": entry.resident_bytes if entry.state == "Resident" else 0,
                "reclaimable": entry.state == "Resident",
                "state": entry.state,
                "path_digest": entry.path_digest,
            }
            for identity, entry in sorted(self._entries.items())
        ]

    def device_generation_value(self):
        return self.device_generation

    def receipt(self):
        return {
            "schema": SCHEMA,
            "device_generation": self.device_generation,
            "entry_count": len(self._entries),
            "resident_entries": sum(e.state == "Resident" for e in self._entries.values()),
            "resident_bytes": sum(e.resident_bytes for e in self._entries.values()),
            "metrics": dict(self.metrics),
            "snapshot_local_path_index_semantic": False,
            "backend_allocator_handles_semantic": False,
            "synthetic_runtime_only": True,
        }


class PathRuntimeMemoryAdapterV1:
    def __init__(self, runtime):
        self.runtime = runtime

    def memory_entries(self):
        return self.runtime.memory_entries()

    def evict_identity(self, identity):
        return self.runtime.evict_identity(identity)

    def invalidate_identity(self, identity):
        return self.runtime.invalidate_identity(identity)

    def device_generation(self):
        return self.runtime.device_generation

    def reset_device(self):
        self.runtime.reset_device()


def invalidated_path_digests_for_patch(base_scene, target_scene, patch):
    def by_node(scene):
        return {
            atom["node_id"]: atom["path_digest"]
            for atom in scene["primitives"].get("paths", [])
        }
    before = by_node(base_scene)
    after = by_node(target_scene)
    affected = set(patch.get("removed_nodes", []))
    affected.update(row["node_id"] for row in patch.get("upsert_nodes", []))
    invalidated = set()
    for node_id in affected:
        old = before.get(node_id)
        new = after.get(node_id)
        if old is not None and old != new:
            invalidated.add(old)
    return sorted(invalidated)


def materialize_clip_stack_v1(runtime, geometries, *, quality_bucket="default"):
    rows = []
    for ordinal, geometry in enumerate(geometries):
        binding = runtime.materialize(
            geometry,
            usage="clip",
            quality_bucket=quality_bucket,
        )
        rows.append({
            "ordinal": ordinal,
            "path_digest": geometry["path_digest"],
            "material_identity": binding["identity"],
        })
    return rows
