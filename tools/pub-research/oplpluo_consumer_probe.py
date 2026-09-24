#!/usr/bin/env python3
"""Feature-scoped MSPUB14 static probe for PUB-T-451 / OPLPLUO-SEM-01.

Boundary:
- exact MSPUB.EXE 14.0.7162.5000 only;
- OplPluo/OplUo/Rguo + validator/constructor/consumer surfaces only;
- static implementation evidence, not a semantic promotion by itself.
"""

from __future__ import annotations

import argparse
import json
import re
import struct
from collections import defaultdict
from pathlib import Path

import capstone
import pefile

EXPECTED_SHA256 = "27c00f7f06957f24d392f9c61fbd3b40282f15dc595caf66785b561f559d7b97"
TARGET_DESCRIPTOR_CLASSES = ("OplMocd", "OplPluo", "OplUo", "OplRguo")
TARGET_RTTI_CLASSES = (
    "OplPluo",
    "OplUo",
    "OplRguo",
    "CGKValOplPluo",
    "CGKValOplUo",
    "CGKValOplRguo",
)
EXPECTED_FIELDS = {
    "OplMocd": {0x02: "Ohpluo"},
    "OplPluo": {0x01: "CUo", 0x02: "Rguo"},
}
ROW_STRIDE = 18
CLASS_SENTINEL = 0x07FF
RAW59 = 0x59
KNOWN_COMMON_OTY_CTOR = 0x2E1142B4


def sha256_file(path: Path) -> str:
    import hashlib
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


class PEView:
    def __init__(self, path: Path):
        self.path = path
        self.blob = path.read_bytes()
        self.pe = pefile.PE(str(path))
        self.image_base = self.pe.OPTIONAL_HEADER.ImageBase
        self.sections = []
        for s in self.pe.sections:
            name = s.Name.rstrip(b"\0").decode("ascii", "replace")
            self.sections.append(
                {
                    "name": name,
                    "rva": s.VirtualAddress,
                    "va": self.image_base + s.VirtualAddress,
                    "raw": s.PointerToRawData,
                    "raw_size": s.SizeOfRawData,
                    "vsize": s.Misc_VirtualSize,
                    "chars": s.Characteristics,
                    "data": self.blob[
                        s.PointerToRawData : s.PointerToRawData + s.SizeOfRawData
                    ],
                }
            )

    def u16(self, off: int) -> int:
        return struct.unpack_from("<H", self.blob, off)[0]

    def u32(self, off: int) -> int:
        return struct.unpack_from("<I", self.blob, off)[0]

    def va_to_off(self, va: int) -> int | None:
        rva = va - self.image_base
        for s in self.sections:
            span = max(s["raw_size"], s["vsize"])
            if s["rva"] <= rva < s["rva"] + span:
                delta = rva - s["rva"]
                if delta >= s["raw_size"]:
                    return None
                return s["raw"] + delta
        return None

    def off_to_va(self, off: int) -> int | None:
        for s in self.sections:
            if s["raw"] <= off < s["raw"] + s["raw_size"]:
                return s["va"] + (off - s["raw"])
        return None

    def section_for_va(self, va: int | None) -> str | None:
        if va is None:
            return None
        rva = va - self.image_base
        for s in self.sections:
            if s["rva"] <= rva < s["rva"] + max(s["raw_size"], s["vsize"]):
                return s["name"]
        return None

    def is_text_va(self, va: int | None) -> bool:
        return self.section_for_va(va) == ".text"

    def read_ascii_va(self, va: int, limit: int = 256) -> str | None:
        off = self.va_to_off(va)
        if off is None:
            return None
        raw = self.blob[off : off + limit].split(b"\0", 1)[0]
        if len(raw) < 2:
            return None
        try:
            value = raw.decode("ascii")
        except UnicodeDecodeError:
            return None
        if not all(0x20 <= ord(ch) <= 0x7E for ch in value):
            return None
        return value

    def read_utf16_va(self, va: int, limit: int = 160) -> str | None:
        off = self.va_to_off(va)
        if off is None:
            return None
        units = bytearray()
        for p in range(off, min(len(self.blob) - 1, off + limit * 2), 2):
            pair = self.blob[p : p + 2]
            if pair == b"\0\0":
                break
            units.extend(pair)
        if len(units) < 4:
            return None
        try:
            value = units.decode("utf-16le")
        except UnicodeDecodeError:
            return None
        if not all(0x20 <= ord(ch) <= 0x7E for ch in value):
            return None
        return value


