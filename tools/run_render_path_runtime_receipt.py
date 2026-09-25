#!/usr/bin/env python3
import json

from render_path_runtime_v1 import PathRuntimeV1
from test_render_path_runtime_v1 import geom

runtime = PathRuntimeV1()
g = geom()
cold = runtime.materialize(g, usage="fill", quality_bucket="scale:1")
warm = runtime.materialize(g, usage="fill", quality_bucket="scale:1")
zoom = runtime.materialize(g, usage="fill", quality_bucket="scale:4")
before = runtime.draw_input(cold["identity"])
runtime.evict_identity(cold["identity"])
rebuilt = runtime.materialize(g, usage="fill", quality_bucket="scale:1")
after = runtime.draw_input(rebuilt["identity"])
print(json.dumps({
    "schema": "chaptera.render-path-runtime-receipt.v1",
    "measurement_class": "synthetic_source_neutral_runtime",
    "real_pub": False,
    "cold_identity": cold["identity"],
    "warm_reused": warm["identity"] == cold["identity"],
    "zoom_bucket_changed_material": zoom["identity"] != cold["identity"],
    "evict_rebuild_draw_input_equivalent": before == after,
    "runtime": runtime.receipt(),
    "limitations": ["No genuine PUB custom-shape fidelity is claimed."],
}, indent=2, sort_keys=True))
