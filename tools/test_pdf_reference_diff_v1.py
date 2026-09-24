#!/usr/bin/env python3
import pathlib
import tempfile
import unittest

import fitz

from pdf_reference_diff_v1 import compare_pdfs

def make_pdf(path,move=0,page_width=612,page_height=792):
    doc=fitz.open()
    page=doc.new_page(width=page_width,height=page_height)
    page.insert_text((72,72),"Chaptera reference fidelity fixture",fontsize=18)
    page.draw_rect(fitz.Rect(100+move,140,260+move,260),color=(0.1,0.1,0.1),fill=(0.8,0.2,0.2))
    page.draw_line((72,320),(500,320),color=(0.2,0.2,0.2),width=2)
    doc.save(path)
    doc.close()

class PdfReferenceDiffTests(unittest.TestCase):
    def test_identical_pdf_has_zero_significant_diff(self):
        with tempfile.TemporaryDirectory() as td:
            p=pathlib.Path(td)/"a.pdf"
            make_pdf(p)
            receipt=compare_pdfs(p,p)
            self.assertTrue(receipt["page_count"]["match"])
            self.assertTrue(receipt["pages"][0]["page_box_match"])
            self.assertEqual(0,receipt["pages"][0]["diff"]["significant_pixel_count"])
            self.assertEqual([],receipt["pages"][0]["diff"]["regions"])

    def test_moved_object_is_localized_without_opaque_score(self):
        with tempfile.TemporaryDirectory() as td:
            a=pathlib.Path(td)/"candidate.pdf"
            b=pathlib.Path(td)/"reference.pdf"
            make_pdf(a,move=36)
            make_pdf(b,move=0)
            receipt=compare_pdfs(a,b)
            diff=receipt["pages"][0]["diff"]
            self.assertGreater(diff["significant_pixel_count"],0)
            self.assertIsNotNone(diff["significant_bbox"])
            self.assertGreaterEqual(len(diff["regions"]),1)
            self.assertIn("mean_abs_channel_delta",diff)
            self.assertNotIn("pass",diff)

    def test_page_box_mismatch_is_explicit(self):
        with tempfile.TemporaryDirectory() as td:
            a=pathlib.Path(td)/"candidate.pdf"
            b=pathlib.Path(td)/"reference.pdf"
            make_pdf(a,page_width=600)
            make_pdf(b,page_width=612)
            receipt=compare_pdfs(a,b)
            self.assertFalse(receipt["pages"][0]["page_box_match"])
            self.assertFalse(receipt["pages"][0]["diff"]["raster_size_match"])

if __name__=="__main__":
    unittest.main()
