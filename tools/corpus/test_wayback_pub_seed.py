#!/usr/bin/env python3
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import wayback_pub_seed as wb


class WaybackSeedTests(unittest.TestCase):
    def test_replay_uses_raw_id_mode(self):
        url = wb.replay_url("20200102030405", "https://example.org/a.pub")
        self.assertEqual(url, "https://web.archive.org/web/20200102030405id_/https://example.org/a.pub")

    def test_convert_preserves_provenance(self):
        row = wb.convert({
            "timestamp": "20011201020304",
            "original": "http://example.org/file.pub",
            "mimetype": "application/octet-stream",
            "statuscode": "200",
            "digest": "ABC",
            "length": "123",
        }, "domain", "example.org")
        assert row is not None
        self.assertEqual(row["candidate_filename"], "file.pub")
        self.assertEqual(row["wayback_original_url"], "http://example.org/file.pub")
        self.assertEqual(row["wayback_query_kind"], "domain")


if __name__ == "__main__":
    unittest.main()
