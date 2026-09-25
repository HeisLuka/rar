#!/usr/bin/env python3
import copy
import json
import pathlib
import sys
import tempfile
import textwrap
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
EDITOR_API = ROOT / "services" / "editor-api"
for path in (str(TOOLS), str(EDITOR_API)):
    if path not in sys.path:
        sys.path.insert(0, path)

from build_editor_fixed_pdf_receipt import (
    EditorFixedPdfReceiptError,
    build_receipt,
)
from story_range_v1 import replace_story_range_v1

SOURCE_HASH = "a" * 64
PAGE_ID = "20000000-0000-4000-8000-000000000001"
NODE_ID = "10000000-0000-4000-8000-000000000001"
STORY_ID = "30000000-0000-4000-8000-000000000001"
BEFORE = {"x": 1000, "y": 2000, "width": 3000, "height": 4000}
AFTER = {"x": 128000, "y": 256000, "width": 3000, "height": 4000}


def graph():
    return {
        "document": {"source_hash": SOURCE_HASH, "pages": [PAGE_ID]},
        "pages": {
            PAGE_ID: {
                "id": PAGE_ID,
                "size": {"width": 914400, "height": 1828800},
                "bleed": None,
                "margins": None,
                "children": [NODE_ID],
                "source_refs": [{"private": "not-renderer-input"}],
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
                    "source_refs": [{"carrier": "Escher"}],
                },
                "payload": {
                    "story_frame": {
                        "story_id": STORY_ID,
                        "ordinal": 0,
                        "previous_frame": None,
                        "next_frame": None,
                    }
                },
            }
        },
        "stories": {
            STORY_ID: {
                "id": STORY_ID,
                "text": "AB",
                "source_refs": [{"carrier": "Quill"}],
            }
        },
    }


def move_operation():
    return {
        "kind": "move_node",
        "node_id": NODE_ID,
        "before": dict(BEFORE),
        "after": dict(AFTER),
    }


def text_operation():
    return replace_story_range_v1(
        story_id=STORY_ID,
        story_text="AB",
        start_scalar=1,
        end_scalar=2,
        expected_before="B",
        replacement_text="C",
    ).operation


def glyph(glyph_id, cluster):
    return {
        "glyph_id": glyph_id,
        "cluster": cluster,
        "x_advance": 500,
        "y_advance": 0,
        "x_offset": 0,
        "y_offset": 0,
    }


def current_state_input():
    return {
        "schema_version": "chaptera.editor-fixed-output-input.v1",
        "resolved_graph": graph(),
        "editor_project": {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE_HASH,
            "operations": [move_operation(), text_operation()],
        },
        "projection_context": None,
        "shaped_flow": {
            "schema_version": "chaptera.shaped-flow-bridge-input.v1",
            "source_hash": SOURCE_HASH,
            "flow_id": "sha256:" + "d" * 64,
            "environment": {
                "font_size_emu": 1000,
                "line_height_emu": 1400,
            },
            "lines": [{
                "frame_node_id": NODE_ID,
                "story_id": STORY_ID,
                "frame_line_index": 0,
                "scalar_start": 0,
                "scalar_end": 2,
                "consumed_scalar_end": 2,
                "text": "AC",
                "units_per_em": 1000,
                "measured_width": 1000,
                "glyphs": [glyph(11, 0), glyph(12, 1)],
            }],
            "diagnostics": [],
        },
    }


STUB = r"""
import json
import os
import pathlib
import sys

mode = sys.argv[1]
request = json.load(sys.stdin)
packet = request["packet"]

node = packet["scene"]["nodes"][0]
run = packet["fixed_text_runs"][0]
if mode not in {"wrong-state"}:
    if node["bounds"]["x"] != 128000 or node["bounds"]["y"] != 256000:
        raise SystemExit("renderer did not receive edited geometry")
    if run["logical_text"] != "AC":
        raise SystemExit("renderer did not receive edited Story text")
    encoded = json.dumps(packet, sort_keys=True)
    if "source_refs" in encoded or "Quill" in encoded or "Escher" in encoded:
        raise SystemExit("renderer packet leaked source-private state")

output = pathlib.Path(os.environ["CHAPTERA_PDF_OUTPUT"])
if mode == "not-pdf":
    output.write_bytes(b"NOTPDF!!")
else:
    output.write_bytes(b"%PDF-1.7\n% chaptera current-state stub\n%%EOF\n")

result = {
    "protocol_version": "chaptera.editor-fixed-pdf-render-result.v1",
    "source_hash": request["source_hash"],
    "project_hash": request["project_hash"],
    "packet_id": request["packet_id"],
    "scene_snapshot_id": request["scene_snapshot_id"],
    "flow_id": request["flow_id"],
    "renderer_revision": "stub-pub-pdf-v0.1",
    "target_profile": "basic-fixed",
    "summary": {
        "page_count": 1,
        "node_painted": 1,
        "node_partial": 0,
        "node_unsupported": 0,
        "diagnostic_codes": [],
    },
}
if mode == "wrong-packet":
    result["packet_id"] = "sha256:" + "f" * 64
if mode == "extra-field":
    result["logical_text"] = "AC"
print(json.dumps(result, separators=(",", ":")))
"""


