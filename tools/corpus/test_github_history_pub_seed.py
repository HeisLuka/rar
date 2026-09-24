#!/usr/bin/env python3
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import github_history_pub_seed as g

class T(unittest.TestCase):
    def test_regexes(self):
        self.assertTrue(g.PUB_RE.search("A.PUB"))
        self.assertTrue(g.ARCHIVE_RE.search("fixtures.zip"))
        self.assertFalse(g.PUB_RE.search("pub.txt"))

if __name__=="__main__": unittest.main()
