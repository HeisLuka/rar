#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import pathlib

SCHEMA = "chaptera.viewer-typography-preflight-receipt.v1"


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--decoded", type=pathlib.Path, required=True)
    ap.add_argument("--output", type=pathlib.Path, required=True)
    ap.add_argument("--expected-sha256", required=True)
    args = ap.parse_args()

    doc = load(args.decoded)
    source = doc["source"]
    if source["sha256"] != args.expected_sha256:
        raise SystemExit(
            f"source SHA mismatch: expected {args.expected_sha256}, got {source['sha256']}"
        )

    fonts = doc["font"]
    fdpc = doc["fdpc"]
    if fonts["total_record_count"] <= 0:
        raise SystemExit("no Quill FONT records")
    if fonts["all_chunks_terminal_exact"] is not True:
        raise SystemExit("Quill FONT record stream does not terminate exactly")
    if fdpc["font_reference_count"] <= 0:
        raise SystemExit("no FDPC font references")
    if fdpc["all_indices_in_range"] is not True:
        raise SystemExit(
            f"out-of-range FDPC font indices: {fdpc['out_of_range_font_indices']}"
        )

    ordinal_names = [
        {"ordinal": ordinal, "name": name}
        for ordinal, name in enumerate(fonts["names"])
    ]
    fdpc_pairs = sorted(
        {
            (int(index), str(name))
            for index, name in zip(fdpc["font_indices"], fdpc["joined_names"])
            if isinstance(index, int) and name
        }
    )

    receipt = {
        "schema": SCHEMA,
        "claim": "quill_font_catalog_and_fdpc_ordinal_join_only_not_viewer_typography_projection",
        "result": "pass",
        "source": {
            "kind": "pinned_public_real_pub",
            "upstream_repository": "apache/poi",
            "git_blob": "94900925af5832c493784f3cb51563f838a64df8",
            "file_name": source["file_name"],
            "sha256": source["sha256"],
            "byte_len": source["byte_len"],
        },
        "quill_font_catalog": {
            "record_count": fonts["total_record_count"],
            "terminal_exact": fonts["all_chunks_terminal_exact"],
            "ordinal_names": ordinal_names,
        },
        "fdpc_font_join": {
            "reference_count": fdpc["font_reference_count"],
            "all_indices_in_range": fdpc["all_indices_in_range"],
            "distinct_ordinal_name_pairs": [
                {"ordinal": ordinal, "name": name} for ordinal, name in fdpc_pairs
            ],
        },
        "viewer_projection": {
            "story_or_run_font_ownership_proven": False,
            "default_format_font_ownership_proven": False,
            "default_format_size_semantics_proven": False,
            "painted_single_frame_story_typography_coverage": None,
            "coverage_state": "not_computable_without_proven_ownership_join",
        },
        "next_discriminators": [
            "prove which Quill FDPC/default-format record owns a specific Story/run or document default",
            "prove bounded font-size semantics for that owned record",
            "only then compute painted single-frame Story source-typography coverage",
        ],
        "privacy_boundary": {
            "raw_pub_retained": False,
            "raw_quill_payload_retained": False,
            "private_reader_source_imported": False,
            "host_font_lookup_used": False,
        },
        "full_viewer_typography_projection_done": False,
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
