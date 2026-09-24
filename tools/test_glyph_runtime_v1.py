#!/usr/bin/env python3
import copy
import unittest
from glyph_runtime_v1 import *
from render_bench_v1 import text_heavy
from render_scene_v1 import compile_render_scene

FONT="f"*64

class GlyphRuntimeTests(unittest.TestCase):
    def runtime(self):
        r=GlyphRuntimeV1(page_capacity=4); r.set_font_state(FONT,"ready"); return r
    def key(self,**kw):
        v=dict(font_fingerprint=FONT,face_index=0,glyph_id=65,rendering_mode="raster",scale_bucket=1,quality="normal"); v.update(kw); return GlyphKey(**v)
    def test_repeated_placements_reuse_one_materialization(self):
        r=self.runtime(); a=r.request(self.key()); b=r.request(self.key())
        self.assertTrue(r.validate_binding(a["binding"])); self.assertTrue(r.validate_binding(b["binding"]))
        self.assertEqual(r.materializer.calls,1); self.assertEqual(r.metrics["hits"],1)
    def test_font_fingerprint_and_face_never_alias(self):
        r=self.runtime(); r.set_font_state("a"*64,"ready")
        r.request(self.key()); r.request(self.key(font_fingerprint="a"*64)); r.request(self.key(face_index=1))
        self.assertEqual(len(r.entries),3)
    def test_zoom_bucket_changes_only_runtime_material_not_geometry(self):
        scene=compile_render_scene(text_heavy(1)); run=scene["primitives"]["glyph_runs"][0]; font=scene["tables"]["resources"][0]
        r=GlyphRuntimeV1(); r.set_font_state(font["content_hash"],"ready")
        a=r.prepare_run(run,font,zoom=1,dpr=1); b=r.prepare_run(run,font,zoom=4,dpr=1)
        self.assertEqual(a["geometry"],b["geometry"]); self.assertEqual(run["glyphs"],a["geometry"])
        self.assertNotEqual(a["scale_bucket"],b["scale_bucket"]); self.assertFalse(a["canonical_geometry_mutated"])
    def test_eviction_and_rebuild_preserve_key_and_draw_geometry(self):
        r=self.runtime(); k=self.key(); first=r.request(k)["binding"]; r.evict(k)
        self.assertFalse(r.validate_binding(first)); second=r.request(k)["binding"]
        self.assertEqual(first["key"],second["key"]); self.assertTrue(r.validate_binding(second))
    def test_repack_keeps_semantic_key_and_invalidates_old_physical_binding(self):
        r=self.runtime(); keys=[self.key(glyph_id=65+i) for i in range(4)]
        bindings=[r.request(k)["binding"] for k in keys]; r.evict(keys[1]); r.repack()
        self.assertFalse(r.validate_binding(bindings[2])); self.assertEqual(set(r.entries),set(keys))
    def test_pending_to_ready_never_reshapes_or_uses_host_font(self):
        scene=compile_render_scene(text_heavy(1)); run=scene["primitives"]["glyph_runs"][0]; font=scene["tables"]["resources"][0]
        r=GlyphRuntimeV1(); r.set_font_state(font["content_hash"],"pending")
        a=r.prepare_run(run,font); self.assertTrue(all(x["state"]=="pending" for x in a["materials"]))
        r.set_font_state(font["content_hash"],"ready"); b=r.prepare_run(run,font)
        self.assertEqual(a["geometry"],b["geometry"]); self.assertTrue(all(x["state"]=="ready" for x in b["materials"]))
        self.assertFalse(r.receipt()["authority"]["discovers_host_fonts"])
    def test_device_loss_rebuilds_and_stale_binding_fails_closed(self):
        r=self.runtime(); k=self.key(); old=r.request(k)["binding"]; r.reset_device()
        self.assertFalse(r.validate_binding(old)); new=r.request(k)["binding"]; self.assertTrue(r.validate_binding(new)); self.assertEqual(new["device_generation"],2)
    def test_high_zoom_uses_vector_instead_of_magnifying_low_res_raster(self):
        self.assertEqual(select_material_policy(9,1)[0],"vector")
        self.assertEqual(select_material_policy(8,1),("raster",8,"high"))
    def test_unsupported_material_mode_is_explicit(self):
        r=self.runtime(); out=r.request(self.key(rendering_mode="color-complex")); self.assertEqual(out["state"],"unsupported")

if __name__=="__main__": unittest.main()
