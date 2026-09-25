#!/usr/bin/env python3
import json
import time

from buffer_residency_v1 import BufferResidencyV1


def run_scale(count: int) -> dict:
    r = BufferResidencyV1(initial_capacity=max(4096, count * 48), coalesce_gap=64)
    payload = b"x" * 32
    handles = []
    t0 = time.perf_counter_ns()
    for i in range(count):
        handles.append(r.allocate(f"atom:{i}", payload, page_id=f"p:{i // 1000}"))
    allocate_ms = (time.perf_counter_ns() - t0) / 1e6
    r.clear_dirty()

    t1 = time.perf_counter_ns()
    r.update(handles[count // 2], b"12345678", byte_offset=8)
    one_plan = r.upload_plan()
    one_patch_ms = (time.perf_counter_ns() - t1) / 1e6
    r.clear_dirty()

    t2 = time.perf_counter_ns()
    stride = max(1, count // 100)
    for i in range(0, count, stride):
        if i // stride >= 100:
            break
        r.update(handles[i], b"abcdefgh", byte_offset=8)
    hundred_plan = r.upload_plan()
    hundred_patch_ms = (time.perf_counter_ns() - t2) / 1e6

    full_buffer_baseline = r.fragmentation()["live_bytes"]
    return {
        "slot_count": count,
        "allocate_ms": allocate_ms,
        "one_node_patch_ms": one_patch_ms,
        "one_node_logical_bytes": one_plan["logical_changed_bytes"],
        "one_node_upload_bytes": one_plan["physical_upload_bytes"],
        "one_node_full_buffer_baseline_bytes": full_buffer_baseline,
        "one_node_upload_vs_full_ratio": one_plan["physical_upload_bytes"] / full_buffer_baseline,
        "hundred_node_patch_ms": hundred_patch_ms,
        "hundred_node_logical_bytes": hundred_plan["logical_changed_bytes"],
        "hundred_node_upload_bytes": hundred_plan["physical_upload_bytes"],
        "fragmentation": r.fragmentation(),
    }


receipt = {
    "schema": "chaptera.buffer-residency-benchmark.v1",
    "measurement_class": "synthetic_fake_backend_buffer_arena",
    "real_pub": False,
    "representative": False,
    "scales": [run_scale(n) for n in (10_000, 50_000, 100_000)],
}
print(json.dumps(receipt, indent=2, sort_keys=True))
