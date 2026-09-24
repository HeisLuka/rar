#!/usr/bin/env python3
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import positive_domain_pub_seed as d

class T(unittest.TestCase):
    def test_secret_query_rejected(self):
        self.assertFalse(d.safe_public_url("https://x/a.pub?X-Amz-Signature=secret"))
        self.assertTrue(d.safe_public_url("https://x/a.pub?id=123"))
    def test_candidate_from_anchor(self):
        self.assertTrue(d.candidate("https://x/download?id=2","Year 1 RE.pub"))
        self.assertEqual(d.filename_from("https://x/download?id=2","Year 1 RE.pub"),"Year 1 RE.pub")
    def test_domain_normalization(self):
        self.assertEqual(d.hostname("https://www.Example.org/a"),"example.org")

if __name__=="__main__": unittest.main()
