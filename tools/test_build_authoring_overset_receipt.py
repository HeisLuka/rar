#!/usr/bin/env python3
import hashlib
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_authoring_overset_receipt import build_receipt
from validate_authoring_overset_receipt import validate_schema, validate_semantics

SOURCE_HASH = "a" * 64
DOCUMENT_ID = "30000000-0000-4000-8000-000000000001"

FAKE_PRODUCER = r"""#!/usr/bin/env python3
import copy
import hashlib
import json
import sys

payload = json.load(sys.stdin)
source_hash = payload["source_hash"]
story_id = "10000000-0000-4000-8000-000000000001"
frame_id = "20000000-0000-4000-8000-000000000001"
env_hash = "sha256:" + "d" * 64
before_text = "short"
replacement_text = "this is deliberately long text that overflows the bounded one-frame fixture"
after_text = replacement_text

def digest(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()

def layout(text, state):
    return {
        "story_hash": "sha256:" + digest(text),
        "scalar_count": len(text),
        "state": state,
        "reason_code": None,
        "environment_authoritative": True,
        "layout_environment_hash": env_hash,
    }

baseline_project = {
    "schema_version": "pub-editor-v0.4",
    "source_hash": source_hash,
    "operations": [],
    "stories": {story_id: before_text},
}
operation = {
    "protocol_version": "chaptera.replace-story-range.v1",
    "kind": "replace_story_range",
    "story_id": story_id,
    "start_scalar": 0,
    "end_scalar": len(before_text),
    "expected_before": before_text,
    "replacement_text": replacement_text,
    "inverse": {
        "start_scalar": 0,
        "end_scalar": len(replacement_text),
        "expected_before": replacement_text,
        "replacement_text": before_text,
    },
    "before_text_hash": digest(before_text),
    "after_text_hash": digest(after_text),
    "before_story_state_id": "sha256:" + hashlib.sha256(json.dumps({"protocol_version":"chaptera.story-state.v1","story_id":story_id,"text":before_text}, sort_keys=True, separators=(",",":")).encode()).hexdigest(),
    "after_story_state_id": "sha256:" + hashlib.sha256(json.dumps({"protocol_version":"chaptera.story-state.v1","story_id":story_id,"text":after_text}, sort_keys=True, separators=(",",":")).encode()).hexdigest(),
}
accepted_project = copy.deepcopy(baseline_project)
accepted_project["operations"] = [copy.deepcopy(operation)]
accepted_project["stories"][story_id] = after_text

action = payload["action"]
if action == "baseline":
    json.dump({
        "source_hash": source_hash,
        "baseline_project": baseline_project,
        "story_id": story_id,
        "frame_node_id": frame_id,
        "baseline_layout_state": layout(before_text, "fits"),
        "edit_intent": {
            "start_scalar": 0,
            "end_scalar": len(before_text),
            "expected_before": before_text,
            "replacement_text": replacement_text,
        },
    }, sys.stdout)
    raise SystemExit(0)

if action == "commit":
    command = payload["command"]
    if command["story_id"] != story_id:
        raise SystemExit(3)
    json.dump({
        "canonical_operation": operation,
        "resulting_project": accepted_project,
        "consequences": [{"key": "story.text", "state": "supported", "note": None}],
        "accepted_layout_state": layout(after_text, "overset"),
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "history":
    kind = payload["kind"]
    if kind == "undo":
        project = baseline_project
        state = layout(before_text, "fits")
    elif kind == "redo":
        project = accepted_project
        state = layout(after_text, "overset")
    else:
        raise SystemExit(4)
    json.dump({
        "resulting_project": project,
        "layout_state": state,
        "consequences": [{"key": "history." + kind, "state": "supported", "note": None}],
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "replay":
    json.dump({
        "replayed_project": accepted_project,
        "layout_state": layout(after_text, "overset"),
        "editable_export_story_hash": digest(after_text),
        "fixed_output_outcome": "explicit_overset_loss",
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

if action == "layout_unknown_probe":
    json.dump({
        "layout_state": {
            "story_hash": "sha256:" + digest(after_text),
            "scalar_count": len(after_text),
            "state": "layout_unknown",
            "reason_code": "layout.environment_unavailable",
            "environment_authoritative": False,
            "layout_environment_hash": None,
        },
        "source_hash_after": source_hash,
    }, sys.stdout)
    raise SystemExit(0)

raise SystemExit(2)
"""


class AuthoringOversetProducerBuilderTests(unittest.TestCase):
    def test_builder_reuses_revision_kernel_and_emits_valid_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            producer = pathlib.Path(tmp) / "fake_overset_producer.py"
            producer.write_text(FAKE_PRODUCER, encoding="utf-8")
            receipt = build_receipt(
                [sys.executable, str(producer)],
                source_hash=SOURCE_HASH,
                document_id=DOCUMENT_ID,
                implementation="fake-authoring-layout",
                commit_or_build="test-build",
            )

        validate_schema(receipt)
        summary = validate_semantics(receipt)
        self.assertEqual("fits", receipt["states"]["baseline"]["state"])
        self.assertEqual("overset", receipt["states"]["accepted"]["state"])
        self.assertEqual(receipt["states"]["baseline"], receipt["states"]["undo"])
        self.assertEqual(receipt["states"]["accepted"], receipt["states"]["redo"])
        self.assertEqual(receipt["states"]["accepted"], receipt["states"]["replay"])
        self.assertEqual("layout_unknown", receipt["layout_unknown_probe"]["state"])
        self.assertTrue(summary["editable_export_preserves_story"])

    def test_builder_fails_closed_when_producer_changes_source_identity(self):
        broken = FAKE_PRODUCER.replace(
            '"source_hash_after": source_hash,',
            '"source_hash_after": "f" * 64,',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            producer = pathlib.Path(tmp) / "bad_overset_producer.py"
            producer.write_text(broken, encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "source identity"):
                build_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    document_id=DOCUMENT_ID,
                    implementation="fake-authoring-layout",
                    commit_or_build="test-build",
                )


if __name__ == "__main__":
    unittest.main()
