#!/usr/bin/env python3
import copy
import unittest
from render_scene_v1 import compile_render_scene
from scene_patch_v1 import diff_render_scenes, apply_patch
from test_render_scene_v1 import SRC

class ScenePatchTests(unittest.TestCase):
    def test_one_node_move_is_bounded_and_equivalent_to_full_compile(self):
        before_src=copy.deepcopy(SRC)
        after_src=copy.deepcopy(SRC)
        after_src["scene_revision"]="sha256:"+"2"*64
        target=next(n for n in after_src["nodes"] if n["node_id"]=="20000000-0000-4000-8000-000000000001")
        target["bounds"]["x"]=-25400
        target["bounds"]["y"]=200000
        base=compile_render_scene(before_src)
        full=compile_render_scene(after_src)
        patch=diff_render_scenes(base,full)
        self.assertEqual(["20000000-0000-4000-8000-000000000001"],[x["node_id"] for x in patch["upsert_nodes"]])
        self.assertEqual([],patch["removed_nodes"])
        self.assertLess(len(str(patch)),len(str(full)))
        applied=apply_patch(base,patch)
        self.assertEqual(full,applied)
        self.assertEqual(-25400,applied["primitives"]["rects"][0]["bounds"]["x"])

    def test_multi_page_interleaved_node_ids_preserve_compiler_order(self):
        before=copy.deepcopy(SRC)
        before["pages"].append({
            "page_id":"10000000-0000-4000-8000-000000000003",
            "order":2,"width_emu":9144000,"height_emu":6858000,
        })
        # Interleave a lower lexical NodeId onto a later page so atom-id sorting
        # would differ from canonical page/paint order.
        before["nodes"].append({
            "node_id":"10000000-0000-4000-8000-000000000099",
            "page_id":"10000000-0000-4000-8000-000000000003",
            "kind":"shape","bounds":{"x":0,"y":0,"width":100000,"height":100000},
            "transform":{"a":"1","b":"0","c":"0","d":"1","tx":0,"ty":0},
            "paint_id":"paint:solid","resource_id":None,"paint_order":0,
        })
        after=copy.deepcopy(before)
        after["scene_revision"]="sha256:"+"9"*64
        after["nodes"][0]["bounds"]["x"]-=127000
        base=compile_render_scene(before)
        full=compile_render_scene(after)
        patch=diff_render_scenes(base,full)
        self.assertEqual(full,apply_patch(base,patch))

    def test_diagnostic_and_order_authority_changes_patch(self):
        after=copy.deepcopy(SRC)
        after["scene_revision"]="sha256:"+"3"*64
        after["order_authority"]="partial"
        after["diagnostics"].append({"code":"render.order_partial","severity":"warning","origin_node_id":None,"detail":"fallback"})
        base=compile_render_scene(copy.deepcopy(SRC))
        full=compile_render_scene(after)
        patch=diff_render_scenes(base,full)
        self.assertIsNotNone(patch["order_deltas"])
        self.assertIsNotNone(patch["diagnostics"])
        self.assertEqual(full,apply_patch(base,patch))

    def test_resource_table_change_patches_without_node_recompile(self):
        after=copy.deepcopy(SRC)
        after["scene_revision"]="sha256:"+"4"*64
        after["resources"][0]["content_hash"]="c"*64
        base=compile_render_scene(copy.deepcopy(SRC))
        full=compile_render_scene(after)
        patch=diff_render_scenes(base,full)
        self.assertEqual([],patch["upsert_nodes"])
        self.assertTrue(patch["resource_deltas"])
        self.assertEqual(full,apply_patch(base,patch))

if __name__=="__main__":
    unittest.main()
