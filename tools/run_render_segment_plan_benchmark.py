#!/usr/bin/env python3
import json
import time

from render_bench_v1 import shape_workload
from render_scene_v1 import compile_render_scene
from render_segment_plan_v1 import plan_scene


def run(count):
    scene = compile_render_scene(shape_workload(count, pages=max(1, count // 5000), overlap=True, label=f"segment-{count}"))
    t0 = time.perf_counter_ns()
    plan = plan_scene(scene)
    elapsed_ms = (time.perf_counter_ns() - t0) / 1e6
    visible = sum(p["visible_atom_count"] for p in plan["pages"])
    segments = sum(p["segment_count"] for p in plan["pages"])
    batches = sum(p["batch_count"] for p in plan["pages"])
    return {
        "atoms": visible,
        "pages": len(plan["pages"]),
        "planning_ms": elapsed_ms,
        "segments": segments,
        "batches": batches,
        "estimated_draws_without_batching": visible,
        "estimated_draws_with_contiguous_batching": batches,
        "estimated_draw_call_reduction": visible - batches,
    }


receipt = {
    "schema": "chaptera.render-segment-plan-benchmark.v1",
    "measurement_class": "synthetic_ordered_shape_stress",
    "real_pub": False,
    "representative": False,
    "scales": [run(n) for n in (10_000, 50_000, 100_000)],
}
print(json.dumps(receipt, indent=2, sort_keys=True))
