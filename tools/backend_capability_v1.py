from __future__ import annotations

from dataclasses import dataclass, asdict
from typing import FrozenSet

SCHEMA = "chaptera.backend-capability.v1"
REQUIREMENT_SCHEMA = "chaptera.render-requirements.v1"


@dataclass(frozen=True)
class BackendCapabilityDescriptor:
    backend_family: str
    implementation_version: str
    device_generation: int
    state: str
    primitive_modes: FrozenSet[str]
    material_modes: FrozenSet[str]
    clip_modes: FrozenSet[str]
    glyph_modes: FrozenSet[str]
    path_modes: FrozenSet[str]
    optional_features: FrozenSet[str]
    max_texture_dimension: int
    max_target_dimension: int
    max_buffer_bytes: int
    recoverable_loss: bool = True

    def receipt(self):
        value = asdict(self)
        for key in ("primitive_modes", "material_modes", "clip_modes", "glyph_modes", "path_modes", "optional_features"):
            value[key] = sorted(value[key])
        value["schema"] = SCHEMA
        return value


@dataclass(frozen=True)
class RenderRequirementSet:
    mandatory_features: FrozenSet[str]
    optional_quality_features: FrozenSet[str]
    performance_features: FrozenSet[str]
    max_texture_dimension: int = 0
    max_target_dimension: int = 0
    max_buffer_bytes: int = 0

    def receipt(self):
        return {
            "schema": REQUIREMENT_SCHEMA,
            "mandatory_features": sorted(self.mandatory_features),
            "optional_quality_features": sorted(self.optional_quality_features),
            "performance_features": sorted(self.performance_features),
            "max_texture_dimension": self.max_texture_dimension,
            "max_target_dimension": self.max_target_dimension,
            "max_buffer_bytes": self.max_buffer_bytes,
        }


def _features(desc: BackendCapabilityDescriptor) -> set[str]:
    out = set(desc.primitive_modes)
    out |= set(desc.material_modes)
    out |= {f"clip:{x}" for x in desc.clip_modes}
    out |= {f"glyph:{x}" for x in desc.glyph_modes}
    out |= {f"path:{x}" for x in desc.path_modes}
    out |= set(desc.optional_features)
    return out


def compatibility(desc: BackendCapabilityDescriptor, req: RenderRequirementSet) -> dict:
    if desc.state not in {"ready", "degraded"}:
        return {"outcome": "Unavailable", "reason_codes": [f"backend_state:{desc.state}"]}

    features = _features(desc)
    reasons = []
    missing_mandatory = sorted(req.mandatory_features - features)
    reasons.extend(f"missing_mandatory:{x}" for x in missing_mandatory)
    if req.max_texture_dimension > desc.max_texture_dimension:
        reasons.append("limit:max_texture_dimension")
    if req.max_target_dimension > desc.max_target_dimension:
        reasons.append("limit:max_target_dimension")
    if req.max_buffer_bytes > desc.max_buffer_bytes:
        reasons.append("limit:max_buffer_bytes")
    if reasons:
        return {"outcome": "Incompatible", "reason_codes": reasons}

    degraded = [f"missing_optional:{x}" for x in sorted(req.optional_quality_features - features)]
    if desc.state == "degraded":
        degraded.append("backend_state:degraded")
    if degraded:
        return {"outcome": "CompatibleDegraded", "reason_codes": degraded}
    return {"outcome": "CompatibleExact", "reason_codes": []}


class BackendLifecycle:
    def __init__(self, descriptor: BackendCapabilityDescriptor):
        self.descriptor = descriptor
        self.generation = descriptor.device_generation
        self.loss_count = 0
        self.rebuild_count = 0
        self.authoring_mutations = 0
        self.layout_mutations = 0

    def make_handle(self, logical_id: str) -> dict:
        if self.descriptor.state not in {"ready", "degraded"}:
            raise RuntimeError("backend unavailable")
        return {"logical_id": logical_id, "device_generation": self.generation}

    def validate_handle(self, handle: dict) -> bool:
        return handle.get("device_generation") == self.generation and self.descriptor.state in {"ready", "degraded"}

    def lose(self, permanent: bool = False):
        self.loss_count += 1
        state = "lost_permanent" if permanent else "lost_recoverable"
        self.descriptor = BackendCapabilityDescriptor(**{**asdict(self.descriptor), "state": state})

    def recreate(self, descriptor: BackendCapabilityDescriptor):
        next_generation = self.generation + 1
        if descriptor.device_generation != next_generation:
            descriptor = BackendCapabilityDescriptor(**{**asdict(descriptor), "device_generation": next_generation})
        self.generation = next_generation
        self.descriptor = descriptor

    def rebuild(self, *, scene_fingerprint: str, resource_fingerprint: str, view_fingerprint: str) -> dict:
        if self.descriptor.state not in {"ready", "degraded"}:
            raise RuntimeError("cannot rebuild unavailable backend")
        self.rebuild_count += 1
        return {
            "device_generation": self.generation,
            "scene_fingerprint": scene_fingerprint,
            "resource_fingerprint": resource_fingerprint,
            "view_fingerprint": view_fingerprint,
            "authoring_mutations": self.authoring_mutations,
            "layout_mutations": self.layout_mutations,
        }

    def receipt(self) -> dict:
        return {
            "schema": "chaptera.backend-lifecycle.v1",
            "device_generation": self.generation,
            "loss_count": self.loss_count,
            "rebuild_count": self.rebuild_count,
            "authoring_mutations": self.authoring_mutations,
            "layout_mutations": self.layout_mutations,
            "descriptor": self.descriptor.receipt(),
        }


def fake_backend(kind: str, generation: int = 1) -> BackendCapabilityDescriptor:
    base = dict(
        backend_family="fake-reference",
        implementation_version="1",
        device_generation=generation,
        state="ready",
        primitive_modes=frozenset({"rect", "image", "glyph", "path"}),
        material_modes=frozenset({"solid", "image_rgba8"}),
        clip_modes=frozenset({"rect", "stencil"}),
        glyph_modes=frozenset({"bitmap_atlas", "vector"}),
        path_modes=frozenset({"fill", "stroke"}),
        optional_features=frozenset({"timestamp_query", "msaa4", "offscreen_target"}),
        max_texture_dimension=8192,
        max_target_dimension=8192,
        max_buffer_bytes=256 * 1024 * 1024,
        recoverable_loss=True,
    )
    if kind == "full":
        return BackendCapabilityDescriptor(**base)
    if kind == "no_stencil":
        return BackendCapabilityDescriptor(**{**base, "clip_modes": frozenset({"rect"})})
    if kind == "low_limit":
        return BackendCapabilityDescriptor(**{**base, "max_texture_dimension": 1024, "max_target_dimension": 2048})
    if kind == "no_timing":
        return BackendCapabilityDescriptor(**{**base, "optional_features": frozenset({"msaa4", "offscreen_target"})})
    if kind == "degraded":
        return BackendCapabilityDescriptor(**{**base, "state": "degraded", "optional_features": frozenset({"offscreen_target"})})
    if kind == "permanent_failure":
        return BackendCapabilityDescriptor(**{**base, "state": "unsupported", "recoverable_loss": False})
    raise ValueError(f"unknown fake backend: {kind}")
