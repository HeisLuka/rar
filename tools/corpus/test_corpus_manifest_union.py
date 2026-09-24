#!/usr/bin/env python3
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("corpus_manifest_union.py")


def row(
    name: str,
    sha: str,
    bucket: str = "natural_wild",
    size: int = 100,
) -> dict[str, object]:
    return {
        "candidate_filename": name,
        "source_page": f"https://example.test/{name}",
        "fetch_status": "ok",
        "classification": "cfb_publisher_hint",
        "bucket": bucket,
        "sha256": sha,
        "size_bytes": size,
        "harvested_at_utc": "2026-09-21T00:00:00+00:00",
    }


class CorpusManifestUnionTests(unittest.TestCase):
    def run_union(
        self,
        manifests: list[tuple[str, list[dict[str, object]]]],
        latest: str,
        audit_rows: list[dict[str, object]] | None = None,
    ) -> dict[str, object]:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            specs: list[str] = []
            for index, (label, rows) in enumerate(manifests):
                path = root / f"m{index}.json"
                path.write_text(json.dumps(rows), encoding="utf-8")
                specs.append(f"{label}={path}")
            cmd = [sys.executable, str(SCRIPT), "--latest-label", latest]
            if audit_rows is not None:
                audit_path = root / "audit.json"
                audit_path.write_text(json.dumps(audit_rows), encoding="utf-8")
                cmd.extend(["--completeness-audit", str(audit_path)])
            completed = subprocess.run(
                [*cmd, *specs],
                check=True,
                capture_output=True,
                text=True,
            )
            return json.loads(completed.stdout)

    def test_union_separates_latest_from_cumulative(self) -> None:
        a = "a" * 64
        b = "b" * 64
        q = "c" * 64
        result = self.run_union(
            [
                (
                    "old",
                    [
                        row("a.pub", a),
                        row("b.pub", b),
                        row("q.pub", q, "quarantine_active_content"),
                    ],
                ),
                (
                    "latest",
                    [
                        row("a.pub", a),
                        row("q.pub", q, "quarantine_active_content"),
                    ],
                ),
            ],
            latest="latest",
        )
        self.assertEqual(result["cumulative"]["publisher_sha256"], 3)
        self.assertEqual(result["cumulative"]["natural_sha256"], 2)
        self.assertEqual(result["cumulative"]["quarantine_sha256"], 1)
        self.assertEqual(result["latest"]["publisher_sha256"], 2)
        self.assertEqual(result["known_but_absent_latest_count"], 1)
        self.assertEqual(result["known_but_absent_latest"][0]["sha256"], b)

    def test_repeated_label_merges_shards_and_dedups_sha(self) -> None:
        a = "a" * 64
        b = "b" * 64
        result = self.run_union(
            [
                ("sharded", [row("a.pub", a)]),
                ("sharded", [row("same-a.pub", a), row("b.pub", b)]),
            ],
            latest="sharded",
        )
        self.assertEqual(len(result["snapshots"]), 1)
        self.assertEqual(result["snapshots"][0]["publisher_sha256"], 2)
        self.assertEqual(result["cumulative"]["publisher_sha256"], 2)

    def test_truncated_classification_is_excluded(self) -> None:
        a = "a" * 64
        partial = row("partial.pub", a)
        partial["classification"] = "cfb_publisher_truncated"
        result = self.run_union([("one", [partial])], latest="one")
        self.assertEqual(result["cumulative"]["publisher_sha256"], 0)
        self.assertEqual(result["partial_sha256"], [a])

    def test_legacy_arc_shorter_than_content_length_is_partial(self) -> None:
        a = "a" * 64
        partial = row("legacy.pub", a, size=130800)
        partial.update({
            "cc_warc_filename": "crawl-001/2008/file.arc.gz",
            "content_length_header": "214528",
            "cc_http_content_encoding": "",
            "cc_http_transfer_encoding": "",
        })
        result = self.run_union([("one", [partial])], latest="one")
        self.assertEqual(result["cumulative"]["publisher_sha256"], 0)
        self.assertEqual(result["partial_sha256"], [a])

    def test_complete_observation_outranks_partial_observation(self) -> None:
        a = "a" * 64
        partial = row("partial.pub", a)
        partial["classification"] = "cfb_publisher_truncated"
        complete = row("complete.pub", a)
        result = self.run_union(
            [("old", [partial]), ("latest", [complete])],
            latest="latest",
        )
        self.assertEqual(result["cumulative"]["publisher_sha256"], 1)
        self.assertEqual(result["partial_sha256_count"], 0)

    def test_complete_audit_outranks_partial_audit(self) -> None:
        a = "a" * 64
        result = self.run_union(
            [("one", [row("a.pub", a)])],
            latest="one",
            audit_rows=[
                {"sha256": a, "status": "partial"},
                {"sha256": a, "status": "complete"},
            ],
        )
        self.assertEqual(result["cumulative"]["publisher_sha256"], 1)
        self.assertEqual(result["partial_sha256_count"], 0)

    def test_non_publisher_rows_are_excluded(self) -> None:
        a = "a" * 64
        other = row("not-pub.pdf", "d" * 64)
        other["classification"] = "pdf_mislabel"
        result = self.run_union(
            [("one", [row("a.pub", a), other])],
            latest="one",
        )
        self.assertEqual(result["cumulative"]["publisher_sha256"], 1)

    def test_conflicts_are_reported(self) -> None:
        a = "a" * 64
        result = self.run_union(
            [
                ("old", [row("a.pub", a, "natural_wild", 100)]),
                (
                    "latest",
                    [row("a.pub", a, "quarantine_active_content", 101)],
                ),
            ],
            latest="latest",
        )
        self.assertEqual(len(result["stratum_conflicts"]), 1)
        self.assertEqual(len(result["size_conflicts"]), 1)
        self.assertEqual(result["cumulative"]["mixed_stratum_sha256"], 1)
        self.assertEqual(result["snapshots"][0]["natural_sha256"], 1)
        self.assertEqual(result["snapshots"][0]["mixed_stratum_sha256"], 0)
        self.assertEqual(result["snapshots"][1]["quarantine_sha256"], 1)
        self.assertEqual(result["snapshots"][1]["mixed_stratum_sha256"], 0)

    def test_same_snapshot_conflicts_survive_observation_dedup(self) -> None:
        a = "a" * 64
        result = self.run_union(
            [
                ("same", [row("a.pub", a, "natural_wild", 100)]),
                ("same", [row("a.pub", a, "quarantine_active_content", 101)]),
            ],
            latest="same",
        )
        self.assertEqual(result["cumulative"]["mixed_stratum_sha256"], 1)
        self.assertEqual(result["snapshots"][0]["mixed_stratum_sha256"], 1)
        self.assertEqual(len(result["stratum_conflicts"]), 1)
        self.assertEqual(len(result["size_conflicts"]), 1)

    def test_partial_completeness_audit_excludes_sha(self) -> None:
        a = "a" * 64
        b = "b" * 64
        result = self.run_union(
            [("one", [row("a.pub", a), row("b.pub", b)])],
            latest="one",
            audit_rows=[
                {"sha256": a, "status": "partial"},
                {"sha256": b, "status": "complete"},
            ],
        )
        self.assertEqual(result["cumulative"]["publisher_sha256"], 1)
        self.assertEqual(result["partial_sha256_count"], 1)
        self.assertEqual(result["partial_sha256"], [a])

    def test_partial_with_failures_is_excluded(self) -> None:
        a = "a" * 64
        result = self.run_union(
            [("one", [row("a.pub", a)])],
            latest="one",
            audit_rows=[
                {"sha256": a, "status": "partial_with_audit_failures"},
            ],
        )
        self.assertEqual(result["cumulative"]["publisher_sha256"], 0)
        self.assertEqual(result["partial_sha256"], [a])


if __name__ == "__main__":
    unittest.main()
