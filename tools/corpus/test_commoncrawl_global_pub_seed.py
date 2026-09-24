#!/usr/bin/env python3
import importlib.util
import sys
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import commoncrawl_global_pub_seed as cc


class GlobalCommonCrawlSeedTests(unittest.TestCase):
    def test_spread_select_covers_edges(self):
        values = [str(i) for i in range(10)]
        self.assertEqual(cc.spread_select(values, 3), ["0", "4", "9"])

    def test_sql_has_no_domain_or_country_filter(self):
        sql = cc.build_sql(25).lower()
        self.assertIn("url_path", sql)
        self.assertIn("publisher", sql)
        self.assertNotIn("url_host_tld in", sql)
        self.assertNotIn("url_host_name =", sql)

    def test_seed_preserves_archive_coordinates(self):
        row = cc.to_seed({
            "url": "https://example.org/a.pub",
            "url_host_name": "example.org",
            "fetch_time": "2021-01-02 03:04:05",
            "content_digest": "sha1:abc",
            "content_mime_type": "application/x-mspublisher",
            "content_mime_detected": "",
            "warc_filename": "crawl-data/x/segments/y.warc.gz",
            "warc_record_offset": 12,
            "warc_record_length": 34,
        }, "CC-MAIN-2021-04")
        assert row is not None
        self.assertEqual(row["candidate_filename"], "a.pub")
        self.assertEqual(row["cc_warc_offset"], "12")
        self.assertEqual(row["cc_warc_length"], "34")
        self.assertEqual(row["source_class"], "common_crawl_global")


if __name__ == "__main__":
    unittest.main()
