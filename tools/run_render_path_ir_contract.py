#!/usr/bin/env python3
import json
from test_render_path_ir_v1 import source
from render_scene_v1 import compile_render_scene
scene=compile_render_scene(source())
print(json.dumps({
 "schema":"chaptera.render-path-ir-receipt.v1",
 "measurement_class":"synthetic_source_neutral_path_ir",
 "real_pub":False,
 "representative":False,
 "path_table_entries":len(scene["tables"]["paths"]),
 "path_instances":len(scene["primitives"]["paths"]),
 "shared_geometry":len({p["path_digest"] for p in scene["primitives"]["paths"]})==1,
 "signed_coordinate_preserved":scene["tables"]["paths"][0]["bounds"]["x"]<0,
 "raw_officeart_types_present":any(x in str(scene).lower() for x in ["officeart","fopt","libmspub"]),
 "limitations":["No genuine PUB custom-shape producer is claimed by this receipt."]
},indent=2,sort_keys=True))
