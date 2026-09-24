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
    def test_pub_member_is_seed(self):
        self.assertTrue(ia.is_pub_or_zip_seed({"name":"fixture.pub","source":"original"}))
    def test_original_zip_can_be_seed_container(self):
        self.assertTrue(ia.is_pub_or_zip_seed({"name":"publisher-templates.zip","source":"original"}))
    def test_generated_scan_zip_is_rejected(self):
        self.assertFalse(ia.is_pub_or_zip_seed({"name":"publisher2002_jp2.zip","source":"derivative"}))
        self.assertFalse(ia.is_pub_or_zip_seed({"name":"publisher2002_thumb_avif.zip","source":"derivative"}))
    def test_explicit_derivative_generic_zip_is_rejected(self):
        self.assertFalse(ia.is_pub_or_zip_seed({"name":"templates.zip","source":"derivative"}))

if __name__=="__main__": unittest.main()
