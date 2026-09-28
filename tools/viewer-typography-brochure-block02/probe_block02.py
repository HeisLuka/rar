#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
from typing import Any

HERE = pathlib.Path(__file__).resolve().parent
DONOR = HERE.parent / "viewer-typography-story-join" / "decode_quill_story_typography.py"

spec = importlib.util.spec_from_file_location("story_join_decoder", DONOR)
if spec is None or spec.loader is None:
    raise RuntimeError(f"cannot load donor {DONOR}")
donor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(donor)

TARGET_TYPE = 0x02
MAX_CANDIDATE_WIDTH = 64


class ProbeError(RuntimeError):
    pass


def parse_descriptors_strict(data: bytes) -> list[dict[str, Any]]:
    """Parse Quill descriptor-list framing without the stale service==0x0018 bug.

    The list node is service:u16 + count:u16 + next:u32. 0x0018 belongs to
    each following 24-byte descriptor as its presence marker; real Publisher
    fixtures commonly carry service=0x01f8 at the root.
    """
    current = 0x18
    seen: set[int] = set()
    descriptors: list[dict[str, Any]] = []
    while current != 0xFFFFFFFF:
        if current in seen:
            raise ProbeError(f"cycle in Quill descriptor list at 0x{current:x}")
        seen.add(current)
        if current + 8 > len(data):
            raise ProbeError(f"descriptor-list node exceeds Quill at 0x{current:x}")
        service = donor.u16(data, current)
        count = donor.u16(data, current + 2)
        next_offset = donor.u32(data, current + 4)
        cursor = current + 8
        for _ in range(count):
            if cursor + 24 > len(data):
                raise ProbeError("descriptor out of bounds")
            presence_marker = donor.u16(data, cursor)
            if presence_marker != 0x18:
                raise ProbeError(
                    f"unexpected descriptor presence marker 0x{presence_marker:04x} "
                    f"at 0x{cursor:x}"
                )
            name = data[cursor + 2:cursor + 6].decode("ascii", errors="replace")
            offset = donor.u32(data, cursor + 16)
            length = donor.u32(data, cursor + 20)
            descriptors.append(
                {
                    "ordinal": len(descriptors),
                    "descriptor_offset": cursor,
                    "service": service,
                    "name": name,
                    "option_a": donor.u16(data, cursor + 6),
                    "option_b": donor.u16(data, cursor + 8),
                    "option_c": donor.u16(data, cursor + 10),
                    "bit_type": data[cursor + 12:cursor + 16].decode(
                        "ascii", errors="replace"
                    ),
                    "offset": offset,
                    "length": length,
                    "end": offset + length,
                }
            )
            cursor += 24
        current = next_offset

    for desc in descriptors:
        if int(desc["end"]) > len(data):
            raise ProbeError(f"{desc['name']} descriptor exceeds Quill stream")
    return descriptors


def _hex_window(data: bytes, offset: int, before: int = 8, after: int = 24) -> dict[str, Any]:
    start = max(0, offset - before)
    end = min(len(data), offset + after)
    return {
        "start": start,
        "end": end,
        "hex": data[start:end].hex(" "),
    }


def parse_one(
    data: bytes,
    cursor: int,
    limit: int,
    width02: int,
    observations: list[dict[str, Any]],
    context: str,
) -> tuple[dict[str, Any], int]:
    if cursor + 2 > limit:
        raise ProbeError(f"{context}: block header exceeds limit at 0x{cursor:x}")
    start = cursor
    block_id = data[cursor]
    block_type = data[cursor + 1]
    cursor += 2
    data_offset = cursor

    if block_type in donor.VARIABLE_BLOCK_TYPES:
        if cursor + 4 > limit:
            raise ProbeError(f"{context}: variable length missing at 0x{start:x}")
        data_length = donor.u32(data, cursor)
        if data_length < 4:
            raise ProbeError(f"{context}: invalid variable length {data_length} at 0x{start:x}")
        block_end = data_offset + data_length
        value = None
    elif block_type == TARGET_TYPE:
        data_length = width02
        block_end = data_offset + data_length
        value = None
        observations.append(
            {
                "offset": start,
                "id": block_id,
                "type": block_type,
                "candidate_payload_width": width02,
                "context": context,
                "style_end": limit,
                "payload_hex": data[data_offset:block_end].hex(" "),
                "window": _hex_window(data, start),
            }
        )
    elif block_type in donor.FIXED_BLOCK_LENGTHS:
        data_length = donor.FIXED_BLOCK_LENGTHS[block_type]
        block_end = data_offset + data_length
        if data_length == 2:
            value = donor.u16(data, data_offset)
        elif data_length == 4:
            value = donor.u32(data, data_offset)
        else:
            value = None
    else:
        raise ProbeError(
            f"{context}: other unknown fixed type 0x{block_type:02x} at 0x{start:x}"
        )

    if block_end > limit:
        raise ProbeError(
            f"{context}: block id=0x{block_id:02x} type=0x{block_type:02x} "
            f"at 0x{start:x} ends 0x{block_end:x} past 0x{limit:x}"
        )
    return {
        "start": start,
        "id": block_id,
        "type": block_type,
        "data_offset": data_offset,
        "data_length": data_length,
        "end": block_end,
        "value": value,
    }, block_end


