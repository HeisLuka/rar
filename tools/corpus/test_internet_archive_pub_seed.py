#!/usr/bin/env python3
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import internet_archive_pub_seed as ia

class T(unittest.TestCase):
    def test_download_url_encodes_member(self):
        u=ia.download_url("item id","folder/A B.pub")
        self.assertIn("item%20id",u)
        self.assertIn("folder/A%20B.pub",u)
    def test_seed_keeps_provider_hashes(self):
        r=ia.seed_row("x",{"title":"t"},{"name":"a.pub","size":"7","md5":"m","sha1":"s"})
        self.assertEqual(r["candidate_filename"],"a.pub")
        self.assertEqual(r["ia_member_md5"],"m")
        self.assertEqual(r["ia_member_sha1"],"s")

if __name__=="__main__": unittest.main()
