#!/usr/bin/env python3
"""Reusable hosted residency instrumentation for CLOUD-HOT-MEMORY-INSTRUMENT-01."""

from __future__ import annotations

import ctypes
import gc
import hashlib
import os
import platform
import threading
import time
from dataclasses import dataclass
from typing import Callable

LAYERS = ("L1", "L2", "L3", "L4", "L5", "L6")
EVICTION_ORDER = ("L5", "L4", "L3", "L2")


class ResidencyProbeError(RuntimeError):
    pass


def process_rss_bytes() -> tuple[int, str]:
    statm = "/proc/self/statm"
    if os.path.exists(statm):
        resident_pages = int(open(statm, "r", encoding="ascii").read().split()[1])
        return resident_pages * os.sysconf("SC_PAGE_SIZE"), "procfs_statm_rss"

    # Portable fallback. ru_maxrss is a high-water mark rather than current RSS,
    # so callers preserve the method label and must not confuse the two.
    import resource

    value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if platform.system() == "Darwin":
        return int(value), "getrusage_ru_maxrss_peak"
    return int(value) * 1024, "getrusage_ru_maxrss_peak"


def trim_allocator() -> None:
    gc.collect()
    if platform.system() != "Linux":
        return
    try:
        libc = ctypes.CDLL(None)
        malloc_trim = libc.malloc_trim
        malloc_trim.argtypes = [ctypes.c_size_t]
        malloc_trim.restype = ctypes.c_int
        malloc_trim(0)
    except (AttributeError, OSError):
        pass


class PeakRssSampler:
    def __init__(self, interval_s: float = 0.002):
        self.interval_s = interval_s
        self.peak = 0
        self.method = ""
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def __enter__(self) -> "PeakRssSampler":
        rss, method = process_rss_bytes()
        self.peak = rss
        self.method = method

        def sample() -> None:
            while not self._stop.wait(self.interval_s):
                rss_now, _ = process_rss_bytes()
                self.peak = max(self.peak, rss_now)

        self._thread = threading.Thread(target=sample, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join()
        rss, _ = process_rss_bytes()
        self.peak = max(self.peak, rss)


@dataclass
class OwnedAllocation:
    layer: str
    token: str
    payload: bytearray

    @property
    def size(self) -> int:
        return len(self.payload)


class ResidencyProbe:
    """Tracks exclusive layer ownership and deterministic lifecycle transitions."""

    def __init__(self) -> None:
        self._owned: dict[str, dict[str, OwnedAllocation]] = {layer: {} for layer in LAYERS}
        self._token_owner: dict[str, str] = {}
        self._shared: dict[str, bytearray] = {}
        self.phase = "COLD"
        self._eviction_index = 0

    def allocate(self, layer: str, token: str, byte_count: int, *, fill: int = 0xA5) -> None:
        if layer not in self._owned:
            raise ResidencyProbeError(f"unknown layer {layer}")
        if byte_count < 0:
            raise ResidencyProbeError("byte_count must be non-negative")
        if token in self._token_owner or token in self._shared:
            raise ResidencyProbeError(f"allocation token already owned: {token}")
        allocation = OwnedAllocation(layer, token, bytearray([fill]) * byte_count)
        self._owned[layer][token] = allocation
        self._token_owner[token] = layer

    def allocate_shared(self, category: str, token: str, byte_count: int) -> None:
        """Register explicit shared bytes outside the L1-L6 exclusive sum."""
        if not category:
            raise ResidencyProbeError("shared category required")
        full_token = f"shared:{category}:{token}"
        if token in self._token_owner or full_token in self._shared:
            raise ResidencyProbeError(f"shared allocation token already owned: {token}")
        self._shared[full_token] = bytearray(byte_count)

    def layer_bytes(self, layer: str) -> int:
        return sum(item.size for item in self._owned[layer].values())

    def layer_metrics(self) -> dict[str, int]:
        return {layer: self.layer_bytes(layer) for layer in LAYERS}

    def shared_bytes(self) -> int:
        return sum(len(value) for value in self._shared.values())

    def transition(self, target: str) -> None:
        allowed = {
            "COLD": {"WARM"},
            "WARM": {"HOT"},
            "HOT": set(),
        }
        if target not in allowed.get(self.phase, set()):
            raise ResidencyProbeError(f"invalid transition {self.phase}->{target}")
        self.phase = target

    def evict(self, layer: str) -> int:
        if self.phase != "HOT":
            raise ResidencyProbeError("eviction requires HOT phase")
        expected = EVICTION_ORDER[self._eviction_index]
        if layer != expected:
            raise ResidencyProbeError(f"eviction order violation: expected {expected}, got {layer}")
        before = self.layer_bytes(layer)
        for token in list(self._owned[layer]):
            del self._token_owner[token]
        self._owned[layer].clear()
        trim_allocator()
        self._eviction_index += 1
        return before

    def reset(self) -> None:
        for layer in LAYERS:
            self._owned[layer].clear()
        self._token_owner.clear()
        self._shared.clear()
        self.phase = "COLD"
        self._eviction_index = 0
        trim_allocator()


def activation_probe(source_bytes: bytes, snapshot_lag_edges: int, tail_bytes: int) -> dict[str, int | float]:
    """Measure a bounded synthetic activation/replay workload without retaining source bytes."""
    rss_before, _ = process_rss_bytes()
    cpu_start = time.process_time_ns()
    wall_start = time.perf_counter_ns()
    with PeakRssSampler() as sampler:
        digest = hashlib.sha256(source_bytes).digest()
        tail = bytes((i * 31) & 0xFF for i in range(tail_bytes))
        replay = digest
        for i in range(snapshot_lag_edges):
            replay = hashlib.sha256(replay + tail[: min(len(tail), 4096)] + i.to_bytes(4, "little")).digest()
        # Force the result to stay live through the measurement.
        if len(replay) != 32:
            raise AssertionError("sha256 replay invariant")
    wall_ms = (time.perf_counter_ns() - wall_start) / 1_000_000
    cpu_ms = (time.process_time_ns() - cpu_start) / 1_000_000
    return {
        "snapshot_lag_edges": snapshot_lag_edges,
        "tail_bytes": tail_bytes,
        "activation_wall_ms": wall_ms,
        "replay_cpu_ms": cpu_ms,
        "bytes_read": len(source_bytes) + tail_bytes,
        "peak_rss_bytes": max(rss_before, sampler.peak),
    }
