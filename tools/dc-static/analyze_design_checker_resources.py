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
