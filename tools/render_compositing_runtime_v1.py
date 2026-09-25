#!/usr/bin/env python3
"""Disposable compositing/offscreen/clip runtime over RenderScene + SegmentPlan."""

from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass
from typing import Any

from render_color_contract_v1 import DEFAULT_CONTRACT, rebuild_target

SCHEMA = "chaptera.render-compositing-runtime.v1"


def _canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _digest(value):
    return "sha256:" + hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


@dataclass
class SurfaceEntryV1:
    identity: str
    descriptor_key: str
    descriptor: dict[str, Any]
    device_generation: int
    lease_generation: int
    resident_bytes: int
    in_use: bool = False
    clear_count: int = 0
    last_owner: str | None = None


class OffscreenSurfacePoolV1:
    def __init__(self, *, max_resident_bytes=64 * 1024 * 1024):
        self.max_resident_bytes = int(max_resident_bytes)
        self._device_generation = 1
        self._entries: dict[str, SurfaceEntryV1] = {}
        self._next_id = 1
        self.metrics = {
            "allocations": 0,
            "reuses": 0,
            "clears": 0,
            "evictions": 0,
            "device_resets": 0,
            "peak_resident_bytes": 0,
        }

    @property
    def device_generation(self):
        return self._device_generation

    def _bytes_for(self, descriptor):
        width = descriptor["width_px"]
        height = descriptor["height_px"]
        samples = descriptor["sample_count"]
        if not all(isinstance(v, int) and not isinstance(v, bool) and v > 0 for v in (width, height, samples)):
            raise ValueError("offscreen dimensions/sample_count must be positive integers")
        return width * height * 4 * samples

    def _descriptor_key(self, descriptor):
        payload = {
            "schema": "chaptera.offscreen-surface-class.v1",
            "target": descriptor["target"],
            "width_px": descriptor["width_px"],
            "height_px": descriptor["height_px"],
            "sample_count": descriptor["sample_count"],
            "quality_bucket": descriptor["quality_bucket"],
        }
        return _digest(payload)

    def acquire(self, descriptor, *, owner):
        descriptor = copy.deepcopy(descriptor)
        key = self._descriptor_key(descriptor)
        for entry in sorted(self._entries.values(), key=lambda row: row.identity):
            if (
                not entry.in_use
                and entry.device_generation == self.device_generation
                and entry.descriptor_key == key
            ):
                entry.in_use = True
                entry.lease_generation += 1
                entry.clear_count += 1
                entry.last_owner = owner
                self.metrics["reuses"] += 1
                self.metrics["clears"] += 1
                return self.binding(entry.identity)

        resident_bytes = self._bytes_for(descriptor)
        current = sum(e.resident_bytes for e in self._entries.values())
        if current + resident_bytes > self.max_resident_bytes:
            self.evict_available_until(max(0, self.max_resident_bytes - resident_bytes))
            current = sum(e.resident_bytes for e in self._entries.values())
        if current + resident_bytes > self.max_resident_bytes:
            raise MemoryError("offscreen pool cannot admit requested bounded surface")

        identity = f"surface:{self._next_id}"
        self._next_id += 1
        entry = SurfaceEntryV1(
            identity=identity,
            descriptor_key=key,
            descriptor=descriptor,
            device_generation=self.device_generation,
            lease_generation=1,
            resident_bytes=resident_bytes,
            in_use=True,
            clear_count=1,
            last_owner=owner,
        )
        self._entries[identity] = entry
        self.metrics["allocations"] += 1
        self.metrics["clears"] += 1
        self.metrics["peak_resident_bytes"] = max(
            self.metrics["peak_resident_bytes"],
            sum(e.resident_bytes for e in self._entries.values()),
        )
        return self.binding(identity)

    def binding(self, identity):
        entry = self._entries[identity]
        return {
            "identity": entry.identity,
            "descriptor_key": entry.descriptor_key,
            "lease_generation": entry.lease_generation,
            "device_generation": entry.device_generation,
        }

    def validate_binding(self, binding):
        entry = self._entries.get(binding.get("identity"))
        return bool(
            entry
            and entry.in_use
            and entry.device_generation == self.device_generation
            and binding.get("device_generation") == self.device_generation
            and binding.get("lease_generation") == entry.lease_generation
        )

    def release(self, binding):
        if not self.validate_binding(binding):
            raise ValueError("stale offscreen surface lease")
        self._entries[binding["identity"]].in_use = False

    def evict_available_until(self, target_resident_bytes):
        for identity in sorted(list(self._entries)):
            resident = sum(e.resident_bytes for e in self._entries.values())
            if resident <= target_resident_bytes:
                break
            entry = self._entries[identity]
            if entry.in_use:
                continue
            self._entries.pop(identity)
            self.metrics["evictions"] += 1

    def memory_entries(self):
        return [
            {
                "identity": identity,
                "resident_bytes": entry.resident_bytes,
                "reclaimable": not entry.in_use,
                "state": "in_use" if entry.in_use else "available",
            }
            for identity, entry in sorted(self._entries.items())
        ]

    def evict_identity(self, identity):
        entry = self._entries.get(identity)
        if entry is None or entry.in_use:
            return {"bytes_reclaimed": 0, "identity": identity}
        reclaimed = entry.resident_bytes
        self._entries.pop(identity)
        self.metrics["evictions"] += 1
        return {"bytes_reclaimed": reclaimed, "identity": identity}

    def invalidate_identity(self, identity):
        out = self.evict_identity(identity)
        out["invalidated"] = identity
        return out

    def reset_device(self):
        self._device_generation += 1
        self._entries.clear()
        self.metrics["device_resets"] += 1

    def receipt(self):
        return {
            "device_generation": self.device_generation,
            "resident_surfaces": len(self._entries),
            "resident_bytes": sum(e.resident_bytes for e in self._entries.values()),
            "metrics": dict(self.metrics),
        }


