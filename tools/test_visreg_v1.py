#!/usr/bin/env python3
import copy
import unittest

from scene_v1 import finalize_snapshot, hash_id
from visreg_v1 import canonical_json, compare_scenes


DOC_ID = "90000000-0000-4000-8000-000000000001"
PAGE_ID = "10000000-0000-4000-8000-000000000001"
NODE_ID = "20000000-0000-4000-8000-000000000001"
STORY_ID = "30000000-0000-4000-8000-000000000001"


def baseline_scene():
    scene = {
        "protocol_version": "chaptera.scene.v1",
        "document_id": DOC_ID,
        "source_hash": "a" * 64,
        "revision_id": "sha256:" + "1" * 64,
        "snapshot_id": "sha256:" + "0" * 64,
        "layout_environment": {
            "environment_id": hash_id({"environment": "visreg-test"}),
            "engine_revision": "visreg-test",
            "font_set_fingerprint": hash_id({"fonts": "test"}),
            "resource_fingerprint": hash_id({"resources": "test"}),
        },
        "stacking_fidelity": "exact",
        "pages": [
            {
                "page_id": PAGE_ID,
                "order": 0,
                "width_emu": 9144000,
                "height_emu": 6858000,
            }
        ],
        "nodes": [
            {
                "node_id": NODE_ID,
                "page_id": PAGE_ID,
                "parent_node_id": None,
                "kind": "text_frame",
                "bounds": {"x": 100, "y": 200, "width": 1000, "height": 500},
                "transform": {"a": "1", "b": "0", "c": "0", "d": "1", "tx": 0, "ty": 0},
                "z_order": 0,
                "paint_order": 0,
                "paint_id": None,
                "resource_id": None,
            }
        ],
        "stories": [
            {"story_id": STORY_ID, "text": "hello", "text_fidelity": "exact"}
        ],
        "story_frames": [
            {"story_id": STORY_ID, "frame_ordinal": 0, "node_id": NODE_ID}
        ],
        "paints": [],
        "resources": [],
        "diagnostics": [],
        "capabilities": [],
        "fidelity": {"state": "supported", "reasons": []},
    }
    return finalize_snapshot(scene)


def render_evidence(value):
    return {
        "pages": [
            {
                "page_id": PAGE_ID,
                "artifact_sha256": value * 64,
                "regions": [
                    {
                        "origin_node_id": NODE_ID,
                        "bbox_px": [10, 20, 30, 40],
                        "sha256": value * 64,
                    }
                ],
            }
        ]
    }


class VisregV1Tests(unittest.TestCase):
    def test_identical_inputs_are_equivalent_and_deterministic(self):
        base = baseline_scene()
        a = compare_scenes(base, copy.deepcopy(base))
        b = compare_scenes(base, copy.deepcopy(base))
        self.assertTrue(a["summary"]["equivalent"])
        self.assertEqual(canonical_json(a), canonical_json(b))

    def test_geometry_regression_is_layout_not_render(self):
        base = baseline_scene()
        candidate = copy.deepcopy(base)
        candidate["nodes"][0]["bounds"]["x"] += 12700
        candidate = finalize_snapshot(candidate)
        report = compare_scenes(base, candidate)
        self.assertEqual(1, report["summary"]["stage_counts"]["layout"])
        self.assertEqual(0, report["summary"]["stage_counts"]["render"])
        self.assertEqual("node.bounds_changed", report["differences"][0]["code"])
        self.assertEqual(NODE_ID, report["differences"][0]["origin_id"])

    def test_text_change_is_text_layout(self):
        base = baseline_scene()
        candidate = copy.deepcopy(base)
        candidate["stories"][0]["text"] = "hello world"
        candidate = finalize_snapshot(candidate)
        report = compare_scenes(base, candidate)
        self.assertEqual(1, report["summary"]["stage_counts"]["text_layout"])
        self.assertEqual("story.text_changed", report["differences"][0]["code"])
        self.assertEqual(STORY_ID, report["differences"][0]["origin_id"])

    def test_render_only_difference_stays_render_only(self):
        base = baseline_scene()
        report = compare_scenes(
            base,
            copy.deepcopy(base),
            baseline_render=render_evidence("b"),
            candidate_render=render_evidence("c"),
        )
        self.assertTrue(report["summary"]["render_only"])
        self.assertFalse(report["summary"]["semantic_or_layout_change"])
        self.assertEqual(2, report["summary"]["stage_counts"]["render"])
        self.assertTrue(all(item["stage"] == "render" for item in report["differences"]))

    def test_environment_mismatch_fails_closed(self):
        base = baseline_scene()
        candidate = copy.deepcopy(base)
        candidate["layout_environment"]["engine_revision"] = "other"
        candidate = finalize_snapshot(candidate)
        with self.assertRaisesRegex(AssertionError, "same Layout Environment"):
            compare_scenes(base, candidate)


if __name__ == "__main__":
    unittest.main()
