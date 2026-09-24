#!/usr/bin/env python3
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import forum_pub_locator as f
class T(unittest.TestCase):
    def test_filename(self):
        self.assertEqual(f.filename_from("please see Sample 2000.pub"),"Sample 2000.pub")
    def test_link(self):
        rows=f.mine("x","https://x/thread",'<a href="/files/a.pub">download</a>','https://x/thread')
        self.assertEqual(rows[0]["candidate_filename"],"a.pub")
    def test_registry_exact_locator(self):
        row=f.registry_exact_row({
            "source_class":"bug",
            "source_page":"https://x/issue/4",
            "exact_url":"https://x/files/a.pub",
            "exact_filename":"Pinned.pub",
            "claimed_version":"Publisher 2021",
            "notes":"pinned",
        })
        self.assertEqual(row["direct_url"],"https://x/files/a.pub")
        self.assertEqual(row["candidate_filename"],"Pinned.pub")
        self.assertEqual(row["forum_locator_status"],"registry_exact")
        self.assertEqual(row["forum_claimed_version"],"Publisher 2021")
if __name__=="__main__":unittest.main()
