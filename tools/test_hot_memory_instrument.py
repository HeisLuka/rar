#!/usr/bin/env python3
import unittest

from hot_memory_probe_v1 import ResidencyProbe, ResidencyProbeError
from run_hot_memory_instrument import build_receipt
from hot_memory_receipt_v1 import validate_receipt


class ResidencyProbeTests(unittest.TestCase):
    def test_exclusive_ownership_rejects_double_count(self):
        probe = ResidencyProbe()
        probe.allocate("L1", "same", 1024)
        with self.assertRaises(ResidencyProbeError):
            probe.allocate("L2", "same", 1024)

    def test_shared_category_is_explicit_and_not_layer_double_counted(self):
        probe = ResidencyProbe()
        probe.allocate_shared("font-bytes", "font-a", 4096)
        self.assertEqual(probe.shared_bytes(), 4096)
        self.assertEqual(sum(probe.layer_metrics().values()), 0)

    def test_transition_and_eviction_order_fail_closed(self):
        probe = ResidencyProbe()
        with self.assertRaises(ResidencyProbeError):
            probe.transition("HOT")
        probe.transition("WARM")
        for layer in ("L1", "L2", "L3", "L4", "L5", "L6"):
            probe.allocate(layer, layer, 1)
        probe.transition("HOT")
        with self.assertRaises(ResidencyProbeError):
            probe.evict("L4")
        for layer in ("L5", "L4", "L3", "L2"):
            probe.evict(layer)

    def test_reset_clears_owned_and_shared_state(self):
        probe = ResidencyProbe()
        probe.allocate("L1", "model", 1024)
        probe.allocate_shared("shared", "x", 2048)
        probe.transition("WARM")
        probe.reset()
        self.assertEqual(probe.phase, "COLD")
        self.assertEqual(sum(probe.layer_metrics().values()), 0)
        self.assertEqual(probe.shared_bytes(), 0)

    def test_runner_emits_valid_non_authoritative_receipt_and_mode_delta(self):
        receipt = build_receipt("test-build")
        validate_receipt(receipt)
        self.assertFalse(receipt["evidence_authority"]["capacity_decision_allowed"])
        modes = {mode["mode"]: mode for mode in receipt["modes"]}
        semantic = modes["semantic_only_server"]["layers"]
        layout = modes["server_layout"]["layers"]
        self.assertEqual(semantic["L3"]["resident"]["bytes"], 0)
        self.assertEqual(semantic["L4"]["resident"]["bytes"], 0)
        self.assertGreater(layout["L3"]["resident"]["bytes"], 0)
        self.assertGreater(layout["L4"]["resident"]["bytes"], 0)
        for mode in modes.values():
            self.assertEqual(
                [row["drop_layer"] for row in mode["eviction"]],
                ["L5", "L4", "L3", "L2"],
            )
            self.assertEqual(mode["layers"]["L6"]["resident"]["bytes"], 0)
            self.assertGreaterEqual(len(mode["activation"]), 3)


if __name__ == "__main__":
    unittest.main()
