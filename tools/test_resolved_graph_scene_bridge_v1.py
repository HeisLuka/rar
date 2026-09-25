#!/usr/bin/env python3
import copy
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from resolved_graph_scene_bridge_v1 import (
    ResolvedGraphSceneError,
    apply_project_to_resolved_graph,
    compact_scene_state,
    compare_viewer_and_adapter_scene,
    project_resolved_graph_scene,
)

SOURCE_HASH = "a" * 64
PAGE_ID = "20000000-0000-4000-8000-000000000001"
NODE_ID = "10000000-0000-4000-8000-000000000001"
STORY_ID = "30000000-0000-4000-8000-000000000001"
BEFORE = {"x": 1000, "y": 2000, "width": 3000, "height": 4000}
AFTER = {"x": 128000, "y": 256000, "width": 3000, "height": 4000}


def graph():
    return {
        "document": {
            "source_hash": SOURCE_HASH,
            "pages": [PAGE_ID],
        },
        "pages": {
            PAGE_ID: {
                "id": PAGE_ID,
                "size": {"width": 914400, "height": 1828800},
                "bleed": None,
                "margins": None,
                "children": [NODE_ID],
                "source_refs": [{"private": "must-not-cross"}],
            }
        },
        "nodes": {
            NODE_ID: {
                "header": {
                    "id": NODE_ID,
                    "parent_id": PAGE_ID,
                    "bounds": dict(BEFORE),
                    "transform": {
                        "a": "1", "b": "0", "c": "0", "d": "1",
                        "tx": 0, "ty": 0,
                    },
                    "source_refs": [{"byte_range": {"offset": 1, "length": 2}}],
                },
                "payload": {
                    "story_frame": {
                        "story_id": STORY_ID,
                        "ordinal": 0,
                        "previous_frame": None,
                        "next_frame": None,
                    },
                    "private": "ignored",
                },
            }
        },
        "stories": {
            STORY_ID: {
                "id": STORY_ID,
                "text": "PRIVATE STORY TEXT",
                "source_refs": [{"carrier": "Quill"}],
            }
        },
    }


def projection_context_sidecar():
    return {
        "schema_version": "chaptera.editor-projection-context-sidecar.v1",
        "source_hash": SOURCE_HASH,
        "context": {
            "schema_version": "chaptera.pub-projection-context.v1",
            "master_relations": [],
            "cmo_relations": [{
                "source_order": 0,
                "cmo_id": 1,
                "carrier_ohpo": 319,
                "carrier_cmo_id": 1,
                "target_qsid": 218,
                "carrier_node_id": NODE_ID,
                "carrier_story_id": None,
                "target_story_id": STORY_ID,
                "target_frame_node_id": NODE_ID,
            }],
        },
    }


def viewer_for(scene):
    return {
        "schema_version": "0.1",
        "document": {
            "schema_version": "0.1",
            "source": {
                "format": "publisher",
                "format_version": "0x2c",
                "source_hash": SOURCE_HASH,
                "byte_len": 1234,
            },
            "pages": [{
                "index": 1,
                "id": PAGE_ID,
                "width_emu": 914400,
                "height_emu": 1828800,
            }],
            "stories": [{"id": STORY_ID, "text": "PRIVATE STORY TEXT"}],
            "diagnostics": [],
        },
        "scene": scene,
        "story_frames": [{
            "story_id": STORY_ID,
            "frame_id": NODE_ID,
            "ordinal": 0,
        }],
    }


