#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import struct
from typing import Any

import olefile

SCHEMA = "chaptera.viewer-typography-story-join.v1"
OLE_MAGIC = bytes.fromhex("d0cf11e0a1b11ae1")
EMU_PER_POINT = 12700

FIXED_BLOCK_LENGTHS = {
    0x78: 0,
    0x05: 0,
    0x08: 0,
    0x0A: 0,
    0x10: 2,
    0x12: 2,
    0x18: 2,
    0x1A: 2,
    0x07: 2,
    0x20: 4,
    0x22: 4,
    0x58: 4,
    0x68: 4,
    0x70: 4,
    0xB8: 4,
    0x28: 8,
    0x38: 16,
    0x48: 24,
}
VARIABLE_BLOCK_TYPES = {0xC0, 0x80, 0x82, 0x88, 0x8A, 0x90, 0x98, 0xA0}
GENERAL_CONTAINER = 0x88
FONT_INDEX_CONTAINER_ID = 0x24
TEXT_SIZE_ID = 0x0C


class DecodeError(RuntimeError):
    pass


def u16(data: bytes, offset: int) -> int:
    if offset < 0 or offset + 2 > len(data):
        raise DecodeError(f"u16 out of bounds at 0x{offset:x}")
    return struct.unpack_from("<H", data, offset)[0]


def u32(data: bytes, offset: int) -> int:
    if offset < 0 or offset + 4 > len(data):
        raise DecodeError(f"u32 out of bounds at 0x{offset:x}")
    return struct.unpack_from("<I", data, offset)[0]


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse_descriptors(data: bytes) -> list[dict[str, Any]]:
    current = 0x18
    seen: set[int] = set()
    descriptors: list[dict[str, Any]] = []
    while current != 0xFFFFFFFF:
        if current in seen:
            raise DecodeError(f"cycle in Quill descriptor list at 0x{current:x}")
        seen.add(current)
        marker = u16(data, current)
        count = u16(data, current + 2)
        next_offset = u32(data, current + 4)
        if marker != 0x18:
            raise DecodeError(f"unexpected descriptor marker 0x{marker:04x} at 0x{current:x}")
        cursor = current + 8
        for _ in range(count):
            if cursor + 24 > len(data):
                raise DecodeError("descriptor out of bounds")
            name = data[cursor + 2:cursor + 6].decode("ascii", errors="replace")
            descriptors.append(
                {
                    "ordinal": len(descriptors),
                    "descriptor_offset": cursor,
                    "name": name,
                    "option_a": u16(data, cursor + 6),
                    "option_b": u16(data, cursor + 8),
                    "option_c": u16(data, cursor + 10),
                    "bit_type": data[cursor + 12:cursor + 16].decode("ascii", errors="replace"),
                    "offset": u32(data, cursor + 16),
                    "length": u32(data, cursor + 20),
                }
            )
            cursor += 24
        current = next_offset
    for desc in descriptors:
        desc["end"] = int(desc["offset"]) + int(desc["length"])
        if desc["end"] > len(data):
            raise DecodeError(f"{desc['name']} descriptor exceeds Quill stream")
    return descriptors


def one_descriptor(descriptors: list[dict[str, Any]], name: str) -> dict[str, Any]:
    matches = [d for d in descriptors if d["name"] == name]
    if len(matches) != 1:
        raise DecodeError(f"expected one {name} descriptor, got {len(matches)}")
    return matches[0]


def parse_font_catalog(data: bytes, descriptors: list[dict[str, Any]]) -> list[str]:
    names: list[str] = []
    for desc in [d for d in descriptors if d["name"] == "FONT"]:
        start = int(desc["offset"])
        end = int(desc["end"])
        count = u32(data, start + 4)
        cursor = start + 20 + count * 4
        if cursor > end:
            raise DecodeError("FONT index area exceeds chunk")
        for _ in range(count):
            units = u16(data, cursor)
            cursor += 2
            byte_len = units * 2
            if cursor + byte_len + 4 > end:
                raise DecodeError("FONT record exceeds chunk")
            names.append(data[cursor:cursor + byte_len].decode("utf-16le", errors="strict"))
            cursor += byte_len + 4
        if cursor != end:
            raise DecodeError(f"FONT chunk does not close exactly: trailing={end-cursor}")
    if not names:
        raise DecodeError("no FONT records")
    return names


