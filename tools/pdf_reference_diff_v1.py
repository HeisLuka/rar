#!/usr/bin/env python3
import hashlib
import platform
import sys

import fitz

RASTER_DPI = 144
SIGNIFICANT_CHANNEL_DELTA = 24
TILE = 16


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _rect(rect):
    return {
        "x0_pt": round(rect.x0, 6),
        "y0_pt": round(rect.y0, 6),
        "x1_pt": round(rect.x1, 6),
        "y1_pt": round(rect.y1, 6),
        "width_pt": round(rect.width, 6),
        "height_pt": round(rect.height, 6),
    }


def page_boxes(page):
    return {
        "render_rect": _rect(page.rect),
        "media_box": _rect(page.mediabox),
        "crop_box": _rect(page.cropbox),
        "rotation": page.rotation,
    }


def render_page(page):
    pix = page.get_pixmap(dpi=RASTER_DPI, colorspace=fitz.csRGB, alpha=False)
    samples = bytes(pix.samples)
    return {
        "width": pix.width,
        "height": pix.height,
        "stride": pix.stride,
        "samples": samples,
        "raster_sha256": sha256_bytes(samples),
    }


def tile_regions(mask_tiles):
    remaining = set(mask_tiles)
    regions = []
    while remaining:
        start = remaining.pop()
        stack = [start]
        group = [start]
        while stack:
            x, y = stack.pop()
            for neighbor in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
                if neighbor in remaining:
                    remaining.remove(neighbor)
                    stack.append(neighbor)
                    group.append(neighbor)
        xs = [p[0] for p in group]
        ys = [p[1] for p in group]
        regions.append({
            "tile_x0": min(xs),
            "tile_y0": min(ys),
            "tile_x1": max(xs),
            "tile_y1": max(ys),
            "pixel_bbox": {
                "x": min(xs) * TILE,
                "y": min(ys) * TILE,
                "width": (max(xs) - min(xs) + 1) * TILE,
                "height": (max(ys) - min(ys) + 1) * TILE,
            },
            "tile_count": len(group),
        })
    regions.sort(key=lambda r: (-r["tile_count"], r["tile_y0"], r["tile_x0"]))
    return regions


def tile_mask_rle(mask_tiles, width, height):
    tile_width = (width + TILE - 1) // TILE
    tile_height = (height + TILE - 1) // TILE
    linear = sorted(y * tile_width + x for x, y in mask_tiles)
    runs = []
    if linear:
        start = previous = linear[0]
        for value in linear[1:]:
            if value == previous + 1:
                previous = value
                continue
            runs.append([start, previous - start + 1])
            start = previous = value
        runs.append([start, previous - start + 1])
    return {
        "encoding": "row-major-tile-rle-v1",
        "tile_px": TILE,
        "tile_width": tile_width,
        "tile_height": tile_height,
        "runs": runs,
    }


def compare_rasters(a, b):
    if (a["width"], a["height"]) != (b["width"], b["height"]):
        return {
            "raster_size_match": False,
            "raw_changed_pixel_count": None,
            "noise_only_pixel_count": None,
            "significant_pixel_count": None,
            "significant_fraction": None,
            "noise_only_fraction": None,
            "mean_abs_channel_delta": None,
            "max_channel_delta": None,
            "significant_bbox": None,
            "significant_mask": None,
            "regions": [],
        }

    sa = a["samples"]
    sb = b["samples"]
    pixels = a["width"] * a["height"]
    total_delta = 0
    max_delta = 0
    raw_changed = 0
    noise_only = 0
    significant = 0
    min_x = min_y = None
    max_x = max_y = None
    changed_tiles = set()

    for p in range(pixels):
        base = p * 3
        dr = abs(sa[base] - sb[base])
        dg = abs(sa[base + 1] - sb[base + 1])
        db = abs(sa[base + 2] - sb[base + 2])
        total_delta += dr + dg + db
        local = max(dr, dg, db)
        max_delta = max(max_delta, local)
        if local > 0:
            raw_changed += 1
        if 0 < local < SIGNIFICANT_CHANNEL_DELTA:
            noise_only += 1
        if local >= SIGNIFICANT_CHANNEL_DELTA:
            significant += 1
            x = p % a["width"]
            y = p // a["width"]
            min_x = x if min_x is None else min(min_x, x)
            max_x = x if max_x is None else max(max_x, x)
            min_y = y if min_y is None else min(min_y, y)
            max_y = y if max_y is None else max(max_y, y)
            changed_tiles.add((x // TILE, y // TILE))

    bbox = None
    if significant:
        bbox = {
            "x": min_x,
            "y": min_y,
            "width": max_x - min_x + 1,
            "height": max_y - min_y + 1,
        }

    return {
        "raster_size_match": True,
        "raw_changed_pixel_count": raw_changed,
        "noise_only_pixel_count": noise_only,
        "significant_pixel_count": significant,
        "significant_fraction": significant / pixels,
        "noise_only_fraction": noise_only / pixels,
        "mean_abs_channel_delta": total_delta / (pixels * 3),
        "max_channel_delta": max_delta,
        "significant_bbox": bbox,
        "significant_mask": tile_mask_rle(changed_tiles, a["width"], a["height"]),
        "regions": tile_regions(changed_tiles),
    }


def compare_pdfs(candidate_path, reference_path):
    candidate = fitz.open(candidate_path)
    reference = fitz.open(reference_path)
    page_count_match = candidate.page_count == reference.page_count
    page_count = min(candidate.page_count, reference.page_count)
    pages = []
    for i in range(page_count):
        cp = candidate.load_page(i)
        rp = reference.load_page(i)
        cb = page_boxes(cp)
        rb = page_boxes(rp)
        cr = render_page(cp)
        rr = render_page(rp)
        diff = compare_rasters(cr, rr)
        pages.append({
            "page_index": i,
            "candidate_boxes": cb,
            "reference_boxes": rb,
            "page_boxes_match": cb == rb,
            "candidate_raster_sha256": cr["raster_sha256"],
            "reference_raster_sha256": rr["raster_sha256"],
            "diff": diff,
        })

    return {
        "receipt_version": "chaptera.fidelity-reference-pdf.v1",
        "candidate_sha256": file_sha256(candidate_path),
        "reference_sha256": file_sha256(reference_path),
        "renderer": {
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
        },
        "page_count": {
            "candidate": candidate.page_count,
            "reference": reference.page_count,
            "match": page_count_match,
        },
        "pages": pages,
        "limitations": [
            "Thresholded raster differences localize visible disagreement but do not prove authoring-semantic equivalence.",
            "Raw changed pixels and below-threshold noise are reported separately from significant differences; the threshold is a visual-noise fence, not a semantic oracle.",
            "The compact significant mask is tile-granular RLE for localization, not a pixel-perfect semantic mask.",
            "PDF conformance, PDF/X and object-level PDF semantics are outside this receipt.",
        ],
    }
