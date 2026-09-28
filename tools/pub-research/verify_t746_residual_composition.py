#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
from collections import Counter
from pathlib import Path

SCHEMA = "chaptera.t746-residual-composition-receipt.v1"
TARGET_TOTAL = 176
TARGET_RAW29_LEAF = 171
TARGET_RAW1E = 5

MARKER_CANDIDATES = (
    "raw_type", "raw_marker", "marker", "chunk_type", "type", "rawType", "rawMarker"
)
SHA_CANDIDATES = (
    "source_sha256", "sha256", "file_sha256", "source_hash", "file_hash"
)
IDENTITY_CANDIDATES = (
    "object_id", "object_identity", "identity", "chunk_id", "id", "seq_num", "seqNum"
)
OFFSET_CANDIDATES = (
    "offset", "chunk_offset", "stream_offset", "absolute_offset", "raw_offset"
)
NESTED_CANDIDATES = (
    "has_direct_0x36", "nested_0x36", "has_0x36_child", "direct_0x36_child",
    "is_nested", "nested"
)

def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def load_rows(path: Path) -> tuple[list[dict[str, str]], list[str]]:
    with path.open("r", encoding="utf-8-sig", newline="") as fh:
        reader = csv.DictReader(fh)
        if not reader.fieldnames:
            raise ValueError(f"{path.name}: missing CSV header")
        rows = [{k: (v or "").strip() for k, v in row.items()} for row in reader]
        return rows, list(reader.fieldnames)

def find_column(fieldnames: list[str], candidates: tuple[str, ...], required: bool = False) -> str | None:
    exact = {name.lower(): name for name in fieldnames}
    for candidate in candidates:
        if candidate.lower() in exact:
            return exact[candidate.lower()]
    normalized = {re.sub(r"[^a-z0-9]", "", name.lower()): name for name in fieldnames}
    for candidate in candidates:
        key = re.sub(r"[^a-z0-9]", "", candidate.lower())
        if key in normalized:
            return normalized[key]
    if required:
        raise ValueError(f"missing required column; expected one of {candidates}")
    return None

def parse_marker(value: str) -> str:
    s = value.strip().lower()
    if not s:
        return "unknown"
    if s.startswith("raw"):
        s = s[3:]
    if s.startswith("0x"):
        n = int(s, 16)
    elif re.fullmatch(r"[0-9a-f]+h", s):
        n = int(s[:-1], 16)
    elif re.fullmatch(r"[0-9]+", s):
        n = int(s, 10)
    elif re.fullmatch(r"[0-9a-f]+", s):
        n = int(s, 16)
    else:
        m = re.search(r"0x([0-9a-f]+)", s)
        if not m:
            return "unknown"
        n = int(m.group(1), 16)
    return f"0x{n:02x}"

def parse_bool(value: str) -> bool | None:
    s = value.strip().lower()
    if s in {"1","true","yes","y","nested","present"}:
        return True
    if s in {"0","false","no","n","leaf","absent",""}:
        return False
    return None

def key_for(row: dict[str, str], sha_col: str | None, id_col: str | None, off_col: str | None) -> tuple[str, ...] | None:
    parts = []
    for col in (sha_col, id_col, off_col):
        if col:
            val = row.get(col, "").strip()
            if val:
                parts.append(val.lower())
    return tuple(parts) if parts else None

