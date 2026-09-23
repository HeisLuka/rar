#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import struct
from typing import Any

import olefile

SCHEMA = "chaptera.quill-font-discriminator.v1"
OLE_MAGIC = bytes.fromhex("d0cf11e0a1b11ae1")

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


def parse_quill_descriptors(data: bytes) -> list[dict[str, Any]]:
    current = 0x18
    seen: set[int] = set()
    descriptors: list[dict[str, Any]] = []
    list_ordinal = 0

    while current != 0xFFFFFFFF:
        if current in seen:
            raise DecodeError(f"cycle in Quill descriptor list at 0x{current:x}")
        seen.add(current)
        if current + 8 > len(data):
            raise DecodeError(f"descriptor list header out of bounds at 0x{current:x}")

        marker = u16(data, current)
        count = u16(data, current + 2)
        next_offset = u32(data, current + 4)
        cursor = current + 8

        for local_ordinal in range(count):
            if cursor + 24 > len(data):
                raise DecodeError(f"descriptor {len(descriptors)} out of bounds")
            header = u16(data, cursor)
            name = data[cursor + 2 : cursor + 6].decode("ascii", errors="replace")
            chunk_id = u16(data, cursor + 6)
            unknown = u32(data, cursor + 8)
            name2 = data[cursor + 12 : cursor + 16].decode("ascii", errors="replace")
            offset = u32(data, cursor + 16)
            length = u32(data, cursor + 20)
            if offset + length > len(data):
                raise DecodeError(
                    f"descriptor {len(descriptors)} {name!r} exceeds Quill stream: "
                    f"0x{offset:x}+0x{length:x} > 0x{len(data):x}"
                )
            descriptors.append(
                {
                    "ordinal": len(descriptors),
                    "list_ordinal": list_ordinal,
                    "local_ordinal": local_ordinal,
                    "descriptor_offset": cursor,
                    "header": header,
                    "name": name,
                    "id": chunk_id,
                    "unknown_u32": unknown,
                    "name2": name2,
                    "offset": offset,
                    "length": length,
                    "end": offset + length,
                }
            )
            cursor += 24

        list_ordinal += 1
        current = next_offset

    return descriptors


