#!/usr/bin/env python3
import copy
import hashlib
import unittest

from buffer_residency_v1 import BufferResidencyV1
from render_scene_v1 import compile_render_scene
from render_segment_plan_v1 import plan_scene
from render_submit_runtime_v1 import (
    FakeSubmitBackendV1,
    RenderSubmitRuntimeV1,
    StaleSubmitGeneration,
    binding_ref_from_buffer_v1,
    binding_ref_from_texture_v1,
)
from test_render_scene_v1 import SRC
from texture_residency_v1 import MaterialKey, TextureResidencyV1


def source():
    src = copy.deepcopy(SRC)
    src["nodes"] = [n for n in src["nodes"] if n["kind"] in {"shape", "picture_frame"}]
    src["glyph_runs"] = []
    return src


def setup_runtime():
    scene = compile_render_scene(source())
    metadata = {}
    for kind, atoms in scene["primitives"].items():
        for atom in atoms:
            metadata[atom["atom_id"]] = {
                "clip_id": None,
                "effect_group_id": atom.get("effect_group_id"),
                "isolation": False,
                "blend_mode": "normal",
            }
    segments = plan_scene(scene, metadata_by_atom=metadata)

    buffers = BufferResidencyV1(initial_capacity=2048)
    texture = TextureResidencyV1()
    bindings = {}
    for kind, atoms in scene["primitives"].items():
        for atom in atoms:
            if kind == "rects":
                handle = buffers.allocate(atom["atom_id"], b"rect-instance-v1")
                bindings[atom["atom_id"]] = [
                    binding_ref_from_buffer_v1(
                        buffers,
                        handle,
                        logical_identity=atom["atom_id"],
                    )
                ]
            elif kind == "images":
                payload = b"image-material-v1"
                key = MaterialKey(
                    resource_id=atom["resource_id"],
                    content_hash=hashlib.sha256(payload).hexdigest(),
                )
                texture.demand(key, payload)
                bindings[atom["atom_id"]] = [binding_ref_from_texture_v1(texture, key)]
    runtime = RenderSubmitRuntimeV1(FakeSubmitBackendV1("fake-reference"))
    return scene, segments, buffers, texture, bindings, runtime


TARGET = {
    "format": "rgba8unorm-srgb",
    "sample_count": 1,
    "composite_mode": "source_over",
    "depth_stencil_mode": "none",
}
VIEW = {"zoom_ppm": 1_000_000, "pan_x_emu": 0, "pan_y_emu": 0}


