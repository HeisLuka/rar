#!/usr/bin/env python3
import hashlib
import json
import pathlib
import sys
import tempfile
import textwrap
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from run_local_editor_desktop_vertical import (
    DesktopVerticalError,
    run_local_desktop_vertical,
)
from validate_editor_desktop_vertical_receipt import validate_schema, validate_semantics

SOURCE_BYTES = b"desktop-vertical-fixture"
SOURCE_HASH = hashlib.sha256(SOURCE_BYTES).hexdigest()
RAR_COMMIT = "a" * 40
STORY = "10000000-0000-4000-8000-000000000001"
NODE = "20000000-0000-4000-8000-000000000001"
INSTANCE = "sha256:" + "3" * 64
STORY_BEFORE = "sha256:" + "1" * 64
STORY_AFTER = "sha256:" + "2" * 64
AFTER_STORY = "sha256:" + "4" * 64
AFTER_MOVE = "sha256:" + "5" * 64

FAKE_ENGINE = r"""#!/usr/bin/env python3
import json
import os
import pathlib
import sys
import zipfile

fixture = pathlib.Path(sys.argv[1])
project_path = pathlib.Path(sys.argv[2])
export_path = pathlib.Path(sys.argv[3])

source_hash = os.environ["CHAPTERA_SOURCE_HASH"]
rar_commit = os.environ["CHAPTERA_RAR_COMMIT"]
assert fixture.is_file()

story = "10000000-0000-4000-8000-000000000001"
node = "20000000-0000-4000-8000-000000000001"
before = {"x": 12700, "y": 25400, "width": 38100, "height": 50800}
after = {"x": 127000, "y": 254000, "width": 38100, "height": 50800}

project = {
    "schema_version": "pub-editor-v0.4",
    "source_hash": source_hash,
    "operations": [
        {
            "kind": "replace_story_range",
            "story_id": story,
            "scalar_start": 0,
            "scalar_end": 1,
            "replacement_text": "ChapteraV0",
            "before_story_state_id": "sha256:" + "1" * 64,
            "after_story_state_id": "sha256:" + "2" * 64
        },
        {
            "kind": "move_node",
            "node_id": node,
            "before": before,
            "after": after
        }
    ]
}
project_path.write_text(json.dumps(project, sort_keys=True), encoding="utf-8")

with zipfile.ZipFile(export_path, "w") as archive:
    archive.writestr("designmap.xml", "<Document/>")
    archive.writestr("Stories/Story_u1.xml", "<Story><Content>ChapteraV0</Content></Story>")
    archive.writestr(
        "Spreads/Spread_u1.xml",
        '<Spread><TextFrame Self="uf20000000000040008000000000000001">'
        '<PathPointType Anchor="10 20"/>'
        '<PathPointType Anchor="10 24"/>'
        '<PathPointType Anchor="13 24"/>'
        '<PathPointType Anchor="13 20"/>'
        '</TextFrame></Spread>'
    )

observation = {
    "protocol_version": "chaptera.editor-desktop-vertical-observation.v1",
    "source_hash": source_hash,
    "rar_commit": rar_commit,
    "story_edit": {
        "story_id": story,
        "operation_kind": "replace_story_range",
        "capability_admitted": True,
        "before_state_id": "sha256:" + "1" * 64,
        "after_state_id": "sha256:" + "2" * 64
    },
    "object_move": {
        "instance_id": "sha256:" + "3" * 64,
        "projection_kind": "direct_page_local",
        "origin_node_id": node,
        "capability_admitted": True,
        "geometry_sync_policy": "apply_authored_origin_geometry",
        "before": before,
        "after": after,
        "durable_move_count": 1,
        "transient_geometry_operation_count": 0
    },
    "history": {
        "after_story_state_id": "sha256:" + "4" * 64,
        "after_move_state_id": "sha256:" + "5" * 64,
        "undo_state_id": "sha256:" + "4" * 64,
        "redo_state_id": "sha256:" + "5" * 64,
        "reopened_state_id": "sha256:" + "5" * 64,
        "story_state_after_move": "sha256:" + "2" * 64,
        "story_state_after_undo": "sha256:" + "2" * 64,
        "story_state_after_redo": "sha256:" + "2" * 64,
        "story_state_reopened": "sha256:" + "2" * 64
    },
    "capability_loss": {
        "observed_before_export": True,
        "blocking_loss_count": 0,
        "approximations_explicit": True,
        "unsupported_partial_semantics_explicit": True
    },
    "export": {
        "format": "idml",
        "edited_story_present": True,
        "moved_geometry_present": True
    },
    "invariants": {
        "native_pub_write_used": False,
        "no_hidden_network_upload": True,
        "direct_page_local_gate_used": True,
        "projected_object_mutation_fails_closed": True,
        "reopen_used_fresh_session": True
    }
}
sys.stdout.write(json.dumps(observation, separators=(",", ":")))
"""


