#!/usr/bin/env python3
import json
import pathlib
from render_bench_v1 import run

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/"target"/"render-bench-v1"
OUT.mkdir(parents=True,exist_ok=True)
receipt=run()
if receipt["incremental_patch"]["apply_equals_full_compile"] is not True:
    raise AssertionError("ScenePatch apply diverged from full compile")
if receipt["preview_overlay_120hz_proxy"]["durable_patch_count"] != 0:
    raise AssertionError("preview overlay emitted durable patch")
if receipt["output_sheet_instancing"]["cloned_authoring_nodes"] != 0:
    raise AssertionError("output-sheet instancing cloned authoring nodes")
if [x["input_nodes"] for x in receipt["synthetic_stress"]] != [10000,50000,100000]:
    raise AssertionError("stress ladder incomplete")
(OUT/"receipt.json").write_text(json.dumps(receipt,indent=2,sort_keys=True)+"\n",encoding="utf-8")
print(json.dumps({
    "receipt_version":receipt["receipt_version"],
    "real_pub_scene_present":receipt["real_pub_scene_present"],
    "closure_blocker":receipt["closure_blocker"],
    "stress_nodes":[x["input_nodes"] for x in receipt["synthetic_stress"]],
    "patch_upserts":receipt["incremental_patch"]["upsert_nodes"],
    "patch_apply_equivalent":receipt["incremental_patch"]["apply_equals_full_compile"],
    "overlay_durable_patches":receipt["preview_overlay_120hz_proxy"]["durable_patch_count"],
    "output_sheet_instances":receipt["output_sheet_instancing"]["instance_count"],
},indent=2,sort_keys=True))
