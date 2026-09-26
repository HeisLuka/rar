#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
from typing import Any

HERE = pathlib.Path(__file__).resolve().parent
DONOR_PATH = HERE.parent / "viewer-typography-story-join" / "decode_quill_story_typography.py"

spec = importlib.util.spec_from_file_location("story_join_decoder", DONOR_PATH)
if spec is None or spec.loader is None:
    raise RuntimeError(f"cannot load story-join decoder from {DONOR_PATH}")
donor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(donor)

SCHEMA = "chaptera.viewer-typography-default-inherit.v1"
PARAGRAPH_DEFAULT_CHAR_STYLE_ID = 0x19


def parse_block_strict(
    data: bytes,
    cursor: int,
    limit: int,
) -> tuple[dict[str, Any], int]:
    if cursor + 2 > limit:
        raise donor.DecodeError(f"block header exceeds limit at 0x{cursor:x}")
    start = cursor
    block_id = data[cursor]
    block_type = data[cursor + 1]
    cursor += 2
    data_offset = cursor
    if block_type in donor.VARIABLE_BLOCK_TYPES:
        if cursor + 4 > limit:
            raise donor.DecodeError("variable block length missing")
        data_length = donor.u32(data, cursor)
        if data_length < 4:
            raise donor.DecodeError("invalid variable block length")
        block_end = data_offset + data_length
        value = None
    else:
        if block_type not in donor.FIXED_BLOCK_LENGTHS:
            raise donor.DecodeError(
                f"unknown fixed block width type 0x{block_type:02x} at 0x{start:x}"
            )
        data_length = donor.FIXED_BLOCK_LENGTHS[block_type]
        block_end = data_offset + data_length
        if data_length == 2:
            value = donor.u16(data, data_offset)
        elif data_length == 4:
            value = donor.u32(data, data_offset)
        else:
            value = None
    if block_end > limit:
        raise donor.DecodeError(f"block at 0x{start:x} exceeds style")
    return {
        "start": start,
        "id": block_id,
        "type": block_type,
        "data_offset": data_offset,
        "data_length": data_length,
        "end": block_end,
        "data": value,
    }, block_end


def extract_font_index_strict(data: bytes, block: dict[str, Any]) -> int | None:
    if int(block["type"]) not in donor.VARIABLE_BLOCK_TYPES:
        return None
    cursor = int(block["data_offset"]) + 4
    end = int(block["end"])
    while cursor < end:
        child, cursor = parse_block_strict(data, cursor, end)
        if int(child["type"]) == donor.GENERAL_CONTAINER:
            inner = int(child["data_offset"]) + 4
            if inner >= int(child["end"]):
                return None
            value_block, _ = parse_block_strict(data, inner, int(child["end"]))
            value = value_block.get("data")
            return value if isinstance(value, int) else None
    return None


def _unique_ints(values: list[int]) -> list[int]:
    return sorted(set(int(value) for value in values))


def _unique_strings(values: list[str]) -> list[str]:
    return sorted(set(str(value) for value in values))


