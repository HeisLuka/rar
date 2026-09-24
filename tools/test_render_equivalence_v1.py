import copy
import unittest

from optimization_receipt_v1 import identity_hash, observed
from render_equivalence_v1 import *


def scene():
    return {
        "protocol_version": "chaptera.scene.v1",
        "document_id": "doc:12345678",
        "source_hash": "sha256:" + "a" * 64,
        "snapshot_id": "sha256:" + "b" * 64,
        "layout_environment": {"environment_id": "env:12345678"},
        "pages": [{"page_id": "page:1", "order": 0, "width_emu": 100, "height_emu": 100}],
        "nodes": [], "stories": [], "story_frames": [], "paints": [], "resources": [], "diagnostics": [], "capabilities": [],
        "fidelity": {"state": "exact", "reasons": []},
    }


def render(sha="a"):
    return {"pages": [{"page_id": "page:1", "artifact_sha256": sha * 64, "regions": []}]}


def env(dpr=1):
    return {"viewport_width_px": 800, "viewport_height_px": 600, "dpr": dpr, "output_width_px": 800, "output_height_px": 600, "color_profile": "srgb", "resource_state": "settled"}


def measurement(sha, value=1.0):
    workload = {"id": "same"}
    return {
        "schema": "chaptera.optimization.measurement.v1",
        "producer": {"receipt_version": "test", "measurement_class": "synthetic"},
        "build_identity": {"sha": sha},
        "runtime_identity": {"runtime": "test"},
        "workload_identity": workload,
        "workload_identity_hash": identity_hash(workload),
        "correctness": {},
        "evidence_authority": {"technology_decision_allowed": False},
        "metrics": {"frame.ms": observed(value, "ms", "lower_is_better")},
        "limitations": [],
    }


class EquivalenceTests(unittest.TestCase):
    def test_settled_equal_passes(self):
        r = compare_render_paths(baseline_scene=scene(), candidate_scene=scene(), baseline_render=render(), candidate_render=render(), baseline_environment=env(), candidate_environment=env(), reference_identity={"backend":"ref"}, candidate_identity={"backend":"candidate"})
        self.assertTrue(r["passed"])

    def test_render_difference_fails_settled_fidelity(self):
        r = compare_render_paths(baseline_scene=scene(), candidate_scene=scene(), baseline_render=render("a"), candidate_render=render("c"), baseline_environment=env(), candidate_environment=env(), reference_identity={}, candidate_identity={})
        self.assertFalse(r["fidelity_equivalent"])
        self.assertTrue(r["correctness_equivalent"])

    def test_transient_render_difference_can_be_admitted_only_if_settled_converges(self):
        r = compare_render_paths(baseline_scene=scene(), candidate_scene=scene(), baseline_render=render("a"), candidate_render=render("c"), baseline_environment=env(), candidate_environment=env(), reference_identity={}, candidate_identity={}, mode="transient", admitted_transient_render_codes={"render.page_artifact_changed"}, settled_baseline_render=render("z"), settled_candidate_render=render("z"))
        self.assertTrue(r["passed"])
        self.assertTrue(r["convergence"]["equivalent"])

    def test_environment_mismatch_fails_closed(self):
        with self.assertRaises(RenderEquivalenceIncomparable):
            compare_render_paths(baseline_scene=scene(), candidate_scene=scene(), baseline_render=render(), candidate_render=render(), baseline_environment=env(1), candidate_environment=env(2), reference_identity={}, candidate_identity={})

    def test_optimization_receipt_consumes_equivalence_fence(self):
        bad = compare_render_paths(baseline_scene=scene(), candidate_scene=scene(), baseline_render=render("a"), candidate_render=render("b"), baseline_environment=env(), candidate_environment=env(), reference_identity={}, candidate_identity={})
        opt = compare_optimization_with_equivalence(equivalence_receipt=bad, optimization_id="opt:1", hot_path="render", baseline_measurement=measurement("base"), candidate_measurement=measurement("cand", 0.5))
        self.assertEqual(opt["decision"], "revert")
        self.assertFalse(opt["correctness_fidelity_fence"]["passed"])


if __name__ == "__main__":
    unittest.main()
