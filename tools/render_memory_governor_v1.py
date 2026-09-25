#!/usr/bin/env python3
"""Renderer-local cross-cache memory arbitration with explicit pin leases."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

SCHEMA = "chaptera.render-memory-governor.v1"
DEFAULT_CLASS_PRIORITY = (
    "submit_command",
    "offscreen",
    "path",
    "glyph",
    "texture",
    "buffer",
)


@dataclass(frozen=True)
class PinLease:
    pool_id: str
    identity: str
    reason: str
    expires_after_frame: int


@dataclass
class PoolRegistration:
    pool_id: str
    residency_class: str
    adapter: Any
    metadata: dict[str, dict[str, Any]] = field(default_factory=dict)


class RenderMemoryGovernorV1:
    def __init__(
        self,
        *,
        soft_target_bytes: int,
        high_target_bytes: int,
        rearm_margin_bytes: int,
        cooldown_frames: int,
        class_priority=DEFAULT_CLASS_PRIORITY,
    ):
        if not (0 <= soft_target_bytes <= high_target_bytes):
            raise ValueError("soft/high targets must be configured and ordered")
        if rearm_margin_bytes < 0 or cooldown_frames < 0:
            raise ValueError("hysteresis configuration must be non-negative")
        self.soft_target_bytes = soft_target_bytes
        self.high_target_bytes = high_target_bytes
        self.rearm_margin_bytes = rearm_margin_bytes
        self.cooldown_frames = cooldown_frames
        self.class_priority = tuple(class_priority)
        self._pools: dict[str, PoolRegistration] = {}
        self._pins: list[PinLease] = []
        self._dependencies: dict[tuple[str, str], list[tuple[str, str]]] = {}
        self._last_evicted_frame: dict[tuple[str, str], int] = {}
        self._churn_rebuilds: dict[tuple[str, str], int] = {}
        self.metrics = {
            "pressure_events": 0,
            "bytes_reclaimed": 0,
            "entries_evicted": 0,
            "inability_to_reclaim_events": 0,
            "dependency_invalidations": 0,
            "pin_blocked_candidates": 0,
            "churn_rebuilds": 0,
        }

    def register_pool(self, pool_id: str, residency_class: str, adapter):
        if pool_id in self._pools:
            raise ValueError("pool_id already registered")
        if residency_class not in self.class_priority:
            raise ValueError("residency_class missing from configured priority")
        for name in ("memory_entries", "evict_identity", "invalidate_identity"):
            if not callable(getattr(adapter, name, None)):
                raise TypeError(f"adapter must provide {name}")
        self._pools[pool_id] = PoolRegistration(pool_id, residency_class, adapter)

    def note_entry(
        self,
        pool_id: str,
        identity: str,
        *,
        last_used_frame: int,
        demand_count: int = 0,
        rebuild_cost: int = 1,
        quality_rank: int = 0,
        active_window: bool = False,
    ):
        pool = self._pools[pool_id]
        key = (pool_id, identity)
        last_evicted = self._last_evicted_frame.get(key)
        if last_evicted is not None and last_used_frame - last_evicted <= self.cooldown_frames:
            self._churn_rebuilds[key] = self._churn_rebuilds.get(key, 0) + 1
            self.metrics["churn_rebuilds"] += 1
        pool.metadata[identity] = {
            "last_used_frame": int(last_used_frame),
            "demand_count": int(demand_count),
            "rebuild_cost": int(rebuild_cost),
            "quality_rank": int(quality_rank),
            "active_window": bool(active_window),
        }

    def pin(self, pool_id: str, identity: str, *, reason: str, expires_after_frame: int):
        if pool_id not in self._pools:
            raise KeyError("unknown pool")
        self._pins.append(PinLease(pool_id, identity, reason, int(expires_after_frame)))

    def add_dependency(
        self,
        source_pool_id: str,
        source_identity: str,
        dependent_pool_id: str,
        dependent_identity: str,
    ):
        if source_pool_id not in self._pools or dependent_pool_id not in self._pools:
            raise KeyError("dependency references unknown pool")
        key = (source_pool_id, source_identity)
        self._dependencies.setdefault(key, []).append((dependent_pool_id, dependent_identity))

    def _active_pins(self, frame: int) -> set[tuple[str, str]]:
        self._pins = [lease for lease in self._pins if lease.expires_after_frame >= frame]
        return {(lease.pool_id, lease.identity) for lease in self._pins}

    def accounting(self, frame: int) -> dict[str, Any]:
        pins = self._active_pins(frame)
        pools = {}
        total_resident = total_reclaimable = total_pinned = 0
        unknown_pools = []
        for pool_id, registration in sorted(self._pools.items()):
            entries = registration.adapter.memory_entries()
            resident = reclaimable = pinned = 0
            exact = True
            for entry in entries:
                if entry.get("resident_bytes") is None:
                    exact = False
                    continue
                bytes_ = int(entry["resident_bytes"])
                resident += bytes_
                if (pool_id, entry["identity"]) in pins or not entry.get("reclaimable", True):
                    pinned += bytes_
                else:
                    reclaimable += bytes_
            pools[pool_id] = {
                "residency_class": registration.residency_class,
                "resident_bytes": resident if exact else None,
                "reclaimable_bytes": reclaimable if exact else None,
                "pinned_bytes": pinned if exact else None,
                "entry_count": len(entries),
                "device_generation": registration.adapter.device_generation(),
                "precise_bytes_known": exact,
            }
            if exact:
                total_resident += resident
                total_reclaimable += reclaimable
                total_pinned += pinned
            else:
                unknown_pools.append(pool_id)
        return {
            "schema": SCHEMA,
            "frame": frame,
            "resident_bytes_known": total_resident,
            "reclaimable_bytes_known": total_reclaimable,
            "pinned_bytes_known": total_pinned,
            "unknown_byte_pools": unknown_pools,
            "pools": pools,
        }

    def pressure_state(self, frame: int) -> str:
        resident = self.accounting(frame)["resident_bytes_known"]
        if resident > self.high_target_bytes:
            return "high_pressure"
        if resident > self.soft_target_bytes:
            return "soft_pressure"
        return "normal"

    def _candidate_rows(self, frame: int):
        pins = self._active_pins(frame)
        class_rank = {name: index for index, name in enumerate(self.class_priority)}
        rows = []
        for pool_id, registration in self._pools.items():
            for entry in registration.adapter.memory_entries():
                identity = entry["identity"]
                resident = entry.get("resident_bytes")
                if resident in (None, 0) or not entry.get("reclaimable", True):
                    continue
                if (pool_id, identity) in pins:
                    self.metrics["pin_blocked_candidates"] += 1
                    continue
                meta = registration.metadata.get(identity, {})
                if meta.get("demand_count", 0) > 0 or meta.get("active_window", False):
                    continue
                last_evicted = self._last_evicted_frame.get((pool_id, identity))
                if (
                    last_evicted is not None
                    and frame - last_evicted <= self.cooldown_frames
                ):
                    continue
                rows.append({
                    "pool_id": pool_id,
                    "residency_class": registration.residency_class,
                    "identity": identity,
                    "resident_bytes": int(resident),
                    "quality_rank": meta.get("quality_rank", 0),
                    "last_used_frame": meta.get("last_used_frame", -1),
                    "rebuild_cost": meta.get("rebuild_cost", 1),
                    "class_rank": class_rank[registration.residency_class],
                })
        rows.sort(key=lambda row: (
            row["class_rank"],
            -row["quality_rank"],
            row["last_used_frame"],
            row["rebuild_cost"],
            row["identity"],
        ))
        return rows

    def reclaim_to(self, *, target_resident_bytes: int, frame: int, reason: str) -> dict[str, Any]:
        if target_resident_bytes < 0:
            raise ValueError("target_resident_bytes must be non-negative")
        before = self.accounting(frame)
        resident = before["resident_bytes_known"]
        requested = max(0, resident - target_resident_bytes)
        self.metrics["pressure_events"] += 1
        reclaimed = 0
        decisions = []
        for candidate in self._candidate_rows(frame):
            if resident - reclaimed <= target_resident_bytes:
                break
            pool_id = candidate["pool_id"]
            identity = candidate["identity"]
            pool = self._pools[pool_id]
            result = pool.adapter.evict_identity(identity)
            bytes_reclaimed = int(result.get("bytes_reclaimed", 0))
            if bytes_reclaimed <= 0:
                continue
            reclaimed += bytes_reclaimed
            self._last_evicted_frame[(pool_id, identity)] = frame
            invalidated = []
            for dependent_pool_id, dependent_identity in sorted(
                self._dependencies.get((pool_id, identity), [])
            ):
                dep = self._pools[dependent_pool_id]
                dep.adapter.invalidate_identity(dependent_identity)
                invalidated.append([dependent_pool_id, dependent_identity])
                self.metrics["dependency_invalidations"] += 1
            decisions.append({
                "pool_id": pool_id,
                "residency_class": candidate["residency_class"],
                "identity": identity,
                "bytes_reclaimed": bytes_reclaimed,
                "dependent_invalidations": invalidated,
            })
        success = resident - reclaimed <= target_resident_bytes
        if not success:
            self.metrics["inability_to_reclaim_events"] += 1
        self.metrics["bytes_reclaimed"] += reclaimed
        self.metrics["entries_evicted"] += len(decisions)
        return {
            "schema": "chaptera.render-memory-reclaim-receipt.v1",
            "reason": reason,
            "frame": frame,
            "target_resident_bytes": target_resident_bytes,
            "requested_reclaim_bytes": requested,
            "bytes_reclaimed": reclaimed,
            "success": success,
            "before": before,
            "after": self.accounting(frame),
            "decisions": decisions,
            "canonical_mutations": 0,
        }

    def reset_device_generation(self):
        for registration in self._pools.values():
            reset = getattr(registration.adapter, "reset_device", None)
            if callable(reset):
                reset()
        self._pins.clear()

    def receipt(self, frame: int):
        return {
            "schema": SCHEMA,
            "pressure_state": self.pressure_state(frame),
            "accounting": self.accounting(frame),
            "metrics": dict(self.metrics),
            "thresholds_are_configuration": True,
            "canonical_authoring_owned_here": False,
            "canonical_layout_owned_here": False,
            "canonical_scene_owned_here": False,
        }


class BufferResidencyMemoryAdapterV1:
    def __init__(self, residency):
        self.residency = residency

    def memory_entries(self):
        return self.residency.memory_entries()

    def evict_identity(self, identity):
        return self.residency.evict_logical(identity)

    def invalidate_identity(self, identity):
        return self.residency.invalidate_logical(identity)

    def device_generation(self):
        return self.residency.device_generation

    def reset_device(self):
        self.residency.reset_device()


class TextureResidencyMemoryAdapterV1:
    def __init__(self, residency):
        self.residency = residency

    def memory_entries(self):
        return self.residency.memory_entries()

    def evict_identity(self, identity):
        return self.residency.evict_identity(identity)

    def invalidate_identity(self, identity):
        return self.residency.invalidate_identity(identity)

    def device_generation(self):
        return self.residency.device_generation

    def reset_device(self):
        self.residency.reset_device()


class RecordingInvalidationPoolV1:
    """Tiny fake dependent cache for bounded dependency invalidation tests."""

    def __init__(self, *, residency_class="submit_command", device_generation=1):
        self.residency_class = residency_class
        self._device_generation = device_generation
        self.entries = {}
        self.invalidated = []

    def add(self, identity, resident_bytes=0):
        self.entries[identity] = int(resident_bytes)

    def memory_entries(self):
        return [
            {
                "identity": identity,
                "resident_bytes": bytes_,
                "reclaimable": True,
            }
            for identity, bytes_ in sorted(self.entries.items())
        ]

    def evict_identity(self, identity):
        bytes_ = self.entries.pop(identity, 0)
        return {"bytes_reclaimed": bytes_}

    def invalidate_identity(self, identity):
        self.invalidated.append(identity)
        self.entries.pop(identity, None)
        return {"invalidated": identity}

    def device_generation(self):
        return self._device_generation

    def reset_device(self):
        self._device_generation += 1
        self.entries.clear()
