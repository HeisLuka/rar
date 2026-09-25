#!/usr/bin/env python3
import copy
import unittest

from render_bench_v1 import shape_workload
from render_scene_v1 import compile_render_scene, hash_id
from scene_patch_v1 import (
    PRIMITIVE_KINDS,
    ScenePatchApplyPoisoned,
    apply_patch,
    apply_patch_in_place,
    diff_render_scenes,
)
from test_render_scene_v1 import SRC

def _legacy_atoms_for_node(scene,node_id):
    out={kind:[] for kind in PRIMITIVE_KINDS}
    for kind in PRIMITIVE_KINDS:
        for atom in scene["primitives"][kind]:
            if atom.get("node_id")==node_id:
                out[kind].append(copy.deepcopy(atom))
    return out

def _legacy_diff_render_scenes(base,target):
    base_nodes={x["node_id"]:x for x in base["atom_map"]}
    target_nodes={x["node_id"]:x for x in target["atom_map"]}
    removed=sorted(set(base_nodes)-set(target_nodes))
    upserts=[]
    for node_id in sorted(target_nodes):
        base_atoms=_legacy_atoms_for_node(base,node_id) if node_id in base_nodes else None
        target_atoms=_legacy_atoms_for_node(target,node_id)
        if base_atoms!=target_atoms:
            upserts.append({"node_id":node_id,"primitives":target_atoms})
    patch={
        "patch_version":"chaptera.scene-patch.v1",
        "base_revision":base["scene_revision"],
        "target_revision":target["scene_revision"],
        "base_render_scene_id":base["render_scene_id"],
        "target_render_scene_id":target["render_scene_id"],
        "removed_nodes":removed,
        "upsert_nodes":upserts,
        "page_deltas": [] if base["pages"]==target["pages"] else copy.deepcopy(target["pages"]),
        "resource_deltas": [] if base["tables"]["resources"]==target["tables"]["resources"] else copy.deepcopy(target["tables"]["resources"]),
        "effect_deltas": {
            "effects": None,
            "effect_groups": None,
        },
        "order_deltas": None if (base["order_authority"],base["paint_seq"])==(target["order_authority"],target["paint_seq"]) else {
            "order_authority":target["order_authority"],
            "paint_seq":copy.deepcopy(target["paint_seq"]),
        },
        "diagnostics": None if base["diagnostics"]==target["diagnostics"] else copy.deepcopy(target["diagnostics"]),
    }
    patch["patch_id"]=hash_id(patch)
    return patch

def _assert_pure_and_in_place(testcase,base,target,patch):
    base_before=copy.deepcopy(base)
    pure_metrics={}
    pure=apply_patch(base,patch,metrics=pure_metrics)
    testcase.assertEqual(base_before,base)
    testcase.assertEqual(target,pure)
    testcase.assertTrue(pure_metrics["full_scene_deepcopy"])

    working=copy.deepcopy(base)
    hot_metrics={}
    hot=apply_patch_in_place(working,patch,metrics=hot_metrics)
    testcase.assertIs(working,hot)
    testcase.assertEqual(target,hot)
    testcase.assertFalse(hot_metrics["full_scene_deepcopy"])
    testcase.assertEqual(
        sum(len(base["primitives"][kind]) for kind in PRIMITIVE_KINDS),
        hot_metrics["primitive_filter_visits"],
    )

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
        _assert_pure_and_in_place(self,base,full,patch)
        self.assertEqual(-25400,full["primitives"]["rects"][0]["bounds"]["x"])

    def test_indexed_diff_matches_legacy_patch_semantics(self):
        before=shape_workload(64,pages=4,off_page=True,label="legacy-equivalence")
        after=copy.deepcopy(before)
        after["scene_revision"]="sha256:"+"8"*64
        after["nodes"][0]["bounds"]["x"]-=127000
        base=compile_render_scene(before)
        target=compile_render_scene(after)
        self.assertEqual(
            _legacy_diff_render_scenes(base,target),
            diff_render_scenes(base,target),
        )

    def test_index_visits_each_primitive_once_per_scene(self):
        before=shape_workload(137,pages=3,off_page=True,label="index-visits")
        after=copy.deepcopy(before)
        after["scene_revision"]="sha256:"+"7"*64
        after["nodes"][0]["bounds"]["x"]-=127000
        base=compile_render_scene(before)
        target=compile_render_scene(after)
        metrics={}
        patch=diff_render_scenes(base,target,metrics=metrics)
        base_primitives=sum(len(base["primitives"][kind]) for kind in PRIMITIVE_KINDS)
        target_primitives=sum(len(target["primitives"][kind]) for kind in PRIMITIVE_KINDS)
        self.assertEqual(base_primitives,metrics["base_primitive_visits"])
        self.assertEqual(target_primitives,metrics["target_primitive_visits"])
        self.assertEqual(base_primitives+target_primitives,metrics["primitive_visits_total"])
        self.assertEqual(len(target["atom_map"]),metrics["node_comparisons"])
        self.assertEqual(1,metrics["changed_node_count"])
        self.assertGreater(
            metrics["legacy_repeated_scan_primitive_visits"],
            metrics["primitive_visits_total"]*50,
        )
        self.assertEqual(target,apply_patch(base,patch))

    def test_wrong_base_rejects_before_in_place_mutation(self):
        before=shape_workload(32,pages=2,label="wrong-base")
        after=copy.deepcopy(before)
        after["scene_revision"]="sha256:"+"6"*64
        after["nodes"][0]["bounds"]["x"]+=127000
        base=compile_render_scene(before)
        target=compile_render_scene(after)
        patch=diff_render_scenes(base,target)
        patch["base_render_scene_id"]="sha256:"+"f"*64
        working=copy.deepcopy(base)
        original=copy.deepcopy(working)
        with self.assertRaisesRegex(ValueError,"base render scene mismatch"):
            apply_patch_in_place(working,patch)
        self.assertEqual(original,working)

    def test_target_identity_failure_poisoning_is_explicit(self):
        before=shape_workload(32,pages=2,label="poison")
        after=copy.deepcopy(before)
        after["scene_revision"]="sha256:"+"5"*64
        after["nodes"][0]["bounds"]["x"]+=127000
        base=compile_render_scene(before)
        target=compile_render_scene(after)
        patch=diff_render_scenes(base,target)
        patch["target_render_scene_id"]="sha256:"+"e"*64
        working=copy.deepcopy(base)
        with self.assertRaises(ScenePatchApplyPoisoned):
            apply_patch_in_place(working,patch)
        self.assertNotEqual(base,working)
        self.assertNotEqual(patch["target_render_scene_id"],working["render_scene_id"])

    def test_multi_page_interleaved_node_ids_preserve_compiler_order(self):
        before=copy.deepcopy(SRC)
        before["pages"].append({
            "page_id":"10000000-0000-4000-8000-000000000003",
            "order":2,"width_emu":9144000,"height_emu":6858000,
        })
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
        _assert_pure_and_in_place(self,base,full,patch)

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
        _assert_pure_and_in_place(self,base,full,patch)

    def test_resource_table_change_patches_without_node_recompile(self):
        after=copy.deepcopy(SRC)
        after["scene_revision"]="sha256:"+"4"*64
        after["resources"][0]["content_hash"]="c"*64
        base=compile_render_scene(copy.deepcopy(SRC))
        full=compile_render_scene(after)
        patch=diff_render_scenes(base,full)
        self.assertEqual([],patch["upsert_nodes"])
        self.assertTrue(patch["resource_deltas"])
        _assert_pure_and_in_place(self,base,full,patch)

if __name__=="__main__":
    unittest.main()
