#!/usr/bin/env python3
import copy
import unittest

from render_color_contract_v1 import *
from render_scene_v1 import compile_render_scene
from test_render_scene_v1 import SRC


class RenderColorContractTests(unittest.TestCase):
    def test_literal_srgb_survives_render_scene_compile_unchanged(self):
        src = copy.deepcopy(SRC)
        src["paints"][0]["fill"] = {"r": 12, "g": 34, "b": 56, "a": 78}
        out = compile_render_scene(src)
        paint = next(p for p in out["tables"]["paints"] if p["paint_id"] == "paint:solid")
        self.assertEqual(paint["fill"], {"r": 12, "g": 34, "b": 56, "a": 78})

    def test_alpha_semantics_are_explicit(self):
        self.assertEqual(DEFAULT_CONTRACT.ir_alpha_mode, "straight")
        self.assertEqual(DEFAULT_CONTRACT.runtime_storage_alpha_mode, "premultiplied")
        self.assertEqual(DEFAULT_CONTRACT.target_alpha_mode, "premultiplied")

    def test_premultiplied_runtime_roundtrip(self):
        rgba = (0.25, 0.5, 0.75, 0.4)
        got = unpremultiply_rgba(premultiply_rgba(rgba))
        for a, b in zip(rgba, got):
            self.assertAlmostEqual(a, b, places=12)

    def test_transparent_edge_rgb_is_zeroed_in_runtime_storage(self):
        self.assertEqual(premultiply_rgba((1.0, 0.0, 1.0, 0.0)), (0.0, 0.0, 0.0, 0.0))
        self.assertEqual(unpremultiply_rgba((0.0, 0.0, 0.0, 0.0)), (0.0, 0.0, 0.0, 0.0))

    def test_reference_oracle_detects_wrong_blend_space(self):
        src = (1.0, 0.0, 0.0, 0.5)
        dst = (0.0, 1.0, 0.0, 1.0)
        linear = composite_over(src, dst)
        encoded = composite_over_encoded_srgb(src, dst)
        self.assertNotEqual(tuple(round(v, 8) for v in linear), tuple(round(v, 8) for v in encoded))
        self.assertEqual(DEFAULT_CONTRACT.compositing_law, "source-over-linear-srgb")

    def test_material_key_cannot_alias_color_dispositions(self):
        exact = material_color_key(logical_material_id="image:1", source_disposition=color_disposition("explicit_srgb"))
        assumed = material_color_key(logical_material_id="image:1", source_disposition=color_disposition("unknown_profile", assume_srgb_allowed=True))
        self.assertNotEqual(exact, assumed)

    def test_unknown_profile_is_not_exact(self):
        self.assertEqual(color_disposition("unknown_profile")["state"], "unknown")
        self.assertEqual(color_disposition("embedded_icc_unsupported")["state"], "unknown")
        self.assertEqual(color_disposition("unknown_profile", assume_srgb_allowed=True)["state"], "assumed")

    def test_backend_rebuild_preserves_target_contract(self):
        self.assertEqual(rebuild_target(), rebuild_target())
        req = backend_requirement_set()
        self.assertIn("color_space:srgb", req["mandatory_correctness"])
        self.assertIn("dynamic_range:hdr", req["optional_quality"])

    def test_production_color_semantics_do_not_leak_into_v1(self):
        text = str(DEFAULT_CONTRACT.receipt()).lower()
        for forbidden in ("cmyk", "devicen", "spot", "outputintent", "overprint"):
            self.assertNotIn(forbidden, text)


if __name__ == "__main__":
    unittest.main()
