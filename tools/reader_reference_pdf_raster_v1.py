#!/usr/bin/env python3
import json
import platform
import sys
from pathlib import Path

import fitz

from pdf_reference_diff_v1 import (
    RASTER_DPI,
    SIGNIFICANT_CHANNEL_DELTA,
    TILE,
    compare_rasters,
    file_sha256,
    page_boxes,
    render_page,
    sha256_bytes,
)

EMU_PER_POINT = 12_700.0
PHYSICAL_SIZE_TOLERANCE_PX = 0.1
PHYSICAL_SIZE_TOLERANCE_PT = 72.0 / RASTER_DPI * PHYSICAL_SIZE_TOLERANCE_PX


def render_png(path):
    pix = fitz.Pixmap(str(path))
    if pix.colorspace is None or pix.colorspace.n != 3:
        pix = fitz.Pixmap(fitz.csRGB, pix)
    if pix.alpha:
        pix = fitz.Pixmap(pix, 0)
    samples = bytes(pix.samples)
    return {
        "width": pix.width,
        "height": pix.height,
        "stride": pix.stride,
        "samples": samples,
        "raster_sha256": sha256_bytes(samples),
    }


def physical_size_comparison(page_receipt, reference_box):
    expected_width = page_receipt["width_emu"] / EMU_PER_POINT
    expected_height = page_receipt["height_emu"] / EMU_PER_POINT
    actual = reference_box["render_rect"]
    width_delta_pt = expected_width - actual["width_pt"]
    height_delta_pt = expected_height - actual["height_pt"]
    return {
        "matches_within_tolerance": (
            abs(width_delta_pt) <= PHYSICAL_SIZE_TOLERANCE_PT
            and abs(height_delta_pt) <= PHYSICAL_SIZE_TOLERANCE_PT
        ),
        "width_delta_pt": round(width_delta_pt, 6),
        "height_delta_pt": round(height_delta_pt, 6),
        "width_delta_px": round(width_delta_pt * RASTER_DPI / 72.0, 6),
        "height_delta_px": round(height_delta_pt * RASTER_DPI / 72.0, 6),
    }


def compare_reader_rasters(golden_receipt_path, reference_pdf_path):
    golden_receipt_path = Path(golden_receipt_path)
    reference_pdf_path = Path(reference_pdf_path)
    golden = json.loads(golden_receipt_path.read_text(encoding="utf-8"))
    if golden.get("schema") != "chaptera.reader-golden-carlton-march.v1":
        raise ValueError(f"unsupported Reader golden schema: {golden.get('schema')!r}")

    reference = fitz.open(reference_pdf_path)
    pages = golden.get("pages", [])
    page_count_match = len(pages) == reference.page_count
    compared = []

    for index in range(min(len(pages), reference.page_count)):
        candidate_meta = pages[index]
        candidate_path = golden_receipt_path.parent / candidate_meta["png"]
        candidate = render_png(candidate_path)
        reference_page = reference.load_page(index)
        reference_box = page_boxes(reference_page)
        reference_raster = render_page(reference_page)
        diff = compare_rasters(candidate, reference_raster)
        physical_size = physical_size_comparison(candidate_meta, reference_box)

        compared.append(
            {
                "page_index": index,
                "candidate_png": candidate_meta["png"],
                "candidate_png_sha256": file_sha256(candidate_path),
                "candidate_raster_sha256": candidate["raster_sha256"],
                "reference_raster_sha256": reference_raster["raster_sha256"],
                "reference_boxes": reference_box,
                "physical_page_size_matches_reference": physical_size[
                    "matches_within_tolerance"
                ],
                "physical_page_size_delta": {
                    "width_pt": physical_size["width_delta_pt"],
                    "height_pt": physical_size["height_delta_pt"],
                    "width_px": physical_size["width_delta_px"],
                    "height_px": physical_size["height_delta_px"],
                },
                "candidate_physical_size": {
                    "width_emu": candidate_meta["width_emu"],
                    "height_emu": candidate_meta["height_emu"],
                    "width_pt": round(candidate_meta["width_emu"] / EMU_PER_POINT, 6),
                    "height_pt": round(candidate_meta["height_emu"] / EMU_PER_POINT, 6),
                },
                "diff": diff,
            }
        )

    return {
        "receipt_version": "chaptera.reader-reference-pdf-raster.v1",
        "reader_source_sha256": golden["source_sha256"],
        "reference_sha256": file_sha256(reference_pdf_path),
        "page_count": {
            "candidate": len(pages),
            "reference": reference.page_count,
            "match": page_count_match,
        },
        "reader_render": {
            "render_backend": golden["render_backend"],
            "raster_dpi": golden["raster_dpi"],
            "shell_ui_rendered": golden["shell_ui_rendered"],
            "selection_overlay_rendered": golden["selection_overlay_rendered"],
            "preview_warning_overlay_rendered": golden["preview_warning_overlay_rendered"],
            "family_profile_applied": golden["family_profile_applied"],
            "source_font_face_claimed": golden["source_font_face_claimed"],
            "publisher_exact_reflow_claimed": golden["publisher_exact_reflow_claimed"],
        },
        "comparator": {
            "engine": "MuPDF",
            "binding": "PyMuPDF",
            "pymupdf_version": fitz.VersionBind,
            "mupdf_version": fitz.VersionFitz,
            "python": sys.version.split()[0],
            "platform": platform.platform(),
            "raster_dpi": RASTER_DPI,
            "colorspace": "RGB",
            "alpha": False,
            "significant_channel_delta": SIGNIFICANT_CHANNEL_DELTA,
            "region_tile_px": TILE,
            "physical_size_tolerance_pt": PHYSICAL_SIZE_TOLERANCE_PT,
            "physical_size_tolerance_px": PHYSICAL_SIZE_TOLERANCE_PX,
        },
        "pages": compared,
        "limitations": [
            "The Reader raster is compared to the published PDF raster at the same nominal DPI; this localizes visible disagreement but does not prove authoring-semantic equivalence.",
            "Current Reader fallback-font execution is measured as rendered; source-font face fidelity and Publisher-exact reflow are not claimed.",
            "Below-threshold raster noise is reported separately from significant differences.",
            "PDF object semantics, PDF/X conformance and Editor-current-state export are outside this receipt.",
        ],
    }


def main(argv):
    if len(argv) != 4:
        raise SystemExit(
            "usage: reader_reference_pdf_raster_v1.py "
            "READER_GOLDEN_RECEIPT.json REFERENCE.pdf OUTPUT.json"
        )
    receipt = compare_reader_rasters(argv[1], argv[2])
    output = Path(argv[3])
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        "READER REFERENCE PDF RASTER COMPLETE "
        + " ".join(
            f"p{page['page_index'] + 1}="
            + (
                f"{page['diff']['significant_fraction']:.6f}"
                if page["diff"]["significant_fraction"] is not None
                else "size-mismatch"
            )
            for page in receipt["pages"]
        )
    )


if __name__ == "__main__":
    main(sys.argv)