@dataclass
class ClipMaskEntryV1:
    identity: str
    clip_id: str
    semantic_digest: str
    device_generation: int
    generation: int
    resident_bytes: int
    state: str = "Resident"


class ClipMaskCacheV1:
    def __init__(self):
        self._device_generation = 1
        self._entries: dict[str, ClipMaskEntryV1] = {}
        self.metrics = {"hits": 0, "misses": 0, "evictions": 0, "device_resets": 0}

    @property
    def device_generation(self):
        return self._device_generation

    def _semantic_payload(self, clip, quality_bucket):
        kind = clip.get("kind")
        if kind == "rect":
            raise ValueError("rect clips must use scissor fast path")
        path_digest = clip.get("path_digest") or clip.get("geometry_digest")
        if not isinstance(path_digest, str) or not path_digest:
            raise ValueError("complex clip requires exact source-neutral geometry digest")
        return {
            "schema": "chaptera.clip-mask-key.v1",
            "clip_id": clip["clip_id"],
            "kind": kind,
            "path_digest": path_digest,
            "clip_rule": clip.get("clip_rule", "nonzero"),
            "transform_bucket": clip.get("transform_bucket", "identity"),
            "quality_bucket": quality_bucket,
        }

    def materialize(self, clip, *, quality_bucket="default"):
        payload = self._semantic_payload(clip, quality_bucket)
        semantic_digest = _digest(payload)
        identity = "clipmask:" + semantic_digest.split(":", 1)[1]
        entry = self._entries.get(identity)
        if entry is not None and entry.state == "Resident" and entry.device_generation == self.device_generation:
            self.metrics["hits"] += 1
            return self.binding(identity)
        generation = 1 if entry is None else entry.generation + 1
        resident_bytes = int(clip.get("mask_bytes_hint", 4096))
        self._entries[identity] = ClipMaskEntryV1(
            identity=identity,
            clip_id=clip["clip_id"],
            semantic_digest=semantic_digest,
            device_generation=self.device_generation,
            generation=generation,
            resident_bytes=max(1, resident_bytes),
        )
        self.metrics["misses"] += 1
        return self.binding(identity)

    def binding(self, identity):
        entry = self._entries[identity]
        return {
            "identity": identity,
            "semantic_digest": entry.semantic_digest,
            "generation": entry.generation,
            "device_generation": entry.device_generation,
        }

    def invalidate_clip_id(self, clip_id):
        count = 0
        for entry in self._entries.values():
            if entry.clip_id == clip_id and entry.state == "Resident":
                entry.state = "Evicted"
                entry.resident_bytes = 0
                count += 1
        return count

    def memory_entries(self):
        return [
            {
                "identity": identity,
                "resident_bytes": entry.resident_bytes if entry.state == "Resident" else 0,
                "reclaimable": entry.state == "Resident",
                "state": entry.state,
            }
            for identity, entry in sorted(self._entries.items())
        ]

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