def parse_stsh1_character_defaults(
    data: bytes,
    descriptors: list[dict[str, Any]],
    font_names: list[str],
) -> list[dict[str, Any]]:
    stsh = [desc for desc in descriptors if desc["name"] == "STSH"]
    if len(stsh) < 2:
        raise donor.DecodeError(f"expected second STSH descriptor, got {len(stsh)}")
    desc = stsh[1]
    start = int(desc["offset"])
    end = int(desc["end"])
    count = donor.u32(data, start + 4)
    offsets_start = start + 20
    offsets_end = offsets_start + count * 4
    if offsets_end > end:
        raise donor.DecodeError("STSH1 offset table exceeds chunk")
    offsets = [donor.u32(data, offsets_start + ordinal * 4) for ordinal in range(count)]
    if any(left > right for left, right in zip(offsets, offsets[1:])):
        raise donor.DecodeError("STSH1 offsets regress in stored order")

    rows: list[dict[str, Any]] = []
    for ordinal in range(0, count, 2):
        record_start = start + 20 + offsets[ordinal]
        if record_start < offsets_end or record_start + 6 > end:
            raise donor.DecodeError("STSH1 character record offset points outside body")
        prefix = donor.u16(data, record_start)
        style_start = record_start + 2
        style_length = donor.u32(data, style_start)
        style_end = style_start + style_length
        if style_length < 4 or style_end > end:
            raise donor.DecodeError(
                f"invalid STSH1 character style length at logical style {ordinal // 2}"
            )

        cursor = style_start + 4
        font_indices: list[int] = []
        text_sizes_emu: list[int] = []
        while cursor < style_end:
            block, cursor = parse_block_strict(data, cursor, style_end)
            block_id = int(block["id"])
            if block_id == donor.FONT_INDEX_CONTAINER_ID:
                index = extract_font_index_strict(data, block)
                if index is not None:
                    font_indices.append(index)
            if block_id == donor.TEXT_SIZE_ID and isinstance(block.get("data"), int):
                text_sizes_emu.append(int(block["data"]))

        unique_indices = _unique_ints(font_indices)
        resolved_names = [
            font_names[index]
            for index in unique_indices
            if 0 <= index < len(font_names)
        ]
        unique_names = _unique_strings(resolved_names)
        unique_sizes = _unique_ints(text_sizes_emu)
        rows.append(
            {
                "logical_style_index": ordinal // 2,
                "stsh1_record_ordinal": ordinal,
                "record_prefix": prefix,
                "font_indices": unique_indices,
                "font_names": unique_names,
                "font_unambiguous": len(unique_indices) == 1 and len(unique_names) == 1,
                "text_size_emu": unique_sizes,
                "text_size_points": [
                    value / donor.EMU_PER_POINT for value in unique_sizes
                ],
                "text_size_unambiguous": len(unique_sizes) == 1,
            }
        )
    if not rows:
        raise donor.DecodeError("no STSH1 character default rows")
    return rows


