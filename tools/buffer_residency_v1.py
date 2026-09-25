#!/usr/bin/env python3
from __future__ import annotations

from dataclasses import dataclass
from typing import Any

SCHEMA = "chaptera.buffer-residency.v1"


@dataclass(frozen=True)
class LogicalHandle:
    slot_id: int
    generation: int


@dataclass
class Slot:
    slot_id: int
    logical_id: str
    page_id: str | None
    generation: int
    placement_generation: int
    state: str
    data: bytearray
    offset: int | None
    length: int


class StaleHandle(ValueError):
    pass


class StagingRingBusy(RuntimeError):
    pass


def _align(value: int, alignment: int) -> int:
    return ((value + alignment - 1) // alignment) * alignment


def _merge_spans(spans: list[tuple[int, int]], gap: int = 0) -> list[tuple[int, int]]:
    if not spans:
        return []
    ordered = sorted((a, b) for a, b in spans if b > a)
    out = [ordered[0]]
    for start, end in ordered[1:]:
        pstart, pend = out[-1]
        if start <= pend + gap:
            out[-1] = (pstart, max(pend, end))
        else:
            out.append((start, end))
    return out


class StagingRing:
    def __init__(self, capacity: int):
        if capacity <= 0:
            raise ValueError("capacity must be positive")
        self.capacity = capacity
        self.cursor = 0
        self.in_flight: list[tuple[int, int, int]] = []

    def plan(self, size: int, *, fence: int, completed_fence: int) -> dict[str, Any]:
        if size <= 0:
            raise ValueError("size must be positive")
        self.in_flight = [row for row in self.in_flight if row[2] > completed_fence]
        if size > self.capacity:
            return {"mode": "oversized", "offset": None, "size": size, "fence": fence}

        candidates = [self.cursor]
        if self.cursor + size > self.capacity:
            candidates = [0]
        elif self.cursor != 0:
            candidates.append(0)

        for start in candidates:
            end = start + size
            overlap = any(not (end <= a or start >= b) for a, b, _ in self.in_flight)
            if not overlap:
                self.in_flight.append((start, end, fence))
                self.cursor = 0 if end == self.capacity else end
                return {"mode": "ring", "offset": start, "size": size, "fence": fence}
        raise StagingRingBusy("no non-overlapping staging region available")


class BufferResidencyV1:
    def __init__(self, *, initial_capacity: int = 4096, alignment: int = 16, coalesce_gap: int = 64):
        if initial_capacity <= 0 or alignment <= 0 or coalesce_gap < 0:
            raise ValueError("invalid allocator configuration")
        self.capacity = _align(initial_capacity, alignment)
        self.alignment = alignment
        self.coalesce_gap = coalesce_gap
        self.device_generation = 1
        self._slots: dict[int, Slot] = {}
        self._logical_to_slot: dict[str, int] = {}
        self._next_slot = 1
        self._free_ranges: list[tuple[int, int]] = [(0, self.capacity)]
        self._dirty: list[tuple[int, int]] = []
        self.metrics = {
            "allocations": 0,
            "frees": 0,
            "growth_events": 0,
            "compactions": 0,
            "page_evictions": 0,
            "page_revisits": 0,
            "device_resets": 0,
            "stale_handle_rejections": 0,
            "stale_binding_rejections": 0,
        }

    def _coalesce_free(self):
        self._free_ranges = _merge_spans(self._free_ranges, 0)

    def _free_range(self, offset: int, length: int):
        self._free_ranges.append((offset, offset + length))
        self._coalesce_free()

    def _grow(self, required: int):
        old = self.capacity
        new = old
        while new - old < required:
            new *= 2
        self.capacity = new
        self._free_ranges.append((old, new))
        self._coalesce_free()
        self.metrics["growth_events"] += 1

    def _allocate_range(self, length: int) -> tuple[int, int]:
        need = _align(length, self.alignment)
        while True:
            for index, (start, end) in enumerate(self._free_ranges):
                aligned = _align(start, self.alignment)
                if aligned + need <= end:
                    before = (start, aligned)
                    after = (aligned + need, end)
                    replacement = []
                    if before[1] > before[0]:
                        replacement.append(before)
                    if after[1] > after[0]:
                        replacement.append(after)
                    self._free_ranges[index:index + 1] = replacement
                    return aligned, need
            self._grow(need)

    def _slot_for_handle(self, handle: LogicalHandle) -> Slot:
        slot = self._slots.get(handle.slot_id)
        if slot is None or slot.generation != handle.generation or slot.state == "free":
            self.metrics["stale_handle_rejections"] += 1
            raise StaleHandle("stale logical buffer handle")
        return slot

    def allocate(self, logical_id: str, data: bytes, *, page_id: str | None = None) -> LogicalHandle:
        if not logical_id or not data:
            raise ValueError("logical_id and non-empty data are required")
        existing_id = self._logical_to_slot.get(logical_id)
        if existing_id is not None and self._slots[existing_id].state != "free":
            raise ValueError("logical_id already allocated")

        if existing_id is None:
            slot_id = self._next_slot
            self._next_slot += 1
            generation = 1
        else:
            slot_id = existing_id
            generation = self._slots[slot_id].generation

        offset, reserved = self._allocate_range(len(data))
        slot = Slot(
            slot_id=slot_id,
            logical_id=logical_id,
            page_id=page_id,
            generation=generation,
            placement_generation=1,
            state="resident",
            data=bytearray(data),
            offset=offset,
            length=reserved,
        )
        self._slots[slot_id] = slot
        self._logical_to_slot[logical_id] = slot_id
        self._dirty.append((offset, offset + len(data)))
        self.metrics["allocations"] += 1
        return LogicalHandle(slot_id, generation)

    def free(self, handle: LogicalHandle):
        slot = self._slot_for_handle(handle)
        if slot.state == "resident" and slot.offset is not None:
            self._free_range(slot.offset, slot.length)
        slot.state = "free"
        slot.offset = None
        slot.data = bytearray()
        slot.length = 0
        slot.generation += 1
        slot.placement_generation += 1
        self.metrics["frees"] += 1

    def update(self, handle: LogicalHandle, data: bytes, *, byte_offset: int = 0):
        slot = self._slot_for_handle(handle)
        if slot.state != "resident":
            raise ValueError("slot is not resident")
        if byte_offset < 0:
            raise ValueError("byte_offset must be non-negative")
        required = byte_offset + len(data)
        if required > slot.length:
            old_offset, old_length = slot.offset, slot.length
            new_offset, new_length = self._allocate_range(required)
            slot.offset = new_offset
            slot.length = new_length
            slot.placement_generation += 1
            if old_offset is not None:
                self._free_range(old_offset, old_length)
            if len(slot.data) < required:
                slot.data.extend(b"\x00" * (required - len(slot.data)))
            self._dirty.append((new_offset, new_offset + required))
        else:
            if len(slot.data) < required:
                slot.data.extend(b"\x00" * (required - len(slot.data)))
            self._dirty.append((slot.offset + byte_offset, slot.offset + required))
        slot.data[byte_offset:required] = data

    def binding(self, handle: LogicalHandle) -> dict[str, Any]:
        slot = self._slot_for_handle(handle)
        if slot.state != "resident" or slot.offset is None:
            raise ValueError("slot is not physically resident")
        return {
            "slot_id": slot.slot_id,
            "slot_generation": slot.generation,
            "placement_generation": slot.placement_generation,
            "device_generation": self.device_generation,
            "offset": slot.offset,
            "length": slot.length,
        }

    def validate_binding(self, binding: dict[str, Any]) -> bool:
        slot = self._slots.get(binding.get("slot_id"))
        ok = bool(
            slot
            and slot.state == "resident"
            and binding.get("slot_generation") == slot.generation
            and binding.get("placement_generation") == slot.placement_generation
            and binding.get("device_generation") == self.device_generation
            and binding.get("offset") == slot.offset
            and binding.get("length") == slot.length
        )
        if not ok:
            self.metrics["stale_binding_rejections"] += 1
        return ok

    def upload_plan(self) -> dict[str, Any]:
        logical = _merge_spans(self._dirty, 0)
        physical = _merge_spans(self._dirty, self.coalesce_gap)
        logical_bytes = sum(end - start for start, end in logical)
        physical_bytes = sum(end - start for start, end in physical)
        return {
            "logical_changed_bytes": logical_bytes,
            "physical_upload_bytes": physical_bytes,
            "upload_amplification_ratio": (physical_bytes / logical_bytes) if logical_bytes else 0.0,
            "spans": [{"offset": start, "length": end - start} for start, end in physical],
        }

    def clear_dirty(self):
        self._dirty.clear()

    def fragmentation(self) -> dict[str, Any]:
        free = [(a, b) for a, b in self._free_ranges if b > a]
        free_bytes = sum(b - a for a, b in free)
        largest = max((b - a for a, b in free), default=0)
        live = sum(slot.length for slot in self._slots.values() if slot.state == "resident")
        return {
            "reserved_bytes": self.capacity,
            "live_bytes": live,
            "free_bytes": free_bytes,
            "largest_free_span": largest,
            "free_range_count": len(free),
            "fragmentation_ratio": 0.0 if free_bytes == 0 else 1.0 - (largest / free_bytes),
        }

    def normalized_logical_state(self) -> list[dict[str, Any]]:
        return [
            {
                "slot_id": slot.slot_id,
                "logical_id": slot.logical_id,
                "page_id": slot.page_id,
                "generation": slot.generation,
                "state": slot.state,
                "data_hex": bytes(slot.data).hex(),
            }
            for slot in sorted(self._slots.values(), key=lambda s: s.slot_id)
            if slot.state != "free"
        ]

    def compact(self) -> dict[str, Any]:
        before = self.normalized_logical_state()
        cursor = 0
        moved = []
        for slot in sorted((s for s in self._slots.values() if s.state == "resident"), key=lambda s: (s.offset, s.slot_id)):
            target = _align(cursor, self.alignment)
            if slot.offset != target:
                moved.append(slot.slot_id)
                slot.offset = target
                slot.placement_generation += 1
                self._dirty.append((target, target + len(slot.data)))
            cursor = target + slot.length
        self._free_ranges = [(cursor, self.capacity)] if cursor < self.capacity else []
        self.metrics["compactions"] += 1
        after = self.normalized_logical_state()
        if before != after:
            raise AssertionError("compaction changed logical buffer state")
        return {"moved_slot_ids": moved, "moved_count": len(moved)}

    def evict_page(self, page_id: str) -> int:
        count = 0
        for slot in self._slots.values():
            if slot.page_id == page_id and slot.state == "resident":
                self._free_range(slot.offset, slot.length)
                slot.offset = None
                slot.state = "evicted"
                slot.placement_generation += 1
                count += 1
        self.metrics["page_evictions"] += count
        return count

    def revisit_page(self, page_id: str) -> int:
        count = 0
        for slot in self._slots.values():
            if slot.page_id == page_id and slot.state == "evicted":
                offset, length = self._allocate_range(max(1, len(slot.data)))
                slot.offset = offset
                slot.length = length
                slot.state = "resident"
                slot.placement_generation += 1
                self._dirty.append((offset, offset + len(slot.data)))
                count += 1
        self.metrics["page_revisits"] += count
        return count

    def memory_entries(self) -> list[dict[str, Any]]:
        """Public governor adapter surface; physical offsets stay private."""
        return [
            {
                "identity": slot.logical_id,
                "resident_bytes": slot.length if slot.state == "resident" else 0,
                "reclaimable": slot.state == "resident",
                "state": slot.state,
                "page_id": slot.page_id,
                "generation": slot.generation,
                "placement_generation": slot.placement_generation,
            }
            for slot in sorted(self._slots.values(), key=lambda row: row.logical_id)
            if slot.state != "free"
        ]

    def evict_logical(self, logical_id: str) -> dict[str, Any]:
        slot_id = self._logical_to_slot.get(logical_id)
        if slot_id is None:
            return {"bytes_reclaimed": 0, "identity": logical_id}
        slot = self._slots[slot_id]
        if slot.state != "resident" or slot.offset is None:
            return {"bytes_reclaimed": 0, "identity": logical_id}
        reclaimed = slot.length
        self._free_range(slot.offset, slot.length)
        slot.offset = None
        slot.state = "evicted"
        slot.placement_generation += 1
        return {"bytes_reclaimed": reclaimed, "identity": logical_id}

    def invalidate_logical(self, logical_id: str) -> dict[str, Any]:
        result = self.evict_logical(logical_id)
        result["invalidated"] = logical_id
        return result

    def reset_device(self):
        resident = [slot for slot in self._slots.values() if slot.state == "resident"]
        self.device_generation += 1
        self.metrics["device_resets"] += 1
        self._free_ranges = [(0, self.capacity)]
        self._dirty.clear()
        for slot in sorted(resident, key=lambda s: s.slot_id):
            offset, length = self._allocate_range(max(1, len(slot.data)))
            slot.offset = offset
            slot.length = length
            slot.placement_generation += 1
            self._dirty.append((offset, offset + len(slot.data)))

    def receipt(self) -> dict[str, Any]:
        return {
            "schema": SCHEMA,
            "device_generation": self.device_generation,
            "slot_count": sum(1 for s in self._slots.values() if s.state != "free"),
            "resident_slot_count": sum(1 for s in self._slots.values() if s.state == "resident"),
            "fragmentation": self.fragmentation(),
            "upload_plan": self.upload_plan(),
            "metrics": dict(self.metrics),
            "authority": {
                "physical_offsets_are_semantic": False,
                "authoring_mutations": 0,
                "layout_mutations": 0,
            },
        }
