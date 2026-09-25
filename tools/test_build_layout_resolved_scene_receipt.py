#!/usr/bin/env python3
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_layout_resolved_scene_receipt import build_receipt
from validate_layout_resolved_scene_receipt import validate_schema, validate_semantics

SOURCE_HASH = "a" * 64
DOCUMENT_ID = "30000000-0000-4000-8000-000000000001"

FAKE_PRODUCER = r"""#!/usr/bin/env python3
import copy
import json
import sys

payload = json.load(sys.stdin)
source_hash = payload["source_hash"]
node_id = "10000000-0000-4000-8000-000000000001"
page_id = "20000000-0000-4000-8000-000000000001"
before = {"x": 1000, "y": 2000, "width": 3000, "height": 4000}
after = {"x": 128000, "y": 256000, "width": 3000, "height": 4000}
base_snap = "sha256:" + "b" * 64
move_snap = "sha256:" + "c" * 64
surface_hash = "sha256:" + "d" * 64
origin_hash = "sha256:" + "e" * 64
geometry_hash = "sha256:" + "f" * 64
context_hash = "sha256:" + "1" * 64
projection_context_state = {
    "projection_context_hash": context_hash,
    "master_relation_count": 0,
    "cmo_relation_count": 0,
    "carried_outside_editor_project": True,
    "cmo_layout_consumed": False,
}

def scene(snapshot, bounds):
    return {
        "scene_snapshot_id": snapshot,
        "node_id": node_id,
        "page_id": page_id,
        "origin_node_id": node_id,
        "bounds": copy.deepcopy(bounds),
        "surface_page_ids_hash": surface_hash,
        "origin_mapping_hash": origin_hash,
        "projection_input": "current_resolved_graph",
    }

baseline_project = {
    "schema_version": "pub-editor-v0.4",
    "source_hash": source_hash,
    "operations": [],
}
operation = {
    "kind": "move_node",
    "node_id": node_id,
    "before": copy.deepcopy(before),
    "after": copy.deepcopy(after),
}
accepted_project = copy.deepcopy(baseline_project)
accepted_project["operations"] = [copy.deepcopy(operation)]

action = payload["action"]
if action == "baseline":
    json.dump({
        "source_hash": source_hash,
        "baseline_project": baseline_project,
        "move_candidate": {
            "node_id": node_id,
            "page_id": page_id,
            "before": before,
            "after": after,
        },
        "baseline_scene_state": scene(base_snap, before),
        "baseline_equivalence": {
            "viewer_geometry_hash": geometry_hash,
            "adapter_geometry_hash": geometry_hash,
            "viewer_surface_hash": surface_hash,
            "adapter_surface_hash": surface_hash,
            "viewer_origin_mapping_hash": origin_hash,
            "adapter_origin_mapping_hash": origin_hash,
        },
        "projection_context_state": projection_context_state,
        "adapter_invariants": {
            "viewer_private_mapping_used": False,
            "browser_layout_authoritative": False,
            "second_geometry_model_created": False,
            "context_extension_seam_present": True,
            "graph_only_wrapper_is_empty_context": True,
            "projection_context_carried_outside_editor_project": True,
            "unsupported_cmo_layout_deferred": True,
        },
    }, sys.stdout)
    raise SystemExit(0)

if action == "commit":
    command = payload["command"]
    if command["node_id"] != node_id:
        raise SystemExit(3)
    json.dump({
        "canonical_operation": operation,
        "resulting_project": accepted_project,
        "consequences": [{"key": "node.geometry.position", "state": "supported", "note": None}],
        "scene_state": scene(move_snap, after),
        "source_hash_after": source_hash,
        "source_reparse_after_edit_count": 0,
        "projection_context_state": projection_context_state,
    }, sys.stdout)
    raise SystemExit(0)

if action == "history":
    if payload["kind"] == "undo":
        project = baseline_project
        state = scene(base_snap, before)
    elif payload["kind"] == "redo":
        project = accepted_project
        state = scene(move_snap, after)
    else:
        raise SystemExit(4)
    json.dump({
        "resulting_project": project,
        "scene_state": state,
        "consequences": [{"key": "history." + payload["kind"], "state": "supported", "note": None}],
        "source_hash_after": source_hash,
        "source_reparse_after_edit_count": 0,
        "projection_context_state": projection_context_state,
    }, sys.stdout)
    raise SystemExit(0)

if action == "replay":
    json.dump({
        "replayed_project": accepted_project,
        "scene_state": scene(move_snap, after),
        "source_hash_after": source_hash,
        "source_reparse_after_edit_count": 0,
        "projection_context_state": projection_context_state,
    }, sys.stdout)
    raise SystemExit(0)

raise SystemExit(2)
"""


class LayoutResolvedSceneProducerBuilderTests(unittest.TestCase):
    def test_builder_reuses_revision_kernel_and_emits_valid_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = pathlib.Path(tmp) / "fake_scene_producer.py"
            producer.write_text(FAKE_PRODUCER, encoding="utf-8")
            receipt = build_receipt(
                [sys.executable, str(producer)],
                source_hash=SOURCE_HASH,
                document_id=DOCUMENT_ID,
                implementation="fake-layout-adapter",
                commit_or_build="test-build",
            )

        validate_schema(receipt)
        summary = validate_semantics(receipt)
        self.assertEqual(receipt["states"]["baseline"], receipt["states"]["undo"])
        self.assertEqual(receipt["states"]["accepted"], receipt["states"]["redo"])
        self.assertEqual(receipt["states"]["accepted"], receipt["states"]["replay"])
        self.assertTrue(summary["accepted_equal_to_replay"])
        self.assertEqual(0, summary["source_reparse_after_edit_count"])

    def test_builder_fails_closed_on_post_edit_source_reparse(self):
        broken = FAKE_PRODUCER.replace(
            '"source_reparse_after_edit_count": 0,',
            '"source_reparse_after_edit_count": 1,',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            producer = pathlib.Path(tmp) / "bad_scene_producer.py"
            producer.write_text(broken, encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "reparsed"):
                build_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    document_id=DOCUMENT_ID,
                    implementation="fake-layout-adapter",
                    commit_or_build="test-build",
                )


if __name__ == "__main__":
    unittest.main()