def find_all(blob: bytes, needle: bytes):
    start = 0
    while True:
        pos = blob.find(needle, start)
        if pos < 0:
            break
        yield pos
        start = pos + 1


def scan_descriptor_tables(v: PEView):
    ident = re.compile(r"^[A-Za-z_?][A-Za-z0-9_.$?@:+-]{1,126}$")
    cache: dict[int, tuple[str, str, int] | None] = {}

    def name_at_va(va: int):
        if va in cache:
            return cache[va]
        off = v.va_to_off(va)
        if off is None:
            cache[va] = None
            return None
        a = v.read_ascii_va(va)
        w = v.read_utf16_va(va)
        if a and a.startswith("Opl") and ident.fullmatch(a):
            result = ("ascii", a, off)
        elif w and w.startswith("Opl") and ident.fullmatch(w):
            result = ("utf16le", w, off)
        elif a and ident.fullmatch(a):
            result = ("ascii", a, off)
        elif w and ident.fullmatch(w):
            result = ("utf16le", w, off)
        else:
            result = None
        cache[va] = result
        return result

    candidates = []
    blob = v.blob
    for off in range(0, len(blob) - ROW_STRIDE + 1):
        if v.u16(off + 4) != CLASS_SENTINEL:
            continue
        name_ptr = v.u32(off)
        resolved = name_at_va(name_ptr)
        if not resolved or not resolved[1].startswith("Opl"):
            continue
        enc, class_name, class_name_off = resolved
        header = {
            "record_offset": off,
            "record_va": v.off_to_va(off),
            "name_ptr": name_ptr,
            "name_file_offset": class_name_off,
            "name_encoding": enc,
            "class_name": class_name,
            "handler_ptr": v.u32(off + 8),
            "handler_section": v.section_for_va(v.u32(off + 8)),
            "metadata_type": v.u16(off + 12),
            "flags": v.u16(off + 16),
        }
        rows = []
        pos = off + ROW_STRIDE
        prev = -1
        while pos + ROW_STRIDE <= len(blob) and len(rows) < 2048:
            fid = v.u16(pos + 4)
            ptr = v.u32(pos)
            item = name_at_va(ptr)
            if fid == CLASS_SENTINEL and item and item[1].startswith("Opl"):
                break
            if fid > 0x07FE or fid <= prev or not item:
                break
            _, field_name, field_name_off = item
            if not ident.fullmatch(field_name):
                break
            handler = v.u32(pos + 8)
            rows.append(
                {
                    "record_offset": pos,
                    "record_va": v.off_to_va(pos),
                    "name_ptr": ptr,
                    "name_file_offset": field_name_off,
                    "field_id": fid,
                    "field_name": field_name,
                    "handler_ptr": handler,
                    "handler_section": v.section_for_va(handler),
                    "metadata_type": v.u16(pos + 12),
                    "flags": v.u16(pos + 16),
                    "raw_hex": blob[pos : pos + ROW_STRIDE].hex(),
                }
            )
            prev = fid
            pos += ROW_STRIDE
        if rows:
            candidates.append({"header": header, "rows": rows})

    by_class = {}
    for c in candidates:
        name = c["header"]["class_name"]
        if name not in by_class or len(c["rows"]) > len(by_class[name]["rows"]):
            by_class[name] = c

    target = {name: by_class.get(name) for name in TARGET_DESCRIPTOR_CLASSES}
    expected_results = []
    for cls, expected in EXPECTED_FIELDS.items():
        table = target.get(cls)
        actual = {} if table is None else {r["field_id"]: r["field_name"] for r in table["rows"]}
        for fid, name in expected.items():
            expected_results.append(
                {
                    "class": cls,
                    "field_id": fid,
                    "expected": name,
                    "actual": actual.get(fid),
                    "match": actual.get(fid) == name,
                }
            )
    return target, expected_results


