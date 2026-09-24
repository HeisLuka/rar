#!/usr/bin/env python3
import json
import pathlib
import tempfile

import fitz

from pdf_reference_diff_v1 import compare_pdfs

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "pdf-reference-diff-v1"
OUT.mkdir(parents=True, exist_ok=True)


def make_pdf(path, move=0):
    doc = fitz.open()
    page = doc.new_page(width=612, height=792)
    page.insert_text((72, 72), "External reference receipt fixture", fontsize=18)
    page.draw_rect(
        fitz.Rect(100 + move, 140, 260 + move, 260),
        color=(0.1, 0.1, 0.1),
        fill=(0.8, 0.2, 0.2),
    )
    page.draw_circle((420, 220), 45, color=(0.1, 0.2, 0.7), fill=(0.2, 0.5, 0.9))
    doc.save(path)
    doc.close()


with tempfile.TemporaryDirectory() as td:
    candidate = pathlib.Path(td) / "candidate.pdf"
    reference = pathlib.Path(td) / "reference.pdf"
    make_pdf(candidate, move=36)
    make_pdf(reference, move=0)
    receipt = compare_pdfs(candidate, reference)

if not receipt["page_count"]["match"]:
    raise AssertionError("fixture page counts diverged")
if not receipt["pages"][0]["page_boxes_match"]:
    raise AssertionError("fixture page boxes diverged")
diff = receipt["pages"][0]["diff"]
if diff["significant_pixel_count"] <= 0:
    raise AssertionError("intentional visible difference was not detected")
if not diff["regions"]:
    raise AssertionError("visible difference was not localized")
if not diff["significant_mask"]["runs"]:
    raise AssertionError("machine-readable significant mask was not emitted")
if diff["raw_changed_pixel_count"] < diff["significant_pixel_count"]:
    raise AssertionError("raw/material pixel accounting is inconsistent")

(OUT / "receipt.json").write_text(
    json.dumps(receipt, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
print(json.dumps({
    "receipt_version": receipt["receipt_version"],
    "renderer": receipt["renderer"],
    "page_count": receipt["page_count"],
    "page0_raw_changed_pixels": diff["raw_changed_pixel_count"],
    "page0_noise_only_pixels": diff["noise_only_pixel_count"],
    "page0_significant_fraction": diff["significant_fraction"],
    "page0_mask_runs": len(diff["significant_mask"]["runs"]),
    "page0_regions": len(diff["regions"]),
    "page0_bbox": diff["significant_bbox"],
}, indent=2, sort_keys=True))