def parse_font_chunk(
    data: bytes, desc: dict[str, Any], global_base: int
) -> dict[str, Any]:
    start = int(desc["offset"])
    end = int(desc["end"])
    if start + 8 > end:
        raise DecodeError("FONT chunk is shorter than its fixed prefix")

    unknown = u32(data, start)
    count = u32(data, start + 4)
    index_table_start = start + 20
    records_start = index_table_start + count * 4
    if records_start > end:
        raise DecodeError(
            f"FONT index area exceeds chunk: records_start=0x{records_start:x}, end=0x{end:x}"
        )

    index_values = [u32(data, index_table_start + i * 4) for i in range(count)]
    cursor = records_start
    records: list[dict[str, Any]] = []

    for local_ordinal in range(count):
        record_start = cursor
        if cursor + 2 > end:
            raise DecodeError(f"FONT record {local_ordinal} missing name length")
        name_units = u16(data, cursor)
        cursor += 2
        name_bytes_len = name_units * 2
        if cursor + name_bytes_len + 4 > end:
            raise DecodeError(
                f"FONT record {local_ordinal} exceeds chunk boundary "
                f"(name_units={name_units})"
            )
        name_bytes = data[cursor : cursor + name_bytes_len]
        cursor += name_bytes_len
        try:
            name = name_bytes.decode("utf-16le", errors="strict")
        except UnicodeDecodeError as exc:
            raise DecodeError(f"FONT record {local_ordinal} invalid UTF-16LE: {exc}") from exc
        trailing_u32 = u32(data, cursor)
        cursor += 4
        records.append(
            {
                "local_ordinal": local_ordinal,
                "global_ordinal": global_base + local_ordinal,
                "start": record_start,
                "end": cursor,
                "name_utf16_units": name_units,
                "name": name,
                "trailing_u32": trailing_u32,
            }
        )

    return {
        "descriptor_ordinal": desc["ordinal"],
        "offset": start,
        "length": desc["length"],
        "end": end,
        "unknown_u32": unknown,
        "stored_count": count,
        "index_table_start": index_table_start,
        "index_values": index_values,
        "records_start": records_start,
        "records": records,
        "cursor_after_records": cursor,
        "terminal_exact": cursor == end,
        "trailing_bytes": end - cursor,
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
            raise DecodeError(f"variable block length missing at 0x{start:x}")
        data_length = u32(data, cursor)
        if data_length < 4:
            raise DecodeError(
                f"variable block at 0x{start:x} has invalid data_length={data_length}"
            )
        block_end = data_offset + data_length
        if block_end > limit:
            raise DecodeError(
                f"variable block at 0x{start:x} ends 0x{block_end:x} beyond 0x{limit:x}"
            )
        value = None
        cursor = block_end
    else:
        data_length = FIXED_BLOCK_LENGTHS.get(block_type, 0)
        block_end = data_offset + data_length
        if block_end > limit:
            raise DecodeError(
                f"fixed block at 0x{start:x} ends 0x{block_end:x} beyond 0x{limit:x}"
            )
        if data_length == 2:
            value = u16(data, data_offset)
        elif data_length == 4:
            value = u32(data, data_offset)
        else:
            value = None
        cursor = block_end

    return (
        {
            "start": start,
            "id": block_id,
            "type": block_type,
            "data_offset": data_offset,
            "data_length": data_length,
            "end": block_end,
            "data": value,
        },
        cursor,
    )


def extract_font_index(data: bytes, font_block: dict[str, Any]) -> dict[str, Any]:
    if int(font_block["type"]) not in VARIABLE_BLOCK_TYPES:
        return {"font_index": None, "reason": "font_index_container_not_variable"}

    outer_start = int(font_block["data_offset"]) + 4
    outer_end = int(font_block["end"])
    cursor = outer_start
    nested: list[dict[str, Any]] = []

    while cursor < outer_end:
        sub, _ = parse_block(data, cursor, outer_end)
        nested.append(sub)
        if int(sub["type"]) == GENERAL_CONTAINER:
            inner_start = int(sub["data_offset"]) + 4
            inner_end = int(sub["end"])
            if inner_start >= inner_end:
                return {
                    "font_index": None,
                    "reason": "general_container_empty",
                    "nested": nested,
                }
            subsub, _ = parse_block(data, inner_start, inner_end)
            return {
                "font_index": subsub["data"],
                "reason": "libmspub_general_container_first_child",
                "nested": nested,
                "value_block": subsub,
            }
        cursor = int(sub["end"])

    return {"font_index": None, "reason": "general_container_not_found", "nested": nested}


def parse_fdpc_chunk(
    data: bytes, desc: dict[str, Any], font_names: list[str]
) -> dict[str, Any]:
    start = int(desc["offset"])
    end = int(desc["end"])
    if start + 8 > end:
        raise DecodeError("FDPC chunk shorter than fixed prefix")

    count = u16(data, start)
    offsets_start = start + 8
    chunk_offsets_start = offsets_start + count * 4
    body_start = chunk_offsets_start + count * 2
    if body_start > end:
        raise DecodeError("FDPC tables exceed chunk boundary")

    text_offsets = [u32(data, offsets_start + i * 4) for i in range(count)]
    chunk_offsets = [u16(data, chunk_offsets_start + i * 2) for i in range(count)]
    styles: list[dict[str, Any]] = []
    out_of_range: list[int] = []

    for ordinal, chunk_offset in enumerate(chunk_offsets):
        style_start = start + chunk_offset
        if style_start + 4 > end:
            raise DecodeError(f"FDPC style {ordinal} starts out of bounds")
        style_length = u32(data, style_start)
        style_end = style_start + style_length
        if style_length < 4 or style_end > end:
            raise DecodeError(
                f"FDPC style {ordinal} invalid length {style_length} at 0x{style_start:x}"
            )

        cursor = style_start + 4
        blocks: list[dict[str, Any]] = []
        refs: list[dict[str, Any]] = []
        while cursor < style_end:
            block, _ = parse_block(data, cursor, style_end)
            blocks.append(block)
            if int(block["id"]) == FONT_INDEX_CONTAINER_ID:
                extracted = extract_font_index(data, block)
                index = extracted.get("font_index")
                joined_name = None
                in_range = False
                if isinstance(index, int):
                    in_range = 0 <= index < len(font_names)
                    if in_range:
                        joined_name = font_names[index]
                    else:
                        out_of_range.append(index)
                refs.append(
                    {
                        "font_index": index,
                        "in_range": in_range,
                        "font_name": joined_name,
                        "extraction_reason": extracted.get("reason"),
                    }
                )
            cursor = int(block["end"])

        styles.append(
            {
                "ordinal": ordinal,
                "text_offset": text_offsets[ordinal],
                "chunk_offset": chunk_offset,
                "style_start": style_start,
                "style_length": style_length,
                "style_end": style_end,
                "font_refs": refs,
                "block_count": len(blocks),
            }
        )

    refs = [ref for style in styles for ref in style["font_refs"]]
    return {
        "descriptor_ordinal": desc["ordinal"],
        "offset": start,
        "length": desc["length"],
        "end": end,
        "stored_count": count,
        "styles": styles,
        "font_reference_count": len(refs),
        "font_indices": [ref["font_index"] for ref in refs],
        "joined_names": [ref["font_name"] for ref in refs],
        "out_of_range_font_indices": sorted(set(out_of_range)),
    }


def decode_pub(path: pathlib.Path, source_label: str) -> dict[str, Any]:
    raw = path.read_bytes()
    if not raw.startswith(OLE_MAGIC):
        raise DecodeError(f"{path} is not an OLE/CFB file")

    with olefile.OleFileIO(str(path)) as ole:
        quill_path = ["Quill", "QuillSub", "CONTENTS"]
        if not ole.exists(quill_path):
            raise DecodeError("missing Quill/QuillSub/CONTENTS")
        quill = ole.openstream(quill_path).read()

    descriptors = parse_quill_descriptors(quill)
    font_descs = [d for d in descriptors if d["name"] == "FONT"]
    fdpc_descs = [d for d in descriptors if d["name"] == "FDPC"]
    if not font_descs:
        raise DecodeError("no FONT descriptor")
    if not fdpc_descs:
        raise DecodeError("no FDPC descriptor")

    font_chunks: list[dict[str, Any]] = []
    font_names: list[str] = []
    global_base = 0
    for desc in font_descs:
        chunk = parse_font_chunk(quill, desc, global_base)
        font_chunks.append(chunk)
        font_names.extend(record["name"] for record in chunk["records"])
        global_base += int(chunk["stored_count"])

    fdpc_chunks = [parse_fdpc_chunk(quill, desc, font_names) for desc in fdpc_descs]
    all_refs = [idx for chunk in fdpc_chunks for idx in chunk["font_indices"]]
    all_joined = [name for chunk in fdpc_chunks for name in chunk["joined_names"]]
    out_of_range = sorted(
        {
            idx
            for chunk in fdpc_chunks
            for idx in chunk["out_of_range_font_indices"]
        }
    )

    return {
        "schema": SCHEMA,
        "source": {
            "label": source_label,
            "file_name": path.name,
            "sha256": sha256_bytes(raw),
            "byte_len": len(raw),
            "ole_magic_ok": True,
        },
        "quill": {
            "sha256": sha256_bytes(quill),
            "byte_len": len(quill),
            "descriptor_count": len(descriptors),
            "font_descriptor_count": len(font_descs),
            "fdpc_descriptor_count": len(fdpc_descs),
        },
        "font": {
            "total_record_count": len(font_names),
            "names": font_names,
            "chunks": font_chunks,
            "all_chunks_terminal_exact": all(c["terminal_exact"] for c in font_chunks),
        },
        "fdpc": {
            "chunks": fdpc_chunks,
            "font_reference_count": len(all_refs),
            "font_indices": all_refs,
            "joined_names": all_joined,
            "all_indices_in_range": not out_of_range,
            "out_of_range_font_indices": out_of_range,
        },
        "provenance": {
            "grammar_source": "LibreOffice/libmspub@2b094242f97923f918972d5f1bfd26cd915f836c",
            "grammar_functions": [
                "MSPUBParser::parseQuillChunkReference",
                "MSPUBParser::parseFonts",
                "MSPUBParser::parseCharacterStyles",
                "MSPUBParser::getCharacterStyle",
                "MSPUBParser::getFontIndex",
                "MSPUBParser::parseBlock",
            ],
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
    rendered = json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    args.output.write_text(rendered, encoding="utf-8")
    print(
        json.dumps(
            {
                "source": result["source"],
                "font_records": result["font"]["total_record_count"],
                "font_terminal_exact": result["font"]["all_chunks_terminal_exact"],
                "fdpc_font_refs": result["fdpc"]["font_reference_count"],
                "fdpc_all_indices_in_range": result["fdpc"]["all_indices_in_range"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
