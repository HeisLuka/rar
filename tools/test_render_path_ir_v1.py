#!/usr/bin/env python3
import copy
import unittest
from render_scene_v1 import compile_render_scene
from scene_patch_v1 import diff_render_scenes, apply_patch
from render_path_ir_v1 import normalize_path_geometry, PathGeometryError

def source():
    return {
      "scene_revision":"sha256:"+"1"*64,"order_authority":"exact",
      "pages":[{"page_id":"p1","order":0,"width_emu":1000,"height_emu":1000}],
      "nodes":[
        {"node_id":"n1","page_id":"p1","kind":"path","bounds":{"x":-10,"y":0,"width":30,"height":20},"transform":{"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0},"paint_id":"paint:1","paint_order":0,"path_id":"g1"},
        {"node_id":"n2","page_id":"p1","kind":"path","bounds":{"x":-10,"y":0,"width":30,"height":20},"transform":{"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0},"paint_id":"paint:1","paint_order":1,"path_id":"g2"},
      ],
      "paints":[{"paint_id":"paint:1","fill":{"r":1,"g":2,"b":3,"a":255},"stroke":None}],
      "resources":[],"clips":[],"glyph_runs":[],"diagnostics":[],
      "path_geometries":[
        {"path_id":"g1","commands":[{"op":"MoveTo","x":-10,"y":0},{"op":"QuadTo","cx":0,"cy":20,"x":10,"y":0},{"op":"CubicTo","c1x":12,"c1y":4,"c2x":18,"c2y":9,"x":20,"y":10},{"op":"Close"}],"fill_rule":"nonzero"},
        {"path_id":"g2","commands":[{"op":"MoveTo","x":-10,"y":0},{"op":"QuadTo","cx":0,"cy":20,"x":10,"y":0},{"op":"CubicTo","c1x":12,"c1y":4,"c2x":18,"c2y":9,"x":20,"y":10},{"op":"Close"}],"fill_rule":"nonzero"},
      ],
    }

class PathIRTests(unittest.TestCase):
    def test_identical_geometry_is_shared_without_node_aliasing(self):
        out=compile_render_scene(source())
        self.assertEqual(len(out["tables"]["paths"]),1)
        a,b=out["primitives"]["paths"]
        self.assertEqual(a["path_digest"],b["path_digest"])
        self.assertEqual(a["path_index"],b["path_index"])
        self.assertNotEqual(a["node_id"],b["node_id"])
        self.assertEqual(a["bounds"]["x"],-10)

    def test_digest_is_stable_across_table_repacking(self):
        a=source()
        b=source()
        b["path_geometries"].reverse()
        ca,cb=compile_render_scene(a),compile_render_scene(b)
        self.assertEqual(ca["tables"]["paths"],cb["tables"]["paths"])
        self.assertEqual([x["path_digest"] for x in ca["primitives"]["paths"]],[x["path_digest"] for x in cb["primitives"]["paths"]])

    def test_patch_full_compile_equivalence_for_path_edit(self):
        before=source(); after=copy.deepcopy(before)
        after["scene_revision"]="sha256:"+"2"*64
        after["path_geometries"][0]["commands"][1]["cy"]=25
        after["path_geometries"][1]["commands"][1]["cy"]=25
        base=compile_render_scene(before); target=compile_render_scene(after)
        patch=diff_render_scenes(base,target)
        self.assertEqual(apply_patch(base,patch),target)
        self.assertIsNotNone(patch["path_deltas"])

    def test_add_remove_and_compaction_keep_atom_identity(self):
        s=source(); base=compile_render_scene(s)
        atom_ids=[x["atom_id"] for x in base["primitives"]["paths"]]
        s["scene_revision"]="sha256:"+"2"*64
        s["nodes"]=s["nodes"][:1]
        s["path_geometries"]=s["path_geometries"][:1]
        target=compile_render_scene(s)
        patched=apply_patch(base,diff_render_scenes(base,target))
        self.assertEqual(patched,target)
        self.assertEqual(target["primitives"]["paths"][0]["atom_id"],atom_ids[0])

    def test_malformed_and_overflow_fail_closed(self):
        with self.assertRaises(PathGeometryError):
            normalize_path_geometry({"commands":[{"op":"LineTo","x":0,"y":0}]})
        with self.assertRaises(PathGeometryError):
            normalize_path_geometry({"commands":[{"op":"MoveTo","x":2**60,"y":0}]})
        with self.assertRaises(PathGeometryError):
            normalize_path_geometry({"commands":[{"op":"MoveTo","x":0,"y":0},{"op":"Close"},{"op":"Close"}]})

if __name__=="__main__":
    unittest.main()
