#!/usr/bin/env python3
import copy
import json
import pathlib
import statistics
import time

from render_bench_v1 import shape_workload
from render_scene_v1 import compile_render_scene
from scene_patch_v1 import PRIMITIVE_KINDS, apply_patch, diff_render_scenes

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/"target"/"scene-patch-index-v1"
OUT.mkdir(parents=True,exist_ok=True)

def json_bytes(value):
    return len(json.dumps(value,sort_keys=True,separators=(",",":")).encode("utf-8"))

def primitive_count(scene):
    return sum(len(scene["primitives"][kind]) for kind in PRIMITIVE_KINDS)

def timed_diff(base,target,repeats):
    samples=[]
    patch=None
    metrics=None
    for _ in range(repeats):
        current_metrics={}
        start=time.perf_counter_ns()
        current_patch=diff_render_scenes(base,target,metrics=current_metrics)
        samples.append((time.perf_counter_ns()-start)/1_000_000)
        patch=current_patch
        metrics=current_metrics
    return patch,metrics,{
        "samples_ms":samples,
        "median_ms":statistics.median(samples),
        "min_ms":min(samples),
        "max_ms":max(samples),
    }

def witness(node_count,repeats):
    before=shape_workload(
        node_count,
        pages=max(1,node_count//1000),
        off_page=True,
        label=f"scene-patch-index-{node_count}",
    )
    after=copy.deepcopy(before)
    after["scene_revision"]="sha256:"+("2" if node_count==2000 else "3")*64
    after["nodes"][0]["bounds"]["x"]-=127000

    base=compile_render_scene(before)
    target=compile_render_scene(after)
    patch,metrics,timing=timed_diff(base,target,repeats)
    applied=apply_patch(base,patch)

    exact_expected_visits=primitive_count(base)+primitive_count(target)
    return {
        "node_count":node_count,
        "base_primitive_count":primitive_count(base),
        "target_primitive_count":primitive_count(target),
        "index_metrics":metrics,
        "generation_timing":timing,
        "patch_json_bytes":json_bytes(patch),
        "full_scene_json_bytes":json_bytes(target),
        "upsert_node_count":len(patch["upsert_nodes"]),
        "removed_node_count":len(patch["removed_nodes"]),
        "visit_reduction_ratio_vs_legacy":(
            metrics["legacy_repeated_scan_primitive_visits"]
            / metrics["primitive_visits_total"]
        ),
        "assertions":{
            "single_pass_exact_visit_count":metrics["primitive_visits_total"]==exact_expected_visits,
            "node_comparisons_equal_target_nodes":metrics["node_comparisons"]==len(target["atom_map"]),
            "one_node_upsert":len(patch["upsert_nodes"])==1,
            "patch_smaller_than_full_scene":json_bytes(patch)<json_bytes(target),
            "apply_equals_full_compile":applied==target,
            "signed_off_page_geometry_preserved":applied["primitives"]["rects"][0]["bounds"]["x"]<0,
        },
    }

rows=[
    witness(2000,5),
    witness(10000,3),
]

for row in rows:
    if not all(row["assertions"].values()):
        raise AssertionError(f"RENDER-PATCH-INDEX-01 assertion failed for {row['node_count']}")
    if row["index_metrics"]["index_strategy"]!="single_pass_node_ownership":
        raise AssertionError("unexpected index strategy")

receipt={
    "receipt_version":"chaptera.scene-patch-index.v1",
    "contract":"RENDER-PATCH-INDEX-01",
    "algorithmic_gate":{
        "correctness_basis":"exact primitive visit counts + legacy patch equivalence tests + full-compile equivalence",
        "timing_is_acceptance_gate":False,
        "expected_complexity":"O(base primitives + target primitives + target nodes)",
    },
    "witnesses":rows,
    "assertions":{
        "all_single_pass":all(r["assertions"]["single_pass_exact_visit_count"] for r in rows),
        "all_full_compile_equivalent":all(r["assertions"]["apply_equals_full_compile"] for r in rows),
        "all_one_node_upserts":all(r["assertions"]["one_node_upsert"] for r in rows),
        "ten_k_visit_reduction_over_1000x":rows[1]["visit_reduction_ratio_vs_legacy"]>1000,
    },
}
if not all(receipt["assertions"].values()):
    raise AssertionError("RENDER-PATCH-INDEX-01 contract failed")

(OUT/"receipt.json").write_text(
    json.dumps(receipt,indent=2,sort_keys=True)+"\n",
    encoding="utf-8",
)
print(json.dumps(receipt,indent=2,sort_keys=True))