class ResolvedGraphSceneBridgeTests(unittest.TestCase):
    def test_bridge_matches_donor_geometry_law_and_ignores_private_state(self):
        left = project_resolved_graph_scene(graph())
        mutated = graph()
        mutated["stories"][STORY_ID]["text"] = "DIFFERENT PRIVATE TEXT"
        mutated["nodes"][NODE_ID]["header"]["source_refs"].append({"carrier": "Escher"})
        right = project_resolved_graph_scene(mutated)

        self.assertEqual(left, right)
        self.assertEqual(1, len(left["surfaces"]))
        self.assertEqual(1, len(left["nodes"]))
        self.assertEqual(BEFORE, left["nodes"][0]["bounds"])
        encoded = json.dumps(left, sort_keys=True)
        self.assertNotIn("PRIVATE", encoded)
        self.assertNotIn("source_refs", encoded)
        self.assertNotIn("Quill", encoded)
        self.assertNotIn("Escher", encoded)

    def test_real_viewer_equivalence_is_exact_not_count_only(self):
        scene = project_resolved_graph_scene(graph())
        eq = compare_viewer_and_adapter_scene(viewer_for(scene), scene)
        self.assertEqual(eq["viewer_geometry_hash"], eq["adapter_geometry_hash"])

        broken = viewer_for(copy.deepcopy(scene))
        broken["scene"]["nodes"][0]["bounds"]["x"] += 1
        with self.assertRaisesRegex(ResolvedGraphSceneError, "differs"):
            compare_viewer_and_adapter_scene(broken, scene)

    def test_typed_cmo_context_withholds_carrier_until_slot_flow(self):
        scene = project_resolved_graph_scene(
            graph(),
            context=projection_context_sidecar()["context"],
        )
        origins = {node["origin"] for node in scene["nodes"]}
        self.assertNotIn(NODE_ID, origins)
        pending = [
            item for item in scene["diagnostics"]
            if item["code"] == "cmo_slot_flow_not_materialized"
        ]
        self.assertEqual([STORY_ID], [item["origin"] for item in pending])

    def test_malformed_cmo_projection_context_still_fails_closed(self):
        with self.assertRaisesRegex(ResolvedGraphSceneError, "cmo_relations"):
            project_resolved_graph_scene(
                graph(),
                context={
                    "schema_version": "chaptera.pub-projection-context.v1",
                    "master_relations": [],
                    "cmo_relations": [{"x": 1}],
                },
            )

    def test_move_project_changes_current_graph_and_scene_snapshot(self):
        baseline_scene = project_resolved_graph_scene(graph())
        baseline_state = compact_scene_state(
            baseline_scene, node_id=NODE_ID, page_id=PAGE_ID
        )
        project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [{
                "kind": "move_node",
                "node_id": NODE_ID,
                "before": dict(BEFORE),
                "after": dict(AFTER),
            }],
        }
        current = apply_project_to_resolved_graph(graph(), project)
        accepted_scene = project_resolved_graph_scene(current)
        accepted_state = compact_scene_state(
            accepted_scene, node_id=NODE_ID, page_id=PAGE_ID
        )
        self.assertEqual(AFTER, accepted_state["bounds"])
        self.assertNotEqual(
            baseline_state["scene_snapshot_id"],
            accepted_state["scene_snapshot_id"],
        )

    def test_task_local_engine_baseline_commit_undo_redo_replay(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            graph_path = root / "resolved-graph.json"
            graph_path.write_text(json.dumps(graph()), encoding="utf-8")
            graph_sha = hashlib.sha256(graph_path.read_bytes()).hexdigest()

            scene = project_resolved_graph_scene(graph())
            viewer_path = root / "viewer.json"
            viewer_path.write_text(json.dumps(viewer_for(scene)), encoding="utf-8")
            state_dir = root / "state"
            fixture = root / "fixture.pub"
            fixture.write_bytes(b"launcher-verifies-real-fixture-in-production")
            context_path = root / "projection-context-sidecar.json"
            context_path.write_text(
                json.dumps(projection_context_sidecar()),
                encoding="utf-8",
            )

            prefix = [
                sys.executable,
                str(TOOLS / "run_sample_newsletter_resolved_scene_bridge.py"),
                "--resolved-graph", str(graph_path),
                "--viewer-receipt", str(viewer_path),
                "--expected-resolved-graph-sha256", graph_sha,
                "--target-node-id", NODE_ID,
                "--target-page-id", PAGE_ID,
                "--after-x-emu", str(AFTER["x"]),
                "--after-y-emu", str(AFTER["y"]),
                "--projection-context-sidecar", str(context_path),
            ]

            def invoke(action, payload, fixture_arg=False):
                command = prefix + [action, "--state-dir", str(state_dir)]
                if fixture_arg:
                    command += ["--fixture", str(fixture)]
                completed = subprocess.run(
                    command,
                    input=json.dumps(payload),
                    text=True,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                )
                if completed.returncode != 0:
                    self.fail(completed.stderr)
                return json.loads(completed.stdout)

            baseline = invoke(
                "baseline",
                {"action": "baseline", "source_hash": SOURCE_HASH},
                fixture_arg=True,
            )
            self.assertEqual(BEFORE, baseline["move_candidate"]["before"])
            self.assertEqual(AFTER, baseline["move_candidate"]["after"])
            self.assertEqual(
                "pub-editor-v0.2",
                baseline["baseline_project"]["schema_version"],
            )
            self.assertEqual(
                1,
                baseline["projection_context_state"]["cmo_relation_count"],
            )
            self.assertFalse(
                baseline["projection_context_state"]["cmo_layout_consumed"],
            )
            context_hash = baseline["projection_context_state"][
                "projection_context_hash"
            ]
            self.assertNotIn("projection_context", baseline["baseline_project"])

            command = {
                "kind": "move_node_to",
                "node_id": NODE_ID,
                "x_emu": AFTER["x"],
                "y_emu": AFTER["y"],
            }
            committed = invoke("commit", {
                "action": "commit",
                "source_hash": SOURCE_HASH,
                "base_project": baseline["baseline_project"],
                "command": command,
            })
            self.assertEqual(AFTER, committed["scene_state"]["bounds"])
            self.assertEqual(
                "pub-editor-v0.4",
                committed["resulting_project"]["schema_version"],
            )
            self.assertEqual(0, committed["source_reparse_after_edit_count"])
            self.assertEqual(
                context_hash,
                committed["projection_context_state"]["projection_context_hash"],
            )
            self.assertNotIn("projection_context", committed["resulting_project"])

            undone = invoke("history", {
                "action": "history",
                "source_hash": SOURCE_HASH,
                "kind": "undo",
                "base_project": committed["resulting_project"],
            })
            self.assertEqual(BEFORE, undone["scene_state"]["bounds"])
            self.assertEqual(
                baseline["baseline_project"],
                undone["resulting_project"],
            )
            self.assertEqual(
                context_hash,
                undone["projection_context_state"]["projection_context_hash"],
            )

            redone = invoke("history", {
                "action": "history",
                "source_hash": SOURCE_HASH,
                "kind": "redo",
                "base_project": undone["resulting_project"],
            })
            self.assertEqual(AFTER, redone["scene_state"]["bounds"])
            self.assertEqual(
                committed["resulting_project"],
                redone["resulting_project"],
            )
            self.assertEqual(
                context_hash,
                redone["projection_context_state"]["projection_context_hash"],
            )

            replayed = invoke("replay", {
                "action": "replay",
                "source_hash": SOURCE_HASH,
                "project": committed["resulting_project"],
            })
            self.assertEqual(
                committed["scene_state"]["scene_snapshot_id"],
                replayed["scene_state"]["scene_snapshot_id"],
            )
            self.assertEqual(
                context_hash,
                replayed["projection_context_state"]["projection_context_hash"],
            )
            self.assertNotIn("projection_context", replayed["replayed_project"])


if __name__ == "__main__":
    unittest.main()
