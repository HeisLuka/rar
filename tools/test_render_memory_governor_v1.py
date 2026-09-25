#!/usr/bin/env python3
import hashlib
import unittest

from buffer_residency_v1 import BufferResidencyV1
from render_memory_governor_v1 import (
    BufferResidencyMemoryAdapterV1,
    RecordingInvalidationPoolV1,
    RenderMemoryGovernorV1,
    TextureResidencyMemoryAdapterV1,
)
from texture_residency_v1 import MaterialKey, TextureResidencyV1


def key(payload: bytes, name="r", derivative="source"):
    return MaterialKey(
        resource_id=name,
        content_hash=hashlib.sha256(payload).hexdigest(),
        derivative=derivative,
    )


def governor():
    return RenderMemoryGovernorV1(
        soft_target_bytes=700,
        high_target_bytes=900,
        rearm_margin_bytes=100,
        cooldown_frames=3,
    )


class RenderMemoryGovernorV1Tests(unittest.TestCase):
    def test_visible_pin_survives_soft_eviction_and_expiry_reclaims(self):
        texture = TextureResidencyV1()
        payload_a = b"a" * 400
        payload_b = b"b" * 400
        a, b = key(payload_a, "a"), key(payload_b, "b")
        texture.demand(a, payload_a)
        texture.release_placement(a)
        texture.demand(b, payload_b)
        texture.release_placement(b)

        g = governor()
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        for identity in texture.memory_entries():
            g.note_entry("texture", identity["identity"], last_used_frame=0)
        a_id = texture.identity_for_key(a)
        g.pin("texture", a_id, reason="visible-frame", expires_after_frame=1)

        receipt = g.reclaim_to(target_resident_bytes=400, frame=1, reason="soft")
        self.assertTrue(receipt["success"])
        self.assertTrue(texture.entry_for_identity(a_id).state == "Resident")

        receipt2 = g.reclaim_to(target_resident_bytes=0, frame=5, reason="high")
        self.assertTrue(receipt2["success"])
        self.assertEqual(0, texture.receipt()["resident_bytes"])

    def test_buffer_off_window_reclaimed_without_losing_logical_data(self):
        buffers = BufferResidencyV1(initial_capacity=1024, alignment=16)
        h1 = buffers.allocate("page:a:rects", b"A" * 256, page_id="page:a")
        h2 = buffers.allocate("page:b:rects", b"B" * 256, page_id="page:b")
        logical_before = buffers.normalized_logical_state()

        g = governor()
        g.register_pool("buffer", "buffer", BufferResidencyMemoryAdapterV1(buffers))
        g.note_entry("buffer", "page:a:rects", last_used_frame=10, active_window=True)
        g.note_entry("buffer", "page:b:rects", last_used_frame=0)
        receipt = g.reclaim_to(target_resident_bytes=256, frame=10, reason="soft")

        self.assertTrue(receipt["success"])
        self.assertTrue(buffers.validate_binding(buffers.binding(h1)))
        with self.assertRaises(ValueError):
            buffers.binding(h2)
        self.assertEqual(
            [(x["logical_id"], x["data_hex"]) for x in logical_before],
            [(x["logical_id"], x["data_hex"]) for x in buffers.normalized_logical_state()],
        )

    def test_texture_eviction_propagates_bounded_binding_invalidation(self):
        texture = TextureResidencyV1()
        payload = b"x" * 512
        k = key(payload, "hero")
        texture.demand(k, payload)
        texture.release_placement(k)
        identity = texture.identity_for_key(k)

        bindings = RecordingInvalidationPoolV1()
        bindings.add("binding:hero", 64)
        g = governor()
        g.register_pool("submit", "submit_command", bindings)
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        g.note_entry("texture", identity, last_used_frame=0)
        g.add_dependency("texture", identity, "submit", "binding:hero")

        receipt = g.reclaim_to(target_resident_bytes=64, frame=10, reason="high")
        self.assertTrue(receipt["success"])
        self.assertEqual(["binding:hero"], bindings.invalidated)
        self.assertEqual(1, g.metrics["dependency_invalidations"])

    def test_pins_make_inability_explicit_not_success(self):
        texture = TextureResidencyV1()
        payload = b"x" * 500
        k = key(payload)
        texture.demand(k, payload)
        texture.release_placement(k)
        identity = texture.identity_for_key(k)

        g = governor()
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        g.note_entry("texture", identity, last_used_frame=0)
        g.pin("texture", identity, reason="in-flight-upload", expires_after_frame=100)
        receipt = g.reclaim_to(target_resident_bytes=0, frame=1, reason="critical")
        self.assertFalse(receipt["success"])
        self.assertEqual(0, receipt["bytes_reclaimed"])
        self.assertEqual(1, g.metrics["inability_to_reclaim_events"])

    def test_class_priority_reclaims_submit_before_texture_before_buffer(self):
        texture = TextureResidencyV1()
        payload = b"t" * 200
        k = key(payload)
        texture.demand(k, payload)
        texture.release_placement(k)
        tid = texture.identity_for_key(k)
        buffers = BufferResidencyV1(initial_capacity=512)
        buffers.allocate("buffer:a", b"b" * 200)
        submit = RecordingInvalidationPoolV1()
        submit.add("command:a", 200)

        g = RenderMemoryGovernorV1(
            soft_target_bytes=300,
            high_target_bytes=500,
            rearm_margin_bytes=50,
            cooldown_frames=0,
        )
        g.register_pool("submit", "submit_command", submit)
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        g.register_pool("buffer", "buffer", BufferResidencyMemoryAdapterV1(buffers))
        g.note_entry("submit", "command:a", last_used_frame=0)
        g.note_entry("texture", tid, last_used_frame=0)
        g.note_entry("buffer", "buffer:a", last_used_frame=0)
        receipt = g.reclaim_to(target_resident_bytes=400, frame=2, reason="soft")
        self.assertEqual("submit_command", receipt["decisions"][0]["residency_class"])

    def test_quality_rank_prefers_redundant_high_quality_variant(self):
        texture = TextureResidencyV1()
        high_payload = b"h" * 300
        low_payload = b"l" * 100
        high = key(high_payload, "r", "2x")
        low = key(low_payload, "r", "1x")
        texture.demand(high, high_payload)
        texture.release_placement(high)
        texture.demand(low, low_payload)
        texture.release_placement(low)
        high_id = texture.identity_for_key(high)
        low_id = texture.identity_for_key(low)

        g = governor()
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        g.note_entry("texture", high_id, last_used_frame=1, quality_rank=2)
        g.note_entry("texture", low_id, last_used_frame=0, quality_rank=1)
        receipt = g.reclaim_to(target_resident_bytes=100, frame=10, reason="soft")
        self.assertEqual(high_id, receipt["decisions"][0]["identity"])

    def test_cooldown_blocks_immediate_evict_rebuild_thrash(self):
        texture = TextureResidencyV1()
        payload = b"x" * 300
        k = key(payload)
        texture.demand(k, payload)
        texture.release_placement(k)
        identity = texture.identity_for_key(k)

        g = governor()
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        g.note_entry("texture", identity, last_used_frame=0)
        first = g.reclaim_to(target_resident_bytes=0, frame=1, reason="soft")
        self.assertTrue(first["success"])
        texture.demand(k, payload)
        texture.release_placement(k)
        g.note_entry("texture", identity, last_used_frame=2)
        second = g.reclaim_to(target_resident_bytes=0, frame=2, reason="soft")
        self.assertFalse(second["success"])
        self.assertEqual(1, g.metrics["churn_rebuilds"])

    def test_device_reset_advances_generations_and_clears_pins(self):
        texture = TextureResidencyV1()
        payload = b"x" * 128
        k = key(payload)
        texture.demand(k, payload)
        identity = texture.identity_for_key(k)
        buffers = BufferResidencyV1()
        buffers.allocate("b", b"1234")
        g = governor()
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        g.register_pool("buffer", "buffer", BufferResidencyMemoryAdapterV1(buffers))
        g.pin("texture", identity, reason="visible", expires_after_frame=99)
        before_t, before_b = texture.device_generation, buffers.device_generation
        g.reset_device_generation()
        self.assertEqual(before_t + 1, texture.device_generation)
        self.assertEqual(before_b + 1, buffers.device_generation)
        self.assertEqual(set(), g._active_pins(1))

    def test_full_renderer_cache_loss_preserves_canonical_truth_and_rebuilds(self):
        texture = TextureResidencyV1()
        payload = b"p" * 256
        k = key(payload)
        binding_before = texture.demand(k, payload)
        texture.release_placement(k)
        buffers = BufferResidencyV1()
        buffers.allocate("page:a", b"geometry", page_id="page:a")
        buffer_truth = buffers.normalized_logical_state()
        resource_truth = (k.resource_id, k.content_hash, k.derivative)

        g = governor()
        g.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
        g.register_pool("buffer", "buffer", BufferResidencyMemoryAdapterV1(buffers))
        for row in texture.memory_entries():
            g.note_entry("texture", row["identity"], last_used_frame=0)
        g.note_entry("buffer", "page:a", last_used_frame=0)
        receipt = g.reclaim_to(target_resident_bytes=0, frame=10, reason="full-cache-loss")
        self.assertTrue(receipt["success"])
        self.assertEqual(0, receipt["canonical_mutations"])

        texture.demand(k, payload)
        buffers.revisit_page("page:a")
        self.assertEqual(resource_truth, (k.resource_id, k.content_hash, k.derivative))
        self.assertEqual(buffer_truth, buffers.normalized_logical_state())
        self.assertFalse(texture.validate_binding(binding_before))

    def test_unknown_gpu_accounting_is_explicit(self):
        unknown = RecordingInvalidationPoolV1()
        unknown.memory_entries = lambda: [{
            "identity": "gpu:opaque",
            "resident_bytes": None,
            "reclaimable": True,
        }]
        g = governor()
        g.register_pool("submit", "submit_command", unknown)
        accounting = g.accounting(0)
        self.assertEqual(["submit"], accounting["unknown_byte_pools"])
        self.assertIsNone(accounting["pools"]["submit"]["resident_bytes"])


if __name__ == "__main__":
    unittest.main()
