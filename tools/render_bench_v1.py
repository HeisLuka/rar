#!/usr/bin/env python3
import copy
import json
import os
import pathlib
import platform
import statistics
import sys
import time
import tracemalloc

from render_scene_v1 import compile_render_scene
from scene_patch_v1 import diff_render_scenes, apply_patch

IDENTITY = {"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0}

def uid(prefix, n):
    return f"{prefix}-0000-4000-8000-{n:012x}"

def base_source(label="bench"):
    return {
        "scene_revision": "sha256:" + "1"*64,
        "order_authority": "exact",
        "pages": [],
        "nodes": [],
        "paints": [{"paint_id":"paint:solid","fill":{"r":64,"g":96,"b":128,"a":255},"stroke":None}],
        "resources": [],
        "clips": [],
        "glyph_runs": [],
        "diagnostics": [],
        "label": label,
    }

def add_pages(src, count):
    for i in range(count):
        src["pages"].append({
            "page_id": uid("10000000", i+1),
            "order": i,
            "width_emu": 9144000,
            "height_emu": 6858000,
        })

def shape_workload(count, pages=1, *, overlap=False, off_page=False, label="shapes"):
    src=base_source(label)
    add_pages(src,pages)
    for i in range(count):
        page=i % pages
        if overlap:
            x,y=250000,250000
        else:
            x=(i % 50)*120000
            y=((i // 50) % 40)*120000
        if off_page and i % 10 == 0:
            x=-12700*(1+(i%7))
        src["nodes"].append({
            "node_id":uid("20000000",i+1),
            "page_id":uid("10000000",page+1),
            "kind":"shape",
            "bounds":{"x":x,"y":y,"width":100000,"height":80000},
            "transform":copy.deepcopy(IDENTITY),
            "paint_id":"paint:solid",
            "resource_id":None,
            "paint_order":i,
        })
    return src

def image_heavy(count=300):
    src=base_source("image-heavy")
    add_pages(src,3)
    for r in range(12):
        src["resources"].append({
            "resource_id":uid("30000000",r+1),
            "kind":"image",
            "content_hash":f"{r+1:064x}"[-64:],
        })
    for i in range(count):
        src["nodes"].append({
            "node_id":uid("21000000",i+1),
            "page_id":uid("10000000",(i%3)+1),
            "kind":"picture_frame",
            "bounds":{"x":(i%10)*700000,"y":((i//10)%8)*700000,"width":600000,"height":500000},
            "transform":copy.deepcopy(IDENTITY),
            "paint_id":None,
            "resource_id":uid("30000000",(i%12)+1),
            "paint_order":i,
        })
    return src

def text_heavy(count=250):
    src=base_source("text-heavy-overset")
    add_pages(src,5)
    font=uid("40000000",1)
    src["resources"].append({"resource_id":font,"kind":"font","content_hash":"f"*64})
    for i in range(count):
        node=uid("22000000",i+1)
        page=uid("10000000",(i%5)+1)
        src["nodes"].append({
            "node_id":node,"page_id":page,"kind":"text_frame",
            "bounds":{"x":100000+(i%5)*900000,"y":100000+((i//5)%8)*700000,"width":800000,"height":600000},
            "transform":copy.deepcopy(IDENTITY),"paint_id":None,"resource_id":None,"paint_order":i,
        })
        src["glyph_runs"].append({
            "page_id":page,"story_id":uid("50000000",i+1),"frame_node_id":node,
            "scalar_start":0,"scalar_end":120,"font_resource_id":font,"paint_id":"paint:solid",
            "glyphs":[{"glyph_id":65+(g%26),"x_emu":100000+g*40000,"y_emu":120000,"advance_emu":40000} for g in range(24)],
        })
        if i % 17 == 0:
            src["diagnostics"].append({
                "code":"layout.story_overset","severity":"warning","origin_node_id":node,"detail":"synthetic-product-grounded",
            })
    return src

def timed(fn, repeats=3):
    values=[]
    result=None
    for _ in range(repeats):
        t0=time.perf_counter_ns()
        result=fn()
        values.append((time.perf_counter_ns()-t0)/1_000_000)
    return result,{
        "samples_ms":values,
        "median_ms":statistics.median(values),
        "min_ms":min(values),
        "max_ms":max(values),
    }

def intersects(bounds, viewport):
    return not (
        bounds["x"]+bounds["width"] < viewport["x"] or
        bounds["y"]+bounds["height"] < viewport["y"] or
        bounds["x"] > viewport["x"]+viewport["width"] or
        bounds["y"] > viewport["y"]+viewport["height"]
    )

def frame_prep(scene, page_id, viewport):
    visible=[]
    for kind in ("rects","images","glyph_runs"):
        for atom in scene["primitives"][kind]:
            if atom.get("page_id") != page_id:
                continue
            bounds=atom.get("bounds")
            if bounds is None or intersects(bounds,viewport):
                visible.append(atom["atom_id"])
    return visible

def overlay_updates(node_id, samples=1200):
    state=None
    for i in range(samples):
        state={"node_id":node_id,"preview_bounds":{"x":i*100,"y":i*50,"width":100000,"height":80000}}
    return state

def compiled_size(scene):
    return len(json.dumps(scene,sort_keys=True,separators=(",",":")).encode("utf-8"))

def benchmark_compile(name, src, repeats=3):
    tracemalloc.start()
    scene,timing=timed(lambda: compile_render_scene(copy.deepcopy(src)),repeats=repeats)
    _,peak=tracemalloc.get_traced_memory()
    tracemalloc.stop()
    return scene,{
        "name":name,
        "input_pages":len(src["pages"]),
        "input_nodes":len(src["nodes"]),
        "glyph_runs":len(src["glyph_runs"]),
        "compile":timing,
        "compiled_json_bytes":compiled_size(scene),
        "peak_tracemalloc_bytes":peak,
        "transform_table_entries":len(scene["tables"]["transforms"]),
        "resource_table_entries":len(scene["tables"]["resources"]),
    }

def run():
    workloads=[
        ("image-heavy",image_heavy()),
        ("text-heavy-overset",text_heavy()),
        ("overlap",shape_workload(1000,overlap=True,label="overlap")),
        ("off-page",shape_workload(1000,off_page=True,label="off-page")),
        ("multi-page",shape_workload(4000,pages=40,label="multi-page")),
    ]
    measured=[]
    compiled={}
    for name,src in workloads:
        scene,row=benchmark_compile(name,src)
        compiled[name]=scene
        measured.append(row)

    stress=[]
    for count in (10_000,50_000,100_000):
        src=shape_workload(count,pages=max(1,count//5000),label=f"stress-{count}")
        scene,row=benchmark_compile(f"stress-{count}",src,repeats=1)
        stress.append(row)
        del scene,src

    # Incremental witness on a bounded but nontrivial 2k-node scene.
    before_src=shape_workload(2000,pages=4,off_page=True,label="patch-witness")
    after_src=copy.deepcopy(before_src)
    after_src["scene_revision"]="sha256:"+"2"*64
    after_src["nodes"][0]["bounds"]["x"]-=127000
    base=compile_render_scene(before_src)
    target=compile_render_scene(after_src)
    patch,patch_gen=timed(lambda: diff_render_scenes(base,target),repeats=3)
    applied,patch_apply=timed(lambda: apply_patch(base,patch),repeats=10)
    patch_metrics={
        "node_count":2000,
        "upsert_nodes":len(patch["upsert_nodes"]),
        "removed_nodes":len(patch["removed_nodes"]),
        "generation":patch_gen,
        "apply":patch_apply,
        "patch_json_bytes":len(json.dumps(patch,sort_keys=True,separators=(",",":")).encode("utf-8")),
        "full_scene_json_bytes":compiled_size(target),
        "apply_equals_full_compile":applied==target,
    }

    multi=compiled["multi-page"]
    page_id=multi["pages"][0]["page_id"]
    viewport={"x":0,"y":0,"width":1800000,"height":1400000}
    _,cull=timed(lambda:frame_prep(multi,page_id,viewport),repeats=50)

    first_page_src=shape_workload(100,pages=1,label="first-visible-page")
    _,first_page=benchmark_compile("first-visible-page",first_page_src,repeats=10)

    _,overlay=timed(lambda:overlay_updates(uid("20000000",1),1200),repeats=20)
    overlay["per_update_median_us"]=(overlay["median_ms"]/1200)*1000

    # Output-sheet instancing: 16 placements reference one compiled page, no cloned authoring nodes.
    source_page=compiled["image-heavy"]["pages"][0]["page_id"]
    output_sheet_instances=[
        {"instance":i,"source_page_id":source_page,"tx_emu":(i%4)*9500000,"ty_emu":(i//4)*7000000}
        for i in range(16)
    ]

    return {
        "receipt_version":"chaptera.render-bench.v1",
        "measurement_class":"public_hosted_mixed_synthetic_product_grounded",
        "real_pub_scene_present":False,
        "closure_blocker":"missing_real_pub_derived_scene_receipt",
        "runtime":{
            "python":sys.version.split()[0],
            "platform":platform.platform(),
            "machine":platform.machine(),
            "runner_os":os.environ.get("RUNNER_OS"),
            "runner_arch":os.environ.get("RUNNER_ARCH"),
            "github_sha":os.environ.get("GITHUB_SHA"),
            "github_run_id":os.environ.get("GITHUB_RUN_ID"),
        },
        "product_grounded_workloads":measured,
        "synthetic_stress":stress,
        "incremental_patch":patch_metrics,
        "frame_prep":{
            "workload":"multi-page",
            "page_id":page_id,
            "viewport":viewport,
            "timing":cull,
        },
        "first_visible_page":first_page,
        "preview_overlay_120hz_proxy":{
            "updates_per_sample":1200,
            "timing":overlay,
            "durable_scene_recompile_count":0,
            "durable_patch_count":0,
        },
        "output_sheet_instancing":{
            "instance_count":16,
            "unique_source_pages":1,
            "cloned_authoring_nodes":0,
            "instances":output_sheet_instances,
        },
        "limitations":[
            "No genuine PUB-derived Scene receipt is currently present in public Rar; this receipt cannot close the real-scene benchmark arm.",
            "CPU Python timings are CI-host characteristics, not GPU/backend performance.",
            "GPU draw time, upload bandwidth and GPU resident memory remain backend-phase metrics.",
        ],
    }

if __name__=="__main__":
    print(json.dumps(run(),indent=2,sort_keys=True))