class CompositingRuntimeV1:
    def __init__(self, *, surface_pool=None, clip_cache=None):
        self.surface_pool = surface_pool or OffscreenSurfacePoolV1()
        self.clip_cache = clip_cache or ClipMaskCacheV1()
        self._group_cache: dict[str, str] = {}
        self.metrics = {
            "group_cache_hits": 0,
            "group_cache_misses": 0,
            "group_invalidations": 0,
            "clip_invalidations": 0,
            "plans_built": 0,
        }

    @property
    def device_generation(self):
        if self.surface_pool.device_generation != self.clip_cache.device_generation:
            raise ValueError("compositing runtime device generations diverged")
        return self.surface_pool.device_generation

    def _group_fingerprint(self, group):
        return _digest({"schema": "chaptera.effect-group-runtime-key.v1", "group": group})

    def _group_chain(self, leaf_group_id, groups):
        if leaf_group_id is None:
            return []
        by_id = {g["effect_group_id"]: g for g in groups}
        chain = []
        cursor = leaf_group_id
        seen = set()
        while cursor is not None:
            if cursor in seen:
                raise ValueError("effect group parent cycle at runtime")
            seen.add(cursor)
            group = by_id.get(cursor)
            if group is None:
                raise ValueError(f"unknown effect group: {cursor}")
            fp = self._group_fingerprint(group)
            if self._group_cache.get(cursor) == fp:
                self.metrics["group_cache_hits"] += 1
            else:
                self._group_cache[cursor] = fp
                self.metrics["group_cache_misses"] += 1
            chain.append(group)
            cursor = group.get("parent_effect_group_id")
        chain.reverse()
        return chain

    def _requires_offscreen(self, group):
        return (
            group["opacity_milli"] != 1000
            or group["isolation"]
            or group["blend_mode"] != "normal"
            or group["composite_mode"] != "source_over"
        )

    def build_plan(
        self,
        scene,
        segment_scene_plan,
        *,
        group_surface_sizes=None,
        quality_bucket="default",
        target=None,
    ):
        group_surface_sizes = group_surface_sizes or {}
        target = copy.deepcopy(target or rebuild_target(DEFAULT_CONTRACT))
        groups = scene["tables"].get("effect_groups", [])
        effects = {e["effect_id"]: e for e in scene["tables"].get("effects", [])}
        clips = {c["clip_id"]: c for c in scene["tables"].get("clips", [])}
        records = []
        coherent = True
        unsupported = []
        offscreen_passes = 0
        scissor_clips = 0
        mask_clips = 0

        for page in segment_scene_plan["pages"]:
            for segment in page["segments"]:
                barrier = segment["barrier_key"]
                clip_id = barrier[0]
                leaf_group_id = barrier[1]
                clip_pop = None
                if clip_id is not None:
                    clip = clips.get(clip_id)
                    if clip is None:
                        coherent = False
                        unsupported.append({"code": "missing_clip", "clip_id": clip_id})
                    elif clip.get("kind") == "rect":
                        rect = clip.get("rect")
                        if not isinstance(rect, dict):
                            coherent = False
                            unsupported.append({"code": "malformed_rect_clip", "clip_id": clip_id})
                        else:
                            records.append({"op": "PushScissor", "clip_id": clip_id, "rect": copy.deepcopy(rect)})
                            clip_pop = {"op": "PopClip", "clip_id": clip_id}
                            scissor_clips += 1
                    else:
                        try:
                            mask = self.clip_cache.materialize(clip, quality_bucket=quality_bucket)
                            records.append({
                                "op": "PushClipMask",
                                "clip_id": clip_id,
                                "mask_semantic_digest": mask["semantic_digest"],
                            })
                            clip_pop = {"op": "PopClip", "clip_id": clip_id}
                            mask_clips += 1
                        except ValueError as exc:
                            coherent = False
                            unsupported.append({"code": "unsupported_complex_clip", "clip_id": clip_id, "detail": str(exc)})

                chain = self._group_chain(leaf_group_id, groups)
                leases = []
                group_unsupported = False
                for group in chain:
                    effect_ids = group.get("effect_ids", [])
                    if effect_ids:
                        group_unsupported = True
                        coherent = False
                        kinds = [effects[eid]["kind"] for eid in effect_ids if eid in effects]
                        unsupported.append({
                            "code": "advanced_effect_execution_not_admitted_v1",
                            "effect_group_id": group["effect_group_id"],
                            "effect_kinds": kinds,
                        })
                        records.append({
                            "op": "UnsupportedEffectGroup",
                            "effect_group_id": group["effect_group_id"],
                            "effect_kinds": kinds,
                        })
                        continue
                    if not self._requires_offscreen(group):
                        records.append({
                            "op": "EnterDirectGroup",
                            "effect_group_id": group["effect_group_id"],
                        })
                        continue
                    size = group_surface_sizes.get(group["effect_group_id"])
                    if (
                        not isinstance(size, dict)
                        or not isinstance(size.get("width_px"), int)
                        or not isinstance(size.get("height_px"), int)
                    ):
                        coherent = False
                        group_unsupported = True
                        unsupported.append({
                            "code": "missing_bounded_group_surface_size",
                            "effect_group_id": group["effect_group_id"],
                        })
                        continue
                    descriptor = {
                        "target": target,
                        "width_px": size["width_px"],
                        "height_px": size["height_px"],
                        "sample_count": int(size.get("sample_count", 1)),
                        "quality_bucket": quality_bucket,
                    }
                    lease = self.surface_pool.acquire(
                        descriptor,
                        owner=f'{page["page_id"]}:{group["effect_group_id"]}',
                    )
                    leases.append((group, lease, descriptor))
                    offscreen_passes += 1
                    records.append({
                        "op": "BeginOffscreenGroup",
                        "effect_group_id": group["effect_group_id"],
                        "surface_class": {
                            "descriptor_key": lease["descriptor_key"],
                            "width_px": descriptor["width_px"],
                            "height_px": descriptor["height_px"],
                            "sample_count": descriptor["sample_count"],
                            "quality_bucket": descriptor["quality_bucket"],
                        },
                    })

                if not group_unsupported and coherent:
                    records.append({
                        "op": "DrawSegment",
                        "page_id": page["page_id"],
                        "segment_fingerprint": segment["fingerprint"],
                        "atom_ids": list(segment["atom_ids"]),
                    })
                elif not group_unsupported:
                    records.append({
                        "op": "DrawSegmentConditionallyUnavailable",
                        "page_id": page["page_id"],
                        "segment_fingerprint": segment["fingerprint"],
                    })

                for group, lease, _ in reversed(leases):
                    records.append({
                        "op": "CompositeOffscreenGroup",
                        "effect_group_id": group["effect_group_id"],
                        "opacity_milli": group["opacity_milli"],
                        "blend_mode": group["blend_mode"],
                        "composite_mode": group["composite_mode"],
                        "color_contract": DEFAULT_CONTRACT.compositing_law,
                    })
                    self.surface_pool.release(lease)
                for group in reversed(chain):
                    if not self._requires_offscreen(group) and not group.get("effect_ids"):
                        records.append({
                            "op": "ExitDirectGroup",
                            "effect_group_id": group["effect_group_id"],
                        })
                if clip_pop is not None:
                    records.append(clip_pop)

        plan = {
            "schema": "chaptera.compositing-plan.v1",
            "segment_plan_fingerprint": segment_scene_plan["fingerprint"],
            "device_generation": self.device_generation,
            "target": target,
            "quality_bucket": quality_bucket,
            "coherent": coherent,
            "unsupported": unsupported,
            "records": records,
            "stats": {
                "offscreen_passes": offscreen_passes,
                "scissor_clips": scissor_clips,
                "mask_clips": mask_clips,
                "nested_group_depth": self._max_group_depth(groups),
            },
        }
        plan["plan_fingerprint"] = _digest({
            key: value for key, value in plan.items() if key != "device_generation"
        })
        self.metrics["plans_built"] += 1
        return plan

    def _max_group_depth(self, groups):
        by_id = {g["effect_group_id"]: g for g in groups}
        maximum = 0
        for group_id in by_id:
            depth = 0
            cursor = group_id
            seen = set()
            while cursor is not None:
                if cursor in seen:
                    raise ValueError("effect group cycle")
                seen.add(cursor)
                depth += 1
                cursor = by_id[cursor].get("parent_effect_group_id")
            maximum = max(maximum, depth)
        return maximum

    def invalidate_from_patch(self, patch, *, target_scene=None):
        effect_deltas = patch.get("effect_deltas") or {}
        changed_groups = set()
        group_delta = effect_deltas.get("effect_groups")
        if group_delta:
            changed_groups.update(group_delta.get("removed", []))
            changed_groups.update(row["effect_group_id"] for row in group_delta.get("upserts", []))

        effect_delta = effect_deltas.get("effects")
        changed_effect_ids = set()
        if effect_delta:
            changed_effect_ids.update(effect_delta.get("removed", []))
            changed_effect_ids.update(row["effect_id"] for row in effect_delta.get("upserts", []))
        if changed_effect_ids and target_scene is not None:
            for group in target_scene["tables"].get("effect_groups", []):
                if changed_effect_ids.intersection(group.get("effect_ids", [])):
                    changed_groups.add(group["effect_group_id"])

        for group_id in changed_groups:
            if self._group_cache.pop(group_id, None) is not None:
                self.metrics["group_invalidations"] += 1

        clip_delta = patch.get("clip_deltas")
        changed_clips = set()
        if clip_delta:
            changed_clips.update(clip_delta.get("removed", []))
            changed_clips.update(row["clip_id"] for row in clip_delta.get("upserts", []))
        for clip_id in changed_clips:
            self.metrics["clip_invalidations"] += self.clip_cache.invalidate_clip_id(clip_id)

        return {
            "effect_group_ids": sorted(changed_groups),
            "clip_ids": sorted(changed_clips),
        }

    def reset_device(self):
        self.surface_pool.reset_device()
        self.clip_cache.reset_device()
        self._group_cache.clear()

    def receipt(self):
        return {
            "schema": SCHEMA,
            "device_generation": self.device_generation,
            "surface_pool": self.surface_pool.receipt(),
            "clip_cache": {
                "device_generation": self.clip_cache.device_generation,
                "metrics": dict(self.clip_cache.metrics),
                "resident_bytes": sum(
                    row["resident_bytes"] for row in self.clip_cache.memory_entries()
                ),
            },
            "metrics": dict(self.metrics),
            "canonical_mutations": 0,
            "backend_handles_semantic": False,
        }


class CompositingMemoryAdapterV1:
    def __init__(self, runtime):
        self.runtime = runtime

    def memory_entries(self):
        rows = []
        for row in self.runtime.surface_pool.memory_entries():
            rows.append({**row, "identity": "offscreen/" + row["identity"]})
        for row in self.runtime.clip_cache.memory_entries():
            rows.append({**row, "identity": "mask/" + row["identity"]})
        return rows

    def evict_identity(self, identity):
        if identity.startswith("offscreen/"):
            return self.runtime.surface_pool.evict_identity(identity.split("/", 1)[1])
        if identity.startswith("mask/"):
            return self.runtime.clip_cache.evict_identity(identity.split("/", 1)[1])
        return {"bytes_reclaimed": 0, "identity": identity}

    def invalidate_identity(self, identity):
        out = self.evict_identity(identity)
        out["invalidated"] = identity
        return out

    def device_generation(self):
        return self.runtime.device_generation

    def reset_device(self):
        self.runtime.reset_device()
