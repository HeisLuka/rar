#!/usr/bin/env python3
import unittest

from buffer_residency_v1 import *


class BufferResidencyTests(unittest.TestCase):
    def test_allocate_free_reuse_increments_generation(self):
        r = BufferResidencyV1(initial_capacity=128)
        h1 = r.allocate("atom:1", b"a" * 16, page_id="p1")
        r.free(h1)
        with self.assertRaises(StaleHandle):
            r.binding(h1)
        h2 = r.allocate("atom:1", b"b" * 16, page_id="p1")
        self.assertEqual(h1.slot_id, h2.slot_id)
        self.assertGreater(h2.generation, h1.generation)

    def test_one_node_update_marks_only_bounded_range(self):
        r = BufferResidencyV1(initial_capacity=1024, coalesce_gap=0)
        a = r.allocate("a", b"a" * 128, page_id="p1")
        r.allocate("b", b"b" * 128, page_id="p1")
        r.clear_dirty()
        r.update(a, b"12345678", byte_offset=8)
        plan = r.upload_plan()
        self.assertEqual(plan["logical_changed_bytes"], 8)
        self.assertEqual(plan["physical_upload_bytes"], 8)
        self.assertLess(plan["physical_upload_bytes"], r.fragmentation()["live_bytes"])

    def test_overlapping_and_nearby_dirty_ranges_coalesce_deterministically(self):
        r = BufferResidencyV1(initial_capacity=256, coalesce_gap=8)
        h = r.allocate("a", b"x" * 128)
        r.clear_dirty()
        r.update(h, b"abcd", byte_offset=8)
        r.update(h, b"efgh", byte_offset=14)
        r.update(h, b"ij", byte_offset=24)
        plan = r.upload_plan()
        self.assertEqual(len(plan["spans"]), 1)
        self.assertEqual(plan["logical_changed_bytes"], 10)
        self.assertGreater(plan["physical_upload_bytes"], plan["logical_changed_bytes"])

    def test_growth_preserves_logical_handle_and_invalidates_old_binding_only(self):
        r = BufferResidencyV1(initial_capacity=32)
        h = r.allocate("a", b"a" * 16)
        old = r.binding(h)
        r.update(h, b"b" * 80)
        self.assertEqual(h, LogicalHandle(h.slot_id, h.generation))
        self.assertFalse(r.validate_binding(old))
        self.assertTrue(r.validate_binding(r.binding(h)))
        self.assertGreaterEqual(r.metrics["growth_events"], 1)

    def test_compaction_preserves_logical_state_and_only_moved_binding_changes(self):
        r = BufferResidencyV1(initial_capacity=256)
        a = r.allocate("a", b"a" * 32)
        b = r.allocate("b", b"b" * 32)
        c = r.allocate("c", b"c" * 32)
        before_c = r.binding(c)
        logical = r.normalized_logical_state()
        r.free(b)
        r.clear_dirty()
        result = r.compact()
        self.assertEqual(r.normalized_logical_state(), [row for row in logical if row["logical_id"] != "b"])
        self.assertIn(c.slot_id, result["moved_slot_ids"])
        self.assertFalse(r.validate_binding(before_c))
        self.assertTrue(r.validate_binding(r.binding(a)))

    def test_page_eviction_leaves_other_page_untouched_and_revisit_rebuilds(self):
        r = BufferResidencyV1(initial_capacity=512)
        p1 = r.allocate("p1:a", b"a" * 32, page_id="p1")
        p2 = r.allocate("p2:a", b"b" * 32, page_id="p2")
        p2_binding = r.binding(p2)
        self.assertEqual(r.evict_page("p1"), 1)
        self.assertTrue(r.validate_binding(p2_binding))
        with self.assertRaises(ValueError):
            r.binding(p1)
        self.assertEqual(r.revisit_page("p1"), 1)
        self.assertEqual(bytes(r._slot_for_handle(p1).data), b"a" * 32)

    def test_device_reset_rejects_old_physical_generation_but_keeps_logical_state(self):
        r = BufferResidencyV1(initial_capacity=256)
        h = r.allocate("a", b"payload", page_id="p1")
        before = r.normalized_logical_state()
        old = r.binding(h)
        r.reset_device()
        self.assertEqual(before, r.normalized_logical_state())
        self.assertFalse(r.validate_binding(old))
        self.assertTrue(r.validate_binding(r.binding(h)))
        self.assertEqual(r.device_generation, 2)

    def test_staging_ring_prevents_inflight_overwrite_and_has_oversized_fallback(self):
        ring = StagingRing(64)
        first = ring.plan(48, fence=1, completed_fence=0)
        self.assertEqual(first["offset"], 0)
        with self.assertRaises(StagingRingBusy):
            ring.plan(32, fence=2, completed_fence=0)
        second = ring.plan(32, fence=2, completed_fence=1)
        self.assertEqual(second["mode"], "ring")
        self.assertEqual(ring.plan(128, fence=3, completed_fence=2)["mode"], "oversized")

    def test_clean_rebuild_equals_incremental_logical_content(self):
        incremental = BufferResidencyV1(initial_capacity=128)
        h = incremental.allocate("a", b"aaaa")
        incremental.clear_dirty()
        incremental.update(h, b"bb", byte_offset=1)
        final_bytes = bytes(incremental._slot_for_handle(h).data)

        clean = BufferResidencyV1(initial_capacity=128)
        hc = clean.allocate("a", final_bytes)
        self.assertEqual(bytes(clean._slot_for_handle(hc).data), final_bytes)


if __name__ == "__main__":
    unittest.main()
