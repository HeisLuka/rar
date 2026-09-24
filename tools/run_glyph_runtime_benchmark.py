#!/usr/bin/env python3
import json,time
from glyph_runtime_v1 import GlyphRuntimeV1
from render_bench_v1 import text_heavy
from render_scene_v1 import compile_render_scene

scene=compile_render_scene(text_heavy(250))
font=next(r for r in scene["tables"]["resources"] if r["kind"]=="font")
runs=scene["primitives"]["glyph_runs"]
runtime=GlyphRuntimeV1(page_capacity=128); runtime.set_font_state(font["content_hash"],"ready")
placements=sum(len(r["glyphs"]) for r in runs)
t0=time.perf_counter_ns()
for run in runs: runtime.prepare_run(run,font,zoom=1,dpr=1)
cold_ms=(time.perf_counter_ns()-t0)/1e6
cold=dict(runtime.receipt())
t1=time.perf_counter_ns()
for run in runs: runtime.prepare_run(run,font,zoom=1,dpr=1)
warm_ms=(time.perf_counter_ns()-t1)/1e6
warm=dict(runtime.receipt())
t2=time.perf_counter_ns()
for run in runs: runtime.prepare_run(run,font,zoom=4,dpr=2)
transition_ms=(time.perf_counter_ns()-t2)/1e6
high=dict(runtime.receipt())
print(json.dumps({
 "schema":"chaptera.glyph-runtime-benchmark.v1","measurement_class":"synthetic_source_neutral_text_heavy",
 "real_pub":False,"representative":False,"glyph_placements":placements,
 "cold_ms":cold_ms,"warm_ms":warm_ms,"zoom_dpr_transition_ms":transition_ms,
 "cold":cold,"warm":warm,"after_transition":high,
 "limitations":["Fake glyph materializer/atlas; not raster quality or GPU timing.","No host font discovery and no reshaping are performed."]
},indent=2,sort_keys=True))