def validate_font_container(
    data: bytes,
    block: dict[str, Any],
    width02: int,
    observations: list[dict[str, Any]],
    context: str,
) -> None:
    if block["type"] not in donor.VARIABLE_BLOCK_TYPES:
        return
    cursor = block["data_offset"] + 4
    end = block["end"]
    while cursor < end:
        child, cursor = parse_one(
            data,
            cursor,
            end,
            width02,
            observations,
            context + "/font-child",
        )
        if child["type"] == donor.GENERAL_CONTAINER:
            inner = child["data_offset"] + 4
            if inner < child["end"]:
                value_block, _ = parse_one(
                    data,
                    inner,
                    child["end"],
                    width02,
                    observations,
                    context + "/font-value",
                )
                if value_block["end"] > child["end"]:
                    raise ProbeError(f"{context}: nested font value overruns container")
    if cursor != end:
        raise ProbeError(f"{context}: font container does not close exactly")


def fdpc_style_spans(data: bytes, descriptors: list[dict[str, Any]]) -> list[dict[str, int]]:
    spans: list[dict[str, int]] = []
    for desc in [d for d in descriptors if d["name"] == "FDPC"]:
        start = int(desc["offset"])
        end = int(desc["end"])
        count = donor.u16(data, start)
        offsets_start = start + 8
        chunk_offsets_start = offsets_start + count * 4
        body_start = chunk_offsets_start + count * 2
        if body_start > end:
            raise ProbeError("FDPC tables exceed chunk")
        text_offsets = [donor.u32(data, offsets_start + i * 4) for i in range(count)]
        chunk_offsets = [donor.u16(data, chunk_offsets_start + i * 2) for i in range(count)]
        for ordinal, (text_offset, rel) in enumerate(zip(text_offsets, chunk_offsets)):
            style_start = start + rel
            if style_start < body_start or style_start + 4 > end:
                raise ProbeError(
                    f"FDPC descriptor {desc['ordinal']} style {ordinal} points outside body"
                )
            style_len = donor.u32(data, style_start)
            style_end = style_start + style_len
            if style_len < 4 or style_end > end:
                raise ProbeError(
                    f"FDPC descriptor {desc['ordinal']} style {ordinal} invalid length"
                )
            spans.append(
                {
                    "descriptor_ordinal": int(desc["ordinal"]),
                    "style_ordinal": ordinal,
                    "text_offset": int(text_offset),
                    "start": style_start,
                    "end": style_end,
                }
            )
    if not spans:
        raise ProbeError("no FDPC styles")
    return spans


