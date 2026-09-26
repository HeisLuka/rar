#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import pathlib

SCHEMA = "chaptera.viewer-typography-story-join-receipt.v1"


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def find_anchor(doc, syid: int, start: int, end: int, font: str, points: float):
    matches = []
    for row in doc["fdpc"]["ranges"]:
        if font not in row.get("font_names", []):
            continue
        if not any(abs(float(value) - points) < 1e-9 for value in row.get("text_size_points", [])):
            continue
        for owner in row.get("story_intersections", []):
            if (
                int(owner["syid"]) == syid
                and int(owner["story_start_utf16"]) == start
                and int(owner["story_end_utf16"]) == end
            ):
                matches.append({
                    "syid": syid,
                    "story_start_utf16": start,
                    "story_end_utf16": end,
                    "font": font,
                    "points": points,
                    "descriptor_ordinal": row["descriptor_ordinal"],
                    "style_ordinal": row["style_ordinal"],
                })
    if len(matches) != 1:
        raise SystemExit(
            f"expected exactly one anchor SYID={syid} [{start},{end}) {font} {points}pt, got {len(matches)}"
        )
    return matches[0]


def require_stats(doc, label: str, stories: int, units: int, ranges: int, explicit: int):
    actual_stories = len(doc["story_catalog"]["stories"])
    actual_units = int(doc["story_catalog"]["total_utf16_units"])
    actual_ranges = len(doc["fdpc"]["ranges"])
    actual_explicit = int(doc["fdpc"]["owned_explicit_font_and_size_range_count"])
    if (actual_stories, actual_units, actual_ranges, actual_explicit) != (
        stories, units, ranges, explicit
    ):
        raise SystemExit(
            f"{label} stats mismatch: got stories={actual_stories} units={actual_units} "
            f"ranges={actual_ranges} explicit={actual_explicit}; expected "
            f"{stories}/{units}/{ranges}/{explicit}"
        )
    if doc["fdpc"]["terminal_exact"] is not True:
        raise SystemExit(f"{label} FDPC terminal closure is not exact")
    return {
        "stories": actual_stories,
        "utf16_units": actual_units,
        "fdpc_ranges": actual_ranges,
        "explicit_font_and_size_ranges": actual_explicit,
        "terminal_exact": True,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--newsletter", type=pathlib.Path, required=True)
    ap.add_argument("--brochure", type=pathlib.Path, required=True)
    ap.add_argument("--output", type=pathlib.Path, required=True)
    args = ap.parse_args()

    newsletter = load(args.newsletter)
    brochure = load(args.brochure)

    newsletter_stats = require_stats(newsletter, "SampleNewsletter", 44, 9364, 43, 11)
    brochure_stats = require_stats(brochure, "SampleBrochure", 20, 3826, 36, 6)

    anchors = [
        find_anchor(newsletter, 75, 0, 14, "Rockwell Condensed", 24.0),
        find_anchor(newsletter, 59, 0, 25, "Rockwell Condensed", 24.0),
        find_anchor(newsletter, 7, 0, 25, "Rockwell Condensed", 24.0),
    ]

    receipt = {
        "schema": SCHEMA,
        "result": "pass",
        "claim": "explicit_fdpc_ranges_have_bounded_story_local_font_and_size_ownership",
        "sample_newsletter": {
            "source": newsletter["source"],
            "stats": newsletter_stats,
            "anchors": anchors,
        },
        "sample_brochure": {
            "source": brochure["source"],
            "stats": brochure_stats,
        },
        "projection_boundary": {
            "explicit_fdpc_subset_proven": True,
            "default_inherited_typography_proven": False,
            "host_font_lookup_used": False,
            "publisher_exact_line_breaks_claimed": False,
        },
    }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(receipt, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
