#!/usr/bin/env python3
import copy
import unittest

from render_path_ir_v1 import normalize_path_geometry
from render_path_runtime_v1 import (
    PathRuntimeV1,
    invalidated_path_digests_for_patch,
    materialize_clip_stack_v1,
    path_material_key_v1,
)
from render_scene_v1 import compile_render_scene
from scene_patch_v1 import diff_render_scenes
from test_render_path_ir_v1 import source as path_source


def geom(delta=0):
    return normalize_path_geometry({
        "commands": [
            {"op": "MoveTo", "x": 0, "y": 0},
            {"op": "LineTo", "x": 100 + delta, "y": 0},
            {"op": "LineTo", "x": 100 + delta, "y": 100},
            {"op": "Close"},
        ],
        "fill_rule": "nonzero",
        "clip_rule": "evenodd",
    })


class PathRuntimeV1Tests(unittest.TestCase):
    def test_identical_geometry_reuses_material_independent_of_snapshot_index(self):
        runtime = PathRuntimeV1()
        g = geom()
        a = runtime.materialize(g, usage="fill")
        b = runtime.materialize(copy.deepcopy(g), usage="fill")
        self.assertEqual(a["identity"], b["identity"])
        self.assertEqual(1, runtime.metrics["cache_misses"])
        self.assertEqual(1, runtime.metrics["cache_hits"])

    def test_changed_geometry_cannot_alias_even_if_snapshot_index_would_be_same(self):
        a = geom(0)
        b = geom(1)
        ka, ia = path_material_key_v1(a, usage="fill")
        kb, ib = path_material_key_v1(b, usage="fill")
        self.assertNotEqual(a["path_digest"], b["path_digest"])
        self.assertNotEqual(ia, ib)
        self.assertNotEqual(ka, kb)

    def test_fill_color_is_not_tessellation_identity_but_stroke_geometry_is(self):
        g = geom()
        _, fill_a = path_material_key_v1(g, usage="fill")
        _, fill_b = path_material_key_v1(copy.deepcopy(g), usage="fill")
        self.assertEqual(fill_a, fill_b)

        _, stroke_a = path_material_key_v1(
            g,
            usage="stroke",
            stroke_geometry={"width_emu": 10, "join": "miter", "cap": "butt"},
        )
        _, stroke_b = path_material_key_v1(
            g,
            usage="stroke",
            stroke_geometry={"width_emu": 20, "join": "miter", "cap": "butt"},
        )
        self.assertNotEqual(stroke_a, stroke_b)

    def test_quality_bucket_changes_disposable_material_not_canonical_commands(self):
        runtime = PathRuntimeV1()
        g = geom()
        before = copy.deepcopy(g)
        a = runtime.materialize(g, usage="fill", quality_bucket="scale:1")
        b = runtime.materialize(g, usage="fill", quality_bucket="scale:4")
        self.assertNotEqual(a["identity"], b["identity"])
        self.assertEqual(before, g)

    def test_scene_patch_invalidates_only_changed_path_digest(self):
        before_src = path_source()
        after_src = copy.deepcopy(before_src)
        after_src["scene_revision"] = "sha256:" + "2" * 64
        after_src["path_geometries"][0]["commands"][1]["cy"] = 30
        base = compile_render_scene(before_src)
        target = compile_render_scene(after_src)
        patch = diff_render_scenes(base, target)
        invalidated = invalidated_path_digests_for_patch(base, target, patch)
        base_digests = {a["path_digest"] for a in base["primitives"]["paths"]}
        self.assertEqual(1, len(invalidated))
        self.assertTrue(set(invalidated).issubset(base_digests))

    def test_eviction_and_rebuild_yield_equivalent_draw_input(self):
        runtime = PathRuntimeV1()
        g = geom()
        binding = runtime.materialize(g, usage="fill")
        before = runtime.draw_input(binding["identity"])
        runtime.evict_identity(binding["identity"])
        rebuilt = runtime.materialize(g, usage="fill")
        after = runtime.draw_input(rebuilt["identity"])
        self.assertEqual(before, after)
        self.assertFalse(runtime.validate_binding(binding))
        self.assertTrue(runtime.validate_binding(rebuilt))

    def test_device_loss_rejects_old_binding_and_rebuilds_same_input(self):
        runtime = PathRuntimeV1()
        g = geom()
        binding = runtime.materialize(g, usage="clip")
        before = runtime.draw_input(binding["identity"])
        runtime.reset_device()
        self.assertFalse(runtime.validate_binding(binding))
        rebuilt = runtime.materialize(g, usage="clip")
        self.assertEqual(before, runtime.draw_input(rebuilt["identity"]))

    def test_clip_stack_preserves_order(self):
        runtime = PathRuntimeV1()
        a, b = geom(0), geom(5)
        rows = materialize_clip_stack_v1(runtime, [a, b])
        self.assertEqual([0, 1], [row["ordinal"] for row in rows])
        self.assertEqual([a["path_digest"], b["path_digest"]], [row["path_digest"] for row in rows])

    def test_invalid_or_source_specific_geometry_fails_closed(self):
        runtime = PathRuntimeV1()
        bad = geom()
        bad["path_digest"] = "sha256:" + "0" * 64
        with self.assertRaisesRegex(ValueError, "path_digest"):
            runtime.materialize(bad, usage="fill")

        raw = geom()
        raw["raw_pub_bytes"] = "00ff"
        with self.assertRaisesRegex(ValueError, "source-specific"):
            runtime.materialize(raw, usage="fill")

    def test_runtime_receipt_declares_non_semantic_backend_state(self):
        runtime = PathRuntimeV1()
        runtime.materialize(geom(), usage="fill")
        receipt = runtime.receipt()
        self.assertFalse(receipt["snapshot_local_path_index_semantic"])
        self.assertFalse(receipt["backend_allocator_handles_semantic"])
        self.assertTrue(receipt["synthetic_runtime_only"])


if __name__ == "__main__":
    unittest.main()
