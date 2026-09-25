#!/usr/bin/env python3
import copy
import json

from render_scene_v1 import compile_render_scene
from render_segment_plan_v1 import plan_scene
from render_submit_runtime_v1 import RenderSubmitRuntimeV1
from test_render_scene_v1 import SRC

src = copy.deepcopy(SRC)
scene = compile_render_scene(src)
metadata = {
    atom["atom_id"]: {
        "clip_id": None,
        "effect_group_id": atom.get("effect_group_id"),
        "isolation": False,
        "blend_mode": "normal",
    }
    for atoms in scene["primitives"].values()
    for atom in atoms
}
segments = plan_scene(scene, metadata_by_atom=metadata)
runtime = RenderSubmitRuntimeV1()
plan = runtime.build_submit_plan(
    segments,
    bindings_by_atom={},
    view_state={"zoom_ppm": 1_000_000, "pan_x_emu": 0, "pan_y_emu": 0},
    target={"format": "rgba8unorm-srgb", "sample_count": 1},
)
warm = runtime.build_submit_plan(
    segments,
    bindings_by_atom={},
    view_state={"zoom_ppm": 1_250_000, "pan_x_emu": 12700, "pan_y_emu": 0},
    target={"format": "rgba8unorm-srgb", "sample_count": 1},
)
print(json.dumps({
    "cold_submit_fingerprint": plan["submit_fingerprint"],
    "warm_submit_fingerprint": warm["submit_fingerprint"],
    "record_count": len(plan["records"]),
    "cache": runtime.cache_receipt(),
    "paint_order_preserved": [
        atom
        for record in plan["records"]
        if record["op"] == "DrawInstances"
        for atom in record["atom_ids"]
    ] == [
        atom
        for page in segments["pages"]
        for atom in page["ordered_atom_ids"]
    ],
}, sort_keys=True, indent=2))