def parse_story_catalog(data: bytes, descriptors: list[dict[str, Any]]) -> dict[str, Any]:
    syid = one_descriptor(descriptors, "SYID")
    strs = one_descriptor(descriptors, "STRS")
    text = one_descriptor(descriptors, "TEXT")

    syid_start = int(syid["offset"])
    syid_count = u32(data, syid_start + 4)
    syids = [u32(data, syid_start + 8 + i * 4) for i in range(syid_count)]

    strs_start = int(strs["offset"])
    strs_count = u32(data, strs_start)
    service_span = u32(data, strs_start + 4)
    lengths_start = strs_start + 4 + service_span
    lengths = [u32(data, lengths_start + i * 4) for i in range(strs_count)]
    if syid_count != strs_count:
        raise DecodeError(f"SYID/STRS count mismatch {syid_count}/{strs_count}")

    text_start = int(text["offset"])
    text_len = int(text["length"])
    expected = sum(lengths) * 2
    if expected != text_len:
        raise DecodeError(f"TEXT length mismatch expected={expected} actual={text_len}")

    stories = []
    global_start = 0
    for index, (syid_value, units) in enumerate(zip(syids, lengths)):
        global_end = global_start + units
        stories.append(
            {
                "index": index,
                "syid": syid_value,
                "utf16_units": units,
                "global_start_utf16": global_start,
                "global_end_utf16": global_end,
            }
        )
        global_start = global_end

    return {
        "text_offset": text_start,
        "text_length": text_len,
        "text_end": text_start + text_len,
        "total_utf16_units": global_start,
        "stories": stories,
    }


def parse_block(data: bytes, cursor: int, limit: int) -> tuple[dict[str, Any], int]:
    if cursor + 2 > limit:
        raise DecodeError(f"block header exceeds limit at 0x{cursor:x}")
    start = cursor
    block_id = data[cursor]
    block_type = data[cursor + 1]
    cursor += 2
    data_offset = cursor
    if block_type in VARIABLE_BLOCK_TYPES:
        if cursor + 4 > limit:
            raise DecodeError("variable block length missing")
        data_length = u32(data, cursor)
        if data_length < 4:
            raise DecodeError("invalid variable block length")
        block_end = data_offset + data_length
        value = None
    else:
        data_length = FIXED_BLOCK_LENGTHS.get(block_type, 0)
        block_end = data_offset + data_length
        if data_length == 2:
            value = u16(data, data_offset)
        elif data_length == 4:
            value = u32(data, data_offset)
        else:
            value = None
    if block_end > limit:
        raise DecodeError(f"block at 0x{start:x} exceeds style")
    return {
        "start": start,
        "id": block_id,
        "type": block_type,
        "data_offset": data_offset,
        "data_length": data_length,
        "end": block_end,
        "data": value,
    }, block_end


def extract_font_index(data: bytes, block: dict[str, Any]) -> int | None:
    if int(block["type"]) not in VARIABLE_BLOCK_TYPES:
        return None
    cursor = int(block["data_offset"]) + 4
    end = int(block["end"])
    while cursor < end:
        child, cursor = parse_block(data, cursor, end)
        if int(child["type"]) == GENERAL_CONTAINER:
            inner = int(child["data_offset"]) + 4
            if inner >= int(child["end"]):
                return None
            value_block, _ = parse_block(data, inner, int(child["end"]))
            value = value_block.get("data")
            return value if isinstance(value, int) else None
    return None


def parse_fdpc_styles(data: bytes, descriptors: list[dict[str, Any]], font_names: list[str]) -> list[dict[str, Any]]:
    styles: list[dict[str, Any]] = []
    for desc in [d for d in descriptors if d["name"] == "FDPC"]:
        start = int(desc["offset"])
        end = int(desc["end"])
        count = u16(data, start)
        offsets_start = start + 8
        chunk_offsets_start = offsets_start + count * 4
        body_start = chunk_offsets_start + count * 2
        if body_start > end:
            raise DecodeError("FDPC tables exceed chunk")
        text_offsets = [u32(data, offsets_start + i * 4) for i in range(count)]
        chunk_offsets = [u16(data, chunk_offsets_start + i * 2) for i in range(count)]
        for ordinal, (text_offset, chunk_offset) in enumerate(zip(text_offsets, chunk_offsets)):
            style_start = start + chunk_offset
            style_length = u32(data, style_start)
            style_end = style_start + style_length
            if style_length < 4 or style_end > end:
                raise DecodeError("invalid FDPC style length")
            cursor = style_start + 4
            font_indices: list[int] = []
            size_emu: list[int] = []
            while cursor < style_end:
                block, cursor = parse_block(data, cursor, style_end)
                if int(block["id"]) == FONT_INDEX_CONTAINER_ID:
                    index = extract_font_index(data, block)
                    if index is not None:
                        font_indices.append(index)
                if int(block["id"]) == TEXT_SIZE_ID and isinstance(block.get("data"), int):
                    size_emu.append(int(block["data"]))
            styles.append(
                {
                    "descriptor_ordinal": desc["ordinal"],
                    "style_ordinal": ordinal,
                    "text_offset": text_offset,
                    "style_start": style_start,
                    "style_end": style_end,
                    "font_indices": font_indices,
                    "font_names": [font_names[i] for i in font_indices if 0 <= i < len(font_names)],
                    "text_size_emu": size_emu,
                    "text_size_points": [value / EMU_PER_POINT for value in size_emu],
                }
            )
    if not styles:
        raise DecodeError("no FDPC styles")
    return styles


