#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import pathlib
from typing import Any

SCHEMA = "chaptera.viewer-typography-projection-receipt.v1"


def load(path: pathlib.Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise SystemExit(f"{path}: expected JSON object")
    return value


def summarize_default(item: dict[str, Any]) -> dict[str, Any]:
    refs = [
        {
            "script_slot_id": ref["script_slot_id"],
            "font_index": ref["font_index"],
            "font_name": ref["font_name"],
        }
        for ref in item["font_refs"]
        if ref["in_range"] and isinstance(ref["font_index"], int)
    ]
    return {
        "logical_style_index": item["logical_style_index"],
        "record_ordinal": item["record_ordinal"],
        "text_size_emu": item["text_size_emu"],
        "text_size_pt": item["text_size_pt"],
        "bidi_text_size_emu": item["bidi_text_size_emu"],
        "bidi_text_size_pt": item["bidi_text_size_pt"],
        "font_refs": refs,
    }


def summarize_source(doc: dict[str, Any]) -> dict[str, Any]:
    if doc.get("schema") != "chaptera.viewer-typography-quill-default-observation.v1":
        raise SystemExit("unexpected observation schema")
    source = doc["source"]
    fonts = doc["font"]
    fdpc = doc["fdpc"]
    defaults = doc["stsh1_defaults"]

    if fonts["total_record_count"] <= 0:
        raise SystemExit(f"{source['file_name']}: no Quill FONT records")
    if fonts["all_chunks_terminal_exact"] is not True:
        raise SystemExit(f"{source['file_name']}: FONT stream is not terminal-exact")
    if fdpc["font_reference_count"] <= 0:
        raise SystemExit(f"{source['file_name']}: no FDPC font references")
    if fdpc["all_indices_in_range"] is not True:
        raise SystemExit(
            f"{source['file_name']}: out-of-range FDPC font indices "
            f"{fdpc['out_of_range_font_indices']}"
        )
    if defaults["all_font_indices_in_range"] is not True:
        raise SystemExit(
            f"{source['file_name']}: out-of-range STSH1 default font indices "
            f"{defaults['out_of_range_font_indices']}"
        )

    default_rows = [
        summarize_default(item)
        for item in defaults["character_defaults"]
        if item["font_refs"]
        or item["text_size_emu"] is not None
        or item["bidi_text_size_emu"] is not None
    ]
    if not default_rows:
        raise SystemExit(f"{source['file_name']}: no observable STSH1 default typography")

    return {
        "source": {
            "kind": "pinned_public_real_pub",
            "file_name": source["file_name"],
            "git_blob": source["git_blob"],
            "sha256": source["sha256"],
            "byte_len": source["byte_len"],
        },
        "quill_font_catalog": {
            "record_count": fonts["total_record_count"],
            "ordinal_names": [
                {"ordinal": ordinal, "name": name}
                for ordinal, name in enumerate(fonts["names"])
            ],
        },
        "fdpc_observation": {
            "font_reference_count": fdpc["font_reference_count"],
            "all_indices_in_range": fdpc["all_indices_in_range"],
            "ownership_claim": "aggregate_character_style_observation_only",
        },
        "stsh1_default_character_typography": default_rows,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--decoded", type=pathlib.Path, action="append", required=True)
    ap.add_argument("--output", type=pathlib.Path, required=True)
    args = ap.parse_args()

    sources = [summarize_source(load(path)) for path in args.decoded]
    if len(sources) < 2:
        raise SystemExit("projection receipt requires at least two pinned real PUB fixtures")

    receipt = {
        "schema": SCHEMA,
        "task": "VIEWER-TYPOGRAPHY-PROJECTION-01",
        "claim": "bounded_quill_font_and_default_format_observation_without_unproven_story_ownership",
        "result": "pass",
        "fixtures": sources,
        "viewer_projection": {
            "story_or_run_font_ownership_proven": False,
            "default_format_catalog_observed": True,
            "default_format_size_semantics_observed": True,
            "painted_single_frame_story_count": None,
            "painted_single_frame_story_with_proven_typography_count": 0,
            "painted_single_frame_story_typography_coverage": None,
            "coverage_state": "ambiguous_not_computable_until_story_or_run_style_ownership_join_is_proven",
            "font_identity_projected_to_viewer": False,
            "font_size_projected_to_viewer": False,
            "host_font_lookup_used": False,
            "host_font_substitution_used": False,
        },
        "next_join_discriminator": {
            "question": (
                "which bounded FDPC/default-style record owns each concrete Story/run range "
                "across the aggregate Quill TEXT partition"
            ),
            "required_proof": [
                "bind FDPC/BTEC character boundaries to the same aggregate TEXT coordinate system used by SYID+STRS",
                "split those proven global ranges at Story boundaries without off-by-one/terminal-CR inference",
                "show at least one pinned real single-frame Story whose complete painted range has unambiguous FONT index and size ownership",
            ],
            "stop_rule": (
                "do not assign a font family or size to Viewer Story/run objects until that join is reproduced"
            ),
        },
        "privacy_boundary": {
            "raw_pub_retained": False,
            "raw_quill_payload_retained": False,
            "private_reader_source_imported": False,
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