def validate_candidate(
    data: bytes,
    spans: list[dict[str, int]],
    width02: int,
) -> dict[str, Any]:
    observations: list[dict[str, Any]] = []
    styles_with_target = 0
    for span in spans:
        cursor = span["start"] + 4
        style_observation_count = len(observations)
        context = (
            f"FDPC[d={span['descriptor_ordinal']},s={span['style_ordinal']},"
            f"text=0x{span['text_offset']:x}]"
        )
        while cursor < span["end"]:
            block, cursor = parse_one(
                data,
                cursor,
                span["end"],
                width02,
                observations,
                context,
            )
            if block["id"] == donor.FONT_INDEX_CONTAINER_ID:
                validate_font_container(
                    data,
                    block,
                    width02,
                    observations,
                    context,
                )
        if cursor != span["end"]:
            raise ProbeError(f"{context}: style does not close exactly")
        if len(observations) > style_observation_count:
            styles_with_target += 1

    if not observations:
        raise ProbeError("candidate parse saw no type 0x02 occurrences")

    unique_offsets = sorted({row["offset"] for row in observations})
    followers: list[dict[str, Any]] = []
    for row in observations:
        follower_start = int(row["offset"]) + 2 + width02
        style_end = int(row["style_end"])
        if follower_start >= style_end:
            continue
        scratch: list[dict[str, Any]] = []
        try:
            follower, _ = parse_one(
                data,
                follower_start,
                style_end,
                width02,
                scratch,
                str(row["context"]) + "/candidate-follower",
            )
        except ProbeError as exc:
            followers.append(
                {
                    "target_offset": row["offset"],
                    "parse_error": str(exc),
                }
            )
            continue
        followers.append(
            {
                "target_offset": row["offset"],
                "start": follower["start"],
                "id": follower["id"],
                "type": follower["type"],
                "data_length": follower["data_length"],
                "value": follower["value"],
            }
        )

    return {
        "width": width02,
        "occurrence_count": len(observations),
        "unique_offset_count": len(unique_offsets),
        "styles_with_target": styles_with_target,
        "offsets": unique_offsets,
        "observations": observations,
        "followers": followers,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pub", type=pathlib.Path)
    ap.add_argument("--output", type=pathlib.Path, required=True)
    args = ap.parse_args()

    raw = args.pub.read_bytes()
    if not raw.startswith(donor.OLE_MAGIC):
        raise SystemExit("not OLE/CFB")

    with donor.olefile.OleFileIO(str(args.pub)) as ole:
        path = ["Quill", "QuillSub", "CONTENTS"]
        if not ole.exists(path):
            raise SystemExit("missing Quill/QuillSub/CONTENTS")
        quill = ole.openstream(path).read()

    descriptors = parse_descriptors_strict(quill)
    story_catalog = donor.parse_story_catalog(quill, descriptors)
    spans = fdpc_style_spans(quill, descriptors)

    candidates: list[dict[str, Any]] = []
    failures: dict[str, str] = {}
    for width in range(MAX_CANDIDATE_WIDTH + 1):
        try:
            candidates.append(validate_candidate(quill, spans, width))
        except ProbeError as exc:
            failures[str(width)] = str(exc)

    result = {
        "schema": "chaptera.viewer-typography-brochure-block02-probe.v1",
        "source": {
            "file_name": args.pub.name,
            "sha256": donor.sha256_bytes(raw),
            "byte_len": len(raw),
        },
        "quill": {
            "sha256": donor.sha256_bytes(quill),
            "byte_len": len(quill),
            "descriptor_count": len(descriptors),
        },
        "story_count": len(story_catalog["stories"]),
        "fdpc_style_count": len(spans),
        "target_fixed_type": TARGET_TYPE,
        "candidate_width_range": [0, MAX_CANDIDATE_WIDTH],
        "valid_candidates": candidates,
        "valid_widths": [row["width"] for row in candidates],
        "unique_width_proven": len(candidates) == 1,
        "failed_candidate_count": len(failures),
    }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({
        "source": result["source"],
        "fdpc_styles": len(spans),
        "valid_widths": result["valid_widths"],
        "unique_width_proven": result["unique_width_proven"],
        "candidate_summaries": [
            {
                "width": row["width"],
                "occurrences": row["occurrence_count"],
                "styles": row["styles_with_target"],
                "offsets": [hex(value) for value in row["offsets"]],
                "followers": [
                    {
                        "target_offset": hex(int(f["target_offset"])),
                        "id": (
                            hex(int(f["id"]))
                            if "id" in f
                            else None
                        ),
                        "type": (
                            hex(int(f["type"]))
                            if "type" in f
                            else None
                        ),
                        "value": f.get("value"),
                        "parse_error": f.get("parse_error"),
                    }
                    for f in row["followers"]
                ],
            }
            for row in candidates
        ],
    }, sort_keys=True))
    return 0 if candidates else 2


if __name__ == "__main__":
    raise SystemExit(main())