def parse_fdpp_styles(
    data: bytes,
    descriptors: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    styles: list[dict[str, Any]] = []
    for desc in [item for item in descriptors if item["name"] == "FDPP"]:
        start = int(desc["offset"])
        end = int(desc["end"])
        count = donor.u16(data, start)
        offsets_start = start + 8
        chunk_offsets_start = offsets_start + count * 4
        body_start = chunk_offsets_start + count * 2
        if body_start > end:
            raise donor.DecodeError("FDPP tables exceed chunk")

        text_offsets = [
            donor.u32(data, offsets_start + ordinal * 4)
            for ordinal in range(count)
        ]
        chunk_offsets = [
            donor.u16(data, chunk_offsets_start + ordinal * 2)
            for ordinal in range(count)
        ]

        for ordinal, (text_offset, chunk_offset) in enumerate(
            zip(text_offsets, chunk_offsets)
        ):
            style_start = start + chunk_offset
            if style_start < body_start or style_start + 4 > end:
                raise donor.DecodeError("FDPP style offset points outside body")
            style_length = donor.u32(data, style_start)
            style_end = style_start + style_length
            if style_length < 4 or style_end > end:
                raise donor.DecodeError("invalid FDPP style length")

            cursor = style_start + 4
            selectors: list[int] = []
            while cursor < style_end:
                block, cursor = parse_block_strict(data, cursor, style_end)
                if (
                    int(block["id"]) == PARAGRAPH_DEFAULT_CHAR_STYLE_ID
                    and isinstance(block.get("data"), int)
                ):
                    selectors.append(int(block["data"]))

            unique_selectors = _unique_ints(selectors)
            styles.append(
                {
                    "descriptor_ordinal": int(desc["ordinal"]),
                    "style_ordinal": ordinal,
                    "text_offset": int(text_offset),
                    "style_start": style_start,
                    "style_end": style_end,
                    "default_style_index_raw": unique_selectors,
                }
            )
    if not styles:
        raise donor.DecodeError("no FDPP styles")
    return styles


def materialize_fdpp_ranges(
    styles: list[dict[str, Any]],
    story_catalog: dict[str, Any],
) -> list[dict[str, Any]]:
    text_start = int(story_catalog["text_offset"])
    text_end = int(story_catalog["text_end"])
    total_utf16 = int(story_catalog["total_utf16_units"])

    endpoints: list[tuple[int, dict[str, Any]]] = []
    for style in styles:
        absolute = int(style["text_offset"])
        if not (text_start <= absolute <= text_end):
            raise donor.DecodeError(
                f"FDPP text_offset 0x{absolute:x} outside TEXT "
                f"[0x{text_start:x},0x{text_end:x}]"
            )
        delta = absolute - text_start
        if delta % 2:
            raise donor.DecodeError(
                f"FDPP text_offset 0x{absolute:x} is not UTF-16 aligned"
            )
        endpoints.append((delta // 2, style))

    if any(left[0] > right[0] for left, right in zip(endpoints, endpoints[1:])):
        raise donor.DecodeError("FDPP endpoints regress in stored order")

    ranges: list[dict[str, Any]] = []
    previous = 0
    for end_utf16, style in endpoints:
        if end_utf16 < previous:
            raise donor.DecodeError("FDPP range endpoint regressed")
        if end_utf16 == previous:
            previous = end_utf16
            continue

        selectors = list(style["default_style_index_raw"])
        if len(selectors) == 0:
            selected_style_index: int | None = 0
            selector_source = "implicit_zero_from_prior_evidence"
        elif len(selectors) == 1:
            selected_style_index = int(selectors[0])
            selector_source = "explicit_fdpp_0x19"
        else:
            selected_style_index = None
            selector_source = "ambiguous_multiple_fdpp_0x19"

        ranges.append(
            {
                "global_start_utf16": previous,
                "global_end_utf16": end_utf16,
                "descriptor_ordinal": style["descriptor_ordinal"],
                "style_ordinal": style["style_ordinal"],
                "default_style_index_raw": selectors,
                "selected_style_index": selected_style_index,
                "selector_source": selector_source,
            }
        )
        previous = end_utf16

    if previous != total_utf16:
        raise donor.DecodeError(
            f"FDPP terminal closure mismatch {previous} != {total_utf16}"
        )
    return ranges


def _covering(
    ranges: list[dict[str, Any]],
    start: int,
    end: int,
) -> dict[str, Any] | None:
    matches = [
        row
        for row in ranges
        if int(row["global_start_utf16"]) <= start
        and int(row["global_end_utf16"]) >= end
    ]
    if len(matches) != 1:
        return None
    return matches[0]


def _story_covering(
    stories: list[dict[str, Any]],
    start: int,
    end: int,
) -> dict[str, Any] | None:
    matches = [
        story
        for story in stories
        if int(story["global_start_utf16"]) <= start
        and int(story["global_end_utf16"]) >= end
    ]
    if len(matches) != 1:
        return None
    return matches[0]


def materialize_effective_segments(
    story_catalog: dict[str, Any],
    fdpc_ranges: list[dict[str, Any]],
    fdpp_ranges: list[dict[str, Any]],
    stsh_defaults: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    defaults_by_index = {
        int(row["logical_style_index"]): row for row in stsh_defaults
    }
    boundaries = {0, int(story_catalog["total_utf16_units"])}
    for story in story_catalog["stories"]:
        boundaries.add(int(story["global_start_utf16"]))
        boundaries.add(int(story["global_end_utf16"]))
    for collection in (fdpc_ranges, fdpp_ranges):
        for row in collection:
            boundaries.add(int(row["global_start_utf16"]))
            boundaries.add(int(row["global_end_utf16"]))
    ordered = sorted(boundaries)

    segments: list[dict[str, Any]] = []
    for start, end in zip(ordered, ordered[1:]):
        if start == end:
            continue
        story = _story_covering(story_catalog["stories"], start, end)
        fdpc = _covering(fdpc_ranges, start, end)
        fdpp = _covering(fdpp_ranges, start, end)
        if story is None or fdpc is None or fdpp is None:
            continue

        selector = fdpp.get("selected_style_index")
        default = (
            defaults_by_index.get(int(selector))
            if isinstance(selector, int)
            else None
        )

        explicit_font_indices = _unique_ints(list(fdpc["font_indices"]))
        explicit_font_names = _unique_strings(list(fdpc["font_names"]))
        explicit_sizes = _unique_ints(list(fdpc["text_size_emu"]))
        explicit_font_present = bool(explicit_font_indices)
        explicit_size_present = bool(explicit_sizes)
        explicit_font_unambiguous = (
            len(explicit_font_indices) == 1 and len(explicit_font_names) == 1
        )
        explicit_size_unambiguous = len(explicit_sizes) == 1

        default_font_unambiguous = bool(default and default["font_unambiguous"])
        default_size_unambiguous = bool(default and default["text_size_unambiguous"])

        effective_font_name: str | None = None
        font_source: str | None = None
        if explicit_font_unambiguous:
            effective_font_name = explicit_font_names[0]
            font_source = "explicit_fdpc"
        elif not explicit_font_present and default_font_unambiguous:
            effective_font_name = str(default["font_names"][0])
            font_source = "inherited_stsh1"

        effective_size_emu: int | None = None
        size_source: str | None = None
        if explicit_size_unambiguous:
            effective_size_emu = explicit_sizes[0]
            size_source = "explicit_fdpc"
        elif not explicit_size_present and default_size_unambiguous:
            effective_size_emu = int(default["text_size_emu"][0])
            size_source = "inherited_stsh1"

        before_complete = explicit_font_unambiguous and explicit_size_unambiguous
        after_complete = effective_font_name is not None and effective_size_emu is not None
        inherited_any = font_source == "inherited_stsh1" or size_source == "inherited_stsh1"

        segments.append(
            {
                "story_index": int(story["index"]),
                "story_syid": int(story["syid"]),
                "global_start_utf16": start,
                "global_end_utf16": end,
                "story_start_utf16": start - int(story["global_start_utf16"]),
                "story_end_utf16": end - int(story["global_start_utf16"]),
                "paragraph_selector_source": fdpp["selector_source"],
                "default_style_index": selector,
                "explicit_font_present": explicit_font_present,
                "explicit_size_present": explicit_size_present,
                "explicit_font_unambiguous": explicit_font_unambiguous,
                "explicit_size_unambiguous": explicit_size_unambiguous,
                "effective_font_name": effective_font_name,
                "effective_size_emu": effective_size_emu,
                "effective_size_points": (
                    effective_size_emu / donor.EMU_PER_POINT
                    if effective_size_emu is not None
                    else None
                ),
                "font_source": font_source,
                "size_source": size_source,
                "before_complete_explicit_font_and_size": before_complete,
                "after_complete_effective_font_and_size": after_complete,
                "inherited_any": inherited_any,
            }
        )
    return segments


def decode_pub(path: pathlib.Path, source_label: str) -> dict[str, Any]:
    raw = path.read_bytes()
    if not raw.startswith(donor.OLE_MAGIC):
        raise donor.DecodeError("not an OLE/CFB file")

    with donor.olefile.OleFileIO(str(path)) as ole:
        quill_path = ["Quill", "QuillSub", "CONTENTS"]
        if not ole.exists(quill_path):
            raise donor.DecodeError("missing Quill/QuillSub/CONTENTS")
        quill = ole.openstream(quill_path).read()

    descriptors = donor.parse_descriptors(quill)
    font_names = donor.parse_font_catalog(quill, descriptors)
    story_catalog = donor.parse_story_catalog(quill, descriptors)
    fdpc_styles = donor.parse_fdpc_styles(quill, descriptors, font_names)
    fdpc_ranges = donor.materialize_ranges(fdpc_styles, story_catalog)
    fdpp_styles = parse_fdpp_styles(quill, descriptors)
    fdpp_ranges = materialize_fdpp_ranges(fdpp_styles, story_catalog)
    stsh_defaults = parse_stsh1_character_defaults(quill, descriptors, font_names)
    segments = materialize_effective_segments(
        story_catalog,
        fdpc_ranges,
        fdpp_ranges,
        stsh_defaults,
    )

    before_complete = sum(
        1 for row in segments if row["before_complete_explicit_font_and_size"]
    )
    after_complete = sum(
        1 for row in segments if row["after_complete_effective_font_and_size"]
    )
    inherited_any = [row for row in segments if row["inherited_any"]]
    newly_complete = [
        row
        for row in segments
        if not row["before_complete_explicit_font_and_size"]
        and row["after_complete_effective_font_and_size"]
    ]
    explicit_selector_gains = [
        row
        for row in newly_complete
        if row["paragraph_selector_source"] == "explicit_fdpp_0x19"
    ]
    implicit_zero_gains = [
        row
        for row in newly_complete
        if row["paragraph_selector_source"] == "implicit_zero_from_prior_evidence"
    ]

    return {
        "schema": SCHEMA,
        "source": {
            "label": source_label,
            "file_name": path.name,
            "sha256": donor.sha256_bytes(raw),
            "byte_len": len(raw),
        },
        "quill": {
            "sha256": donor.sha256_bytes(quill),
            "byte_len": len(quill),
            "descriptor_count": len(descriptors),
        },
        "story_catalog": {
            "story_count": len(story_catalog["stories"]),
            "total_utf16_units": story_catalog["total_utf16_units"],
        },
        "fdpc": {
            "range_count": len(fdpc_ranges),
        },
        "fdpp": {
            "style_count": len(fdpp_styles),
            "range_count": len(fdpp_ranges),
            "explicit_selector_range_count": sum(
                1
                for row in fdpp_ranges
                if row["selector_source"] == "explicit_fdpp_0x19"
            ),
            "implicit_zero_range_count": sum(
                1
                for row in fdpp_ranges
                if row["selector_source"] == "implicit_zero_from_prior_evidence"
            ),
            "terminal_exact": True,
        },
        "stsh1_character_defaults": stsh_defaults,
        "coverage": {
            "segment_count": len(segments),
            "complete_explicit_font_and_size_before": before_complete,
            "complete_effective_font_and_size_after": after_complete,
            "inherited_any_segment_count": len(inherited_any),
            "newly_complete_segment_count": len(newly_complete),
            "newly_complete_explicit_selector_segment_count": len(explicit_selector_gains),
            "newly_complete_implicit_zero_segment_count": len(implicit_zero_gains),
        },
        "newly_complete_segments": newly_complete,
        "privacy_boundary": {
            "raw_pub_retained": False,
            "raw_quill_payload_retained": False,
            "host_font_lookup_used": False,
            "publisher_exact_reflow_claimed": False,
        },
        "scope": {
            "properties": ["font_name", "text_size_emu"],
            "explicit_fdpc_overrides_inherited": True,
            "bold_italic_underline_inheritance_evaluated": False,
            "stsh2_base_style_traversal_used": False,
            "implicit_zero_selector_source": "prior established corpus/native evidence",
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
    print(
        json.dumps(
            {
                "source": result["source"]["file_name"],
                "stories": result["story_catalog"]["story_count"],
                "fdpc_ranges": result["fdpc"]["range_count"],
                "fdpp_ranges": result["fdpp"]["range_count"],
                **result["coverage"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
