#!/usr/bin/env python3
"""Backend-neutral submit runtime over deterministic RenderSegment plans."""

from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass
from typing import Any

SCHEMA = "chaptera.render-submit-runtime.v1"


def _canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _digest(value):
    return "sha256:" + hashlib.sha256(_canonical(value).encode("utf-8")).hexdigest()


@dataclass(frozen=True)
class PipelineDescriptorV1:
    backend_id: str
    program_version: str
    primitive_kind: str
    material_mode: str
    target_format: str
    sample_count: int
    blend_mode: str
    composite_mode: str
    depth_stencil_mode: str
    clip_mode: str

    def key(self):
        return _digest({"schema": "chaptera.pipeline-descriptor.v1", **self.__dict__})


@dataclass(frozen=True)
class RuntimeBindingRefV1:
    identity: str
    binding_kind: str
    material_generation: int
    device_generation: int
    layout_version: str
    placement_generation: int | None = None

    def normalized(self):
        return {
            "identity": self.identity,
            "binding_kind": self.binding_kind,
            "material_generation": self.material_generation,
            "device_generation": self.device_generation,
            "layout_version": self.layout_version,
            "placement_generation": self.placement_generation,
        }


class StaleSubmitGeneration(ValueError):
    pass


class FakeSubmitBackendV1:
    def __init__(self, backend_id="fake"):
        self.backend_id = backend_id
        self.device_generation = 1
        self.pipeline_builds = 0
        self.binding_builds = 0
        self.command_builds = 0

    def _handle(self, kind, logical_key, ordinal):
        return f"{kind}:g{self.device_generation}:{ordinal}:{logical_key[-12:]}"

    def create_pipeline(self, logical_key):
        self.pipeline_builds += 1
        return self._handle("pipeline", logical_key, self.pipeline_builds)

    def create_binding(self, logical_key):
        self.binding_builds += 1
        return self._handle("binding", logical_key, self.binding_builds)

    def create_command(self, logical_key):
        self.command_builds += 1
        return self._handle("command", logical_key, self.command_builds)

    def reset_device(self):
        self.device_generation += 1


