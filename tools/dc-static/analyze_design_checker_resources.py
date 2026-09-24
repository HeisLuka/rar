#!/usr/bin/env python3
"""DC-STATIC-01: map Design Checker strings to PE resources and code references.

The tool is deliberately bounded. It records exact resource matches, whole-PE
ASCII/UTF-16 matches, and a companion-module census without promoting string
proximity to a checker dispatcher claim.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
from collections import defaultdict
from pathlib import Path

import pefile
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from capstone.x86 import X86_OP_IMM, X86_OP_MEM

RT_DIALOG = 5
RT_STRING = 6
IMAGE_SCN_MEM_EXECUTE = 0x20000000
RESOURCE_TYPE_NAMES = {
    1: "CURSOR",
    2: "BITMAP",
    3: "ICON",
    4: "MENU",
    5: "DIALOG",
    6: "STRING",
    7: "FONTDIR",
    8: "FONT",
    9: "ACCELERATOR",
    10: "RCDATA",
    11: "MESSAGETABLE",
    12: "GROUP_CURSOR",
    14: "GROUP_ICON",
    16: "VERSION",
    17: "DLGINCLUDE",
    23: "HTML",
    24: "MANIFEST",
}
KEYWORDS = [
    "design checker",
    "checker",
    "nonprintable",
    "off page",
    "low-resolution",
    "picture is",
    "overflow",
    "scratch area",
    "transparent",
    "hyperlink",
    "never run",
]


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def read_anchors(path: Path) -> list[str]:
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        value = line.strip()
        if value and not value.startswith("#"):
            rows.append(value)
    if not rows:
        raise SystemExit("anchor list is empty")
    return rows


def lang_id(entry) -> int | str:
    if getattr(entry, "id", None) is not None:
        return int(entry.id)
    return str(entry.name)


def resource_blob(pe: pefile.PE, lang_entry) -> bytes:
    data_entry = lang_entry.data.struct
    return pe.get_data(data_entry.OffsetToData, data_entry.Size)


def parse_string_tables(pe: pefile.PE) -> list[dict]:
    out: list[dict] = []
    if not hasattr(pe, "DIRECTORY_ENTRY_RESOURCE"):
        return out
    for type_entry in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        if getattr(type_entry, "id", None) != RT_STRING or not hasattr(type_entry, "directory"):
            continue
        for block_entry in type_entry.directory.entries:
            if getattr(block_entry, "id", None) is None or not hasattr(block_entry, "directory"):
                continue
            block_id = int(block_entry.id)
            for language_entry in block_entry.directory.entries:
                data = resource_blob(pe, language_entry)
                pos = 0
                for index in range(16):
                    if pos + 2 > len(data):
                        break
                    length = struct.unpack_from("<H", data, pos)[0]
                    pos += 2
                    byte_len = length * 2
                    if pos + byte_len > len(data):
                        break
                    raw = data[pos : pos + byte_len]
                    pos += byte_len
                    if not length:
                        continue
                    text = raw.decode("utf-16le", errors="replace")
                    out.append(
                        {
                            "kind": "stringtable",
                            "block_id": block_id,
                            "string_id": (block_id - 1) * 16 + index,
                            "language": lang_id(language_entry),
                            "text": text,
                        }
                    )
    return out


def parse_dialogs(pe: pefile.PE) -> list[dict]:
    out: list[dict] = []
    if not hasattr(pe, "DIRECTORY_ENTRY_RESOURCE"):
        return out
    for type_entry in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        if getattr(type_entry, "id", None) != RT_DIALOG or not hasattr(type_entry, "directory"):
            continue
        for dialog_entry in type_entry.directory.entries:
            dialog_id = (
                int(dialog_entry.id)
                if getattr(dialog_entry, "id", None) is not None
                else str(dialog_entry.name)
            )
            if not hasattr(dialog_entry, "directory"):
                continue
            for language_entry in dialog_entry.directory.entries:
                data = resource_blob(pe, language_entry)
                text = data.decode("utf-16le", errors="ignore")
                out.append(
                    {
                        "kind": "dialog",
                        "dialog_id": dialog_id,
                        "language": lang_id(language_entry),
                        "decoded_text": text,
                    }
                )
    return out


def resource_inventory(pe: pefile.PE) -> list[dict]:
    rows: list[dict] = []
    if not hasattr(pe, "DIRECTORY_ENTRY_RESOURCE"):
        return rows
    for type_entry in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        rid = getattr(type_entry, "id", None)
        name = str(type_entry.name) if getattr(type_entry, "name", None) is not None else None
        child_count = len(type_entry.directory.entries) if hasattr(type_entry, "directory") else 0
        rows.append(
            {
                "id": int(rid) if rid is not None else None,
                "name": name or RESOURCE_TYPE_NAMES.get(int(rid), str(rid) if rid is not None else None),
                "child_count": child_count,
            }
        )
    return rows


def match_anchors(anchors: list[str], strings: list[dict], dialogs: list[dict]) -> list[dict]:
    matches: list[dict] = []
    for anchor in anchors:
        needle = anchor.casefold()
        for item in strings:
            if needle in item["text"].casefold():
                matches.append({"anchor": anchor, **item})
        for item in dialogs:
            if needle in item["decoded_text"].casefold():
                matches.append(
                    {
                        "anchor": anchor,
                        "kind": "dialog",
                        "dialog_id": item["dialog_id"],
                        "language": item["language"],
                    }
                )
    return matches


def extract_printable_strings(data: bytes) -> list[dict]:
    out: list[dict] = []
    for m in re.finditer(rb"[\x20-\x7e]{4,}", data):
        out.append({"encoding": "ascii", "offset": m.start(), "text": m.group().decode("ascii", errors="replace")})
    for m in re.finditer(rb"(?:[\x20-\x7e]\x00){4,}", data):
        out.append({"encoding": "utf16le", "offset": m.start(), "text": m.group().decode("utf-16le", errors="replace")})
    return out


def section_for_offset(pe: pefile.PE, offset: int) -> str | None:
    for section in pe.sections:
        start = int(section.PointerToRawData)
        end = start + int(section.SizeOfRawData)
        if start <= offset < end:
            return section.Name.rstrip(b"\\x00").decode("ascii", errors="replace")
    return None


def checker_identifier_rows(pe: pefile.PE, path: Path, instructions: list) -> list[dict]:
    data = path.read_bytes()
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    rows: list[dict] = []
    for item in extract_printable_strings(data):
        text = item["text"]
        folded = text.casefold()
        if not (
            re.fullmatch(r"Check[A-Za-z0-9_]{2,96}", text)
            or re.fullmatch(r"[A-Za-z0-9_]{0,48}Checker[A-Za-z0-9_]{0,48}", text)
            or folded in {"checkers", "checker.poc"}
        ):
            continue
        offset = int(item["offset"])
        try:
            rva = int(pe.get_rva_from_offset(offset))
        except Exception:
            rva = None
        va = image_base + rva if rva is not None else None
        refs: list[dict] = []
        if va is not None:
            for insn in instructions:
                hit = False
                for op in insn.operands:
                    if op.type == X86_OP_IMM and int(op.imm) == va:
                        hit = True
                    elif op.type == X86_OP_MEM and int(op.mem.disp) == va:
                        hit = True
                if hit:
                    refs.append(
                        {
                            "va": f"0x{insn.address:08X}",
                            "rva": f"0x{insn.address - image_base:08X}",
                            "instruction": f"{insn.mnemonic} {insn.op_str}".strip(),
                        }
                    )
                    if len(refs) >= 32:
                        break
        rows.append(
            {
                **item,
                "section": section_for_offset(pe, offset),
                "rva": f"0x{rva:08X}" if rva is not None else None,
                "va": f"0x{va:08X}" if va is not None else None,
                "code_refs": refs,
            }
        )
    rows.sort(key=lambda row: (row["offset"], row["encoding"], row["text"]))
    return rows


def checker_identifier_clusters(rows: list[dict], max_gap: int = 1024) -> list[dict]:
    if not rows:
        return []
    clusters: list[list[dict]] = [[rows[0]]]
    for row in rows[1:]:
        if row["offset"] - clusters[-1][-1]["offset"] <= max_gap:
            clusters[-1].append(row)
        else:
            clusters.append([row])
    return [
        {
            "start_offset": group[0]["offset"],
            "end_offset": group[-1]["offset"],
            "count": len(group),
            "items": group,
        }
        for group in clusters
    ]


def checker_pointer_rows(pe: pefile.PE, path: Path, checker_rows: list[dict]) -> list[dict]:
    """Find exact little-endian RVA/VA pointers to checker identifier strings."""
    data = path.read_bytes()
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    rows: list[dict] = []
    for checker in checker_rows:
        if not checker.get("rva") or not checker.get("va"):
            continue
        target_rva = int(checker["rva"], 16)
        target_va = int(checker["va"], 16)
        for encoding, value in (("va32", target_va), ("rva32", target_rva)):
            needle = struct.pack("<I", value & 0xFFFFFFFF)
            start = 0
            while True:
                offset = data.find(needle, start)
                if offset < 0:
                    break
                start = offset + 1
                try:
                    slot_rva = int(pe.get_rva_from_offset(offset))
                except Exception:
                    slot_rva = None
                slot_va = image_base + slot_rva if slot_rva is not None else None
                rows.append(
                    {
                        "identifier": checker["text"],
                        "target_rva": checker["rva"],
                        "target_va": checker["va"],
                        "pointer_encoding": encoding,
                        "pointer_offset": offset,
                        "pointer_section": section_for_offset(pe, offset),
                        "pointer_rva": f"0x{slot_rva:08X}" if slot_rva is not None else None,
                        "pointer_va": f"0x{slot_va:08X}" if slot_va is not None else None,
                    }
                )
    rows.sort(key=lambda row: (row["pointer_offset"], row["identifier"], row["pointer_encoding"]))
    return rows


def checker_pointer_clusters(rows: list[dict], max_gap: int = 32) -> list[dict]:
    """Cluster nearby pointer slots; keep only clusters spanning >=2 distinct identifiers."""
    if not rows:
        return []
    groups: list[list[dict]] = [[rows[0]]]
    for row in rows[1:]:
        if row["pointer_offset"] - groups[-1][-1]["pointer_offset"] <= max_gap:
            groups[-1].append(row)
        else:
            groups.append([row])

    clusters: list[dict] = []
    for group in groups:
        identifiers = sorted({row["identifier"] for row in group})
        if len(identifiers) < 2:
            continue
        valid_rvas = [int(row["pointer_rva"], 16) for row in group if row.get("pointer_rva")]
        valid_vas = [int(row["pointer_va"], 16) for row in group if row.get("pointer_va")]
        clusters.append(
            {
                "start_offset": group[0]["pointer_offset"],
                "end_offset": group[-1]["pointer_offset"],
                "pointer_count": len(group),
                "distinct_identifier_count": len(identifiers),
                "identifiers": identifiers,
                "sections": sorted({row["pointer_section"] for row in group if row.get("pointer_section")}),
                "start_rva": f"0x{min(valid_rvas):08X}" if valid_rvas else None,
                "end_rva": f"0x{max(valid_rvas):08X}" if valid_rvas else None,
                "start_va": f"0x{min(valid_vas):08X}" if valid_vas else None,
                "end_va": f"0x{max(valid_vas):08X}" if valid_vas else None,
                "items": group,
            }
        )
    clusters.sort(key=lambda row: (-row["distinct_identifier_count"], row["start_offset"]))
    return clusters


def checker_pointer_cluster_code_refs(pe: pefile.PE, instructions: list, clusters: list[dict]) -> list[dict]:
    """Find code operands that point into or at a candidate checker pointer cluster."""
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    out: list[dict] = []
    for cluster in clusters:
        if not cluster.get("start_va") or not cluster.get("end_va"):
            continue
        lo = int(cluster["start_va"], 16)
        hi = int(cluster["end_va"], 16) + 4
        refs: list[dict] = []
        for insn in instructions:
            hit_kind = None
            hit_value = None
            for op in insn.operands:
                if op.type == X86_OP_IMM and lo <= int(op.imm) <= hi:
                    hit_kind = "imm"
                    hit_value = int(op.imm)
                elif op.type == X86_OP_MEM and lo <= int(op.mem.disp) <= hi:
                    hit_kind = "mem_disp"
                    hit_value = int(op.mem.disp)
            if hit_kind:
                refs.append(
                    {
                        "va": f"0x{insn.address:08X}",
                        "rva": f"0x{insn.address - image_base:08X}",
                        "instruction": f"{insn.mnemonic} {insn.op_str}".strip(),
                        "hit_kind": hit_kind,
                        "hit_value": f"0x{hit_value:08X}",
                    }
                )
                if len(refs) >= 128:
                    break
        out.append(
            {
                "start_offset": cluster["start_offset"],
                "end_offset": cluster["end_offset"],
                "distinct_identifier_count": cluster["distinct_identifier_count"],
                "identifiers": cluster["identifiers"],
                "refs": refs,
            }
        )
    out.sort(key=lambda row: (-len(row["refs"]), -row["distinct_identifier_count"], row["start_offset"]))
    return out


def classify_pe_dword(pe: pefile.PE, value: int) -> dict:
    """Classify a 32-bit value without assigning semantics beyond PE location."""
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    image_size = int(pe.OPTIONAL_HEADER.SizeOfImage)
    if value == 0:
        return {"value": "0x00000000", "kind": "null"}

    if image_base <= value < image_base + image_size:
        rva = value - image_base
        try:
            offset = int(pe.get_offset_from_rva(rva))
            section = section_for_offset(pe, offset)
        except Exception:
            offset = None
            section = None
        return {
            "value": f"0x{value:08X}",
            "kind": "va",
            "rva": f"0x{rva:08X}",
            "offset": offset,
            "section": section,
        }

    if value <= 0xFFFF:
        return {"value": f"0x{value:08X}", "kind": "small_immediate", "decimal": value}

    if 0 < value < image_size:
        try:
            offset = int(pe.get_offset_from_rva(value))
            section = section_for_offset(pe, offset)
        except Exception:
            offset = None
            section = None
        return {
            "value": f"0x{value:08X}",
            "kind": "rva",
            "rva": f"0x{value:08X}",
            "offset": offset,
            "section": section,
        }

    return {"value": f"0x{value:08X}", "kind": "scalar_or_external"}


def decode_checker_fixed_records(
    pe: pefile.PE, path: Path, clusters: list[dict], record_size: int = 16
) -> list[dict]:
    """Decode candidate fixed-stride arrays that begin with checker-name pointers."""
    data = path.read_bytes()
    out: list[dict] = []
    for cluster in clusters:
        by_offset: dict[int, dict] = {}
        for item in cluster["items"]:
            # Prefer the VA-form hit when both an RVA and VA happen to collide.
            current = by_offset.get(item["pointer_offset"])
            if current is None or item["pointer_encoding"] == "va32":
                by_offset[item["pointer_offset"]] = item
        offsets = sorted(by_offset)
        if len(offsets) < 4:
            continue
        strides = [b - a for a, b in zip(offsets, offsets[1:])]
        if not strides or any(stride != record_size for stride in strides):
            continue

        records: list[dict] = []
        executable_pointer_fields = 0
        field_kind_counts: list[dict[str, int]] = [defaultdict(int) for _ in range(4)]
        field_section_counts: list[dict[str, int]] = [defaultdict(int) for _ in range(4)]
        for offset in offsets:
            if offset + record_size > len(data):
                continue
            dwords = list(struct.unpack_from("<IIII", data, offset))
            fields = []
            for field_index, value in enumerate(dwords):
                classified = classify_pe_dword(pe, value)
                field_kind_counts[field_index][classified["kind"]] += 1
                if classified.get("section"):
                    field_section_counts[field_index][classified["section"]] += 1
                if field_index > 0 and classified.get("section") == ".text":
                    executable_pointer_fields += 1
                fields.append({"index": field_index, **classified})
            first = by_offset[offset]
            records.append(
                {
                    "offset": offset,
                    "rva": first["pointer_rva"],
                    "va": first["pointer_va"],
                    "identifier": first["identifier"],
                    "fields": fields,
                }
            )

        out.append(
            {
                "record_size": record_size,
                "record_count": len(records),
                "start_offset": offsets[0],
                "end_offset": offsets[-1] + record_size - 1,
                "start_rva": records[0]["rva"] if records else None,
                "end_rva": records[-1]["rva"] if records else None,
                "executable_pointer_fields_after_name": executable_pointer_fields,
                "field_kind_counts": [
                    {"field_index": i, "counts": dict(sorted(counts.items()))}
                    for i, counts in enumerate(field_kind_counts)
                ],
                "field_section_counts": [
                    {"field_index": i, "counts": dict(sorted(counts.items()))}
                    for i, counts in enumerate(field_section_counts)
                ],
                "records": records,
            }
        )
    out.sort(key=lambda row: (-row["record_count"], row["start_offset"]))
    return out


def import_iat_map(pe: pefile.PE, wanted: set[str]) -> dict[int, str]:
    out: dict[int, str] = {}
    if not hasattr(pe, "DIRECTORY_ENTRY_IMPORT"):
        return out
    for module in pe.DIRECTORY_ENTRY_IMPORT:
        dll = module.dll.decode("ascii", errors="replace")
        for imp in module.imports:
            if not imp.name:
                continue
            name = imp.name.decode("ascii", errors="replace")
            if name in wanted:
                out[int(imp.address)] = f"{dll}!{name}"
    return out


def import_thunk_map(pe: pefile.PE, instructions: list, iat_map: dict[int, str]) -> dict[int, str]:
    """Resolve simple x86 import thunks: JMP [IAT] -> imported API name."""
    out: dict[int, str] = {}
    for insn in instructions:
        if insn.mnemonic != "jmp":
            continue
        for op in insn.operands:
            if op.type == X86_OP_MEM and int(op.mem.disp) in iat_map:
                out[int(insn.address)] = iat_map[int(op.mem.disp)]
                break
    return out


def pointer_occurrence_instructions(
    pe: pefile.PE, instructions: list, pointer_rows: list[dict]
) -> list[dict]:
    """Bind raw DWORD pointer occurrences in executable sections to containing instructions."""
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    out: list[dict] = []
    for row in pointer_rows:
        if row.get("section") != ".text" or not row.get("rva"):
            continue
        occurrence_va = image_base + int(row["rva"], 16)
        containing = None
        for insn in instructions:
            if int(insn.address) <= occurrence_va < int(insn.address) + int(insn.size):
                containing = {
                    "instruction_va": f"0x{insn.address:08X}",
                    "instruction_rva": f"0x{insn.address - image_base:08X}",
                    "instruction_size": int(insn.size),
                    "instruction": f"{insn.mnemonic} {insn.op_str}".strip(),
                    "byte_offset_within_instruction": occurrence_va - int(insn.address),
                }
                break
        out.append({**row, "containing_instruction": containing})
    return out


def code_refs_to_pointer_slots(
    pe: pefile.PE, instructions: list, pointer_rows: list[dict]
) -> list[dict]:
    """Find code operands that reference second-order data pointer slots exactly."""
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    slots = {
        int(row["va"], 16): row
        for row in pointer_rows
        if row.get("va") and row.get("section") != ".text"
    }
    out: list[dict] = []
    for insn in instructions:
        for op in insn.operands:
            if op.type == X86_OP_IMM:
                value = int(op.imm)
                kind = "imm"
            elif op.type == X86_OP_MEM:
                value = int(op.mem.disp)
                kind = "mem_disp"
            else:
                continue
            if value in slots:
                row = slots[value]
                out.append(
                    {
                        "slot_va": row["va"],
                        "slot_rva": row["rva"],
                        "cluster_index": row["cluster_index"],
                        "va": f"0x{insn.address:08X}",
                        "rva": f"0x{insn.address - image_base:08X}",
                        "instruction": f"{insn.mnemonic} {insn.op_str}".strip(),
                        "operand_kind": kind,
                    }
                )
    return out


def shared_record_target_headers(pe: pefile.PE, path: Path, record_arrays: list[dict]) -> list[dict]:
    """Decode small shared targets referenced by a fixed record field, without assigning semantics."""
    data = path.read_bytes()
    seen: set[int] = set()
    out: list[dict] = []
    for array in record_arrays:
        for record in array.get("records", []):
            for field in record.get("fields", [])[1:]:
                if field.get("kind") != "va" or field.get("offset") is None:
                    continue
                target_offset = int(field["offset"])
                if target_offset in seen:
                    continue
                seen.add(target_offset)
                start = max(0, target_offset - 16)
                end = min(len(data), target_offset + 32)
                window = data[start:end]
                dwords = []
                for pos in range(0, len(window) - 3, 4):
                    value = struct.unpack_from("<I", window, pos)[0]
                    dwords.append(
                        {
                            "relative_offset": start + pos - target_offset,
                            **classify_pe_dword(pe, value),
                        }
                    )
                out.append(
                    {
                        "target_va": field["value"],
                        "target_rva": field.get("rva"),
                        "target_offset": target_offset,
                        "target_section": field.get("section"),
                        "window_start_offset": start,
                        "window_end_offset": end,
                        "dwords": dwords,
                    }
                )
    return out


def imported_call_sites(
    pe: pefile.PE,
    instructions: list,
    iat_map: dict[int, str],
    thunk_map: dict[int, str] | None = None,
) -> list[dict]:
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    thunk_map = thunk_map or {}
    out: list[dict] = []
    for idx, insn in enumerate(instructions):
        if insn.mnemonic != "call":
            continue
        target = None
        resolution = None
        for op in insn.operands:
            if op.type == X86_OP_MEM and int(op.mem.disp) in iat_map:
                target = iat_map[int(op.mem.disp)]
                resolution = "direct_iat"
                break
            if op.type == X86_OP_IMM and int(op.imm) in thunk_map:
                target = thunk_map[int(op.imm)]
                resolution = "import_thunk"
                break
        if target:
            out.append(
                {
                    "instruction_index": idx,
                    "va": f"0x{insn.address:08X}",
                    "rva": f"0x{insn.address - image_base:08X}",
                    "target": target,
                    "resolution": resolution,
                    "instruction": f"{insn.mnemonic} {insn.op_str}".strip(),
                }
            )
    return out


def cluster_base_pointer_rows(pe: pefile.PE, path: Path, clusters: list[dict]) -> list[dict]:
    """Find second-order DWORD pointers to candidate pointer-array bases."""
    data = path.read_bytes()
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    out: list[dict] = []
    for cluster_index, cluster in enumerate(clusters):
        if not cluster.get("start_va") or not cluster.get("start_rva"):
            continue
        targets = (
            ("base_va32", int(cluster["start_va"], 16)),
            ("base_rva32", int(cluster["start_rva"], 16)),
        )
        for encoding, value in targets:
            needle = struct.pack("<I", value & 0xFFFFFFFF)
            start = 0
            while True:
                offset = data.find(needle, start)
                if offset < 0:
                    break
                start = offset + 1
                # Ignore the array's own first element if its value happens to equal its base.
                if cluster["start_offset"] <= offset <= cluster["end_offset"]:
                    continue
                try:
                    rva = int(pe.get_rva_from_offset(offset))
                except Exception:
                    rva = None
                va = image_base + rva if rva is not None else None
                out.append(
                    {
                        "cluster_index": cluster_index,
                        "cluster_start_rva": cluster["start_rva"],
                        "cluster_start_va": cluster["start_va"],
                        "encoding": encoding,
                        "offset": offset,
                        "section": section_for_offset(pe, offset),
                        "rva": f"0x{rva:08X}" if rva is not None else None,
                        "va": f"0x{va:08X}" if va is not None else None,
                    }
                )
    out.sort(key=lambda row: (row["cluster_index"], row["offset"], row["encoding"]))
    return out


def loader_proximity(
    pe: pefile.PE,
    instructions: list,
    call_sites: list[dict],
    clusters: list[dict],
    second_order_rows: list[dict],
    checker_rows: list[dict],
    window_size: int = 32,
) -> list[dict]:
    """Record bounded operands near loader calls that touch checker arrays/names."""
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    checker_vas = {
        int(row["va"], 16): row["text"]
        for row in checker_rows
        if row.get("va")
    }
    second_order_vas = {
        int(row["va"], 16): row
        for row in second_order_rows
        if row.get("va")
    }
    cluster_ranges = []
    for index, cluster in enumerate(clusters):
        if cluster.get("start_va") and cluster.get("end_va"):
            cluster_ranges.append(
                (index, int(cluster["start_va"], 16), int(cluster["end_va"], 16) + 4)
            )

    out: list[dict] = []
    for call in call_sites:
        idx = int(call["instruction_index"])
        hits: list[dict] = []
        for prev in instructions[max(0, idx - window_size):idx]:
            for op in prev.operands:
                if op.type == X86_OP_IMM:
                    value = int(op.imm)
                    kind = "imm"
                elif op.type == X86_OP_MEM:
                    value = int(op.mem.disp)
                    kind = "mem_disp"
                else:
                    continue
                detail = None
                if value in checker_vas:
                    detail = {"class": "checker_string_va", "identifier": checker_vas[value]}
                elif value in second_order_vas:
                    detail = {
                        "class": "checker_cluster_base_pointer_va",
                        "cluster_index": second_order_vas[value]["cluster_index"],
                    }
                else:
                    for cluster_index, lo, hi in cluster_ranges:
                        if lo <= value <= hi:
                            detail = {
                                "class": "checker_cluster_range",
                                "cluster_index": cluster_index,
                            }
                            break
                if detail:
                    hits.append(
                        {
                            "va": f"0x{prev.address:08X}",
                            "rva": f"0x{prev.address - image_base:08X}",
                            "instruction": f"{prev.mnemonic} {prev.op_str}".strip(),
                            "operand_kind": kind,
                            "value": f"0x{value:08X}",
                            **detail,
                        }
                    )
        out.append({**call, "nearby_checker_hits": hits})
    return out


def raw_string_matches(path: Path, anchors: list[str]) -> tuple[list[dict], list[dict]]:
    data = path.read_bytes()
    strings = extract_printable_strings(data)
    anchor_matches: list[dict] = []
    keyword_hits: list[dict] = []
    seen_anchor = set()
    seen_keyword = set()
    anchor_needles = [(a, a.casefold()) for a in anchors]

    for row in strings:
        folded = row["text"].casefold()
        for anchor, needle in anchor_needles:
            if needle in folded:
                key = (anchor, row["encoding"], row["offset"])
                if key not in seen_anchor:
                    seen_anchor.add(key)
                    anchor_matches.append({"anchor": anchor, **row})
        for kw in KEYWORDS:
            if kw in folded:
                key = (kw, row["encoding"], row["offset"])
                if key not in seen_keyword and len(keyword_hits) < 200:
                    seen_keyword.add(key)
                    keyword_hits.append({"keyword": kw, **row})
    return anchor_matches, keyword_hits


def load_string_imports(pe: pefile.PE) -> dict[int, str]:
    out: dict[int, str] = {}
    if not hasattr(pe, "DIRECTORY_ENTRY_IMPORT"):
        return out
    for module in pe.DIRECTORY_ENTRY_IMPORT:
        dll = module.dll.decode("ascii", errors="replace")
        for imp in module.imports:
            name = imp.name.decode("ascii", errors="replace") if imp.name else f"ord:{imp.ordinal}"
            if name in {"LoadStringA", "LoadStringW"}:
                out[int(imp.address)] = f"{dll}!{name}"
    return out


def import_modules(pe: pefile.PE) -> list[str]:
    if not hasattr(pe, "DIRECTORY_ENTRY_IMPORT"):
        return []
    return sorted({module.dll.decode("ascii", errors="replace") for module in pe.DIRECTORY_ENTRY_IMPORT})


def export_rows(pe: pefile.PE) -> list[dict]:
    rows: list[dict] = []
    if not hasattr(pe, "DIRECTORY_ENTRY_EXPORT"):
        return rows
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    for symbol in pe.DIRECTORY_ENTRY_EXPORT.symbols:
        name = symbol.name.decode("ascii", errors="replace") if symbol.name else None
        rva = int(symbol.address)
        rows.append(
            {
                "name": name,
                "ordinal": int(symbol.ordinal),
                "rva": f"0x{rva:08X}",
                "va": f"0x{image_base + rva:08X}",
                "forwarder": (
                    symbol.forwarder.decode("ascii", errors="replace")
                    if getattr(symbol, "forwarder", None)
                    else None
                ),
            }
        )
    return rows


def checker_export_rows(pe: pefile.PE) -> list[dict]:
    needles = ("check", "checker", "problem", "design")
    return [
        row
        for row in export_rows(pe)
        if row.get("name") and any(n in row["name"].casefold() for n in needles)
    ]


def disassemble(pe: pefile.PE) -> list:
    md = Cs(CS_ARCH_X86, CS_MODE_32)
    md.detail = True
    image_base = int(pe.OPTIONAL_HEADER.ImageBase)
    instructions = []
    for section in pe.sections:
        if not (int(section.Characteristics) & IMAGE_SCN_MEM_EXECUTE):
            continue
        data = section.get_data()
        va = image_base + int(section.VirtualAddress)
        instructions.extend(md.disasm(data, va))
    return instructions


def collect_code_refs(
    pe: pefile.PE, instructions: list, string_ids: set[int], load_string_iat: dict[int, str]
) -> tuple[list[dict], list[dict], list[dict]]:
    direct_refs: list[dict] = []
    load_string_refs: list[dict] = []
    by_cluster: dict[int, set[int]] = defaultdict(set)

    for idx, insn in enumerate(instructions):
        matched_ids: set[int] = set()
        for op in insn.operands:
            if op.type == X86_OP_IMM and int(op.imm) in string_ids:
                matched_ids.add(int(op.imm))
        for value in sorted(matched_ids):
            direct_refs.append(
                {
                    "string_id": value,
                    "va": f"0x{insn.address:08X}",
                    "rva": f"0x{insn.address - int(pe.OPTIONAL_HEADER.ImageBase):08X}",
                    "instruction": f"{insn.mnemonic} {insn.op_str}".strip(),
                }
            )
            by_cluster[insn.address & ~0xFFF].add(value)

        if insn.mnemonic != "call":
            continue
        target_name = None
        for op in insn.operands:
            if op.type == X86_OP_MEM and int(op.mem.disp) in load_string_iat:
                target_name = load_string_iat[int(op.mem.disp)]
                break
        if not target_name:
            continue

        window = instructions[max(0, idx - 12) : idx]
        candidates: list[dict] = []
        for prev in window:
            for op in prev.operands:
                if op.type == X86_OP_IMM and int(op.imm) in string_ids:
                    candidates.append(
                        {
                            "string_id": int(op.imm),
                            "va": f"0x{prev.address:08X}",
                            "instruction": f"{prev.mnemonic} {prev.op_str}".strip(),
                        }
                    )
        load_string_refs.append(
            {
                "call_va": f"0x{insn.address:08X}",
                "call_rva": f"0x{insn.address - int(pe.OPTIONAL_HEADER.ImageBase):08X}",
                "target": target_name,
                "nearby_anchor_ids": candidates,
            }
        )

    clusters = [
        {
            "cluster_va": f"0x{base:08X}",
            "cluster_rva": f"0x{base - int(pe.OPTIONAL_HEADER.ImageBase):08X}",
            "unique_string_ids": sorted(ids),
            "unique_count": len(ids),
        }
        for base, ids in by_cluster.items()
        if len(ids) >= 2
    ]
    clusters.sort(key=lambda row: (-row["unique_count"], row["cluster_va"]))
    return direct_refs, load_string_refs, clusters


def version_strings(pe: pefile.PE) -> list[str]:
    values: list[str] = []
    try:
        for file_info in pe.FileInfo:
            for group in file_info:
                if getattr(group, "Key", b"") == b"StringFileInfo":
                    for table in group.StringTable:
                        for key, value in table.entries.items():
                            if key in {b"FileVersion", b"ProductVersion"}:
                                values.append(value.decode(errors="replace"))
    except Exception:
        pass
    return values


def scan_module(path: Path, anchors: list[str]) -> dict:
    row = {
        "path": str(path),
        "size": path.stat().st_size,
        "sha256": sha256(path),
        "pe": False,
        "version_strings": [],
        "resource_inventory": [],
        "resource_anchor_matches": [],
        "raw_anchor_matches": [],
        "keyword_hits": [],
        "checker_exports": [],
    }
    try:
        pe = pefile.PE(str(path), fast_load=False)
        pe.parse_data_directories()
    except Exception as exc:
        row["error"] = str(exc)
        return row

    row["pe"] = True
    row["version_strings"] = version_strings(pe)
    row["resource_inventory"] = resource_inventory(pe)
    strings = parse_string_tables(pe)
    dialogs = parse_dialogs(pe)
    row["resource_anchor_matches"] = match_anchors(anchors, strings, dialogs)
    raw_matches, keyword_hits = raw_string_matches(path, anchors)
    row["raw_anchor_matches"] = raw_matches
    row["keyword_hits"] = keyword_hits
    row["imports"] = import_modules(pe)
    row["checker_exports"] = checker_export_rows(pe)
    return row


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--pe", required=True)
    ap.add_argument("--anchors", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--expected-sha256")
    ap.add_argument("--min-matched", type=int, default=0)
    ap.add_argument("--scan-root")
    args = ap.parse_args()

    pe_path = Path(args.pe)
    anchors_path = Path(args.anchors)
    out_path = Path(args.out)
    actual_sha = sha256(pe_path)
    if args.expected_sha256 and actual_sha.lower() != args.expected_sha256.lower():
        raise SystemExit(
            f"PE SHA-256 mismatch: expected {args.expected_sha256.lower()} got {actual_sha.lower()}"
        )

    anchors = read_anchors(anchors_path)
    pe = pefile.PE(str(pe_path), fast_load=False)
    pe.parse_data_directories()

    strings = parse_string_tables(pe)
    dialogs = parse_dialogs(pe)
    matches = match_anchors(anchors, strings, dialogs)
    raw_matches, keyword_hits = raw_string_matches(pe_path, anchors)

    matched_string_ids = {
        int(row["string_id"])
        for row in matches
        if row.get("kind") == "stringtable" and "string_id" in row
    }
    imports = load_string_imports(pe)
    instructions = disassemble(pe)
    direct_refs, load_string_refs, clusters = collect_code_refs(
        pe, instructions, matched_string_ids, imports
    )
    checker_ids = checker_identifier_rows(pe, pe_path, instructions)
    checker_id_clusters = checker_identifier_clusters(checker_ids)
    checker_pointer_hits = checker_pointer_rows(pe, pe_path, checker_ids)
    checker_pointer_tables = checker_pointer_clusters(checker_pointer_hits)
    checker_pointer_table_refs = checker_pointer_cluster_code_refs(
        pe, instructions, checker_pointer_tables
    )
    checker_fixed_records = decode_checker_fixed_records(
        pe, pe_path, checker_pointer_tables
    )
    checker_shared_target_headers = shared_record_target_headers(
        pe, pe_path, checker_fixed_records
    )
    checker_cluster_base_pointers = cluster_base_pointer_rows(
        pe, pe_path, checker_pointer_tables
    )
    checker_cluster_base_occurrence_instructions = pointer_occurrence_instructions(
        pe, instructions, checker_cluster_base_pointers
    )
    checker_cluster_base_slot_code_refs = code_refs_to_pointer_slots(
        pe, instructions, checker_cluster_base_pointers
    )
    loader_iat = import_iat_map(
        pe,
        {
            "GetProcAddress",
            "LoadLibraryA",
            "LoadLibraryW",
            "GetModuleHandleA",
            "GetModuleHandleW",
            "FreeLibrary",
        },
    )
    loader_thunks = import_thunk_map(pe, instructions, loader_iat)
    loader_calls = imported_call_sites(pe, instructions, loader_iat, loader_thunks)
    loader_checker_proximity = loader_proximity(
        pe,
        instructions,
        loader_calls,
        checker_pointer_tables,
        checker_cluster_base_pointers,
        checker_ids,
    )

    companion_modules: list[dict] = []
    if args.scan_root:
        root = Path(args.scan_root)
        if root.exists():
            seen_hashes = set()
            for path in sorted(p for p in root.rglob("*") if p.is_file() and p.suffix.lower() in {".exe", ".dll"}):
                digest = sha256(path)
                if digest in seen_hashes:
                    continue
                seen_hashes.add(digest)
                companion_modules.append(scan_module(path, anchors))

    matched_anchor_names = sorted({row["anchor"] for row in matches})
    raw_anchor_names = sorted({row["anchor"] for row in raw_matches})
    companion_anchor_names = sorted(
        {
            match["anchor"]
            for module in companion_modules
            for match in module.get("resource_anchor_matches", []) + module.get("raw_anchor_matches", [])
        }
    )

    report = {
        "schema": "dc-static-01.v2",
        "pe": {
            "path": str(pe_path),
            "sha256": actual_sha,
            "machine": int(pe.FILE_HEADER.Machine),
            "image_base": f"0x{int(pe.OPTIONAL_HEADER.ImageBase):08X}",
            "version_strings": version_strings(pe),
            "imports": import_modules(pe),
            "resource_inventory": resource_inventory(pe),
            "checker_exports": checker_export_rows(pe),
            "checker_identifier_strings": checker_ids,
            "checker_identifier_clusters": checker_id_clusters,
            "checker_pointer_hits": checker_pointer_hits,
            "checker_pointer_clusters": checker_pointer_tables,
            "checker_pointer_cluster_code_refs": checker_pointer_table_refs,
            "checker_fixed_stride_records": checker_fixed_records,
            "checker_shared_record_target_headers": checker_shared_target_headers,
            "checker_cluster_base_pointers": checker_cluster_base_pointers,
            "checker_cluster_base_occurrence_instructions": checker_cluster_base_occurrence_instructions,
            "checker_cluster_base_slot_code_refs": checker_cluster_base_slot_code_refs,
            "loader_import_iat": [
                {"iat_va": f"0x{va:08X}", "name": name}
                for va, name in sorted(loader_iat.items())
            ],
            "loader_import_thunks": [
                {"thunk_va": f"0x{va:08X}", "target": name}
                for va, name in sorted(loader_thunks.items())
            ],
            "loader_call_sites": loader_calls,
            "loader_checker_proximity": loader_checker_proximity,
        },
        "anchors_total": len(anchors),
        "resource_anchors_matched": len(matched_anchor_names),
        "resource_matched_anchor_names": matched_anchor_names,
        "raw_anchors_matched": len(raw_anchor_names),
        "raw_matched_anchor_names": raw_anchor_names,
        "resource_matches": matches,
        "raw_anchor_matches": raw_matches,
        "keyword_hits": keyword_hits,
        "load_string_imports": [
            {"iat_va": f"0x{va:08X}", "name": name} for va, name in sorted(imports.items())
        ],
        "direct_immediate_xrefs": direct_refs,
        "load_string_call_xrefs": load_string_refs,
        "candidate_code_clusters": clusters,
        "companion_modules": companion_modules,
        "companion_anchor_names": companion_anchor_names,
        "guardrail": (
            "Resource identity, raw-string identity and code proximity are static observations only; "
            "candidate clusters are not claimed as Design Checker dispatchers."
        ),
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")

    module_hits = [
        (
            Path(m["path"]).name,
            len({x["anchor"] for x in m.get("resource_anchor_matches", [])}),
            len({x["anchor"] for x in m.get("raw_anchor_matches", [])}),
            len(m.get("keyword_hits", [])),
            len(m.get("checker_exports", [])),
        )
        for m in companion_modules
    ]
    summary = out_path.with_suffix(".md")
    lines = [
        "# DC-STATIC-01 static resource/raw pass",
        "",
        f"- PE SHA-256: `{actual_sha}`",
        f"- anchors: {len(anchors)}",
        f"- MSPUB resource anchors: {len(matched_anchor_names)}",
        f"- MSPUB raw-string anchors: {len(raw_anchor_names)}",
        f"- MSPUB keyword hits: {len(keyword_hits)}",
        f"- MSPUB checker-like exports: {len(checker_export_rows(pe))}",
        f"- MSPUB checker identifier strings: {len(checker_ids)}",
        f"- checker identifiers with code refs: {sum(1 for row in checker_ids if row['code_refs'])}",
        f"- checker identifier clusters: {len(checker_id_clusters)}",
        f"- checker pointer hits (exact RVA/VA dwords): {len(checker_pointer_hits)}",
        f"- checker pointer clusters (>=2 identifiers): {len(checker_pointer_tables)}",
        f"- checker pointer clusters with code refs: {sum(1 for row in checker_pointer_table_refs if row['refs'])}",
        f"- fixed 16-byte checker record arrays: {len(checker_fixed_records)}",
        f"- executable pointer fields after name: {sum(row['executable_pointer_fields_after_name'] for row in checker_fixed_records)}",
        f"- shared fixed-record VA targets decoded: {len(checker_shared_target_headers)}",
        f"- second-order pointers to checker array bases: {len(checker_cluster_base_pointers)}",
        f"- second-order .text occurrences bound to instructions: {sum(1 for row in checker_cluster_base_occurrence_instructions if row['containing_instruction'])}",
        f"- code refs to second-order data slots: {len(checker_cluster_base_slot_code_refs)}",
        f"- loader API imports: {len(loader_iat)}",
        f"- loader import thunks: {len(loader_thunks)}",
        f"- loader API call sites: {len(loader_calls)}",
        f"- loader calls with nearby checker hits: {sum(1 for row in loader_checker_proximity if row['nearby_checker_hits'])}",
        f"- STRINGTABLE IDs matched: {len(matched_string_ids)}",
        f"- direct code immediates to matched IDs: {len(direct_refs)}",
        f"- LoadString call sites: {len(load_string_refs)}",
        f"- multi-ID 4 KiB code clusters: {len(clusters)}",
        f"- companion modules scanned: {len(companion_modules)}",
        "",
        "## Companion module hits",
    ]
    if module_hits:
        for name, resource_count, raw_count, keyword_count, export_count in module_hits:
            lines.append(
                f"- {name}: resource anchors={resource_count}, raw anchors={raw_count}, "
                f"keyword hits={keyword_count}, checker-like exports={export_count}"
            )
    else:
        lines.append("- none")
    lines += [
        "",
        "A zero exact-anchor result is a bounded negative for the scanned neutral modules, not evidence that Design Checker is absent.",
        "Static proximity is not promoted to a dispatcher without a later runtime join.",
        "",
    ]
    summary.write_text("\n".join(lines), encoding="utf-8")

    print(
        json.dumps(
            {
                "resource_anchors": len(matched_anchor_names),
                "raw_anchors": len(raw_anchor_names),
                "companion_anchors": len(companion_anchor_names),
                "checker_exports": len(checker_export_rows(pe)),
                "checker_identifiers": len(checker_ids),
                "checker_identifiers_with_refs": sum(1 for row in checker_ids if row["code_refs"]),
                "checker_identifier_clusters": len(checker_id_clusters),
                "checker_pointer_hits": len(checker_pointer_hits),
                "checker_pointer_clusters": len(checker_pointer_tables),
                "checker_pointer_clusters_with_code_refs": sum(
                    1 for row in checker_pointer_table_refs if row["refs"]
                ),
                "checker_fixed_record_arrays": len(checker_fixed_records),
                "checker_executable_pointer_fields_after_name": sum(
                    row["executable_pointer_fields_after_name"]
                    for row in checker_fixed_records
                ),
                "checker_shared_record_targets": len(checker_shared_target_headers),
                "checker_cluster_base_pointers": len(checker_cluster_base_pointers),
                "checker_cluster_base_text_occurrences_bound_to_instructions": sum(
                    1 for row in checker_cluster_base_occurrence_instructions
                    if row["containing_instruction"]
                ),
                "checker_cluster_base_slot_code_refs": len(checker_cluster_base_slot_code_refs),
                "loader_imports": len(loader_iat),
                "loader_import_thunks": len(loader_thunks),
                "loader_call_sites": len(loader_calls),
                "loader_calls_with_checker_proximity": sum(
                    1 for row in loader_checker_proximity
                    if row["nearby_checker_hits"]
                ),
                "string_ids": len(matched_string_ids),
                "direct_refs": len(direct_refs),
                "clusters": len(clusters),
            }
        )
    )
    if len(matched_anchor_names) < args.min_matched:
        raise SystemExit(
            f"static resource gate failed: matched {len(matched_anchor_names)} anchors, "
            f"required at least {args.min_matched}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