class RenderSubmitRuntimeV1Tests(unittest.TestCase):
    def test_identical_descriptor_reuses_pipeline_and_target_or_sample_never_alias(self):
        _, segments, _, _, bindings, runtime = setup_runtime()
        runtime.build_submit_plan(segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET)
        first_builds = runtime.backend.pipeline_builds
        runtime.build_submit_plan(segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET)
        self.assertEqual(first_builds, runtime.backend.pipeline_builds)
        changed = dict(TARGET, sample_count=4)
        runtime.build_submit_plan(segments, bindings_by_atom=bindings, view_state=VIEW, target=changed)
        self.assertGreater(runtime.backend.pipeline_builds, first_builds)

    def test_pan_zoom_changes_frame_constants_without_rebuilding_pipelines_or_commands(self):
        _, segments, _, _, bindings, runtime = setup_runtime()
        a = runtime.build_submit_plan(
            segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET
        )
        builds = (
            runtime.backend.pipeline_builds,
            runtime.backend.binding_builds,
            runtime.backend.command_builds,
        )
        moved_view = {"zoom_ppm": 1_250_000, "pan_x_emu": 12700, "pan_y_emu": -25400}
        b = runtime.build_submit_plan(
            segments, bindings_by_atom=bindings, view_state=moved_view, target=TARGET
        )
        self.assertEqual(builds, (
            runtime.backend.pipeline_builds,
            runtime.backend.binding_builds,
            runtime.backend.command_builds,
        ))
        self.assertNotEqual(a["frame_constants"], b["frame_constants"])

    def test_baked_view_commands_are_view_fenced(self):
        _, segments, _, _, bindings, runtime = setup_runtime()
        runtime.build_submit_plan(
            segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET,
            bake_view_into_commands=True,
        )
        first = runtime.backend.command_builds
        runtime.build_submit_plan(
            segments,
            bindings_by_atom=bindings,
            view_state={"zoom_ppm": 2_000_000, "pan_x_emu": 0, "pan_y_emu": 0},
            target=TARGET,
            bake_view_into_commands=True,
        )
        self.assertGreater(runtime.backend.command_builds, first)

    def test_segment_fingerprint_invalidation_keeps_global_pipelines(self):
        _, segments, _, _, bindings, runtime = setup_runtime()
        runtime.build_submit_plan(segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET)
        pipeline_count = len(runtime.pipeline_cache)
        segment_fp = segments["pages"][0]["segments"][0]["fingerprint"]
        removed = runtime.invalidate_segment(segment_fp)
        self.assertGreaterEqual(removed, 1)
        self.assertEqual(pipeline_count, len(runtime.pipeline_cache))

    def test_texture_generation_change_invalidates_dependent_binding_only(self):
        _, segments, _, texture, bindings, runtime = setup_runtime()
        runtime.build_submit_plan(segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET)
        pipelines = len(runtime.pipeline_cache)
        texture_ref = next(
            ref
            for refs in bindings.values()
            for ref in refs
            if ref.binding_kind == "texture"
        )
        result = runtime.invalidate_binding_identity(texture_ref.identity)
        self.assertGreaterEqual(result["bindings_invalidated"], 1)
        self.assertEqual(pipelines, len(runtime.pipeline_cache))

    def test_one_node_buffer_content_patch_does_not_flush_unrelated_pipeline(self):
        _, segments, buffers, _, bindings, runtime = setup_runtime()
        runtime.build_submit_plan(segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET)
        pipeline_builds = runtime.backend.pipeline_builds
        handle = next(
            buffers._slot_for_handle(h)
            for h in []
        ) if False else None
        buffer_ref = next(
            ref
            for refs in bindings.values()
            for ref in refs
            if ref.binding_kind == "buffer"
        )
        slot_id = buffers._logical_to_slot[buffer_ref.identity]
        slot = buffers._slots[slot_id]
        from buffer_residency_v1 import LogicalHandle
        buffers.update(LogicalHandle(slot.slot_id, slot.generation), b"R", byte_offset=0)
        runtime.build_submit_plan(segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET)
        self.assertEqual(pipeline_builds, runtime.backend.pipeline_builds)

    def test_stale_device_generation_binding_fails_closed(self):
        _, segments, _, _, bindings, runtime = setup_runtime()
        runtime.reset_device()
        with self.assertRaises(StaleSubmitGeneration):
            runtime.build_submit_plan(
                segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET
            )

    def test_device_reset_drops_handles_and_clean_rebuild_normalizes_equivalently(self):
        _, segments, buffers, texture, bindings, runtime = setup_runtime()
        cold = runtime.build_submit_plan(
            segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET
        )
        runtime.reset_device()
        buffers.reset_device()
        texture.reset_device()

        refreshed = {}
        for atom_id, refs in bindings.items():
            new_refs = []
            for ref in refs:
                if ref.binding_kind == "buffer":
                    slot_id = buffers._logical_to_slot[ref.identity]
                    slot = buffers._slots[slot_id]
                    from buffer_residency_v1 import LogicalHandle
                    new_refs.append(binding_ref_from_buffer_v1(
                        buffers,
                        LogicalHandle(slot.slot_id, slot.generation),
                        logical_identity=ref.identity,
                    ))
                else:
                    entry = texture.entry_for_identity(ref.identity)
                    payload = b"image-material-v1"
                    texture.demand(entry.key, payload)
                    new_refs.append(binding_ref_from_texture_v1(texture, entry.key))
            refreshed[atom_id] = new_refs
        rebuilt = runtime.build_submit_plan(
            segments, bindings_by_atom=refreshed, view_state=VIEW, target=TARGET
        )
        def structural(plan):
            value = copy.deepcopy(plan)
            for record in value["records"]:
                if "binding_key" in record:
                    record["binding_key"] = "<generation-local-binding>"
                if "command_key" in record:
                    record["command_key"] = "<generation-local-command>"
                if "pipeline_key" in record:
                    record["pipeline_key"] = record["pipeline_key"]
            value.pop("submit_fingerprint", None)
            return value
        self.assertEqual(structural(cold), structural(rebuilt))

    def test_command_cache_never_reorders_across_segment_barriers(self):
        _, segments, _, _, bindings, runtime = setup_runtime()
        plan = runtime.build_submit_plan(
            segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET
        )
        emitted = []
        for record in plan["records"]:
            if record["op"] == "DrawInstances":
                emitted.extend(record["atom_ids"])
        expected = []
        for page in segments["pages"]:
            expected.extend(page["ordered_atom_ids"])
        self.assertEqual(expected, emitted)

    def test_no_backend_handle_leaks_into_segment_or_normalized_submit_payload(self):
        _, segments, _, _, bindings, runtime = setup_runtime()
        plan = runtime.build_submit_plan(
            segments, bindings_by_atom=bindings, view_state=VIEW, target=TARGET
        )
        text = repr({"segments": segments, "submit": plan}).lower()
        self.assertNotIn("pipeline:g", text)
        self.assertNotIn("binding:g", text)
        self.assertNotIn("command:g", text)
        self.assertNotIn("memory_address", text)


if __name__ == "__main__":
    unittest.main()
