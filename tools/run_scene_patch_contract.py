#!/usr/bin/env python3
import copy
import json
import pathlib
from render_scene_v1 import compile_render_scene
from scene_patch_v1 import diff_render_scenes, apply_patch
from test_render_scene_v1 import SRC

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/"target"/"scene-patch-v1"
OUT.mkdir(parents=True,exist_ok=True)

before_src=copy.deepcopy(SRC)
after_src=copy.deepcopy(SRC)
after_src["scene_revision"]="sha256:"+"2"*64
node=next(n for n in after_src["nodes"] if n["node_id"]=="20000000-0000-4000-8000-000000000001")
node["bounds"]["x"]=-25400
node["bounds"]["y"]=200000

base=compile_render_scene(before_src)
target=compile_render_scene(after_src)
patch=diff_render_scenes(base,target)
applied=apply_patch(base,patch)
receipt={
    "contract":"RENDER-PATCH-01",
    "base_render_scene_id":base["render_scene_id"],
    "target_render_scene_id":target["render_scene_id"],
    "patch_id":patch["patch_id"],
    "upsert_node_count":len(patch["upsert_nodes"]),
    "removed_node_count":len(patch["removed_nodes"]),
    "patch_chars":len(json.dumps(patch,sort_keys=True)),
    "full_scene_chars":len(json.dumps(target,sort_keys=True)),
    "assertions":{
        "patch_smaller_than_full_scene":len(json.dumps(patch,sort_keys=True))<len(json.dumps(target,sort_keys=True)),
        "one_node_upsert":len(patch["upsert_nodes"])==1,
        "signed_off_page_geometry_preserved":applied["primitives"]["rects"][0]["bounds"]["x"]<0,
        "apply_equals_full_compile":applied==target,
        "transient_preview_requires_patch":False
    }
}
(OUT/"patch.json").write_text(json.dumps(patch,indent=2,sort_keys=True)+"\n",encoding="utf-8")
(OUT/"receipt.json").write_text(json.dumps(receipt,indent=2,sort_keys=True)+"\n",encoding="utf-8")
print(json.dumps(receipt,indent=2,sort_keys=True))
