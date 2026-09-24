#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
from typing import Any

import olefile

ROOT = pathlib.Path(__file__).resolve().parent
DONOR_PATH = ROOT / "decode_quill_typography.py"
EMUS_PER_POINT = 12700
GENERAL_CONTAINER = 0x88
FONT_INDEX_CONTAINER_ID = 0x24
TEXT_SIZE_ID = 0x0C
BIDI_TEXT_SIZE_ID = 0x39


class DefaultDecodeError(RuntimeError):
    pass


def load_donor():
    spec = importlib.util.spec_from_file_location("viewer_typography_quill_donor", DONOR_PATH)
    if spec is None or spec.loader is None:
        raise DefaultDecodeError("cannot load bounded Quill donor decoder")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def parse_font_map(donor, data: bytes, block: dict[str, Any], font_names: list[str]) -> list[dict[str, Any]]:
    if int(block["type"]) not in donor.VARIABLE_BLOCK_TYPES:
        raise DefaultDecodeError("STSH1 font map is not a variable Quill block")

    cursor = int(block["data_offset"]) + 4
    limit = int(block["end"])
    entries: list[dict[str, Any]] = []
    while cursor < limit:
        sub, _ = donor.parse_block(data, cursor, limit)
        if int(sub["type"]) == GENERAL_CONTAINER:
            inner = int(sub["data_offset"]) + 4
            inner_limit = int(sub["end"])
            if inner >= inner_limit:
                raise DefaultDecodeError("empty STSH1 script font container")
            value_block, _ = donor.parse_block(data, inner, inner_limit)
            index = value_block["data"]
            if not isinstance(index, int):
                raise DefaultDecodeError("STSH1 script font reference is not scalar")
            in_range = 0 <= index < len(font_names)
            entries.append(
                {
                    "script_slot_id": sub["id"],
                    "font_index": index,
                    "in_range": in_range,
                    "font_name": font_names[index] if in_range else None,
                    "value_block_id": value_block["id"],
                    "value_block_type": value_block["type"],
                }
            )
        cursor = int(sub["end"])
    return entries


def parse_character_default(donor, data: bytes, style_start: int, limit: int, font_names: list[str]) -> dict[str, Any]:
    style_length = donor.u32(data, style_start)
    if style_length < 4:
        raise DefaultDecodeError(f"invalid STSH1 character style length {style_length}")
    style_end = style_start + style_length
    if style_end > limit:
        raise DefaultDecodeError("STSH1 character style exceeds chunk boundary")

    cursor = style_start + 4
    text_size_emu = None
    bidi_text_size_emu = None
    font_refs: list[dict[str, Any]] = []
    block_count = 0
    while cursor < style_end:
        block, _ = donor.parse_block(data, cursor, style_end)
        block_count += 1
        if int(block["id"]) == TEXT_SIZE_ID and isinstance(block["data"], int):
            text_size_emu = block["data"]
        elif int(block["id"]) == BIDI_TEXT_SIZE_ID and isinstance(block["data"], int):
            bidi_text_size_emu = block["data"]
        elif int(block["id"]) == FONT_INDEX_CONTAINER_ID:
            font_refs.extend(parse_font_map(donor, data, block, font_names))
        cursor = int(block["end"])

    return {
        "style_start": style_start,
        "style_length": style_length,
        "style_end": style_end,
        "block_count": block_count,
        "text_size_emu": text_size_emu,
        "text_size_pt": round(text_size_emu / EMUS_PER_POINT, 6) if isinstance(text_size_emu, int) else None,
        "bidi_text_size_emu": bidi_text_size_emu,
        "bidi_text_size_pt": round(bidi_text_size_emu / EMUS_PER_POINT, 6) if isinstance(bidi_text_size_emu, int) else None,
        "font_refs": font_refs,
    }


def add_stsh1_defaults(path: pathlib.Path, source_label: str, source_git_blob: str | None) -> dict[str, Any]:
    donor = load_donor()
    result = donor.decode_pub(path, source_label)
    result["schema"] = "chaptera.viewer-typography-quill-default-observation.v1"
    result["source"]["git_blob"] = source_git_blob

    with olefile.OleFileIO(str(path)) as ole:
        quill = ole.openstream(["Quill", "QuillSub", "CONTENTS"]).read()

    descriptors = donor.parse_quill_descriptors(quill)
    stsh = [d for d in descriptors if d["name"] == "STSH"]
    if len(stsh) < 2:
        raise DefaultDecodeError("fewer than two STSH descriptors")
    desc = stsh[1]
    start = int(desc["offset"])
    end = int(desc["end"])
    if start + 20 > end:
        raise DefaultDecodeError("STSH1 shorter than fixed prefix")

    count = donor.u32(quill, start + 4)
    offsets_start = start + 20
    offsets_end = offsets_start + count * 4
    if offsets_end > end:
        raise DefaultDecodeError("STSH1 offset table exceeds chunk boundary")
    offsets = [donor.u32(quill, offsets_start + i * 4) for i in range(count)]
    font_names = list(result["font"]["names"])

    defaults: list[dict[str, Any]] = []
    for ordinal in range(0, count, 2):
        record_start = start + 20 + offsets[ordinal]
        if record_start + 2 > end:
            raise DefaultDecodeError(f"STSH1 record {ordinal} prefix out of bounds")
        prefix = donor.u16(quill, record_start)
        parsed = parse_character_default(donor, quill, record_start + 2, end, font_names)
        defaults.append(
            {
                "record_ordinal": ordinal,
                "logical_style_index": ordinal // 2,
                "record_offset": record_start,
                "record_prefix_u16": prefix,
                **parsed,
            }
        )

    refs = [ref for item in defaults for ref in item["font_refs"]]
    out_of_range = sorted({ref["font_index"] for ref in refs if not ref["in_range"]})
    result["stsh1_defaults"] = {
        "descriptor_ordinal": desc["ordinal"],
        "descriptor_id": desc["id"],
        "offset": start,
        "length": desc["length"],
        "stored_record_count": count,
        "character_defaults": defaults,
        "font_reference_count": len(refs),
        "all_font_indices_in_range": not out_of_range,
        "out_of_range_font_indices": out_of_range,
    }
    result["provenance"]["default_format_extension"] = {
        "grammar": "second STSH descriptor; even records are default character styles",
        "text_size": "0x0C scalar in EMU; 12700 EMU per point",
        "font_map": "0x24 nested script-slot to Quill FONT ordinal",
        "ownership_boundary": "catalog/default observation only; no Story/run assignment",
    }
    return result


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pub", type=pathlib.Path)
    ap.add_argument("--source-label", required=True)
    ap.add_argument("--source-git-blob")
    ap.add_argument("--output", type=pathlib.Path, required=True)
    args = ap.parse_args()

    result = add_stsh1_defaults(args.pub, args.source_label, args.source_git_blob)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({
        "source": result["source"],
        "font_records": result["font"]["total_record_count"],
        "fdpc_font_refs": result["fdpc"]["font_reference_count"],
        "stsh1_default_character_records": len(result["stsh1_defaults"]["character_defaults"]),
        "stsh1_default_font_refs": result["stsh1_defaults"]["font_reference_count"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
