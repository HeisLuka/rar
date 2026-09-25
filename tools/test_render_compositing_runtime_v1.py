#!/usr/bin/env python3
import copy
import unittest

from render_compositing_runtime_v1 import (
    CompositingRuntimeV1,
    OffscreenSurfacePoolV1,
)
from render_effect_ir_v1 import RenderEffectIrV1Error, canonical_effect_tables_v1
from render_scene_v1 import compile_render_scene
from render_segment_plan_v1 import plan_scene
from scene_patch_v1 import diff_render_scenes


def base_source():
    return {
        "scene_revision": "sha256:" + "1" * 64,
        "order_authority": "exact",
        "pages": [{"page_id": "p1", "order": 0, "width_emu": 1000, "height_emu": 1000}],
        "nodes": [{
            "node_id": "n1",
            "page_id": "p1",
            "kind": "shape",
            "bounds": {"x": 0, "y": 0, "width": 100, "height": 100},
            "transform": {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0},
            "paint_id": "paint:1",
            "paint_order": 0,
        }],
        "paints": [{"paint_id": "paint:1", "fill": {"r": 1, "g": 2, "b": 3, "a": 255}, "stroke": None}],
        "resources": [],
        "clips": [],
        "glyph_runs": [],
        "diagnostics": [],
        "effects": [],
        "effect_groups": [],
        "path_geometries": [],
    }


def compile_and_plan(src):
    scene = compile_render_scene(src)
    return scene, plan_scene(scene)


