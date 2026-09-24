#!/usr/bin/env python3
import copy
import json
import pathlib
import statistics
import time

from render_bench_v1 import shape_workload
from render_scene_v1 import compile_render_scene
from scene_patch_v1 import (
    PRIMITIVE_KINDS,
    apply_patch,
    apply_patch_in_place,
    diff_render_scenes,
)

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/"target"/"scene-patch-apply-v1"
OUT.mkdir(parents=True,exist_ok=True)

def timed_pure(base,patch,repeats):
    samples=[]
    result=None
    for _ in range(repeats):
        start=time.perf_counter_ns()
        result=apply_patch(base,patch)
        samples.append((time.perf_counter_ns()-start)/1_000_000)
    return result,{
        "samples_ms":samples,
        "median_ms":statistics.median(samples),
        "min_ms":min(samples),
        "max_ms":max(samples),
    }

def timed_in_place(base,patch,repeats):
    samples=[]
    result=None
    for _ in range(repeats):
        # Renderer ownership is established before the hot apply operation.
        working=copy.deepcopy(base)
        start=time.perf_counter_ns()
        result=apply_patch_in_place(working,patch)
        samples.append((time.perf_counter_ns()-start)/1_000_000)
    return result,{
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
        label=f"scene-patch-apply-{node_count}",
    )
    after=copy.deepcopy(before)
    after["scene_revision"]="sha256:"+("4" if node_count==2000 else "5")*64
    after["nodes"][0]["bounds"]["x"]-=127000

    base=compile_render_scene(before)
    target=compile_render_scene(after)
    patch=diff_render_scenes(base,target)

    base_snapshot=copy.deepcopy(base)
    pure,pure_timing=timed_pure(base,patch,repeats)
    if base!=base_snapshot:
        raise AssertionError("pure apply mutated base")

    hot,hot_timing=timed_in_place(base,patch,repeats)

    pure_metrics={}
    apply_patch(base,patch,metrics=pure_metrics)
    hot_metrics={}
    metric_working=copy.deepcopy(base)
    apply_patch_in_place(metric_working,patch,metrics=hot_metrics)

    base_primitive_count=sum(len(base["primitives"][kind]) for kind in PRIMITIVE_KINDS)
    target_primitive_count=sum(len(target["primitives"][kind]) for kind in PRIMITIVE_KINDS)

    assertions={
        "pure_equals_full_compile":pure==target,
        "in_place_equals_full_compile":hot==target,
        "pure_does_not_mutate_base":base==base_snapshot,
        "pure_reports_full_deepcopy":pure_metrics["full_scene_deepcopy"] is True,
        "in_place_reports_no_full_deepcopy":hot_metrics["full_scene_deepcopy"] is False,
        "in_place_filter_is_single_pass":hot_metrics["primitive_filter_visits"]==base_primitive_count,
        "in_place_reorder_covers_target_atoms":hot_metrics["reorder_atoms"]==target_primitive_count,
        "one_node_affected":hot_metrics["affected_node_count"]==1,
        "one_node_upsert":len(patch["upsert_nodes"])==1,
    }
    if not all(assertions.values()):
        raise AssertionError(f"apply witness failed for {node_count}: {assertions}")

    return {
        "node_count":node_count,
        "primitive_count":base_primitive_count,
        "patch_upsert_nodes":len(patch["upsert_nodes"]),
        "pure_apply":pure_timing,
        "in_place_apply":hot_timing,
        "observed_median_speedup_ratio":(
            pure_timing["median_ms"]/hot_timing["median_ms"]
            if hot_timing["median_ms"]>0 else None
        ),
        "pure_metrics":pure_metrics,
        "in_place_metrics":hot_metrics,
        "assertions":assertions,
    }

rows=[
    witness(2000,7),
    witness(10000,5),
]

receipt={
    "receipt_version":"chaptera.scene-patch-apply.v1",
    "contract":"RENDER-PATCH-APPLY-01",
    "measurement_rule":{
        "pure_apply_timing_includes_full_deepcopy":True,
        "in_place_timing_excludes_preexisting_renderer_ownership_setup":True,
        "timing_is_acceptance_gate":False,
        "correctness_gate":"pure == in-place == clean full compile; wrong-base preflight and poisoned-state tests live in unit contract",
    },
    "witnesses":rows,
    "assertions":{
        "all_equivalent":all(
            row["assertions"]["pure_equals_full_compile"]
            and row["assertions"]["in_place_equals_full_compile"]
            for row in rows
        ),
        "all_hot_paths_skip_full_deepcopy":all(
            row["assertions"]["in_place_reports_no_full_deepcopy"] for row in rows
        ),
        "all_filter_single_pass":all(
            row["assertions"]["in_place_filter_is_single_pass"] for row in rows
        ),
    },
}
if not all(receipt["assertions"].values()):
    raise AssertionError("RENDER-PATCH-APPLY-01 contract failed")

(OUT/"receipt.json").write_text(
    json.dumps(receipt,indent=2,sort_keys=True)+"\n",
    encoding="utf-8",
)
print(json.dumps(receipt,indent=2,sort_keys=True))
