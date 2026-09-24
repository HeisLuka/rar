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
if __name__=="__main__":unittest.main()
