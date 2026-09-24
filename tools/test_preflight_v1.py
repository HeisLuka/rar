#!/usr/bin/env python3
import copy
import unittest
from preflight_v1 import evaluate

NODE_RESOURCE_MISSING = "10000000-0000-4000-8000-000000000001"
NODE_STORY = "10000000-0000-4000-8000-000000000002"
NODE_RESOURCE_MODIFIED = "10000000-0000-4000-8000-000000000003"
RESOURCE_MISSING = "20000000-0000-4000-8000-000000000001"
RESOURCE_MODIFIED = "20000000-0000-4000-8000-000000000002"
STORY_ID = "30000000-0000-4000-8000-000000000001"

BASE_SCENE = {
    "source_hash": "a" * 64,
    "revision_id": "sha256:" + "b" * 64,
    "snapshot_id": "sha256:" + "c" * 64,
    "nodes": [
        {"node_id": NODE_RESOURCE_MISSING, "resource_id": RESOURCE_MISSING},
        {"node_id": NODE_STORY, "resource_id": None},
        {"node_id": NODE_RESOURCE_MODIFIED, "resource_id": RESOURCE_MODIFIED},
    ],
    "resources": [
        {
            "resource_id": RESOURCE_MISSING,
            "availability": "missing",
            "content_hash": None,
        },
        {
            "resource_id": RESOURCE_MODIFIED,
            "availability": "available",
            "content_hash": "d" * 64,
        },
    ],
    "stories": [{
        "story_id": STORY_ID,
        "text_fidelity": "opaque",
    }],
    "story_frames": [{
        "story_id": STORY_ID,
        "frame_ordinal": 0,
        "node_id": NODE_STORY,
    }],
    "diagnostics": [{
        "severity": "error",
        "code": "layout.story_overset",
        "origin_node_id": NODE_STORY,
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
    "origin_node_id": NODE_STORY,
    "detail": "pdf-v0",
}]

RESOURCE_EXPECTATIONS = {
    RESOURCE_MODIFIED: "e" * 64,
}


class PreflightTests(unittest.TestCase):
    def test_representative_defects_are_object_scoped_and_typed(self):
        receipt = evaluate(
            copy.deepcopy(BASE_SCENE),
            copy.deepcopy(RISKS),
            copy.deepcopy(RESOURCE_EXPECTATIONS),
        )
        codes = [d["code"] for d in receipt["diagnostics"]]
        for code in [
            "layout.story_overset",
            "preflight.resource_missing",
            "preflight.resource_modified",
            "preflight.story_semantics_opaque",
            "preflight.capability_partial",
            "preflight.output_transparency_risk",
        ]:
            self.assertIn(code, codes)

        missing = next(d for d in receipt["diagnostics"] if d["code"] == "preflight.resource_missing")
        modified = next(d for d in receipt["diagnostics"] if d["code"] == "preflight.resource_modified")
        opaque = next(d for d in receipt["diagnostics"] if d["code"] == "preflight.story_semantics_opaque")
        self.assertEqual(NODE_RESOURCE_MISSING, missing["origin_node_id"])
        self.assertEqual(NODE_RESOURCE_MODIFIED, modified["origin_node_id"])
        self.assertEqual(NODE_STORY, opaque["origin_node_id"])
        self.assertTrue(receipt["summary"]["blocking"])
        self.assertEqual(
            f"{receipt['summary']['error_count']} error(s), "
            f"{receipt['summary']['warning_count']} warning(s), "
            f"{receipt['summary']['info_count']} info; blocking=yes",
            receipt["summary"]["human"],
        )

    def test_bounded_fix_removes_only_resolved_diagnostic(self):
        before = evaluate(
            copy.deepcopy(BASE_SCENE),
            copy.deepcopy(RISKS),
            copy.deepcopy(RESOURCE_EXPECTATIONS),
        )
        fixed = copy.deepcopy(BASE_SCENE)
        fixed["resources"][0]["availability"] = "available"
        after = evaluate(
            fixed,
            copy.deepcopy(RISKS),
            copy.deepcopy(RESOURCE_EXPECTATIONS),
        )
        before_codes = [d["code"] for d in before["diagnostics"]]
        after_codes = [d["code"] for d in after["diagnostics"]]
        self.assertIn("preflight.resource_missing", before_codes)
        self.assertNotIn("preflight.resource_missing", after_codes)
        for code in [
            "layout.story_overset",
            "preflight.resource_modified",
            "preflight.story_semantics_opaque",
            "preflight.capability_partial",
            "preflight.output_transparency_risk",
        ]:
            self.assertIn(code, after_codes)

    def test_identical_inputs_are_deterministic(self):
        args = (
            copy.deepcopy(BASE_SCENE),
            copy.deepcopy(RISKS),
            copy.deepcopy(RESOURCE_EXPECTATIONS),
        )
        a = evaluate(*args)
        b = evaluate(
            copy.deepcopy(BASE_SCENE),
            copy.deepcopy(RISKS),
            copy.deepcopy(RESOURCE_EXPECTATIONS),
        )
        self.assertEqual(a, b)


if __name__ == "__main__":
    unittest.main()
