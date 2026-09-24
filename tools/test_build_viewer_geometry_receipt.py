#!/usr/bin/env python3
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from build_viewer_geometry_receipt import build_viewer_receipt

SOURCE_HASH = "a" * 64
SOURCE_LEN = 291840

FAKE_PRODUCER = r"""#!/usr/bin/env python3
import json
import sys

request = json.load(sys.stdin)
source_hash = request["source_hash"]
source_len = request["source_byte_len"]
receipt = {
    "schema_version": "0.1",
    "document": {
        "schema_version": "0.1",
        "source": {
            "format": "publisher",
            "format_version": "0x2c",
            "source_hash": source_hash,
            "byte_len": source_len,
        },
        "pages": [{
            "index": 1,
            "id": "10000000-0000-4000-8000-000000000001",
            "width_emu": 914400,
            "height_emu": 914400,
        }],
        "stories": [],
        "diagnostics": [],
    },
    "scene": {
        "environment": {
            "engine_revision": "viewer-geometry-v0.1",
            "font_set_fingerprint": "fonts:not-consumed:geometry-only",
            "resource_fingerprint": "resources:not-consumed:geometry-only",
        },
        "surfaces": [{
            "origin": "10000000-0000-4000-8000-000000000001",
            "size": {"width": 914400, "height": 914400},
            "bleed": None,
            "margins": None,
        }],
        "nodes": [],
        "origin_mapping": [],
        "diagnostics": [],
    },
    "paints": [],
    "story_frames": [],
    "images": [],
}
sys.stdout.write(json.dumps(receipt, separators=(",", ":")))
"""


class ViewerGeometryBuilderTests(unittest.TestCase):
    def make_producer(self, source):
        temp = tempfile.TemporaryDirectory()
        path = pathlib.Path(temp.name) / "producer.py"
        path.write_text(source, encoding="utf-8")
        return temp, path

    def test_builder_admits_exact_source_free_output_and_preserves_bytes(self):
        temp, producer = self.make_producer(FAKE_PRODUCER)
        try:
            receipt, raw = build_viewer_receipt(
                [sys.executable, str(producer)],
                source_hash=SOURCE_HASH,
                source_byte_len=SOURCE_LEN,
            )
        finally:
            temp.cleanup()

        self.assertEqual(SOURCE_HASH, receipt["document"]["source"]["source_hash"])
        self.assertEqual(SOURCE_LEN, receipt["document"]["source"]["byte_len"])
        self.assertEqual(b"{", raw[:1])
        self.assertNotIn(b"private", raw)

    def test_source_hash_mismatch_fails_closed(self):
        broken = FAKE_PRODUCER.replace(
            '"source_hash": source_hash,',
            '"source_hash": "f" * 64,',
            1,
        )
        temp, producer = self.make_producer(broken)
        try:
            with self.assertRaisesRegex(RuntimeError, "source_hash"):
                build_viewer_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    source_byte_len=SOURCE_LEN,
                )
        finally:
            temp.cleanup()

    def test_byte_len_mismatch_fails_closed(self):
        broken = FAKE_PRODUCER.replace(
            '"byte_len": source_len,',
            '"byte_len": source_len + 1,',
            1,
        )
        temp, producer = self.make_producer(broken)
        try:
            with self.assertRaisesRegex(RuntimeError, "byte_len"):
                build_viewer_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    source_byte_len=SOURCE_LEN,
                )
        finally:
            temp.cleanup()

    def test_private_field_is_rejected_by_public_allowlist(self):
        broken = FAKE_PRODUCER.replace(
            '"schema_version": "0.1",',
            '"schema_version": "0.1", "private_checkout_path": "/private/core",',
            1,
        )
        temp, producer = self.make_producer(broken)
        try:
            with self.assertRaises(AssertionError):
                build_viewer_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    source_byte_len=SOURCE_LEN,
                )
        finally:
            temp.cleanup()

    def test_stdout_logs_make_output_invalid_instead_of_being_stripped(self):
        broken = FAKE_PRODUCER + '\nprint("debug-log")\n'
        temp, producer = self.make_producer(broken)
        try:
            with self.assertRaisesRegex(RuntimeError, "exactly one JSON"):
                build_viewer_receipt(
                    [sys.executable, str(producer)],
                    source_hash=SOURCE_HASH,
                    source_byte_len=SOURCE_LEN,
                )
        finally:
            temp.cleanup()


if __name__ == "__main__":
    unittest.main()
