#!/usr/bin/env python3
import copy
import unittest
from preflight_v1 import evaluate

BASE_SCENE = {
    "source_hash": "a" * 64,
    "revision_id": "sha256:" + "b" * 64,
    "snapshot_id": "sha256:" + "c" * 64,
    "nodes": [
        {"node_id": "10000000-0000-4000-8000-000000000001", "resource_id": "20000000-0000-4000-8000-000000000001"},
        {"node_id": "10000000-0000-4000-8000-000000000002", "resource_id": None},
    ],
    "resources": [{
        "resource_id": "20000000-0000-4000-8000-000000000001",
        "availability": "missing",
    }],
    "stories": [{
        "story_id": "30000000-0000-4000-8000-000000000001",
        "text_fidelity": "opaque",
    }],
    "diagnostics": [{
        "severity": "error",
        "code": "layout.story_overset",
        "origin_node_id": "10000000-0000-4000-8000-000000000002",
        "message_key": "layout.story_overset",
    }],
    "capabilities": [{
        "key": "render.paint",
        "state": "partial",
        "note": "bounded",
    }],
}

RISKS = [{
    "code": "preflight.output_transparency_risk",
    "severity": "warning",
    "message_key": "preflight.output_transparency_risk",
    "origin_node_id": "10000000-0000-4000-8000-000000000002",
    "detail": "pdf-v0",
}]

class PreflightTests(unittest.TestCase):
    def test_representative_defects_are_object_scoped_and_typed(self):
        receipt = evaluate(copy.deepcopy(BASE_SCENE), copy.deepcopy(RISKS))
        codes = [d["code"] for d in receipt["diagnostics"]]
        self.assertIn("layout.story_overset", codes)
        self.assertIn("preflight.resource_missing", codes)
        self.assertIn("preflight.story_semantics_opaque", codes)
        self.assertIn("preflight.capability_partial", codes)
        self.assertIn("preflight.output_transparency_risk", codes)
        missing = next(d for d in receipt["diagnostics"] if d["code"] == "preflight.resource_missing")
        self.assertEqual("10000000-0000-4000-8000-000000000001", missing["origin_node_id"])
        self.assertTrue(receipt["summary"]["blocking"])

    def test_bounded_fix_removes_only_resolved_diagnostic(self):
        before = evaluate(copy.deepcopy(BASE_SCENE), copy.deepcopy(RISKS))
        fixed = copy.deepcopy(BASE_SCENE)
        fixed["resources"][0]["availability"] = "available"
        after = evaluate(fixed, copy.deepcopy(RISKS))
        before_codes = [d["code"] for d in before["diagnostics"]]
        after_codes = [d["code"] for d in after["diagnostics"]]
        self.assertIn("preflight.resource_missing", before_codes)
        self.assertNotIn("preflight.resource_missing", after_codes)
        for code in ["layout.story_overset", "preflight.story_semantics_opaque", "preflight.capability_partial", "preflight.output_transparency_risk"]:
            self.assertIn(code, after_codes)

    def test_identical_inputs_are_deterministic(self):
        a = evaluate(copy.deepcopy(BASE_SCENE), copy.deepcopy(RISKS))
        b = evaluate(copy.deepcopy(BASE_SCENE), copy.deepcopy(RISKS))
        self.assertEqual(a, b)

if __name__ == "__main__":
    unittest.main()