class DesktopVerticalRunnerTests(unittest.TestCase):
    def make_files(self, root, engine=FAKE_ENGINE):
        root = pathlib.Path(root)
        fixture = root / "SampleNewsletter.pub"
        fixture.write_bytes(SOURCE_BYTES)
        engine_path = root / "engine.py"
        engine_path.write_text(textwrap.dedent(engine), encoding="utf-8")
        project = root / "session.chaptera.json"
        export = root / "edited.idml"
        receipt = root / "desktop-vertical.real.json"
        command = [
            sys.executable,
            str(engine_path),
            "{fixture}",
            "{project}",
            "{export}",
        ]
        return fixture, project, export, receipt, command

    def test_real_boundary_verifies_project_export_and_history(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, project, export, receipt_path, command = self.make_files(tmp)
            receipt = run_local_desktop_vertical(
                fixture=fixture,
                project_output=project,
                export_output=export,
                receipt_output=receipt_path,
                command_template=command,
                expected_hash=SOURCE_HASH,
                expected_len=len(SOURCE_BYTES),
                rar_commit=RAR_COMMIT,
            )
            saved = json.loads(receipt_path.read_text(encoding="utf-8"))

        validate_schema(saved)
        summary = validate_semantics(saved)
        self.assertEqual(receipt, saved)
        self.assertEqual(summary["story_id"], STORY)
        self.assertEqual(summary["moved_node_id"], NODE)
        self.assertEqual(saved["object_move"]["instance_id"], INSTANCE)
        self.assertEqual(saved["history"]["undo_state_id"], AFTER_STORY)
        self.assertEqual(saved["history"]["redo_state_id"], AFTER_MOVE)
        self.assertEqual(saved["project"]["operation_count"], 2)
        self.assertEqual(saved["export"]["format"], "idml")
        serialized = json.dumps(saved)
        self.assertNotIn("replacement", serialized)
        self.assertNotIn("SampleNewsletter.pub", serialized)

    def test_project_move_must_match_observation(self):
        broken = FAKE_ENGINE.replace(
            '"node_id": node,\n            "before": before,',
            '"node_id": "20000000-0000-4000-8000-000000000002",\n            "before": before,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, project, export, receipt_path, command = self.make_files(tmp, broken)
            with self.assertRaisesRegex(DesktopVerticalError, "MoveNode identity mismatch"):
                run_local_desktop_vertical(
                    fixture=fixture,
                    project_output=project,
                    export_output=export,
                    receipt_output=receipt_path,
                    command_template=command,
                    expected_hash=SOURCE_HASH,
                    expected_len=len(SOURCE_BYTES),
                    rar_commit=RAR_COMMIT,
                )

    def test_source_mutation_fails_closed(self):
        broken = FAKE_ENGINE.replace(
            'assert fixture.is_file()',
            'assert fixture.is_file()\nfixture.write_bytes(fixture.read_bytes() + b"x")',
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, project, export, receipt_path, command = self.make_files(tmp, broken)
            with self.assertRaisesRegex(DesktopVerticalError, "source PUB changed"):
                run_local_desktop_vertical(
                    fixture=fixture,
                    project_output=project,
                    export_output=export,
                    receipt_output=receipt_path,
                    command_template=command,
                    expected_hash=SOURCE_HASH,
                    expected_len=len(SOURCE_BYTES),
                    rar_commit=RAR_COMMIT,
                )

    def test_projected_instance_cannot_be_claimed_as_move_target(self):
        broken = FAKE_ENGINE.replace(
            '"projection_kind": "direct_page_local"',
            '"projection_kind": "cmo_story_slot"',
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, project, export, receipt_path, command = self.make_files(tmp, broken)
            with self.assertRaisesRegex(DesktopVerticalError, "schema validation"):
                run_local_desktop_vertical(
                    fixture=fixture,
                    project_output=project,
                    export_output=export,
                    receipt_output=receipt_path,
                    command_template=command,
                    expected_hash=SOURCE_HASH,
                    expected_len=len(SOURCE_BYTES),
                    rar_commit=RAR_COMMIT,
                )

    def test_export_must_be_real_package(self):
        broken = FAKE_ENGINE.replace(
            'with zipfile.ZipFile(export_path, "w") as archive:',
            'export_path.write_bytes(b"not-a-zip")\nif False:\n    with zipfile.ZipFile(export_path, "w") as archive:',
            1,
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, project, export, receipt_path, command = self.make_files(tmp, broken)
            with self.assertRaisesRegex(DesktopVerticalError, "not a ZIP package"):
                run_local_desktop_vertical(
                    fixture=fixture,
                    project_output=project,
                    export_output=export,
                    receipt_output=receipt_path,
                    command_template=command,
                    expected_hash=SOURCE_HASH,
                    expected_len=len(SOURCE_BYTES),
                    rar_commit=RAR_COMMIT,
                )


if __name__ == "__main__":
    unittest.main()
