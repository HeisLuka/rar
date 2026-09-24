#!/usr/bin/env python3
import pathlib
import tempfile
import unittest

import fitz

from pdf_reference_diff_v1 import compare_pdfs, compare_rasters


def make_pdf(path, move=0, page_width=612, page_height=792, pages=1):
    doc = fitz.open()
    for index in range(pages):
        page = doc.new_page(width=page_width, height=page_height)
        page.insert_text((72, 72), f"Chaptera reference fidelity fixture {index}", fontsize=18)
        page.draw_rect(
            fitz.Rect(100 + move, 140, 260 + move, 260),
            color=(0.1, 0.1, 0.1),
            fill=(0.8, 0.2, 0.2),
        )
        page.draw_line((72, 320), (500, 320), color=(0.2, 0.2, 0.2), width=2)
    doc.save(path)
    doc.close()


class PdfReferenceDiffTests(unittest.TestCase):
    def test_identical_pdf_has_zero_significant_diff(self):
        with tempfile.TemporaryDirectory() as td:
            p = pathlib.Path(td) / "a.pdf"
            make_pdf(p)
            receipt = compare_pdfs(p, p)
            self.assertTrue(receipt["page_count"]["match"])
            self.assertTrue(receipt["pages"][0]["page_boxes_match"])
            diff = receipt["pages"][0]["diff"]
            self.assertEqual(0, diff["raw_changed_pixel_count"])
            self.assertEqual(0, diff["noise_only_pixel_count"])
            self.assertEqual(0, diff["significant_pixel_count"])
            self.assertEqual([], diff["regions"])
            self.assertEqual([], diff["significant_mask"]["runs"])

    def test_moved_object_is_localized_without_opaque_score(self):
        with tempfile.TemporaryDirectory() as td:
            a = pathlib.Path(td) / "candidate.pdf"
            b = pathlib.Path(td) / "reference.pdf"
            make_pdf(a, move=36)
            make_pdf(b, move=0)
            receipt = compare_pdfs(a, b)
            diff = receipt["pages"][0]["diff"]
            self.assertGreater(diff["significant_pixel_count"], 0)
            self.assertIsNotNone(diff["significant_bbox"])
            self.assertGreaterEqual(len(diff["regions"]), 1)
            self.assertTrue(diff["significant_mask"]["runs"])
            self.assertIn("mean_abs_channel_delta", diff)
            self.assertNotIn("pass", diff)

    def test_page_box_mismatch_is_explicit(self):
        with tempfile.TemporaryDirectory() as td:
            a = pathlib.Path(td) / "candidate.pdf"
            b = pathlib.Path(td) / "reference.pdf"
            make_pdf(a, page_width=600)
            make_pdf(b, page_width=612)
            receipt = compare_pdfs(a, b)
            self.assertFalse(receipt["pages"][0]["page_boxes_match"])
            self.assertFalse(receipt["pages"][0]["diff"]["raster_size_match"])
            self.assertIn("media_box", receipt["pages"][0]["candidate_boxes"])
            self.assertIn("crop_box", receipt["pages"][0]["candidate_boxes"])

    def test_page_count_mismatch_is_explicit(self):
        with tempfile.TemporaryDirectory() as td:
            a = pathlib.Path(td) / "candidate.pdf"
            b = pathlib.Path(td) / "reference.pdf"
            make_pdf(a, pages=2)
            make_pdf(b, pages=1)
            receipt = compare_pdfs(a, b)
            self.assertFalse(receipt["page_count"]["match"])
            self.assertEqual(2, receipt["page_count"]["candidate"])
            self.assertEqual(1, receipt["page_count"]["reference"])
            self.assertEqual(1, len(receipt["pages"]))

    def test_subthreshold_raster_noise_is_separate_from_material_difference(self):
        a = {"width": 1, "height": 1, "samples": bytes([100, 100, 100])}
        b = {"width": 1, "height": 1, "samples": bytes([110, 100, 100])}
        diff = compare_rasters(a, b)
        self.assertEqual(1, diff["raw_changed_pixel_count"])
        self.assertEqual(1, diff["noise_only_pixel_count"])
        self.assertEqual(0, diff["significant_pixel_count"])
        self.assertEqual([], diff["significant_mask"]["runs"])


if __name__ == "__main__":
    unittest.main()
