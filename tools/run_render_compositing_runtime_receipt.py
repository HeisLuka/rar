#!/usr/bin/env python3
import json

from render_compositing_runtime_v1 import CompositingRuntimeV1
from render_scene_v1 import compile_render_scene
from render_segment_plan_v1 import plan_scene
from test_render_compositing_runtime_v1 import base_source

src = base_source()
src["effect_groups"] = [{
    "effect_group_id": "g1",
    "effect_ids": [],
    "opacity_milli": 750,
    "isolation": True,
    "blend_mode": "normal",
    "composite_mode": "source_over",
}]
src["nodes"][0]["effect_group_id"] = "g1"
scene = compile_render_scene(src)
segments = plan_scene(scene)
runtime = CompositingRuntimeV1()
first = runtime.build_plan(
    scene,
    segments,
    group_surface_sizes={"g1": {"width_px": 64, "height_px": 64}},
)
second = runtime.build_plan(
    scene,
    segments,
    group_surface_sizes={"g1": {"width_px": 64, "height_px": 64}},
)
print(json.dumps({
    "schema": "chaptera.render-compositing-runtime-receipt.v1",
    "first_plan_fingerprint": first["plan_fingerprint"],
    "warm_plan_equivalent": first == second,
    "coherent": first["coherent"],
    "offscreen_passes": first["stats"]["offscreen_passes"],
    "surface_allocations": runtime.surface_pool.metrics["allocations"],
    "surface_reuses": runtime.surface_pool.metrics["reuses"],
    "surface_clears": runtime.surface_pool.metrics["clears"],
    "runtime": runtime.receipt(),
}, indent=2, sort_keys=True))