def disassemble_text(v: PEView):
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
    md.detail = True
    md.skipdata = True
    instructions = []
    for s in v.sections:
        if s["name"] != ".text":
            continue
        for ins in md.disasm(s["data"], s["va"]):
            if ins.mnemonic == ".byte":
                continue
            refs = []
            calls = []
            for op in ins.operands:
                if op.type == capstone.x86.X86_OP_IMM:
                    val = op.imm & 0xFFFFFFFF
                    refs.append(val)
                    if ins.mnemonic == "call":
                        calls.append(val)
                elif op.type == capstone.x86.X86_OP_MEM:
                    if op.mem.base == 0 and op.mem.index == 0:
                        refs.append(op.mem.disp & 0xFFFFFFFF)
            instructions.append(
                {
                    "address": ins.address,
                    "size": ins.size,
                    "mnemonic": ins.mnemonic,
                    "op_str": ins.op_str,
                    "refs": refs,
                    "calls": calls,
                    "raw": bytes(ins.bytes).hex(),
                }
            )
    by_addr = {x["address"]: i for i, x in enumerate(instructions)}
    return instructions, by_addr


def slim(x):
    return {
        "address": f"0x{x['address']:08X}",
        "mnemonic": x["mnemonic"],
        "op_str": x["op_str"],
    }


def context(instructions, idx, before=20, after=32):
    lo = max(0, idx - before)
    hi = min(len(instructions), idx + after + 1)
    base = instructions[idx]["address"]
    return [
        slim(x)
        for x in instructions[lo:hi]
        if abs(x["address"] - base) <= 0x180
    ]


def code_refs(instructions, targets: dict[str, int]):
    rev = defaultdict(list)
    target_values = set(targets.values())
    for idx, ins in enumerate(instructions):
        matched = target_values.intersection(ins["refs"])
        if not matched:
            continue
        for val in matched:
            labels = [k for k, x in targets.items() if x == val]
            for label in labels:
                rev[label].append(
                    {
                        "instruction": slim(ins),
                        "context": context(instructions, idx),
                    }
                )
    return dict(rev)


def annotate_ref(v: PEView, value: int):
    sec = v.section_for_va(value)
    if not sec:
        return None
    return {
        "value": f"0x{value:08X}",
        "section": sec,
        "ascii": v.read_ascii_va(value),
        "utf16": v.read_utf16_va(value),
    }


def method_body(v: PEView, instructions, by_addr, start_va: int, limit=220):
    idx = by_addr.get(start_va)
    if idx is None:
        return []
    rows = []
    for ins in instructions[idx : idx + limit]:
        anns = []
        for value in ins["refs"]:
            ann = annotate_ref(v, value)
            if ann and (ann["section"] != ".text" or ann["ascii"] or ann["utf16"]):
                anns.append(ann)
        rows.append({**slim(ins), "annotations": anns})
        if ins["mnemonic"].startswith("ret"):
            break
    return rows


