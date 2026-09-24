#!/usr/bin/env python3
import hashlib
import json
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_layout_resolved_scene_receipt import build_receipt
from run_local_layout_resolved_scene_engine import (
    LocalResolvedSceneError,
    run_local_resolved_scene,
)

DOCUMENT_ID = "30000000-0000-4000-8000-000000000001"

FAKE_ENGINE = r"""#!/usr/bin/env python3
import copy
import hashlib
import json
import pathlib
import sys

action = sys.argv[1]
if sys.argv[2] != "--state-dir":
    raise SystemExit(9)
state_dir = pathlib.Path(sys.argv[3])
state_dir.mkdir(parents=True, exist_ok=True)
fixture = None
if len(sys.argv) == 6:
    if sys.argv[4] != "--fixture":
        raise SystemExit(10)
    fixture = pathlib.Path(sys.argv[5])
elif len(sys.argv) != 4:
    raise SystemExit(11)

payload = json.load(sys.stdin)
if payload["action"] != action:
    raise SystemExit(12)

with (state_dir / "calls.jsonl").open("a", encoding="utf-8") as handle:
    handle.write(json.dumps({"action": action, "argv": sys.argv[1:]}) + "\n")

session_path = state_dir / "engine-session.json"
node_id = "10000000-0000-4000-8000-000000000001"
page_id = "20000000-0000-4000-8000-000000000001"
before = {"x": 1000, "y": 2000, "width": 3000, "height": 4000}
after = {"x": 128000, "y": 256000, "width": 3000, "height": 4000}
base_snap = "sha256:" + "b" * 64
move_snap = "sha256:" + "c" * 64
surface_hash = "sha256:" + "d" * 64
origin_hash = "sha256:" + "e" * 64
geometry_hash = "sha256:" + "f" * 64

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

source_hash = payload["source_hash"]
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

if action == "baseline":
    if fixture is None:
        raise SystemExit(20)
    raw = fixture.read_bytes()
    if hashlib.sha256(raw).hexdigest() != source_hash:
        raise SystemExit(21)
    session_path.write_text(
        json.dumps({"source_hash": source_hash, "bootstrapped": True}),
        encoding="utf-8",
    )
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
        "adapter_invariants": {
            "viewer_private_mapping_used": False,
            "browser_layout_authoritative": False,
            "second_geometry_model_created": False,
            "context_extension_seam_present": True,
            "graph_only_wrapper_is_empty_context": True,
        },
    }, sys.stdout)
    raise SystemExit(0)

if fixture is not None:
    raise SystemExit(30)
if not session_path.is_file():
    raise SystemExit(31)
session = json.loads(session_path.read_text(encoding="utf-8"))
if session["source_hash"] != source_hash:
    raise SystemExit(32)

if action == "commit":
    json.dump({
        "canonical_operation": operation,
        "resulting_project": accepted_project,
        "consequences": [{"key": "node.geometry.position", "state": "supported", "note": None}],
        "scene_state": scene(move_snap, after),
        "source_hash_after": source_hash,
        "source_reparse_after_edit_count": 0,
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
        raise SystemExit(40)
    json.dump({
        "resulting_project": project,
        "scene_state": state,
        "consequences": [{"key": "history." + payload["kind"], "state": "supported", "note": None}],
        "source_hash_after": source_hash,
        "source_reparse_after_edit_count": 0,
    }, sys.stdout)
    raise SystemExit(0)

if action == "replay":
    json.dump({
        "replayed_project": accepted_project,
        "scene_state": scene(move_snap, after),
        "source_hash_after": source_hash,
        "source_reparse_after_edit_count": 0,
    }, sys.stdout)
    raise SystemExit(0)

raise SystemExit(50)
"""


