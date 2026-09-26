#!/usr/bin/env python3
import json
import pathlib
from render_bench_v1 import run

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/"target"/"render-bench-v1"
OUT.mkdir(parents=True,exist_ok=True)
receipt=run()
if receipt["real_pub_scene_present"] is not True:
    raise AssertionError("real PUB-derived scene arm missing")
real_pub = receipt["real_pub_scene"]
if real_pub["provenance"]["source_sha256"] != "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf":
    raise AssertionError("real PUB source identity drifted")
if real_pub["provenance"]["page_count"] != 8 or real_pub["provenance"]["node_count"] != 68:
    raise AssertionError("real PUB scene counts drifted")
if real_pub["input_pages"] != 8 or real_pub["input_nodes"] != 68:
    raise AssertionError("real PUB benchmark did not consume the canonical scene")
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
    "real_pub_source_sha256":receipt["real_pub_scene"]["provenance"]["source_sha256"],
    "real_pub_pages":receipt["real_pub_scene"]["input_pages"],
    "real_pub_nodes":receipt["real_pub_scene"]["input_nodes"],
    "closure_blocker":receipt["closure_blocker"],
    "stress_nodes":[x["input_nodes"] for x in receipt["synthetic_stress"]],
    "patch_upserts":receipt["incremental_patch"]["upsert_nodes"],
    "patch_apply_equivalent":receipt["incremental_patch"]["apply_equals_full_compile"],
    "overlay_durable_patches":receipt["preview_overlay_120hz_proxy"]["durable_patch_count"],
    "output_sheet_instances":receipt["output_sheet_instancing"]["instance_count"],
},indent=2,sort_keys=True))
