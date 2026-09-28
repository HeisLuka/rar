#!/usr/bin/env python3
import argparse
import hashlib
import json
import pathlib

import fitz
from PIL import Image, ImageChops

from pdf_reference_diff_v1 import (
    RASTER_DPI,
    SIGNIFICANT_CHANNEL_DELTA,
    compare_rasters,
    file_sha256,
    page_box,
)

EXPECTED_SOURCE_SHA256 = "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3"
EXPECTED_REFERENCE_PDF_SHA256 = "c1288571ee8da91afa27ca5c49191aacefe82123df294c11f58ca047d0ced769"
EXPECTED_REFERENCE_PDF_SIZE = 1_108_627
EXPECTED_PAGE_COUNT = 3


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def pil_raster(image: Image.Image) -> dict:
    rgb = image.convert("RGB")
    samples = rgb.tobytes()
    return {
        "width": rgb.width,
        "height": rgb.height,
        "stride": rgb.width * 3,
        "samples": samples,
        "raster_sha256": sha256_bytes(samples),
    }


def fitz_raster(page) -> dict:
    pix = page.get_pixmap(dpi=RASTER_DPI, colorspace=fitz.csRGB, alpha=False)
    samples = bytes(pix.samples)
    return {
        "width": pix.width,
        "height": pix.height,
        "stride": pix.stride,
        "samples": samples,
        "raster_sha256": sha256_bytes(samples),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reader-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--reference-pdf", required=True, type=pathlib.Path)
    parser.add_argument("--reader-dir", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--reference-raster-dir", required=True, type=pathlib.Path)
    parser.add_argument("--diff-dir", required=True, type=pathlib.Path)
    args = parser.parse_args()

    reader_receipt = json.loads(args.reader_receipt.read_text(encoding="utf-8"))
    if reader_receipt["source_sha256"] != EXPECTED_SOURCE_SHA256:
        raise SystemExit("Reader receipt source SHA-256 drifted")
    if reader_receipt["reader_page_count"] != EXPECTED_PAGE_COUNT:
        raise SystemExit(
            f"Reader page count {reader_receipt['reader_page_count']} != {EXPECTED_PAGE_COUNT}"
        )
    if not reader_receipt.get("family_profile_applied"):
        raise SystemExit("Carlton family profile was not admitted by Reader")

    reference_size = args.reference_pdf.stat().st_size
    reference_sha256 = file_sha256(args.reference_pdf)
    if reference_size != EXPECTED_REFERENCE_PDF_SIZE:
        raise SystemExit(
            f"reference PDF size {reference_size} != {EXPECTED_REFERENCE_PDF_SIZE}"
        )
    if reference_sha256 != EXPECTED_REFERENCE_PDF_SHA256:
        raise SystemExit(
            f"reference PDF SHA-256 {reference_sha256} != {EXPECTED_REFERENCE_PDF_SHA256}"
        )

    args.reference_raster_dir.mkdir(parents=True, exist_ok=True)
    args.diff_dir.mkdir(parents=True, exist_ok=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)

    reference = fitz.open(args.reference_pdf)
    if reference.page_count != EXPECTED_PAGE_COUNT:
        raise SystemExit(
            f"reference PDF page count {reference.page_count} != {EXPECTED_PAGE_COUNT}"
        )

    pages = []
    raster_size_mismatches = []
    for page_index, page_receipt in enumerate(reader_receipt["pages"]):
        reader_png = args.reader_dir / page_receipt["png"]
        reader_image = Image.open(reader_png).convert("RGB")
        reader = pil_raster(reader_image)

        reference_page = reference.load_page(page_index)
        reference_raster = fitz_raster(reference_page)
        reference_image = Image.frombytes(
            "RGB",
            (reference_raster["width"], reference_raster["height"]),
            reference_raster["samples"],
        )

        reference_png = args.reference_raster_dir / (
            f"carlton-march-reference-page-{page_index + 1:03}.png"
        )
        reference_image.save(reference_png)

        diff = compare_rasters(reader, reference_raster)
        if not diff["raster_size_match"]:
            raster_size_mismatches.append(
                {
                    "page_index": page_index,
                    "reader": [reader["width"], reader["height"]],
                    "reference": [
                        reference_raster["width"],
                        reference_raster["height"],
                    ],
                }
            )
            diff_png = None
        else:
            visual_diff = ImageChops.difference(reader_image, reference_image)
            diff_png_path = args.diff_dir / (
                f"carlton-march-reader-vs-reference-page-{page_index + 1:03}.png"
            )
            visual_diff.save(diff_png_path)
            diff_png = diff_png_path.name

        pages.append(
            {
                "page_index": page_index,
                "reader_page_number": page_index + 1,
                "reader_page_id": page_receipt["page_id"],
                "reader_png": page_receipt["png"],
                "reader_png_sha256": file_sha256(reader_png),
                "reference_page_box": page_box(reference_page),
                "reference_raster_png": reference_png.name,
                "reference_raster_sha256": reference_raster["raster_sha256"],
                "diff_png": diff_png,
                "diff": diff,
            }
        )

    receipt = {
        "schema": "chaptera.carlton-reader-published-pdf-visual.v1",
        "source_pub_sha256": EXPECTED_SOURCE_SHA256,
        "reference_pdf_sha256": reference_sha256,
        "reference_pdf_size": reference_size,
        "reference_pdf_page_count": reference.page_count,
        "reader_page_count": reader_receipt["reader_page_count"],
        "renderer": reader_receipt["renderer"],
        "comparison": {
            "reference_renderer": "MuPDF via PyMuPDF",
            "pymupdf_version": fitz.VersionBind,
            "mupdf_version": fitz.VersionFitz,
            "raster_dpi": RASTER_DPI,
            "significant_channel_delta": SIGNIFICANT_CHANNEL_DELTA,
            "visual_equivalence_claimed": False,
            "semantic_equivalence_claimed": False,
            "interpretation": (
                "This receipt localizes visible disagreement between the current "
                "Chaptera Reader page paint and the school's published PDF. "
                "A successful job means the comparison completed on the exact pair, "
                "not that the pixels are equivalent."
            ),
        },
        "summary": {
            "raster_size_mismatch_count": len(raster_size_mismatches),
            "pages_with_significant_difference": sum(
                1
                for page in pages
                if page["diff"]["significant_pixel_count"] not in (None, 0)
            ),
            "significant_fraction_by_page": [
                page["diff"]["significant_fraction"] for page in pages
            ],
            "mean_abs_channel_delta_by_page": [
                page["diff"]["mean_abs_channel_delta"] for page in pages
            ],
            "max_channel_delta_by_page": [
                page["diff"]["max_channel_delta"] for page in pages
            ],
        },
        "raster_size_mismatches": raster_size_mismatches,
        "pages": pages,
    }

    args.output.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt["summary"], indent=2, sort_keys=True))

    if raster_size_mismatches:
        raise SystemExit("Reader/reference raster sizes differ at the fixed physical DPI")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