class EditorFixedPdfReceiptTests(unittest.TestCase):
    def renderer_command(self, root, mode="ok"):
        stub = pathlib.Path(root) / "renderer_stub.py"
        stub.write_text(textwrap.dedent(STUB), encoding="utf-8")
        return [sys.executable, str(stub), mode]

    def test_current_move_and_story_edit_reach_pdf_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            receipt = build_receipt(
                self.renderer_command(tmp),
                current_state_input=current_state_input(),
                implementation="rar-editor-fixed-pdf-producer-boundary-v1",
                commit_or_build="deadbeef",
            )
        self.assertEqual("pdf", receipt["artifact"]["format"])
        self.assertTrue(receipt["artifact"]["header"].startswith("%PDF-"))
        self.assertGreater(receipt["artifact"]["byte_len"], 8)
        self.assertEqual(1, receipt["fixed_run_count"])
        self.assertEqual(0, receipt["cmo_target_count"])
        self.assertEqual(0, receipt["cmo_visible_slot_count"])
        self.assertEqual(0, receipt["cmo_overset_story_count"])
        self.assertTrue(
            receipt["invariants"]["canonical_cmo_slot_flow_authoritative"]
        )
        self.assertEqual(0, receipt["invariants"]["source_reparse_after_edit_count"])
        self.assertFalse(receipt["invariants"]["renderer_received_source_bytes"])

    def test_receipt_is_public_safe(self):
        with tempfile.TemporaryDirectory() as tmp:
            receipt = build_receipt(
                self.renderer_command(tmp),
                current_state_input=current_state_input(),
                implementation="rar-editor-fixed-pdf-producer-boundary-v1",
                commit_or_build="deadbeef",
            )
        encoded = json.dumps(receipt, sort_keys=True)
        self.assertNotIn('"AC"', encoded)
        self.assertNotIn("logical_text", encoded)
        self.assertNotIn("expected_before", encoded)
        self.assertNotIn("replacement_text", encoded)
        self.assertNotIn("source_refs", encoded)
        self.assertNotIn("Quill", encoded)
        self.assertNotIn("Escher", encoded)

    def test_same_current_state_and_renderer_are_deterministic(self):
        with tempfile.TemporaryDirectory() as tmp:
            command = self.renderer_command(tmp)
            first = build_receipt(
                command,
                current_state_input=current_state_input(),
                implementation="rar-editor-fixed-pdf-producer-boundary-v1",
                commit_or_build="deadbeef",
            )
            second = build_receipt(
                command,
                current_state_input=copy.deepcopy(current_state_input()),
                implementation="rar-editor-fixed-pdf-producer-boundary-v1",
                commit_or_build="deadbeef",
            )
        self.assertEqual(first["packet_id"], second["packet_id"])
        self.assertEqual(first["artifact"]["sha256"], second["artifact"]["sha256"])

    def test_renderer_cannot_claim_different_packet(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(EditorFixedPdfReceiptError, "packet_id"):
                build_receipt(
                    self.renderer_command(tmp, "wrong-packet"),
                    current_state_input=current_state_input(),
                    implementation="rar-editor-fixed-pdf-producer-boundary-v1",
                    commit_or_build="deadbeef",
                )

    def test_non_pdf_artifact_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(EditorFixedPdfReceiptError, "not a PDF"):
                build_receipt(
                    self.renderer_command(tmp, "not-pdf"),
                    current_state_input=current_state_input(),
                    implementation="rar-editor-fixed-pdf-producer-boundary-v1",
                    commit_or_build="deadbeef",
                )

    def test_renderer_result_extra_fields_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(EditorFixedPdfReceiptError, "fields mismatch"):
                build_receipt(
                    self.renderer_command(tmp, "extra-field"),
                    current_state_input=current_state_input(),
                    implementation="rar-editor-fixed-pdf-producer-boundary-v1",
                    commit_or_build="deadbeef",
                )


if __name__ == "__main__":
    unittest.main()