def materialize_ranges(
    styles: list[dict[str, Any]],
    story_catalog: dict[str, Any],
) -> list[dict[str, Any]]:
    text_start = int(story_catalog["text_offset"])
    text_end = int(story_catalog["text_end"])
    endpoints: list[tuple[int, dict[str, Any]]] = []
    for style in styles:
        absolute = int(style["text_offset"])
        if not (text_start <= absolute <= text_end):
            raise DecodeError(
                f"FDPC text_offset 0x{absolute:x} outside TEXT [0x{text_start:x},0x{text_end:x}]"
            )
        delta = absolute - text_start
        if delta % 2:
            raise DecodeError(f"FDPC text_offset 0x{absolute:x} is not UTF-16 aligned")
        endpoints.append((delta // 2, style))
    if any(left[0] > right[0] for left, right in zip(endpoints, endpoints[1:])):
        raise DecodeError("FDPC endpoints regress in stored order")

    ranges = []
    previous = 0
    stories = story_catalog["stories"]
    for end_utf16, style in endpoints:
        if end_utf16 < previous:
            raise DecodeError("FDPC range endpoint regressed")
        if end_utf16 == previous:
            previous = end_utf16
            continue
        owners = []
        for story in stories:
            lo = max(previous, int(story["global_start_utf16"]))
            hi = min(end_utf16, int(story["global_end_utf16"]))
            if lo < hi:
                owners.append(
                    {
                        "story_index": story["index"],
                        "syid": story["syid"],
                        "global_start_utf16": lo,
                        "global_end_utf16": hi,
                        "story_start_utf16": lo - int(story["global_start_utf16"]),
                        "story_end_utf16": hi - int(story["global_start_utf16"]),
                    }
                )
        ranges.append(
            {
                "global_start_utf16": previous,
                "global_end_utf16": end_utf16,
                "descriptor_ordinal": style["descriptor_ordinal"],
                "style_ordinal": style["style_ordinal"],
                "font_indices": style["font_indices"],
                "font_names": style["font_names"],
                "text_size_emu": style["text_size_emu"],
                "text_size_points": style["text_size_points"],
                "story_intersections": owners,
            }
        )
        previous = end_utf16
    if previous != int(story_catalog["total_utf16_units"]):
        raise DecodeError(
            f"FDPC terminal closure mismatch {previous} != {story_catalog['total_utf16_units']}"
        )
    return ranges


def decode_pub(path: pathlib.Path, source_label: str) -> dict[str, Any]:
    raw = path.read_bytes()
    if not raw.startswith(OLE_MAGIC):
        raise DecodeError("not an OLE/CFB file")
    with olefile.OleFileIO(str(path)) as ole:
        quill_path = ["Quill", "QuillSub", "CONTENTS"]
        if not ole.exists(quill_path):
            raise DecodeError("missing Quill/QuillSub/CONTENTS")
        quill = ole.openstream(quill_path).read()

    descriptors = parse_descriptors(quill)
    font_names = parse_font_catalog(quill, descriptors)
    stories = parse_story_catalog(quill, descriptors)
    styles = parse_fdpc_styles(quill, descriptors, font_names)
    ranges = materialize_ranges(styles, stories)

    owned = []
    for row in ranges:
        if len(row["story_intersections"]) == 1 and row["font_names"] and row["text_size_points"]:
            owned.append(row)

    return {
        "schema": SCHEMA,
        "source": {
            "label": source_label,
            "file_name": path.name,
            "sha256": sha256_bytes(raw),
            "byte_len": len(raw),
        },
        "quill": {
            "sha256": sha256_bytes(quill),
            "byte_len": len(quill),
            "descriptor_count": len(descriptors),
        },
        "font_catalog": {
            "record_count": len(font_names),
            "names": font_names,
        },
        "story_catalog": stories,
        "fdpc": {
            "style_count": len(styles),
            "ranges": ranges,
            "terminal_exact": True,
            "owned_explicit_font_and_size_range_count": len(owned),
        },
        "privacy_boundary": {
            "raw_pub_retained": False,
            "raw_quill_payload_retained": False,
            "host_font_lookup_used": False,
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pub", type=pathlib.Path)
    ap.add_argument("--source-label", required=True)
    ap.add_argument("--output", type=pathlib.Path, required=True)
    args = ap.parse_args()
    result = decode_pub(args.pub, args.source_label)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({
        "source": result["source"],
        "stories": len(result["story_catalog"]["stories"]),
        "utf16_units": result["story_catalog"]["total_utf16_units"],
        "fdpc_ranges": len(result["fdpc"]["ranges"]),
        "owned_explicit_font_and_size_ranges": result["fdpc"]["owned_explicit_font_and_size_range_count"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
