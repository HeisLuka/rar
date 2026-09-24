#!/usr/bin/env python3
import copy
import unittest
from render_scene_v1 import compile_render_scene

SRC = {
    "scene_revision": "sha256:" + "1"*64,
    "order_authority": "exact",
    "pages": [
        {"page_id":"10000000-0000-4000-8000-000000000002","order":1,"width_emu":9144000,"height_emu":6858000},
        {"page_id":"10000000-0000-4000-8000-000000000001","order":0,"width_emu":9144000,"height_emu":6858000}
    ],
    "nodes": [
        {
            "node_id":"20000000-0000-4000-8000-000000000001","page_id":"10000000-0000-4000-8000-000000000001",
            "kind":"shape","bounds":{"x":-12700,"y":100000,"width":500000,"height":300000},
            "transform":{"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0},"paint_id":"paint:solid","resource_id":None,"paint_order":0
        },
        {
            "node_id":"20000000-0000-4000-8000-000000000002","page_id":"10000000-0000-4000-8000-000000000001",
            "kind":"picture_frame","bounds":{"x":600000,"y":100000,"width":900000,"height":700000},
            "transform":{"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0},"paint_id":None,
            "resource_id":"30000000-0000-4000-8000-000000000001","paint_order":1
        },
        {
            "node_id":"20000000-0000-4000-8000-000000000003","page_id":"10000000-0000-4000-8000-000000000001",
            "kind":"text_frame","bounds":{"x":100000,"y":900000,"width":1500000,"height":600000},
            "transform":{"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0},"paint_id":None,"resource_id":None,"paint_order":2
        },
        {
            "node_id":"20000000-0000-4000-8000-000000000004","page_id":"10000000-0000-4000-8000-000000000002",
            "kind":"table","bounds":{"x":0,"y":0,"width":500000,"height":500000},
            "transform":{"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0},"paint_id":None,"resource_id":None,"paint_order":0
        }
    ],
    "paints":[{"paint_id":"paint:solid","fill":{"r":1,"g":2,"b":3,"a":255},"stroke":None}],
    "resources":[
        {"resource_id":"30000000-0000-4000-8000-000000000001","kind":"image","content_hash":"a"*64},
        {"resource_id":"40000000-0000-4000-8000-000000000001","kind":"font","content_hash":"b"*64}
    ],
    "clips":[],
    "glyph_runs":[{
        "page_id":"10000000-0000-4000-8000-000000000001",
        "story_id":"50000000-0000-4000-8000-000000000001",
        "frame_node_id":"20000000-0000-4000-8000-000000000003",
        "scalar_start":0,"scalar_end":5,
        "font_resource_id":"40000000-0000-4000-8000-000000000001",
        "paint_id":"paint:solid",
        "glyphs":[
            {"glyph_id":10,"x_emu":110000,"y_emu":910000,"advance_emu":70000},
            {"glyph_id":11,"x_emu":180000,"y_emu":910000,"advance_emu":70000}
        ]
    }],
    "diagnostics":[{"code":"layout.partial_crop","severity":"warning","origin_node_id":"20000000-0000-4000-8000-000000000002","detail":"crop-unresolved"}]
}

class RenderSceneTests(unittest.TestCase):
    def test_compiler_preserves_core_invariants(self):
        out = compile_render_scene(copy.deepcopy(SRC))
        self.assertEqual("chaptera.render-scene.v1", out["render_scene_version"])
        self.assertEqual("exact", out["order_authority"])
        self.assertEqual("10000000-0000-4000-8000-000000000001", out["pages"][0]["page_id"])
        self.assertEqual(-12700, out["primitives"]["rects"][0]["bounds"]["x"])
        image = out["primitives"]["images"][0]
        self.assertNotEqual(image["node_id"], image["resource_id"])
        glyph = out["primitives"]["glyph_runs"][0]
        self.assertEqual("50000000-0000-4000-8000-000000000001", glyph["story_id"])
        self.assertEqual((0,5),(glyph["scalar_start"],glyph["scalar_end"]))
        self.assertEqual("20000000-0000-4000-8000-000000000003",glyph["frame_node_id"])
        self.assertTrue(any(d["code"]=="render.unsupported_node_kind" for d in out["diagnostics"]))
        self.assertTrue(any(x["node_id"]=="20000000-0000-4000-8000-000000000003" and x["atoms"] for x in out["atom_map"]))
        self.assertNotIn("gpu_buffer_offset", str(out))

    def test_output_is_deterministic(self):
        a = compile_render_scene(copy.deepcopy(SRC))
        b = compile_render_scene(copy.deepcopy(SRC))
        self.assertEqual(a,b)

if __name__ == "__main__":
    unittest.main()
