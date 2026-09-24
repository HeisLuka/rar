import unittest
from backend_capability_v1 import *


class BackendCapabilityTests(unittest.TestCase):
    def req(self, mandatory=frozenset({"rect", "image_rgba8", "clip:rect"}), optional=frozenset({"timestamp_query"}), **limits):
        return RenderRequirementSet(mandatory, optional, frozenset({"timestamp_query"}), **limits)

    def test_deterministic_exact(self):
        a = compatibility(fake_backend("full"), self.req())
        b = compatibility(fake_backend("full"), self.req())
        self.assertEqual(a, b)
        self.assertEqual(a["outcome"], "CompatibleExact")

    def test_missing_mandatory_fails_closed(self):
        got = compatibility(fake_backend("no_stencil"), self.req(mandatory=frozenset({"rect", "clip:stencil"})))
        self.assertEqual(got["outcome"], "Incompatible")
        self.assertIn("missing_mandatory:clip:stencil", got["reason_codes"])

    def test_optional_quality_degrades_and_performance_does_not_reject(self):
        got = compatibility(fake_backend("no_timing"), self.req(optional=frozenset({"timestamp_query", "msaa4"})))
        self.assertEqual(got["outcome"], "CompatibleDegraded")
        self.assertIn("missing_optional:timestamp_query", got["reason_codes"])

    def test_low_limit_is_workload_specific_incompatible(self):
        got = compatibility(fake_backend("low_limit"), self.req(max_texture_dimension=4096))
        self.assertEqual(got["outcome"], "Incompatible")
        self.assertIn("limit:max_texture_dimension", got["reason_codes"])

    def test_generation_invalidates_old_handle_and_rebuild_mutates_no_authority(self):
        life = BackendLifecycle(fake_backend("full"))
        old = life.make_handle("tex:1")
        life.lose()
        self.assertFalse(life.validate_handle(old))
        life.recreate(fake_backend("no_timing", generation=2))
        self.assertFalse(life.validate_handle(old))
        rebuild = life.rebuild(scene_fingerprint="scene:a", resource_fingerprint="res:a", view_fingerprint="view:a")
        self.assertEqual(rebuild["device_generation"], 2)
        self.assertEqual(rebuild["authoring_mutations"], 0)
        self.assertEqual(rebuild["layout_mutations"], 0)

    def test_permanent_failure_is_unavailable(self):
        got = compatibility(fake_backend("permanent_failure"), self.req())
        self.assertEqual(got["outcome"], "Unavailable")


if __name__ == "__main__":
    unittest.main()
