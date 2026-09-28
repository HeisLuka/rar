#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import threading
import unittest
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = pathlib.Path(__file__).resolve().parents[3]
MODULE_PATH = ROOT / "tools" / "research-runner" / "acquire_public.py"


def load_module():
    spec = importlib.util.spec_from_file_location("hosted_research_acquire", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class Handler(BaseHTTPRequestHandler):
    payload = b"chaptera-hosted-research\n"

    def do_GET(self):
        if self.path == "/ok":
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(self.payload)))
            self.end_headers()
            self.wfile.write(self.payload)
            return
        if self.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", "/ok")
            self.end_headers()
            return
        if self.path == "/large":
            body = b"x" * 64
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, fmt, *args):
        return


class AcquisitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.m = load_module()
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = f"http://127.0.0.1:{cls.server.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def test_private_destination_rejected_without_test_override(self):
        with self.assertRaises(ValueError):
            self.m.validate_public_url("https://127.0.0.1/private")

    def test_redirect_hash_and_receipt(self):
        with tempfile.TemporaryDirectory() as td:
            root = pathlib.Path(td)
            manifest = {
                "manifest_version": self.m.MANIFEST_VERSION,
                "task_key": "HOSTED-RESEARCH-RUNNER-01",
                "pub_task_id": "PUB-HOSTED-RESEARCH-RUNNER-01",
                "run_id": "PUB-RUN-1045",
                "scope": "synthetic localhost self-test only",
                "sources": [
                    {
                        "id": "redirect",
                        "kind": "https",
                        "url": self.base + "/redirect",
                        "max_bytes": 1024,
                    }
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            out = root / "out"
            rc = self.m.run_manifest(manifest_path, out, allow_local_http=True)
            self.assertEqual(rc, 0)
            receipt = json.loads((out / "acquisition-receipt.json").read_text())
            record = receipt["sources"][0]
            self.assertEqual(record["result"], "ok")
            self.assertEqual(record["sha256"], hashlib.sha256(Handler.payload).hexdigest())
            self.assertTrue(record["final_url"].endswith("/ok"))
            self.assertEqual(len(record["redirect_chain"]), 1)
            canonical = json.loads((out / "pub-research-receipt.json").read_text())
            self.assertEqual(canonical["receipt_version"], "chaptera.pub-research-receipt.v1")
            self.assertEqual(canonical["counts"]["supporting"], 1)

    def test_required_oversize_is_fail_closed_but_receipt_survives(self):
        with tempfile.TemporaryDirectory() as td:
            root = pathlib.Path(td)
            manifest = {
                "manifest_version": self.m.MANIFEST_VERSION,
                "task_key": "HOSTED-RESEARCH-RUNNER-01",
                "pub_task_id": "PUB-HOSTED-RESEARCH-RUNNER-01",
                "run_id": "PUB-RUN-1045",
                "scope": "oversize self-test",
                "sources": [
                    {
                        "id": "oversize",
                        "kind": "https",
                        "url": self.base + "/large",
                        "max_bytes": 8,
                    }
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            out = root / "out"
            rc = self.m.run_manifest(manifest_path, out, allow_local_http=True)
            self.assertEqual(rc, 2)
            receipt = json.loads((out / "acquisition-receipt.json").read_text())
            self.assertEqual(receipt["sources"][0]["result"], "error")
            self.assertIn("max_bytes", receipt["sources"][0]["error"])


    def test_generic_task_key_does_not_require_pub_receipt(self):
        with tempfile.TemporaryDirectory() as td:
            root = pathlib.Path(td)
            manifest = {
                "manifest_version": self.m.MANIFEST_VERSION,
                "task_key": "KENNETH-YOUNG-OPL",
                "run_id": "PUB-RUN-1045",
                "scope": "generic hosted research owner without PUB experiment identity",
                "sources": [
                    {
                        "id": "ok",
                        "kind": "https",
                        "url": self.base + "/ok",
                        "max_bytes": 1024,
                    }
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            out = root / "out"
            rc = self.m.run_manifest(manifest_path, out, allow_local_http=True)
            self.assertEqual(rc, 0)
            self.assertTrue((out / "acquisition-receipt.json").exists())
            self.assertFalse((out / "pub-research-receipt.json").exists())
            receipt = json.loads((out / "acquisition-receipt.json").read_text())
            self.assertEqual(receipt["task_key"], "KENNETH-YOUNG-OPL")
            self.assertIsNone(receipt["pub_task_id"])

    def test_wayback_query_is_bounded(self):
        url = self.m.build_cdx_url(
            {
                "kind": "wayback_cdx",
                "id": "cdx",
                "target_url": "example.com/*",
                "limit": 7,
                "from": "2007",
                "to": "2010",
            }
        )
        self.assertIn("web.archive.org/cdx/search/cdx", url)
        self.assertIn("limit=7", url)
        self.assertIn("from=2007", url)
        self.assertIn("to=2010", url)

    def test_zip_inspection_rejects_traversal(self):
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, "w") as archive:
            archive.writestr("../escape.txt", b"no")
        with self.assertRaises(ValueError):
            self.m.inspect_zip(stream.getvalue(), max_members=10, max_uncompressed_bytes=1024)

    def test_retain_body_requires_explicit_disclosure_safe(self):
        source = {
            "id": "body",
            "kind": "https",
            "url": self.base + "/ok",
            "max_bytes": 1024,
            "retain_body": True,
        }
        result = self.m.acquire_source(source, allow_local_http=True)
        self.assertEqual(result["result"], "error")
        self.assertIn("disclosure_safe", result["error"])


if __name__ == "__main__":
    unittest.main()