class CompositingRuntimeTests(unittest.TestCase):
    def test_opaque_page_allocates_zero_effect_offscreens(self):
        scene, segments = compile_and_plan(base_source())
        runtime = CompositingRuntimeV1()
        plan = runtime.build_plan(scene, segments)
        self.assertTrue(plan["coherent"])
        self.assertEqual(0, plan["stats"]["offscreen_passes"])
        self.assertEqual(0, runtime.surface_pool.receipt()["metrics"]["allocations"])
        self.assertTrue(any(r["op"] == "DrawSegment" for r in plan["records"]))

    def test_one_opacity_group_allocates_only_bounded_surface(self):
        src = base_source()
        src["effect_groups"] = [{
            "effect_group_id": "g1",
            "effect_ids": [],
            "opacity_milli": 500,
            "isolation": True,
            "blend_mode": "normal",
            "composite_mode": "source_over",
        }]
        src["nodes"][0]["effect_group_id"] = "g1"
        scene, segments = compile_and_plan(src)
        runtime = CompositingRuntimeV1()
        plan = runtime.build_plan(
            scene,
            segments,
            group_surface_sizes={"g1": {"width_px": 64, "height_px": 32}},
        )
        self.assertTrue(plan["coherent"])
        self.assertEqual(1, plan["stats"]["offscreen_passes"])
        begin = next(r for r in plan["records"] if r["op"] == "BeginOffscreenGroup")
        self.assertEqual(64, begin["surface_class"]["width_px"])
        self.assertEqual(32, begin["surface_class"]["height_px"])

    def test_nested_group_order_is_outer_begin_inner_begin_then_reverse_composite(self):
        src = base_source()
        src["effect_groups"] = [
            {
                "effect_group_id": "outer",
                "parent_effect_group_id": None,
                "effect_ids": [],
                "opacity_milli": 800,
                "isolation": True,
                "blend_mode": "normal",
                "composite_mode": "source_over",
            },
            {
                "effect_group_id": "inner",
                "parent_effect_group_id": "outer",
                "effect_ids": [],
                "opacity_milli": 700,
                "isolation": True,
                "blend_mode": "normal",
                "composite_mode": "source_over",
            },
        ]
        src["nodes"][0]["effect_group_id"] = "inner"
        scene, segments = compile_and_plan(src)
        runtime = CompositingRuntimeV1()
        plan = runtime.build_plan(
            scene,
            segments,
            group_surface_sizes={
                "outer": {"width_px": 80, "height_px": 80},
                "inner": {"width_px": 40, "height_px": 40},
            },
        )
        ops = [(r["op"], r.get("effect_group_id")) for r in plan["records"]]
        self.assertLess(ops.index(("BeginOffscreenGroup", "outer")), ops.index(("BeginOffscreenGroup", "inner")))
        self.assertLess(ops.index(("CompositeOffscreenGroup", "inner")), ops.index(("CompositeOffscreenGroup", "outer")))
        self.assertEqual(2, plan["stats"]["nested_group_depth"])

    def test_effect_group_parent_cycle_fails_closed(self):
        groups = [
            {"effect_group_id": "a", "parent_effect_group_id": "b", "effect_ids": []},
            {"effect_group_id": "b", "parent_effect_group_id": "a", "effect_ids": []},
        ]
        with self.assertRaisesRegex(RenderEffectIrV1Error, "cycle"):
            canonical_effect_tables_v1({"effect_groups": groups})

    def test_rect_clip_uses_scissor_without_offscreen(self):
        src = base_source()
        src["clips"] = [{"clip_id": "c1", "kind": "rect", "rect": {"x": 1, "y": 2, "width": 50, "height": 60}}]
        src["nodes"][0]["clip_id"] = "c1"
        scene, segments = compile_and_plan(src)
        self.assertEqual("c1", scene["primitives"]["rects"][0]["clip_id"])
        self.assertEqual("c1", segments["pages"][0]["segments"][0]["barrier_key"][0])
        runtime = CompositingRuntimeV1()
        plan = runtime.build_plan(scene, segments)
        self.assertTrue(any(r["op"] == "PushScissor" for r in plan["records"]))
        self.assertEqual(0, plan["stats"]["offscreen_passes"])
        self.assertEqual(1, plan["stats"]["scissor_clips"])

    def test_complex_clip_mask_cache_is_exact_and_reused(self):
        src = base_source()
        src["clips"] = [{
            "clip_id": "cpx",
            "kind": "path",
            "path_digest": "sha256:" + "a" * 64,
            "clip_rule": "evenodd",
            "mask_bytes_hint": 2048,
        }]
        src["nodes"][0]["clip_id"] = "cpx"
        scene, segments = compile_and_plan(src)
        runtime = CompositingRuntimeV1()
        a = runtime.build_plan(scene, segments)
        b = runtime.build_plan(scene, segments)
        self.assertTrue(a["coherent"] and b["coherent"])
        self.assertEqual(1, runtime.clip_cache.metrics["misses"])
        self.assertEqual(1, runtime.clip_cache.metrics["hits"])
        self.assertEqual(1, a["stats"]["mask_clips"])

    def test_clip_and_group_patch_invalidation_is_bounded(self):
        before = base_source()
        before["clips"] = [{"clip_id": "cpx", "kind": "path", "path_digest": "sha256:" + "a" * 64}]
        before["effect_groups"] = [{
            "effect_group_id": "g1",
            "effect_ids": [],
            "opacity_milli": 500,
            "isolation": True,
            "blend_mode": "normal",
            "composite_mode": "source_over",
        }]
        before["nodes"][0]["clip_id"] = "cpx"
        before["nodes"][0]["effect_group_id"] = "g1"
        after = copy.deepcopy(before)
        after["scene_revision"] = "sha256:" + "2" * 64
        after["clips"][0]["path_digest"] = "sha256:" + "b" * 64
        after["effect_groups"][0]["opacity_milli"] = 600

        base_scene, base_segments = compile_and_plan(before)
        target_scene, _ = compile_and_plan(after)
        runtime = CompositingRuntimeV1()
        runtime.build_plan(base_scene, base_segments, group_surface_sizes={"g1": {"width_px": 32, "height_px": 32}})
        patch = diff_render_scenes(base_scene, target_scene)
        invalidated = runtime.invalidate_from_patch(patch, target_scene=target_scene)
        self.assertEqual(["cpx"], invalidated["clip_ids"])
        self.assertEqual(["g1"], invalidated["effect_group_ids"])
        self.assertEqual(1, runtime.metrics["clip_invalidations"])
        self.assertEqual(1, runtime.metrics["group_invalidations"])

    def test_surface_pool_reuse_clears_between_owners_and_plan_has_no_surface_handle(self):
        pool = OffscreenSurfacePoolV1()
        descriptor = {
            "target": {"color_space": "srgb"},
            "width_px": 32,
            "height_px": 32,
            "sample_count": 1,
            "quality_bucket": "default",
        }
        a = pool.acquire(descriptor, owner="group:a")
        pool.release(a)
        b = pool.acquire(descriptor, owner="group:b")
        self.assertEqual(a["identity"], b["identity"])
        self.assertNotEqual(a["lease_generation"], b["lease_generation"])
        self.assertEqual(1, pool.metrics["allocations"])
        self.assertEqual(1, pool.metrics["reuses"])
        self.assertEqual(2, pool.metrics["clears"])

        src = base_source()
        scene, segments = compile_and_plan(src)
        plan = CompositingRuntimeV1().build_plan(scene, segments)
        self.assertNotIn("surface:", repr(plan))

    def test_device_loss_rejects_old_surface_lease(self):
        pool = OffscreenSurfacePoolV1()
        descriptor = {
            "target": {"color_space": "srgb"},
            "width_px": 16,
            "height_px": 16,
            "sample_count": 1,
            "quality_bucket": "default",
        }
        binding = pool.acquire(descriptor, owner="g")
        self.assertTrue(pool.validate_binding(binding))
        pool.reset_device()
        self.assertFalse(pool.validate_binding(binding))

    def test_destroy_rebuild_yields_equivalent_normalized_plan(self):
        src = base_source()
        src["effect_groups"] = [{
            "effect_group_id": "g1",
            "effect_ids": [],
            "opacity_milli": 900,
            "isolation": True,
            "blend_mode": "normal",
            "composite_mode": "source_over",
        }]
        src["nodes"][0]["effect_group_id"] = "g1"
        scene, segments = compile_and_plan(src)
        kwargs = {"group_surface_sizes": {"g1": {"width_px": 40, "height_px": 20}}}
        a = CompositingRuntimeV1().build_plan(scene, segments, **kwargs)
        b = CompositingRuntimeV1().build_plan(scene, segments, **kwargs)
        self.assertEqual(a, b)

    def test_advanced_effect_is_explicit_unsupported_not_flattened(self):
        src = base_source()
        src["effects"] = [{
            "effect_id": "fx:blur",
            "kind": "blur",
            "region": {"x": 0, "y": 0, "width": 100, "height": 100},
            "radius_emu": 10,
            "fidelity": {"state": "exact"},
            "source_format": "synthetic",
        }]
        src["effect_groups"] = [{
            "effect_group_id": "g1",
            "effect_ids": ["fx:blur"],
            "opacity_milli": 1000,
            "isolation": False,
            "blend_mode": "normal",
            "composite_mode": "source_over",
        }]
        src["nodes"][0]["effect_group_id"] = "g1"
        scene, segments = compile_and_plan(src)
        plan = CompositingRuntimeV1().build_plan(scene, segments)
        self.assertFalse(plan["coherent"])
        self.assertTrue(any(r["op"] == "UnsupportedEffectGroup" for r in plan["records"]))
        self.assertFalse(any(r["op"] == "DrawSegment" for r in plan["records"]))

    def test_color_contract_is_explicit_on_group_composite(self):
        src = base_source()
        src["effect_groups"] = [{
            "effect_group_id": "g1",
            "effect_ids": [],
            "opacity_milli": 500,
            "isolation": True,
            "blend_mode": "normal",
            "composite_mode": "source_over",
        }]
        src["nodes"][0]["effect_group_id"] = "g1"
        scene, segments = compile_and_plan(src)
        plan = CompositingRuntimeV1().build_plan(
            scene,
            segments,
            group_surface_sizes={"g1": {"width_px": 20, "height_px": 20}},
        )
        composite = next(r for r in plan["records"] if r["op"] == "CompositeOffscreenGroup")
        self.assertEqual("source-over-linear-srgb", composite["color_contract"])
        self.assertEqual("srgb", plan["target"]["color_space"])


if __name__ == "__main__":
    unittest.main()