def analyse(residual_csv: Path, raw29_csv: Path | None) -> dict:
    residual_rows, residual_fields = load_rows(residual_csv)
    marker_col = find_column(residual_fields, MARKER_CANDIDATES, required=True)
    res_sha = find_column(residual_fields, SHA_CANDIDATES)
    res_id = find_column(residual_fields, IDENTITY_CANDIDATES)
    res_off = find_column(residual_fields, OFFSET_CANDIDATES)

    marker_counts = Counter(parse_marker(row[marker_col]) for row in residual_rows)
    residual_keys = [key_for(r, res_sha, res_id, res_off) for r in residual_rows]
    residual_keys_nonnull = [k for k in residual_keys if k is not None]
    duplicate_residual_keys = len(residual_keys_nonnull) - len(set(residual_keys_nonnull))

    raw29_leaf = None
    raw29_nested = None
    raw29_join_missing = None
    raw29_join_extra = None
    nesting_detected = False
    raw29_sha = None

    if raw29_csv is not None:
        raw29_rows, raw29_fields = load_rows(raw29_csv)
        raw29_sha = sha256(raw29_csv)
        raw29_marker_col = find_column(raw29_fields, MARKER_CANDIDATES)
        raw29_sha_col = find_column(raw29_fields, SHA_CANDIDATES)
        raw29_id_col = find_column(raw29_fields, IDENTITY_CANDIDATES)
        raw29_off_col = find_column(raw29_fields, OFFSET_CANDIDATES)
        nested_col = find_column(raw29_fields, NESTED_CANDIDATES)

        filtered = raw29_rows
        if raw29_marker_col:
            filtered = [r for r in raw29_rows if parse_marker(r[raw29_marker_col]) == "0x29"]

        if nested_col:
            nesting_detected = True
            nested_values = [parse_bool(r[nested_col]) for r in filtered]
            if any(v is None for v in nested_values):
                raise ValueError(f"{raw29_csv.name}: unrecognized nesting value in {nested_col}")
            raw29_nested = sum(1 for v in nested_values if v)
            raw29_leaf = sum(1 for v in nested_values if not v)

        raw29_keys = {
            key_for(r, raw29_sha_col, raw29_id_col, raw29_off_col)
            for r in filtered
        }
        raw29_keys.discard(None)
        residual_raw29_keys = {
            key_for(r, res_sha, res_id, res_off)
            for r in residual_rows if parse_marker(r[marker_col]) == "0x29"
        }
        residual_raw29_keys.discard(None)

        if raw29_keys and residual_raw29_keys:
            raw29_join_missing = len(residual_raw29_keys - raw29_keys)
            raw29_join_extra = len(raw29_keys - residual_raw29_keys)

    other_markers = sum(
        count for marker, count in marker_counts.items()
        if marker not in {"0x29", "0x1e"}
    )

    checks = {
        "total_is_176": len(residual_rows) == TARGET_TOTAL,
        "raw0x29_count_is_171": marker_counts.get("0x29", 0) == TARGET_RAW29_LEAF,
        "raw0x1e_count_is_5": marker_counts.get("0x1e", 0) == TARGET_RAW1E,
        "other_marker_count_is_0": other_markers == 0,
        "residual_duplicate_key_count_is_0": duplicate_residual_keys == 0,
    }

    if raw29_csv is not None and nesting_detected:
        checks["raw29_reference_leaf_count_is_171"] = raw29_leaf == TARGET_RAW29_LEAF
        checks["raw29_reference_nested_count_is_6"] = raw29_nested == 6
    if raw29_join_missing is not None:
        checks["residual_raw29_missing_from_reference_is_0"] = raw29_join_missing == 0

    if all(checks.values()):
        verdict = "confirmed"
    elif all(
        checks[k]
        for k in ("total_is_176","raw0x29_count_is_171","raw0x1e_count_is_5","other_marker_count_is_0")
    ) and any(v is False for k, v in checks.items() if k not in {
        "total_is_176","raw0x29_count_is_171","raw0x1e_count_is_5","other_marker_count_is_0"
    }):
        verdict = "indeterminate"
    else:
        verdict = "refuted"

    return {
        "schema": SCHEMA,
        "hypothesis": "176 residuals = 171 leaf raw0x29 + 5 raw0x1e",
        "verdict": verdict,
        "inputs": {
            "residual_csv_sha256": sha256(residual_csv),
            "raw29_reference_csv_sha256": raw29_sha,
        },
        "counts": {
            "residual_rows": len(residual_rows),
            "marker_counts": dict(sorted(marker_counts.items())),
            "other_marker_rows": other_markers,
            "duplicate_residual_keys": duplicate_residual_keys,
            "raw29_reference_leaf": raw29_leaf,
            "raw29_reference_nested": raw29_nested,
            "raw29_residual_missing_from_reference": raw29_join_missing,
            "raw29_reference_extra_vs_residual": raw29_join_extra,
        },
        "detected_columns": {
            "residual_marker": marker_col,
            "residual_sha": res_sha,
            "residual_identity": res_id,
            "residual_offset": res_off,
            "raw29_nesting_detected": nesting_detected,
        },
        "checks": checks,
        "privacy": {
            "row_identities_emitted": False,
            "local_paths_emitted": False,
            "pub_bytes_emitted": False,
            "document_text_emitted": False,
        },
    }

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--residual-csv", required=True, type=Path)
    p.add_argument("--raw29-csv", type=Path)
    p.add_argument("--output", required=True, type=Path)
    args = p.parse_args()
    receipt = analyse(args.residual_csv, args.raw29_csv)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"verdict": receipt["verdict"], "counts": receipt["counts"]}, sort_keys=True))
    return 0 if receipt["verdict"] == "confirmed" else 2 if receipt["verdict"] == "refuted" else 3

if __name__ == "__main__":
    raise SystemExit(main())