class RenderSubmitRuntimeV1:
    def __init__(self, backend=None, *, program_version="program:v1", binding_layout_version="layout:v1"):
        self.backend = backend or FakeSubmitBackendV1()
        self.program_version = program_version
        self.binding_layout_version = binding_layout_version
        self.pipeline_cache = {}
        self.binding_cache = {}
        self.command_cache = {}
        self.metrics = {
            "pipeline_hits": 0,
            "pipeline_misses": 0,
            "binding_hits": 0,
            "binding_misses": 0,
            "command_hits": 0,
            "command_misses": 0,
            "stale_generation_rejections": 0,
            "device_resets": 0,
        }

    @property
    def device_generation(self):
        return self.backend.device_generation

    def _cache_get(self, cache, key, hit_key, miss_key, creator):
        entry = cache.get(key)
        if entry is not None:
            if entry["device_generation"] != self.device_generation:
                self.metrics["stale_generation_rejections"] += 1
                raise StaleSubmitGeneration("cached backend handle belongs to stale device generation")
            self.metrics[hit_key] += 1
            return entry
        handle = creator(key)
        entry = {
            "logical_key": key,
            "handle": handle,
            "device_generation": self.device_generation,
        }
        cache[key] = entry
        self.metrics[miss_key] += 1
        return entry

    def _pipeline(self, descriptor):
        key = descriptor.key()
        return self._cache_get(
            self.pipeline_cache,
            key,
            "pipeline_hits",
            "pipeline_misses",
            self.backend.create_pipeline,
        )

    def _binding(self, refs):
        normalized = [ref.normalized() for ref in refs]
        for ref in refs:
            if ref.device_generation != self.device_generation:
                self.metrics["stale_generation_rejections"] += 1
                raise StaleSubmitGeneration(
                    f"binding {ref.identity} belongs to stale device generation"
                )
            if ref.layout_version != self.binding_layout_version:
                raise ValueError("binding layout version mismatch")
        key = _digest({
            "schema": "chaptera.binding-key.v1",
            "device_generation": self.device_generation,
            "refs": normalized,
        })
        entry = self._cache_get(
            self.binding_cache,
            key,
            "binding_hits",
            "binding_misses",
            self.backend.create_binding,
        )
        entry["refs"] = normalized
        return entry

    def _command(self, *, segment_fingerprint, batch_ordinal, pipeline_key, binding_key, baked_view_key=None):
        logical = {
            "schema": "chaptera.command-key.v1",
            "segment_fingerprint": segment_fingerprint,
            "batch_ordinal": batch_ordinal,
            "pipeline_key": pipeline_key,
            "binding_key": binding_key,
            "baked_view_key": baked_view_key,
        }
        key = _digest(logical)
        entry = self._cache_get(
            self.command_cache,
            key,
            "command_hits",
            "command_misses",
            self.backend.create_command,
        )
        entry["segment_fingerprint"] = segment_fingerprint
        entry["binding_key"] = binding_key
        entry["pipeline_key"] = pipeline_key
        entry["baked_view_key"] = baked_view_key
        return entry

    def _descriptor(self, batch, target):
        key = batch["batch_key"]
        primitive_kind = key[0]
        paint_id = key[1]
        resource_id = key[2]
        clip_id = key[3]
        effect_group_id = key[4]
        blend_mode = key[5] or "normal"
        material_mode = (
            "resource" if resource_id else "paint" if paint_id else "unmaterialized"
        )
        return PipelineDescriptorV1(
            backend_id=self.backend.backend_id,
            program_version=self.program_version,
            primitive_kind=primitive_kind,
            material_mode=material_mode,
            target_format=target["format"],
            sample_count=int(target["sample_count"]),
            blend_mode=blend_mode,
            composite_mode=target.get("composite_mode", "source_over"),
            depth_stencil_mode=target.get("depth_stencil_mode", "none"),
            clip_mode="clip" if clip_id else "none",
        )

    def build_submit_plan(
        self,
        segment_scene_plan,
        *,
        bindings_by_atom,
        view_state,
        target,
        bake_view_into_commands=False,
    ):
        if segment_scene_plan.get("schema") != "chaptera.render-segment-scene-plan.v1":
            raise ValueError("SubmitRuntime requires RenderSegment scene plan")
        frame_constants = {
            "view_state": copy.deepcopy(view_state),
            "target": copy.deepcopy(target),
        }
        view_key = _digest(frame_constants["view_state"]) if bake_view_into_commands else None
        records = []
        for page in segment_scene_plan["pages"]:
            for segment in page["segments"]:
                records.append({
                    "op": "BeginSegment",
                    "page_id": page["page_id"],
                    "segment_fingerprint": segment["fingerprint"],
                })
                for batch_ordinal, batch in enumerate(segment["batches"]):
                    descriptor = self._descriptor(batch, target)
                    pipeline = self._pipeline(descriptor)
                    refs = []
                    for atom_id in batch["atom_ids"]:
                        atom_refs = bindings_by_atom.get(atom_id, [])
                        if not atom_refs:
                            atom_refs = [
                                RuntimeBindingRefV1(
                                    identity=f"atom:{atom_id}:no-material",
                                    binding_kind="logical",
                                    material_generation=1,
                                    device_generation=self.device_generation,
                                    layout_version=self.binding_layout_version,
                                )
                            ]
                        refs.extend(atom_refs)
                    binding = self._binding(refs)
                    command = self._command(
                        segment_fingerprint=segment["fingerprint"],
                        batch_ordinal=batch_ordinal,
                        pipeline_key=pipeline["logical_key"],
                        binding_key=binding["logical_key"],
                        baked_view_key=view_key,
                    )
                    records.extend([
                        {
                            "op": "SetPipeline",
                            "pipeline_key": pipeline["logical_key"],
                        },
                        {
                            "op": "SetBindings",
                            "binding_key": binding["logical_key"],
                            "binding_identities": [ref.identity for ref in refs],
                        },
                        {
                            "op": "DrawInstances",
                            "command_key": command["logical_key"],
                            "atom_ids": list(batch["atom_ids"]),
                        },
                    ])
                records.append({
                    "op": "EndSegment",
                    "segment_fingerprint": segment["fingerprint"],
                })
        normalized = {
            "schema": "chaptera.normalized-submit-plan.v1",
            "segment_plan_fingerprint": segment_scene_plan["fingerprint"],
            "frame_constants": frame_constants,
            "view_baked_into_commands": bake_view_into_commands,
            "records": records,
        }
        normalized["submit_fingerprint"] = _digest(normalized)
        return normalized

    def invalidate_segment(self, segment_fingerprint):
        doomed = [
            key
            for key, entry in self.command_cache.items()
            if entry.get("segment_fingerprint") == segment_fingerprint
        ]
        for key in doomed:
            self.command_cache.pop(key, None)
        return len(doomed)

    def invalidate_binding_identity(self, identity):
        doomed_bindings = []
        for key, entry in self.binding_cache.items():
            if any(ref["identity"] == identity for ref in entry.get("refs", [])):
                doomed_bindings.append(key)
        for key in doomed_bindings:
            self.binding_cache.pop(key, None)
        doomed_commands = [
            key
            for key, entry in self.command_cache.items()
            if entry.get("binding_key") in doomed_bindings
        ]
        for key in doomed_commands:
            self.command_cache.pop(key, None)
        return {
            "bindings_invalidated": len(doomed_bindings),
            "commands_invalidated": len(doomed_commands),
        }

    def reset_device(self):
        self.backend.reset_device()
        self.pipeline_cache.clear()
        self.binding_cache.clear()
        self.command_cache.clear()
        self.metrics["device_resets"] += 1

    def cache_receipt(self):
        return {
            "schema": SCHEMA,
            "backend_id": self.backend.backend_id,
            "device_generation": self.device_generation,
            "pipeline_entries": len(self.pipeline_cache),
            "binding_entries": len(self.binding_cache),
            "command_entries": len(self.command_cache),
            "metrics": dict(self.metrics),
            "backend_builds": {
                "pipelines": self.backend.pipeline_builds,
                "bindings": self.backend.binding_builds,
                "commands": self.backend.command_builds,
            },
            "backend_handles_semantic": False,
        }


def binding_ref_from_texture_v1(texture_residency, key, *, layout_version="layout:v1"):
    binding = texture_residency.binding(key)
    return RuntimeBindingRefV1(
        identity=texture_residency.identity_for_key(key),
        binding_kind="texture",
        material_generation=binding["generation"],
        device_generation=binding["device_generation"],
        layout_version=layout_version,
    )


def binding_ref_from_buffer_v1(buffer_residency, handle, *, logical_identity, layout_version="layout:v1"):
    binding = buffer_residency.binding(handle)
    return RuntimeBindingRefV1(
        identity=logical_identity,
        binding_kind="buffer",
        material_generation=binding["slot_generation"],
        placement_generation=binding["placement_generation"],
        device_generation=binding["device_generation"],
        layout_version=layout_version,
    )
