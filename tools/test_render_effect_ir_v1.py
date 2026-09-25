#!/usr/bin/env python3
import copy
import unittest

from render_effect_ir_v1 import (
    RenderEffectIrV1Error,
    canonical_effect_tables_v1,
    effect_tables_digest_v1,
)
from render_scene_v1 import compile_render_scene
from scene_patch_v1 import apply_patch, diff_render_scenes
from test_render_scene_v1 import SRC


def shadow(effect_id="effect:shadow"):
    return {
        "effect_id": effect_id,
        "kind": "shadow",
        "region": {"x": -25400, "y": -12700, "width": 900000, "height": 700000},
        "offset_x_emu": 12700,
        "offset_y_emu": 25400,
        "blur_radius_emu": 6350,
        "color": {"r": 10, "g": 20, "b": 30, "a": 120},
        "fidelity": {"state": "exact"},
        "source_format": "synthetic",
    }


def group(effect_id="effect:shadow"):
    return {
        "effect_group_id": "group:fx",
        "effect_ids": [effect_id],
        "opacity_milli": 875,
        "isolation": True,
        "blend_mode": "multiply",
        "composite_mode": "source_over",
    }


def with_effect(source=None):
    src = copy.deepcopy(SRC if source is None else source)
    src["effects"] = [shadow()]
    src["effect_groups"] = [group()]
    src["nodes"][0]["effect_group_id"] = "group:fx"
    return src


class RenderEffectIrV1Tests(unittest.TestCase):
    def test_mask_and_clip_remain_distinct_semantic_tables(self):
        src = copy.deepcopy(SRC)
        src["clips"] = [{"clip_id": "clip:1", "kind": "rect"}]
        src["effects"] = [{
            "effect_id": "effect:mask",
            "kind": "mask",
            "region": {"x": 0, "y": 0, "width": 100, "height": 100},
            "coverage_mode": "alpha",
            "coverage_source_ref": "resource:mask",
            "fidelity": {"state": "exact"},
            "source_format": "synthetic",
        }]
        effects, groups = canonical_effect_tables_v1(src)
        self.assertEqual("mask", effects[0]["kind"])
        self.assertEqual("alpha", effects[0]["coverage_mode"])
        scene = compile_render_scene(src)
        self.assertEqual("clip:1", scene["tables"]["clips"][0]["clip_id"])
        self.assertEqual("effect:mask", scene["tables"]["effects"][0]["effect_id"])
        self.assertEqual([], groups)

    def test_signed_off_page_effect_region_survives_exactly(self):
        scene = compile_render_scene(with_effect())
        region = scene["tables"]["effects"][0]["region"]
        self.assertEqual(-25400, region["x"])
        self.assertEqual(-12700, region["y"])

    def test_attach_remove_and_parameter_update_patch_equivalent_to_full_compile(self):
        base_src = copy.deepcopy(SRC)
        attached_src = with_effect(base_src)
        attached_src["scene_revision"] = "sha256:" + "2" * 64
        base = compile_render_scene(base_src)
        attached = compile_render_scene(attached_src)
        patch = diff_render_scenes(base, attached)
        self.assertEqual(base["paint_seq"], attached["paint_seq"])
        self.assertEqual(
            [row["atoms"] for row in base["atom_map"]],
            [row["atoms"] for row in attached["atom_map"]],
        )
        self.assertEqual(1, len(patch["upsert_nodes"]))
        self.assertTrue(patch["effect_deltas"]["effects"]["upserts"])
        self.assertEqual(attached, apply_patch(base, patch))

        updated_src = copy.deepcopy(attached_src)
        updated_src["scene_revision"] = "sha256:" + "3" * 64
        updated_src["effects"][0]["blur_radius_emu"] = 19050
        updated = compile_render_scene(updated_src)
        parameter_patch = diff_render_scenes(attached, updated)
        self.assertEqual([], parameter_patch["upsert_nodes"])
        self.assertEqual(attached["paint_seq"], updated["paint_seq"])
        self.assertEqual(1, len(parameter_patch["effect_deltas"]["effects"]["upserts"]))
        self.assertEqual(updated, apply_patch(attached, parameter_patch))

        removed_src = copy.deepcopy(updated_src)
        removed_src["scene_revision"] = "sha256:" + "4" * 64
        removed_src["nodes"][0].pop("effect_group_id")
        removed_src["effects"] = []
        removed_src["effect_groups"] = []
        removed = compile_render_scene(removed_src)
        remove_patch = diff_render_scenes(updated, removed)
        self.assertEqual(["effect:shadow"], remove_patch["effect_deltas"]["effects"]["removed"])
        self.assertEqual(["group:fx"], remove_patch["effect_deltas"]["effect_groups"]["removed"])
        self.assertEqual(removed, apply_patch(updated, remove_patch))

    def test_partial_or_unsupported_effect_is_explicit_diagnostic(self):
        src = copy.deepcopy(SRC)
        fx = shadow("effect:unknown")
        fx["fidelity"] = {"state": "unsupported", "reason": "upstream_parameters_unknown"}
        src["effects"] = [fx]
        scene = compile_render_scene(src)
        self.assertTrue(any(
            d["code"] == "render.effect_unsupported" and "effect:unknown" in d["detail"]
            for d in scene["diagnostics"]
        ))

    def test_backend_or_officeart_fields_are_rejected(self):
        for key in ("officeart_property_id", "backend_surface_id", "shader_handle"):
            fx = shadow()
            fx[key] = 7
            with self.subTest(key=key):
                with self.assertRaises(RenderEffectIrV1Error):
                    canonical_effect_tables_v1({"effects": [fx]})

    def test_synthetic_shadow_and_filter_chain_hash_deterministically(self):
        filter_fx = {
            "effect_id": "effect:filter",
            "kind": "filter_chain",
            "region": {"x": 0, "y": 0, "width": 1000, "height": 2000},
            "filters": [
                {"kind": "opacity", "amount_milli": 750},
                {"kind": "color_matrix", "values_milli": [1000,0,0,0,0, 0,1000,0,0,0, 0,0,1000,0,0, 0,0,0,1000,0]},
            ],
            "fidelity": {"state": "exact"},
            "source_format": "synthetic",
        }
        a = canonical_effect_tables_v1({"effects": [filter_fx, shadow()]})
        b = canonical_effect_tables_v1({"effects": [shadow(), filter_fx]})
        self.assertEqual(a, b)
        self.assertEqual(effect_tables_digest_v1(*a), effect_tables_digest_v1(*b))

    def test_publisher_exact_requires_upstream_producer_receipt(self):
        fx = shadow()
        fx["source_format"] = "publisher"
        with self.assertRaisesRegex(RenderEffectIrV1Error, "producer receipt"):
            canonical_effect_tables_v1({"effects": [fx]})
        fx["producer_receipt"] = "receipt:resolved-shadow:v1"
        effects, _ = canonical_effect_tables_v1({"effects": [fx]})
        self.assertEqual("exact", effects[0]["fidelity"]["state"])

    def test_atom_effect_reference_is_source_neutral_and_stable(self):
        scene = compile_render_scene(with_effect())
        atom = scene["primitives"]["rects"][0]
        self.assertEqual("group:fx", atom["effect_group_id"])
        self.assertNotIn("backend", str(atom).lower())
        self.assertNotIn("officeart", str(scene).lower())


if __name__ == "__main__":
    unittest.main()
