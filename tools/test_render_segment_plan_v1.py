#!/usr/bin/env python3
import copy
import unittest

from render_bench_v1 import shape_workload
from render_scene_v1 import compile_render_scene
from render_segment_plan_v1 import *


def scene(count=6):
    return compile_render_scene(shape_workload(count, pages=1, overlap=True, label="segment-test"))


class SegmentPlanTests(unittest.TestCase):
    def test_canonical_paint_order_preserved(self):
        s = scene()
        p = plan_scene(s)
        self.assertEqual(p["pages"][0]["ordered_atom_ids"], s["paint_seq"])

    def test_culling_is_order_preserving_filter(self):
        s = scene()
        keep = {s["paint_seq"][1], s["paint_seq"][4], s["paint_seq"][5]}
        pid = s["pages"][0]["page_id"]
        p = plan_scene(s, visible_by_page={pid: keep})
        self.assertEqual(p["pages"][0]["ordered_atom_ids"], [x for x in s["paint_seq"] if x in keep])

    def test_contiguous_compatible_atoms_batch_but_separated_materials_never_gather(self):
        s = scene(5)
        aids = s["paint_seq"]
        meta = {
            aids[0]: {"blend_mode": "source-over"},
            aids[1]: {"blend_mode": "source-over"},
            aids[2]: {"blend_mode": "multiply"},
            aids[3]: {"blend_mode": "source-over"},
            aids[4]: {"blend_mode": "source-over"},
        }
        p = plan_scene(s, metadata_by_atom=meta)["pages"][0]
        flattened = [batch["atom_ids"] for seg in p["segments"] for batch in seg["batches"]]
        self.assertEqual(flattened[0], aids[:2])
        self.assertEqual(flattened[1], [aids[2]])
        self.assertEqual(flattened[2], aids[3:])
        self.assertNotIn(aids[3], flattened[0])

    def test_clip_effect_isolation_changes_split_segments(self):
        s = scene(4)
        a = s["paint_seq"]
        meta = {
            a[0]: {"clip_id": "clip:1"},
            a[1]: {"clip_id": "clip:1"},
            a[2]: {"clip_id": "clip:2", "effect_group_id": "fx:1"},
            a[3]: {"clip_id": "clip:2", "effect_group_id": "fx:1", "isolation": True},
        }
        p = plan_scene(s, metadata_by_atom=meta)["pages"][0]
        self.assertEqual([len(seg["atom_ids"]) for seg in p["segments"]], [2, 1, 1])

    def test_incremental_plan_equals_clean_full_plan_and_reuses_unaffected_page(self):
        src = shape_workload(8, pages=2, label="segment-patch")
        before = compile_render_scene(src)
        previous = plan_scene(before)
        after_src = copy.deepcopy(src)
        after_src["scene_revision"] = "sha256:" + "2" * 64
        after_src["nodes"][0]["bounds"]["x"] += 100
        after = compile_render_scene(after_src)
        changed = after["atom_map"][0]["atoms"]
        inc = incremental_replan(previous, after, changed_atom_ids=changed)
        self.assertEqual(inc["plan"], plan_scene(after))
        self.assertTrue(inc["full_plan_equivalent"])
        self.assertGreaterEqual(inc["reused_pages"], 1)

    def test_unknown_barrier_widens_to_page_rebuild(self):
        s = scene(3)
        previous = plan_scene(s)
        aid = s["paint_seq"][1]
        inc = incremental_replan(previous, s, changed_atom_ids=[aid], metadata_by_atom={aid: {"barrier_unknown": True}}, unknown_dependency=True)
        self.assertEqual(inc["rebuild_scope"], "affected_pages")
        self.assertGreaterEqual(inc["rebuilt_segments"], 1)

    def test_page_instances_reuse_source_plan_without_atom_cloning(self):
        p = plan_scene(scene())["pages"][0]
        placements = [
            {"instance_id": f"i:{i}", "transform": {"tx": i * 1000, "ty": 0, "scale": 1}}
            for i in range(16)
        ]
        inst = instantiate_page_plan(p, placements)
        self.assertEqual(inst["cloned_atom_count"], 0)
        self.assertEqual(len(inst["instances"]), 16)
        self.assertEqual(len({x["source_plan_fingerprint"] for x in inst["instances"]}), 1)

    def test_fingerprint_is_deterministic(self):
        s = scene()
        self.assertEqual(plan_scene(s), plan_scene(s))


if __name__ == "__main__":
    unittest.main()
