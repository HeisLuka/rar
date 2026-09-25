#!/usr/bin/env python3
"""Bounded four-way raw audit for FALSE-OMISSION-01 outputs.

Input is the canonical JSON emitted by:

    cargo run -q -p pub-cli -- contents FILE.pub --json

The tool deliberately does NOT assert that COM Fill.Visible/Line.Visible map to
FFilled/FLine. It only finds syntactically bounded OplOdpoFillStyle-like and
OplOdpoLineStyle-like variable blocks and reports whether the candidate boolean
headers are absent/present across source, resave-control, fill-on and line-on.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path

from false_omission_packed import bounds, decode_header, packed_header


ROLE_ORDER = ("source", "control", "fill", "line")

STYLE_SPECS = {
    "fill": {
        "container_field_id": 0x02,
        "container_meta": 0x13,
        "target_field_id": 0x3C,
        "target_meta": 0x00,
        "target_name": "FFilled",
        "owner_class": "OplOdpoFillStyle",
        "known_fields": {
            (0x01, 0x20),
            (0x02, 0x20),
            (0x04, 0x20),
            (0x0D, 0x20),
            (0x1F, 0x20),
            (0x21, 0x20),
            (0x3A, 0x08),
            (0x3B, 0x08),
            (0x3C, 0x08),
        },
    },
    "line": {
        "container_field_id": 0x03,
        "container_meta": 0x13,
        "target_field_id": 0x3D,
        "target_meta": 0x00,
        "target_name": "FLine",
        "owner_class": "OplOdpoLineStyle",
        "known_fields": {
            (0x01, 0x20),
            (0x03, 0x20),
            (0x0C, 0x20),
            (0x0F, 0x20),
            (0x17, 0x20),
            (0x18, 0x20),
            (0x3A, 0x08),
            (0x3B, 0x08),
            (0x3C, 0x08),
            (0x3D, 0x08),
        },
    },
}


def load_contents(path: Path, role: str) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    refs = {}
    duplicate_seq = []
    for row in data.get("references", []):
        hx = row.get("chunk_hex")
        if not hx:
            continue
        seq = int(row["seq_num"])
        if seq in refs:
            duplicate_seq.append(seq)
            continue
        refs[seq] = {
            "seq_num": seq,
            "chunk": bytes.fromhex(hx),
            "raw_types": [
                int(x["value"]) if isinstance(x, dict) else int(x)
                for x in row.get("raw_types", [])
            ],
        }
    return {
        "role": role,
        "path": str(path),
        "family": data.get("family"),
        "serialization_revision": data.get("serialization_revision"),
        "references": refs,
        "duplicate_seq": sorted(set(duplicate_seq)),
    }


def parse_block_range(chunk: bytes, start: int, end: int) -> list[dict]:
    if not 0 <= start <= end <= len(chunk):
        raise ValueError("invalid bounded block range")

    out = []
    cursor = start
    while cursor < end:
        if cursor + 2 > end:
            raise ValueError("truncated block header inside bounded container")
        field_id, wire = decode_header(chunk[cursor], chunk[cursor + 1])
        block_end, content_start, content_end = bounds(chunk, cursor, wire)
        if block_end > end:
            raise ValueError("child block crosses parent bound")
        out.append(
            {
                "offset": cursor,
                "field_id": field_id,
                "wire_base": wire,
                "end": block_end,
                "content_start": content_start,
                "content_end": content_end,
            }
        )
        cursor = block_end

    if cursor != end:
        raise ValueError("bounded block sequence did not end exactly")
    return out


def find_style_candidates(chunk: bytes, style_name: str) -> list[dict]:
    spec = STYLE_SPECS[style_name]
    container_header = packed_header(
        spec["container_field_id"],
        spec["container_meta"],
    )
    target_header = packed_header(
        spec["target_field_id"],
        spec["target_meta"],
    )
    candidates = []
    search_from = 4

    while True:
        pos = chunk.find(container_header, search_from)
        if pos < 0:
            break
        search_from = pos + 1

        try:
            field_id, wire = decode_header(chunk[pos], chunk[pos + 1])
            block_end, content_start, content_end = bounds(chunk, pos, wire)
            if content_start is None or content_end is None:
                continue
            children = parse_block_range(chunk, content_start, content_end)
        except (IndexError, ValueError):
            continue

        child_keys = [(row["field_id"], row["wire_base"]) for row in children]
        recognized = [key for key in child_keys if key in spec["known_fields"]]
        target_offsets = [
            row["offset"]
            for row in children
            if row["field_id"] == spec["target_field_id"]
            and row["wire_base"] == (target_header[1] & 0xF8)
        ]

        candidates.append(
            {
                "style": style_name,
                "owner_class": spec["owner_class"],
                "container_offset": pos,
                "container_end": block_end,
                "container_len": block_end - pos,
                "container_field_id": field_id,
                "container_wire_base": wire,
                "parse_complete": True,
                "child_count": len(children),
                "recognized_field_count": len(recognized),
                "recognized_fields": [
                    f"0x{field_id:03X}/0x{wire:02X}"
                    for field_id, wire in recognized
                ],
                "target_name": spec["target_name"],
                "target_field_id": spec["target_field_id"],
                "target_header": target_header.hex().upper(),
                "target_present": bool(target_offsets),
                "target_offsets": target_offsets,
            }
        )

    return candidates


def scan_role(role_data: dict) -> list[dict]:
    out = []
    for seq, ref in sorted(role_data["references"].items()):
        chunk = ref["chunk"]
        chunk_sha = hashlib.sha256(chunk).hexdigest()
        for style_name in STYLE_SPECS:
            for candidate in find_style_candidates(chunk, style_name):
                out.append(
                    {
                        "role": role_data["role"],
                        "seq_num": seq,
                        "raw_types": ref["raw_types"],
                        "chunk_sha256": chunk_sha,
                        **candidate,
                    }
                )
    return out


def unique_candidate_for_seq(
    rows: list[dict],
    role: str,
    style: str,
    seq_num: int,
) -> dict | None:
    matches = [
        row
        for row in rows
        if row["role"] == role
        and row["style"] == style
        and row["seq_num"] == seq_num
        and row["recognized_field_count"] >= 2
    ]
    return matches[0] if len(matches) == 1 else None


def scored_candidates(
    rows: list[dict],
    role: str,
    style: str,
) -> list[dict]:
    return [
        row
        for row in rows
        if row["role"] == role
        and row["style"] == style
        and row["recognized_field_count"] >= 2
    ]


def build_identity_bridges(rows: list[dict]) -> list[dict]:
    bridges = []
    for style_name, spec in STYLE_SPECS.items():
        per_role = {
            role: scored_candidates(rows, role, style_name)
            for role in ROLE_ORDER
        }
        counts = {
            role: len(per_role[role])
            for role in ROLE_ORDER
        }
        document_unique = all(counts[role] == 1 for role in ROLE_ORDER)

        role_rows = {
            role: per_role[role][0] if counts[role] == 1 else None
            for role in ROLE_ORDER
        }

        stable_seq = False
        stable_raw_types = False
        source_absent = control_absent = fill_present = line_present = None

        if document_unique:
            seqs = {role_rows[role]["seq_num"] for role in ROLE_ORDER}
            stable_seq = len(seqs) == 1

            raw_type_sets = {
                tuple(sorted(set(role_rows[role]["raw_types"])))
                for role in ROLE_ORDER
            }
            stable_raw_types = len(raw_type_sets) == 1

            source_absent = not role_rows["source"]["target_present"]
            control_absent = not role_rows["control"]["target_present"]
            fill_present = role_rows["fill"]["target_present"]
            line_present = role_rows["line"]["target_present"]

        expected_pattern = False
        if document_unique and stable_seq and stable_raw_types:
            if style_name == "fill":
                expected_pattern = (
                    source_absent
                    and control_absent
                    and fill_present
                    and not line_present
                )
            else:
                expected_pattern = (
                    source_absent
                    and control_absent
                    and not fill_present
                    and line_present
                )

        seq_num = (
            role_rows["control"]["seq_num"]
            if document_unique and stable_seq
            else None
        )

        bridges.append(
            {
                "style": style_name,
                "owner_class": spec["owner_class"],
                "target_name": spec["target_name"],
                "target_field_id": spec["target_field_id"],
                "candidate_counts": counts,
                "document_unique_candidates": document_unique,
                "stable_seq_num": stable_seq,
                "seq_num": seq_num,
                "stable_raw_types": stable_raw_types,
                "source_target_absent": source_absent,
                "control_target_absent": control_absent,
                "fill_target_present": fill_present,
                "line_target_present": line_present,
                "strict_document_unique_pattern": expected_pattern,
                "interpretation": (
                    "strong_candidate_identity_not_semantic_proof"
                    if expected_pattern
                    else "identity_or_materialization_gate_not_closed"
                ),
            }
        )
    return bridges


def build_matrices(rows: list[dict]) -> list[dict]:
    matrices = []
    for style_name, spec in STYLE_SPECS.items():
        seqs = sorted(
            {
                row["seq_num"]
                for row in rows
                if row["style"] == style_name
                and row["recognized_field_count"] >= 2
            }
        )
        for seq in seqs:
            role_rows = {
                role: unique_candidate_for_seq(rows, role, style_name, seq)
                for role in ROLE_ORDER
            }
            mutation_roles_complete = all(
                role_rows[role] is not None
                for role in ("control", "fill", "line")
            )
            expected_pattern = False
            if mutation_roles_complete:
                control_present = role_rows["control"]["target_present"]
                fill_present = role_rows["fill"]["target_present"]
                line_present = role_rows["line"]["target_present"]
                if style_name == "fill":
                    expected_pattern = (
                        not control_present
                        and fill_present
                        and not line_present
                    )
                else:
                    expected_pattern = (
                        not control_present
                        and not fill_present
                        and line_present
                    )

            source_absent = (
                None
                if role_rows["source"] is None
                else not role_rows["source"]["target_present"]
            )
            matrices.append(
                {
                    "style": style_name,
                    "owner_class": spec["owner_class"],
                    "target_name": spec["target_name"],
                    "target_field_id": spec["target_field_id"],
                    "seq_num": seq,
                    "source_candidate": role_rows["source"] is not None,
                    "control_candidate": role_rows["control"] is not None,
                    "fill_candidate": role_rows["fill"] is not None,
                    "line_candidate": role_rows["line"] is not None,
                    "source_target_absent": source_absent,
                    "control_target_present": (
                        None
                        if role_rows["control"] is None
                        else role_rows["control"]["target_present"]
                    ),
                    "fill_target_present": (
                        None
                        if role_rows["fill"] is None
                        else role_rows["fill"]["target_present"]
                    ),
                    "line_target_present": (
                        None
                        if role_rows["line"] is None
                        else role_rows["line"]["target_present"]
                    ),
                    "bounded_candidate_pattern": expected_pattern,
                    "interpretation": (
                        "candidate_only_not_semantic_proof"
                        if expected_pattern
                        else "no_expected_materialization_pattern"
                    ),
                }
            )
    return matrices


def summarize(
    role_data: dict[str, dict],
    rows: list[dict],
    matrices: list[dict],
    identity_bridges: list[dict],
) -> dict:
    return {
        "roles": {
            role: {
                "path": data["path"],
                "family": data["family"],
                "serialization_revision": data["serialization_revision"],
                "chunk_count": len(data["references"]),
                "duplicate_seq": data["duplicate_seq"],
            }
            for role, data in role_data.items()
        },
        "candidate_rows": len(rows),
        "bounded_candidate_patterns": sum(
            bool(row["bounded_candidate_pattern"]) for row in matrices
        ),
        "fill_patterns": sum(
            row["style"] == "fill" and row["bounded_candidate_pattern"]
            for row in matrices
        ),
        "line_patterns": sum(
            row["style"] == "line" and row["bounded_candidate_pattern"]
            for row in matrices
        ),
        "strict_document_unique_patterns": sum(
            bool(row["strict_document_unique_pattern"])
            for row in identity_bridges
        ),
        "strict_fill_pattern": any(
            row["style"] == "fill"
            and row["strict_document_unique_pattern"]
            for row in identity_bridges
        ),
        "strict_line_pattern": any(
            row["style"] == "line"
            and row["strict_document_unique_pattern"]
            for row in identity_bridges
        ),
        "guardrails": [
            "A bounded candidate pattern is not COM-to-wire semantic proof.",
            "Strict identity requires exactly one scored style candidate document-wide in every role, not merely one candidate at a selected seq_num.",
            "Strict identity also requires one stable Contents seq_num and one stable raw-type set across source/control/fill/line.",
            "recognized_field_count>=2 is a structural sanity check, not owner identity proof.",
            "Source/control/mutations must come from one controlled FALSE-OMISSION-01 fixture lineage.",
            "Current Publisher evidence must not be relabeled as Publisher 11 writer behavior.",
        ],
    }


def write_candidate_tsv(path: Path, rows: list[dict]) -> None:
    cols = [
        "role",
        "seq_num",
        "style",
        "owner_class",
        "container_offset",
        "container_end",
        "container_len",
        "recognized_field_count",
        "recognized_fields",
        "target_name",
        "target_field_id",
        "target_header",
        "target_present",
        "target_offsets",
        "raw_types",
        "chunk_sha256",
    ]
    with path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=cols, delimiter="\t")
        writer.writeheader()
        for row in rows:
            out = {key: row.get(key) for key in cols}
            out["recognized_fields"] = ",".join(row["recognized_fields"])
            out["target_offsets"] = ",".join(str(x) for x in row["target_offsets"])
            out["raw_types"] = ",".join(f"0x{x:02X}" for x in row["raw_types"])
            writer.writerow(out)


def fixed_block(field_id: int, meta: int, payload: bytes = b"") -> bytes:
    header = packed_header(field_id, meta)
    wire = header[1] & 0xF8
    expected_len = {
        0x08: 2,
        0x10: 4,
        0x18: 4,
        0x20: 6,
        0x28: 10,
        0x38: 18,
        0x68: 6,
        0x70: 6,
        0x78: 2,
    }[wire]
    if len(payload) > expected_len - 2:
        raise ValueError("synthetic payload too large")
    return header + payload.ljust(expected_len - 2, b"\x00")


def variable_block(field_id: int, meta: int, content: bytes) -> bytes:
    header = packed_header(field_id, meta)
    declared = 4 + len(content)
    return header + declared.to_bytes(4, "little") + content


def synthetic_chunk(fill_on: bool, line_on: bool) -> bytes:
    fill_children = (
        fixed_block(0x01, 0x04)
        + fixed_block(0x02, 0x04)
        + (fixed_block(0x3C, 0x00) if fill_on else b"")
    )
    line_children = (
        fixed_block(0x01, 0x04)
        + fixed_block(0x0C, 0x04)
        + (fixed_block(0x3D, 0x00) if line_on else b"")
    )
    body = (
        variable_block(0x02, 0x13, fill_children)
        + variable_block(0x03, 0x13, line_children)
    )
    chunk = (4 + len(body)).to_bytes(4, "little") + body
    return chunk


def synthetic_role(role: str, fill_on: bool, line_on: bool) -> dict:
    chunk = synthetic_chunk(fill_on, line_on)
    return {
        "role": role,
        "path": f"<synthetic:{role}>",
        "family": "Family0x2c",
        "serialization_revision": 0x1A,
        "duplicate_seq": [],
        "references": {
            7: {
                "seq_num": 7,
                "chunk": chunk,
                "raw_types": [0x20],
            }
        },
    }


def self_test() -> None:
    roles = {
        "source": synthetic_role("source", False, False),
        "control": synthetic_role("control", False, False),
        "fill": synthetic_role("fill", True, False),
        "line": synthetic_role("line", False, True),
    }
    rows = []
    for role in ROLE_ORDER:
        rows.extend(scan_role(roles[role]))
    matrices = build_matrices(rows)
    bridges = build_identity_bridges(rows)
    fill_bridge = next(row for row in bridges if row["style"] == "fill")
    line_bridge = next(row for row in bridges if row["style"] == "line")
    assert fill_bridge["strict_document_unique_pattern"]
    assert line_bridge["strict_document_unique_pattern"]
    fill = [
        row for row in matrices
        if row["style"] == "fill"
        and row["seq_num"] == 7
    ]
    line = [
        row for row in matrices
        if row["style"] == "line"
        and row["seq_num"] == 7
    ]
    assert len(fill) == 1 and fill[0]["bounded_candidate_pattern"]
    assert len(line) == 1 and line[0]["bounded_candidate_pattern"]
    assert fill[0]["source_target_absent"] is True
    assert line[0]["source_target_absent"] is True

    # Negative control: no mutation materialization.
    roles["fill"] = synthetic_role("fill", False, False)
    rows = []
    for role in ROLE_ORDER:
        rows.extend(scan_role(roles[role]))
    matrices = build_matrices(rows)
    bridges = build_identity_bridges(rows)
    assert not any(
        row["style"] == "fill" and row["bounded_candidate_pattern"]
        for row in matrices
    )
    assert not next(
        row for row in bridges if row["style"] == "fill"
    )["strict_document_unique_pattern"]

    # Negative control: an additional structurally valid candidate anywhere in
    # the document must invalidate the strict identity bridge even when one seq
    # still shows the expected materialization pattern.
    roles = {
        "source": synthetic_role("source", False, False),
        "control": synthetic_role("control", False, False),
        "fill": synthetic_role("fill", True, False),
        "line": synthetic_role("line", False, True),
    }
    for role in ROLE_ORDER:
        roles[role]["references"][8] = {
            "seq_num": 8,
            "chunk": synthetic_chunk(False, False),
            "raw_types": [0x20],
        }
    rows = []
    for role in ROLE_ORDER:
        rows.extend(scan_role(roles[role]))
    bridges = build_identity_bridges(rows)
    assert not any(
        row["strict_document_unique_pattern"]
        for row in bridges
    )
    assert all(
        row["candidate_counts"][role] == 2
        for row in bridges
        for role in ROLE_ORDER
    )

    print("FALSE_OMISSION_RAW_AUDIT_SELF_TEST=OK")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path)
    parser.add_argument("--control", type=Path)
    parser.add_argument("--fill", type=Path)
    parser.add_argument("--line", type=Path)
    parser.add_argument("--out-json", type=Path)
    parser.add_argument("--out-tsv", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    required = {
        "source": args.source,
        "control": args.control,
        "fill": args.fill,
        "line": args.line,
        "out_json": args.out_json,
    }
    missing = [name for name, value in required.items() if value is None]
    if missing:
        parser.error("missing required arguments: " + ", ".join(missing))

    roles = {
        "source": load_contents(args.source, "source"),
        "control": load_contents(args.control, "control"),
        "fill": load_contents(args.fill, "fill"),
        "line": load_contents(args.line, "line"),
    }

    rows = []
    for role in ROLE_ORDER:
        rows.extend(scan_role(roles[role]))
    matrices = build_matrices(rows)
    identity_bridges = build_identity_bridges(rows)
    report = {
        "schema": "pub-false-omission-01/raw-audit/v2",
        "summary": summarize(roles, rows, matrices, identity_bridges),
        "identity_bridges": identity_bridges,
        "matrices": matrices,
        "candidates": rows,
    }
    args.out_json.write_text(
        json.dumps(report, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    if args.out_tsv:
        write_candidate_tsv(args.out_tsv, rows)

    print(json.dumps(report["summary"], indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
