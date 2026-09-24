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

from run_local_viewer_geometry_cli import LocalViewerProducerError, run_local_viewer

FAKE_ENGINE = r"""#!/usr/bin/env python3
import hashlib
import json
import pathlib
import sys

if len(sys.argv) != 4 or sys.argv[1] != "view-scene" or sys.argv[3] != "--json":
    raise SystemExit(9)
path = pathlib.Path(sys.argv[2])
raw = path.read_bytes()
source_hash = hashlib.sha256(raw).hexdigest()
receipt = {
    "schema_version": "0.1",
    "document": {
        "schema_version": "0.1",
        "source": {
            "format": "publisher",
            "format_version": "0x2c",
            "source_hash": source_hash,
            "byte_len": len(raw),
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
}
sys.stdout.write(json.dumps(receipt, separators=(",", ":")))
"""


class LocalViewerGeometryCliTests(unittest.TestCase):
    def make_fixture_and_engine(self, root, engine=FAKE_ENGINE):
        root = pathlib.Path(root)
        fixture = root / "SampleNewsletter.pub"
        fixture.write_bytes(b"real-pub-placeholder-for-launcher-test")
        engine_path = root / "viewer_engine.py"
        engine_path.write_text(engine, encoding="utf-8")
        source_hash = hashlib.sha256(fixture.read_bytes()).hexdigest()
        request = json.dumps({
            "action": "viewer_geometry",
            "source_hash": source_hash,
            "source_byte_len": fixture.stat().st_size,
        })
        command = [
            sys.executable,
            str(engine_path),
            "view-scene",
            "{fixture}",
            "--json",
        ]
        return fixture, request, command

    def test_exact_engine_stdout_passes_through(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, request, command = self.make_fixture_and_engine(tmp)
            raw = run_local_viewer(
                fixture=fixture,
                command_template=command,
                request_raw=request,
            )
        receipt = json.loads(raw)
        self.assertEqual(
            hashlib.sha256(b"real-pub-placeholder-for-launcher-test").hexdigest(),
            receipt["document"]["source"]["source_hash"],
        )
        self.assertNotIn(b"SampleNewsletter.pub", raw)

    def test_fixture_hash_mismatch_fails_before_engine(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, request, command = self.make_fixture_and_engine(tmp)
            bad = json.loads(request)
            bad["source_hash"] = "f" * 64
            with self.assertRaisesRegex(LocalViewerProducerError, "fixture SHA-256 mismatch"):
                run_local_viewer(
                    fixture=fixture,
                    command_template=command,
                    request_raw=json.dumps(bad),
                )

    def test_fixture_length_mismatch_fails_before_engine(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, request, command = self.make_fixture_and_engine(tmp)
            bad = json.loads(request)
            bad["source_byte_len"] += 1
            with self.assertRaisesRegex(LocalViewerProducerError, "fixture byte length mismatch"):
                run_local_viewer(
                    fixture=fixture,
                    command_template=command,
                    request_raw=json.dumps(bad),
                )

    def test_command_requires_exactly_one_fixture_placeholder(self):
        with tempfile.TemporaryDirectory() as tmp:
            fixture, request, command = self.make_fixture_and_engine(tmp)
            with self.assertRaisesRegex(LocalViewerProducerError, "exactly once"):
                run_local_viewer(
                    fixture=fixture,
                    command_template=command[:-2] + [str(fixture), "--json"],
                    request_raw=request,
                )

    def test_engine_source_identity_mismatch_fails(self):
        broken = FAKE_ENGINE.replace(
            '"source_hash": source_hash,',
            '"source_hash": "f" * 64,',
        )
        with tempfile.TemporaryDirectory() as tmp:
            fixture, request, command = self.make_fixture_and_engine(tmp, broken)
            with self.assertRaisesRegex(LocalViewerProducerError, "source_hash differs"):
                run_local_viewer(
                    fixture=fixture,
                    command_template=command,
                    request_raw=request,
                )

    def test_engine_stdout_logs_fail_closed(self):
        broken = FAKE_ENGINE + '\nprint("debug-log")\n'
        with tempfile.TemporaryDirectory() as tmp:
            fixture, request, command = self.make_fixture_and_engine(tmp, broken)
            with self.assertRaisesRegex(LocalViewerProducerError, "exactly one JSON"):
                run_local_viewer(
                    fixture=fixture,
                    command_template=command,
                    request_raw=request,
                )


if __name__ == "__main__":
    unittest.main()
