#!/usr/bin/env python3
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_text_frame_columns_runtime_receipt import build_receipt

SOURCE_HASH = "a" * 64
DOCUMENT_ID = "40000000-0000-4000-8000-000000000001"
BINARY_SHA = "b" * 64

FAKE_PRODUCER = r"""#!/usr/bin/env python3
import copy
import hashlib
import json
import sys

payload = json.load(sys.stdin)
source_hash = payload["source_hash"]
frame_id = "frame:1"
story_id = "story:1"
env = "c" * 64

before = {"column_count": 1, "gutter_emu": 0}
count_after = {"column_count": 3, "gutter_emu": 0}
gutter_after = {"column_count": 3, "gutter_emu": 91440}

def project(columns, operations):
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": source_hash,
        "operations": copy.deepcopy(operations),
        "stories": {story_id: "full story text"},
        "text_frames": {
            frame_id: {
                "story_id": story_id,
                "bounds": {"x": 0, "y": 0, "width": 3000000, "height": 1800000},
                "columns": copy.deepcopy(columns),
                "supported": True,
            }
        },
    }

def operation(before_state, after_state):
    return {
        "kind": "set_text_frame_columns",
        "node_id": frame_id,
        "before": copy.deepcopy(before_state),
        "after": copy.deepcopy(after_state),
    }

baseline_project = project(before, [])
count_op = operation(before, count_after)
count_project = project(count_after, [count_op])
gutter_op = operation(count_after, gutter_after)
gutter_project = project(gutter_after, [count_op, gutter_op])

def runtime_state(columns):
    count = columns["column_count"]
    gutter = columns["gutter_emu"]
    usable = 3000000 - max(0, count - 1) * gutter
    token = f"{count}:{gutter}:{usable}"
    return {
        "column_count": count,
        "gutter_emu": gutter,
        "sample_band_slot_count": count,
        "sample_band_total_usable_width_emu": usable,
        "line_region_partition_sha256": hashlib.sha256(token.encode()).hexdigest(),
        "overflow_state": "fits" if usable >= 2600000 else "overset",
        "overflow_rederived": True,
        "environment_authoritative": True,
        "layout_environment_sha256": env,
    }

action = payload["action"]

if action == "baseline":
    json.dump({
        "source_hash": source_hash,
        "fixture_kind": "author_created_one_frame",
        "baseline_project": baseline_project,
        "frame_node_id": frame_id,
        "before": before,
        "count_after": count_after,
        "gutter_after": gutter_after,
        "target_gate": {
            "ordinary_one_frame": True,
            "unlinked": True,
            "autofit_off": True,
            "vertical_text_off": True,
            "wrap_obstacles_absent": True,
        },
    }, sys.stdout)
    raise SystemExit(0)

if action == "runtime":
    columns = payload["project"]["text_frames"][frame_id]["columns"]
    json.dump({
        "runtime_state": runtime_state(columns),
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "commit":
    command = payload["command"]
    if command["expected_before"] == before and command["after"] == count_after:
        op = count_op
        result = count_project
    elif command["expected_before"] == count_after and command["after"] == gutter_after:
        op = gutter_op
        result = gutter_project
    else:
        raise SystemExit(3)
    json.dump({
        "canonical_operation": op,
        "resulting_project": result,
        "consequences": [
            {"key": "text_frame.columns", "state": "supported", "note": None},
            {"key": "layout.reflow", "state": "invalidated", "note": None},
            {"key": "story.overset", "state": "invalidated", "note": None},
        ],
        "mutation_invariants": {
            "story_identity_preserved": True,
            "story_text_preserved": True,
            "outer_bounds_preserved": True,
            "source_bytes_written": False,
        },
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "history":
    if payload["kind"] == "undo":
        result = payload["count_project"]
    elif payload["kind"] == "redo":
        result = payload["gutter_project"]
    else:
        raise SystemExit(4)
    json.dump({
        "resulting_project": result,
        "consequences": [{"key": "history." + payload["kind"], "state": "supported", "note": None}],
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "replay":
    json.dump({
        "replayed_project": payload["project"],
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

raise SystemExit(2)
"""


class TextFrameColumnsRuntimeBuilderTests(unittest.TestCase):
    def make_producer(self, root, content=FAKE_PRODUCER):
        path = pathlib.Path(root) / "fake_columns_runtime.py"
        path.write_text(content, encoding="utf-8")
        return path

    def build(self, producer):
        return build_receipt(
            [sys.executable, str(producer)],
            source_hash=SOURCE_HASH,
            document_id=DOCUMENT_ID,
            chaptera_version="0.1.0-test",
            platform="windows",
            binary_sha256=BINARY_SHA,
        )

    def test_builder_proves_count_gutter_history_and_replay_consumption(self):
        with tempfile.TemporaryDirectory() as tmp:
            receipt = self.build(self.make_producer(tmp))

        self.assertTrue(receipt["count_arm"]["line_region_partition_changed"])
        self.assertEqual(1, receipt["count_arm"]["slot_count_before"])
        self.assertEqual(3, receipt["count_arm"]["slot_count_after"])
        self.assertTrue(receipt["gutter_arm"]["slot_count_held_constant"])
        self.assertLess(
            receipt["gutter_arm"]["usable_width_after_emu"],
            receipt["gutter_arm"]["usable_width_before_emu"],
        )
        self.assertTrue(receipt["gutter_arm"]["line_region_partition_changed"])
        self.assertTrue(receipt["history_replay"]["undo_restores_count_runtime"])
        self.assertTrue(receipt["history_replay"]["redo_restores_final_runtime"])
        self.assertTrue(receipt["history_replay"]["fresh_replay_restores_final_runtime"])
        self.assertNotIn("source_hash", receipt)
        self.assertNotIn("frame_node_id", receipt)

    def test_builder_rejects_runtime_that_ignores_column_count(self):
        broken = FAKE_PRODUCER.replace(
            '"sample_band_slot_count": count,',
            '"sample_band_slot_count": 1,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "slot count does not match"):
                self.build(self.make_producer(tmp, broken))

    def test_builder_rejects_runtime_that_ignores_gutter(self):
        broken = FAKE_PRODUCER.replace(
            'usable = 3000000 - max(0, count - 1) * gutter',
            'usable = 3000000',
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "increased gutter did not reduce"):
                self.build(self.make_producer(tmp, broken))

    def test_builder_rejects_non_authoritative_environment(self):
        broken = FAKE_PRODUCER.replace(
            '"environment_authoritative": True,',
            '"environment_authoritative": False,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "authoritative layout environment"):
                self.build(self.make_producer(tmp, broken))

    def test_builder_rejects_source_mutation(self):
        broken = FAKE_PRODUCER.replace(
            '"source_hash_after": source_hash,',
            '"source_hash_after": "f" * 64,',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(RuntimeError, "source identity"):
                self.build(self.make_producer(tmp, broken))


if __name__ == "__main__":
    unittest.main()
