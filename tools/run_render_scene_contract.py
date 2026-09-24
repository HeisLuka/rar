#!/usr/bin/env python3
import copy
import json
import pathlib
from test_render_scene_v1 import SRC
from render_scene_v1 import compile_render_scene

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/"target"/"render-scene-v1"
OUT.mkdir(parents=True,exist_ok=True)
scene=compile_render_scene(copy.deepcopy(SRC))
receipt={
    "contract":"RENDER-SCENE-IR-01",
    "render_scene_id":scene["render_scene_id"],
    "order_authority":scene["order_authority"],
    "page_count":len(scene["pages"]),
    "rect_count":len(scene["primitives"]["rects"]),
    "image_count":len(scene["primitives"]["images"]),
    "glyph_run_count":len(scene["primitives"]["glyph_runs"]),
    "node_mapping_count":len(scene["atom_map"]),
    "unsupported_diagnostic_count":sum(1 for d in scene["diagnostics"] if d["code"]=="render.unsupported_node_kind"),
    "assertions":{
        "signed_off_page_emu_preserved":scene["primitives"]["rects"][0]["bounds"]["x"]<0,
        "frame_and_resource_identity_separate":scene["primitives"]["images"][0]["node_id"]!=scene["primitives"]["images"][0]["resource_id"],
        "glyph_provenance_preserved":scene["primitives"]["glyph_runs"][0]["scalar_start"]==0,
        "gpu_offsets_absent": "gpu_buffer_offset" not in json.dumps(scene),
    }
}
(OUT/"render-scene.json").write_text(json.dumps(scene,indent=2,sort_keys=True)+"\n",encoding="utf-8")
(OUT/"receipt.json").write_text(json.dumps(receipt,indent=2,sort_keys=True)+"\n",encoding="utf-8")
print(json.dumps(receipt,indent=2,sort_keys=True))
