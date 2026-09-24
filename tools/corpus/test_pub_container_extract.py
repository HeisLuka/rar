#!/usr/bin/env python3
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import pub_container_extract as c

class T(unittest.TestCase):
    def test_path_safety(self):
        self.assertTrue(c.safe_member("a/b.pub"))
        self.assertFalse(c.safe_member("../x.pub"))
        self.assertFalse(c.safe_member("/abs.pub"))
    def test_kind(self):
        self.assertEqual(c.ext_kind("x.PUB"),"pub")
        self.assertEqual(c.ext_kind("x.iso"),"container")
        self.assertEqual(c.ext_kind("x.txt"),"")

if __name__=="__main__": unittest.main()