class LocalResolvedSceneEngineTests(unittest.TestCase):
    def make_runtime(self, root):
        root = pathlib.Path(root)
        fixture = root / "SampleNewsletter.pub"
        fixture.write_bytes(b"pinned-pub-bytes-for-local-scene-launcher-test")
        engine = root / "fake_engine.py"
        engine.write_text(FAKE_ENGINE, encoding="utf-8")
        state_dir = root / "state"
        source_hash = hashlib.sha256(fixture.read_bytes()).hexdigest()
        producer = [
            sys.executable,
            str(TOOLS / "run_local_layout_resolved_scene_engine.py"),
            "--fixture",
            str(fixture),
            "--source-byte-len",
            str(fixture.stat().st_size),
            "--state-dir",
            str(state_dir),
            "--",
            sys.executable,
            str(engine),
        ]
        return fixture, engine, state_dir, source_hash, producer

    def test_full_builder_composition_passes_fixture_only_on_baseline(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, _, state_dir, source_hash, producer = self.make_runtime(tmp)
            receipt = build_receipt(
                producer,
                source_hash=source_hash,
                document_id=DOCUMENT_ID,
                implementation="fake-local-current-graph-engine",
                commit_or_build="test-build",
            )
            calls = [
                json.loads(line)
                for line in (state_dir / "calls.jsonl").read_text(encoding="utf-8").splitlines()
            ]

        self.assertEqual(
            ["baseline", "commit", "history", "history", "replay"],
            [item["action"] for item in calls],
        )
        fixture_text = str(fixture.resolve())
        self.assertIn(fixture_text, calls[0]["argv"])
        for item in calls[1:]:
            self.assertNotIn("--fixture", item["argv"])
            self.assertNotIn(fixture_text, item["argv"])
        self.assertEqual(receipt["states"]["baseline"], receipt["states"]["undo"])
        self.assertEqual(receipt["states"]["accepted"], receipt["states"]["redo"])
        self.assertEqual(receipt["states"]["accepted"], receipt["states"]["replay"])
        self.assertEqual(0, receipt["invariants"]["source_reparse_after_edit_count"])

    def test_post_baseline_action_fails_without_bootstrapped_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, engine, state_dir, source_hash, _ = self.make_runtime(tmp)
            request = json.dumps({
                "action": "commit",
                "source_hash": source_hash,
                "base_project": {},
                "command": {},
            })
            with self.assertRaisesRegex(LocalResolvedSceneError, "not bootstrapped"):
                run_local_resolved_scene(
                    fixture=fixture,
                    source_byte_len=fixture.stat().st_size,
                    state_dir=state_dir,
                    engine_prefix=[sys.executable, str(engine)],
                    request_raw=request,
                )

    def test_baseline_fails_closed_on_wrong_fixture_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, engine, state_dir, _, _ = self.make_runtime(tmp)
            request = json.dumps({
                "action": "baseline",
                "source_hash": "f" * 64,
            })
            with self.assertRaisesRegex(LocalResolvedSceneError, "fixture SHA-256 mismatch"):
                run_local_resolved_scene(
                    fixture=fixture,
                    source_byte_len=fixture.stat().st_size,
                    state_dir=state_dir,
                    engine_prefix=[sys.executable, str(engine)],
                    request_raw=request,
                )

    def test_post_edit_reparse_claim_fails_closed(self):
        broken = FAKE_ENGINE.replace(
            '"source_reparse_after_edit_count": 0,',
            '"source_reparse_after_edit_count": 1,',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, engine, state_dir, source_hash, _ = self.make_runtime(tmp)
            engine.write_text(broken, encoding="utf-8")
            baseline_request = json.dumps({
                "action": "baseline",
                "source_hash": source_hash,
            })
            run_local_resolved_scene(
                fixture=fixture,
                source_byte_len=fixture.stat().st_size,
                state_dir=state_dir,
                engine_prefix=[sys.executable, str(engine)],
                request_raw=baseline_request,
            )
            commit_request = json.dumps({
                "action": "commit",
                "source_hash": source_hash,
                "base_project": {},
                "command": {},
            })
            with self.assertRaisesRegex(LocalResolvedSceneError, "reparse"):
                run_local_resolved_scene(
                    fixture=fixture,
                    source_byte_len=fixture.stat().st_size,
                    state_dir=state_dir,
                    engine_prefix=[sys.executable, str(engine)],
                    request_raw=commit_request,
                )


if __name__ == "__main__":
    unittest.main()
