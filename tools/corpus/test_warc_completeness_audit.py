#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


harvest = load_module("harvest_pub", HERE / "harvest_pub.py")
audit = load_module("warc_completeness_audit", HERE / "warc_completeness_audit.py")


class WarcCompletenessAuditTests(unittest.TestCase):
    def row(self, sha: str = "") -> dict[str, str]:
        return {
            "sha256": sha,
            "candidate_filename": "sample.pub",
            "direct_url": "https://example.org/sample.pub",
            "cc_crawl": "CC-MAIN-2021-49",
            "cc_warc_filename": "crawl-data/x/warc/file.warc.gz",
            "cc_warc_offset": "10",
            "cc_warc_length": "100",
            "classification": "cfb_publisher_hint",
        }

    def test_complete_capture_requires_sha_match(self) -> None:
        data = b"publisher-bytes"
        import hashlib
        row = self.row(hashlib.sha256(data).hexdigest())
        with patch.object(
            harvest,
            "common_crawl_fetch",
            return_value=(data, {"cc_warc_truncated": ""}),
        ):
            result = audit.audit_row(row, timeout=1.0, max_bytes=1024)
        self.assertEqual(result["status"], "complete")

    def test_truncated_capture_is_partial(self) -> None:
        data = b"publisher-bytes"
        import hashlib
        row = self.row(hashlib.sha256(data).hexdigest())
        with patch.object(
            harvest,
            "common_crawl_fetch",
            return_value=(data, {"cc_warc_truncated": "length"}),
        ):
            result = audit.audit_row(row, timeout=1.0, max_bytes=1024)
        self.assertEqual(result["status"], "partial")
        self.assertEqual(result["warc_truncated"], "length")

    def test_sha_mismatch_fails_closed(self) -> None:
        row = self.row("0" * 64)
        with patch.object(
            harvest,
            "common_crawl_fetch",
            return_value=(b"different", {"cc_warc_truncated": ""}),
        ):
            result = audit.audit_row(row, timeout=1.0, max_bytes=1024)
        self.assertEqual(result["status"], "sha_mismatch")

    def test_legacy_arc_matching_http_length_is_complete(self) -> None:
        data = b"publisher-bytes"
        import hashlib
        row = self.row(hashlib.sha256(data).hexdigest())
        row["cc_warc_filename"] = "crawl-data/x/file.arc.gz"
        with patch.object(
            harvest,
            "common_crawl_fetch",
            return_value=(
                data,
                {
                    "cc_archive_format": "arc",
                    "content_length_header": str(len(data)),
                    "cc_warc_truncated": "",
                },
            ),
        ):
            result = audit.audit_row(row, timeout=1.0, max_bytes=1024)
        self.assertEqual(result["status"], "complete")

    def test_legacy_arc_length_mismatch_is_partial(self) -> None:
        data = b"publisher-bytes"
        import hashlib
        row = self.row(hashlib.sha256(data).hexdigest())
        row["cc_warc_filename"] = "crawl-data/x/file.arc.gz"
        with patch.object(
            harvest,
            "common_crawl_fetch",
            return_value=(
                data,
                {
                    "cc_archive_format": "arc",
                    "content_length_header": str(len(data) + 10),
                    "cc_warc_truncated": "",
                },
            ),
        ):
            result = audit.audit_row(row, timeout=1.0, max_bytes=1024)
        self.assertEqual(result["status"], "partial")
        self.assertIn("Content-Length mismatch", result["error"])

    def test_sha_aggregate_promotes_if_any_capture_is_complete(self) -> None:
        rows = [
            {"sha256": "a", "status": "partial", "candidate_filename": "a.pub"},
            {"sha256": "a", "status": "complete", "candidate_filename": "a.pub"},
        ]
        result = audit.aggregate_by_sha(rows)
        self.assertEqual(result[0]["status"], "complete")
        self.assertEqual(result[0]["capture_count"], 2)


if __name__ == "__main__":
    unittest.main()