def scan_rtti(v: PEView, instructions, by_addr):
    results = {}
    for cls in TARGET_RTTI_CLASSES:
        decorated = f".?AV{cls}@@"
        hits = list(find_all(v.blob, decorated.encode("ascii") + b"\0"))
        cls_rows = []
        for name_off in hits:
            td_off = name_off - 8
            td_va = v.off_to_va(td_off)
            if td_va is None:
                continue
            cols = []
            packed = struct.pack("<I", td_va)
            for ptr_off in find_all(v.blob, packed):
                col_off = ptr_off - 12
                if col_off < 0 or col_off + 20 > len(v.blob):
                    continue
                if v.u32(col_off) not in (0, 1):
                    continue
                if v.u32(col_off + 12) != td_va:
                    continue
                class_desc = v.u32(col_off + 16)
                if v.section_for_va(class_desc) not in (".rdata", ".data"):
                    continue
                col_va = v.off_to_va(col_off)
                if col_va is None:
                    continue
                cols.append(
                    {
                        "col_va": col_va,
                        "class_descriptor": class_desc,
                        "object_offset": v.u32(col_off + 4),
                        "cd_offset": v.u32(col_off + 8),
                    }
                )
            vtables = []
            for col in cols:
                needle = struct.pack("<I", col["col_va"])
                for slot_off in find_all(v.blob, needle):
                    vt_off = slot_off + 4
                    vt_va = v.off_to_va(vt_off)
                    if vt_va is None or vt_off + 4 > len(v.blob):
                        continue
                    first = v.u32(vt_off)
                    if not v.is_text_va(first):
                        continue
                    entries = []
                    for n in range(40):
                        p = vt_off + n * 4
                        if p + 4 > len(v.blob):
                            break
                        method = v.u32(p)
                        if not v.is_text_va(method):
                            break
                        entries.append(method)
                    if not entries:
                        continue
                    vtables.append(
                        {
                            "vtable_va": vt_va,
                            "col_va": col["col_va"],
                            "entries": entries,
                        }
                    )
            dedup = {}
            for row in vtables:
                dedup[row["vtable_va"]] = row
            vtables = list(dedup.values())

            cls_rows.append(
                {
                    "decorated_name": decorated,
                    "name_offset": name_off,
                    "type_descriptor_va": td_va,
                    "cols": cols,
                    "vtables": vtables,
                }
            )

        # Code references to vtables and bounded method summaries.
        all_vtables = sorted(
            {
                vt["vtable_va"]
                for row in cls_rows
                for vt in row["vtables"]
            }
        )
        vt_ref_labels = {f"vtable_{i}": va for i, va in enumerate(all_vtables)}
        refs = code_refs(instructions, vt_ref_labels) if vt_ref_labels else {}
        methods = {}
        for row in cls_rows:
            for vt in row["vtables"]:
                for method in vt["entries"][:24]:
                    key = f"0x{method:08X}"
                    if key not in methods:
                        body = method_body(v, instructions, by_addr, method)
                        methods[key] = {
                            "body": body,
                            "direct_calls": sorted(
                                {
                                    call
                                    for ins in instructions[
                                        by_addr.get(method, 0) : by_addr.get(method, 0) + min(220, len(body) + 16)
                                    ]
                                    for call in ins["calls"]
                                }
                            )
                            if method in by_addr
                            else [],
                        }
        results[cls] = {
            "type_descriptors": cls_rows,
            "vtable_code_refs": refs,
            "methods": methods,
        }
    return results


def infer_function_start(instructions, idx):
    # Conservative heuristic: nearest classic x86 frame prologue inside 0x100 bytes.
    here = instructions[idx]["address"]
    lo = max(0, idx - 80)
    for j in range(idx, lo - 1, -1):
        if here - instructions[j]["address"] > 0x100:
            break
        if (
            instructions[j]["mnemonic"] == "push"
            and instructions[j]["op_str"] == "ebp"
            and j + 1 < len(instructions)
            and instructions[j + 1]["mnemonic"] == "mov"
            and instructions[j + 1]["op_str"].replace(" ", "") == "ebp,esp"
        ):
            return instructions[j]["address"]
    return None


def constructor_probe(v: PEView, instructions):
    # Use independently established OplPluo RTTI vtable when present via RTTI scan,
    # but discover it from decorated type metadata in this exact binary.
    rtti = scan_rtti(v, instructions, {x["address"]: i for i, x in enumerate(instructions)})
    pluo_vtables = sorted(
        {
            vt["vtable_va"]
            for row in rtti.get("OplPluo", {}).get("type_descriptors", [])
            for vt in row.get("vtables", [])
        }
    )
    candidates = []
    for idx, ins in enumerate(instructions):
        matched = [va for va in pluo_vtables if va in ins["refs"]]
        if not matched:
            continue
        prev = instructions[max(0, idx - 24) : idx + 1]
        raw59_hits = [
            x for x in prev
            if RAW59 in x["refs"]
            and x["mnemonic"] in ("push", "mov", "cmp")
        ]
        base_ctor_calls = [
            x for x in prev
            if KNOWN_COMMON_OTY_CTOR in x["calls"]
        ]
        start = infer_function_start(instructions, idx)
        direct_callers = []
        if start is not None:
            direct_callers = [
                x["address"]
                for x in instructions
                if start in x["calls"]
            ]
        candidates.append(
            {
                "assignment": slim(ins),
                "matched_vtables": [f"0x{x:08X}" for x in matched],
                "raw59_nearby": [slim(x) for x in raw59_hits],
                "common_oty_ctor_nearby": [slim(x) for x in base_ctor_calls],
                "heuristic_function_start": None if start is None else f"0x{start:08X}",
                "direct_callers_to_heuristic_start": [f"0x{x:08X}" for x in direct_callers],
                "context": context(instructions, idx, before=36, after=28),
            }
        )
    return rtti, candidates


def descriptor_consumers(v: PEView, target_tables, instructions, by_addr):
    targets = {}
    target_meta = {}
    for cls, table in target_tables.items():
        if not table:
            continue
        hva = table["header"].get("record_va")
        if hva:
            label = f"{cls}.class_header"
            targets[label] = hva
            target_meta[label] = {
                "class": cls,
                "kind": "class_header",
                "handler_ptr": table["header"].get("handler_ptr"),
            }
        hptr = table["header"].get("handler_ptr")
        if hptr:
            label = f"{cls}.class_handler"
            targets[label] = hptr
            target_meta[label] = {"class": cls, "kind": "class_handler"}
        for row in table["rows"]:
            label = f"{cls}.field0x{row['field_id']:03X}.{row['field_name']}.row"
            if row.get("record_va"):
                targets[label] = row["record_va"]
                target_meta[label] = {
                    "class": cls,
                    "kind": "field_row",
                    "field_id": row["field_id"],
                    "field_name": row["field_name"],
                }
            if row.get("handler_ptr"):
                hlabel = f"{cls}.field0x{row['field_id']:03X}.{row['field_name']}.handler"
                targets[hlabel] = row["handler_ptr"]
                target_meta[hlabel] = {
                    "class": cls,
                    "kind": "field_handler",
                    "field_id": row["field_id"],
                    "field_name": row["field_name"],
                }

    refs = code_refs(instructions, targets)
    handler_bodies = {}
    for label, va in targets.items():
        meta = target_meta[label]
        if meta["kind"] not in ("class_handler", "field_handler") or not v.is_text_va(va):
            continue
        body = method_body(v, instructions, by_addr, va)
        direct_callers = [
            ins["address"] for ins in instructions if va in ins["calls"]
        ]
        handler_bodies[label] = {
            "va": f"0x{va:08X}",
            "section": v.section_for_va(va),
            "direct_callers": [f"0x{x:08X}" for x in direct_callers],
            "body": body,
        }
    return targets, target_meta, refs, handler_bodies


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("exe", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    actual = sha256_file(args.exe)
    if actual.lower() != EXPECTED_SHA256:
        raise SystemExit(f"MSPUB hash mismatch: {actual}")

    v = PEView(args.exe)
    tables, expected = scan_descriptor_tables(v)
    bad = [x for x in expected if not x["match"]]
    if bad:
        raise SystemExit("descriptor anchor mismatch: " + json.dumps(bad))

    if tables.get("OplPluo") is None:
        raise SystemExit("OplPluo descriptor table not recovered")
    if tables.get("OplUo") is None:
        raise SystemExit("OplUo descriptor table not recovered")

    instructions, by_addr = disassemble_text(v)
    desc_targets, desc_meta, desc_refs, handlers = descriptor_consumers(
        v, tables, instructions, by_addr
    )
    rtti, ctor_candidates = constructor_probe(v, instructions)

    # Cross-link RTTI method bodies against descriptor/table targets.
    descriptor_values = set(desc_targets.values())
    method_descriptor_links = []
    for cls, info in rtti.items():
        for method_va, m in info.get("methods", {}).items():
            for ins in m.get("body", []):
                # Re-resolve instruction from address to get refs.
                addr = int(ins["address"], 16)
                idx = by_addr.get(addr)
                if idx is None:
                    continue
                matched = descriptor_values.intersection(instructions[idx]["refs"])
                if matched:
                    method_descriptor_links.append(
                        {
                            "rtti_class": cls,
                            "method": method_va,
                            "instruction": ins,
                            "matched_targets": [
                                label
                                for label, va in desc_targets.items()
                                if va in matched
                            ],
                        }
                    )

    result = {
        "scope": {
            "task": "PUB-T-451 / OPLPLUO-SEM-01",
            "binary": "MSPUB.EXE 14.0.7162.5000 x86",
            "sha256": actual,
            "boundary": (
                "Feature-scoped static implementation evidence only. "
                "Do not expand abbreviations, infer PAGE identity, or promote "
                "field semantics without a consumer/persisted/runtime join."
            ),
        },
        "descriptor_anchor_results": expected,
        "descriptor_tables": tables,
        "descriptor_targets": {
            k: f"0x{v:08X}" for k, v in desc_targets.items()
        },
        "descriptor_target_meta": desc_meta,
        "descriptor_code_refs": desc_refs,
        "handler_bodies": handlers,
        "rtti": rtti,
        "oplpluo_constructor_candidates": ctor_candidates,
        "rtti_method_descriptor_links": method_descriptor_links,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    pluo = tables["OplPluo"]
    uo = tables["OplUo"]
    summary = {
        "oplpluo_fields": [
            {
                "id": f"0x{x['field_id']:03X}",
                "name": x["field_name"],
                "meta": f"0x{x['metadata_type']:02X}",
                "handler": f"0x{x['handler_ptr']:08X}",
                "handler_section": x["handler_section"],
            }
            for x in pluo["rows"]
        ],
        "opluo_fields": [
            {
                "id": f"0x{x['field_id']:03X}",
                "name": x["field_name"],
                "meta": f"0x{x['metadata_type']:02X}",
                "handler": f"0x{x['handler_ptr']:08X}",
                "handler_section": x["handler_section"],
            }
            for x in uo["rows"]
        ],
        "descriptor_ref_counts": {k: len(v) for k, v in desc_refs.items()},
        "handler_body_count": len(handlers),
        "rtti_classes_found": {
            cls: sum(len(row.get("vtables", [])) for row in info.get("type_descriptors", []))
            for cls, info in rtti.items()
        },
        "oplpluo_constructor_candidate_count": len(ctor_candidates),
        "raw59_constructor_candidates": sum(
            1
            for x in ctor_candidates
            if x["raw59_nearby"] and x["common_oty_ctor_nearby"]
        ),
        "direct_constructor_callers": sorted(
            {
                caller
                for x in ctor_candidates
                for caller in x["direct_callers_to_heuristic_start"]
            }
        ),
        "rtti_method_descriptor_links": len(method_descriptor_links),
    }

    summary_path = args.out.with_name("oplpluo-consumer-summary.json")
    summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    text_lines = [
        "PUB-T-451 / OPLPLUO-SEM-01 exact MSPUB14 static consumer pass",
        f"MSPUB SHA256: {actual}",
        "",
        "OplPluo fields:",
    ]
    for row in summary["oplpluo_fields"]:
        text_lines.append(
            f"  {row['id']} {row['name']} meta={row['meta']} handler={row['handler']} {row['handler_section']}"
        )
    text_lines.append("")
    text_lines.append("OplUo fields:")
    for row in summary["opluo_fields"]:
        text_lines.append(
            f"  {row['id']} {row['name']} meta={row['meta']} handler={row['handler']} {row['handler_section']}"
        )
    text_lines.extend(
        [
            "",
            f"handler bodies: {summary['handler_body_count']}",
            f"OplPluo constructor candidates: {summary['oplpluo_constructor_candidate_count']}",
            f"raw59 + common OTY ctor candidates: {summary['raw59_constructor_candidates']}",
            f"direct callers to inferred constructor starts: {summary['direct_constructor_callers']}",
            f"RTTI method -> descriptor links: {summary['rtti_method_descriptor_links']}",
            "",
            "RTTI vtable counts:",
        ]
    )
    for cls, count in summary["rtti_classes_found"].items():
        text_lines.append(f"  {cls}: {count}")
    (args.out.with_name("summary.txt")).write_text("\n".join(text_lines) + "\n", encoding="utf-8")
    print("\n".join(text_lines))


if __name__ == "__main__":
    main()
