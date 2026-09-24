#!/usr/bin/env python3
"""Logical-identity-weighted version/provenance coverage for the Rar PUB corpus.

Physical SHA identity remains a separate provenance metric. Research evidence is
weighted by exact logical stream-set identity (content_topology_fingerprint_sha256)
so timestamp-only CFB copies cannot vote more than once.
"""
from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path

import structural_novelty as novelty

SCHEMA = "chaptera.corpus-version-identity-matrix.v1"


def load_rows(root: Path) -> list[dict]:
    rows: dict[str, dict] = {}
    for path in sorted(root.rglob("fingerprints.json")):
        payload = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(payload, list):
            continue
        for row in payload:
            if not isinstance(row, dict):
                continue
            sha = str(row.get("sha256") or row.get("source_sha256") or "").lower()
            if not sha:
                continue
            current = rows.get(sha)
            if current is not None and current != row:
                raise ValueError(f"conflicting fingerprint rows for physical SHA {sha}")
            rows[sha] = row
    return [rows[k] for k in sorted(rows)]


def load_provenance(path: Path | None) -> dict[str, dict]:
    if path is None:
        return {}
    payload = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(payload, dict) and isinstance(payload.get("rows"), list):
        payload = payload["rows"]
    if not isinstance(payload, list):
        raise ValueError("provenance input must be a JSON list or {rows:[...]}")
    out: dict[str, dict] = {}
    for row in payload:
        if not isinstance(row, dict):
            continue
        sha = str(row.get("sha256") or "").lower()
        if not sha:
            continue
        out[sha] = {
            "version_label": str(row.get("version_label") or "").strip(),
            "provenance_class": str(row.get("provenance_class") or "").strip(),
            "provenance_note": str(row.get("provenance_note") or "").strip(),
        }
    return out


def one_value(values: set, *, field: str, logical_identity: str):
    clean = {v for v in values if v not in {"", None}}
    if len(clean) > 1:
        raise ValueError(
            f"logical identity {logical_identity} disagrees on {field}: {sorted(clean, key=str)}"
        )
    return next(iter(clean)) if clean else None


def build(rows: list[dict], provenance: dict[str, dict] | None = None) -> dict:
    provenance = provenance or {}
    status = Counter(str(row.get("status") or "") for row in rows)
    ok = [row for row in rows if row.get("status") == "ok"]

    groups: dict[str, list[dict]] = defaultdict(list)
    for row in ok:
        logical = str(row.get("content_topology_fingerprint_sha256") or "")
        if not logical:
            raise ValueError(f"successful row lacks logical stream identity: {row.get('sha256')}")
        groups[logical].append(row)

    logical_rows = []
    cells: dict[tuple[str, str, str], dict] = {}
    source_logical_counts = Counter()
    version_label_counts = Counter()
    family_counts = Counter()
    revision_counts = Counter()

    for logical, members in sorted(groups.items()):
        families = {row.get("contents_family") for row in members}
        revisions = {row.get("contents_serialization_revision") for row in members}
        family = one_value(families, field="contents_family", logical_identity=logical) or "unknown"
        revision_value = one_value(
            revisions, field="contents_serialization_revision", logical_identity=logical
        )
        revision = "unknown" if revision_value is None else str(revision_value)

        shas = sorted(str(row.get("sha256") or row.get("source_sha256") or "") for row in members)
        sources = sorted({src for row in members for src in (row.get("sources") or [])})
        filenames = sorted({name for row in members for name in (row.get("filenames") or [])})

        labels = sorted(
            {
                provenance.get(sha, {}).get("version_label", "")
                for sha in shas
                if provenance.get(sha, {}).get("version_label", "")
            }
        )
        provenance_classes = sorted(
            {
                provenance.get(sha, {}).get("provenance_class", "")
                for sha in shas
                if provenance.get(sha, {}).get("provenance_class", "")
            }
        )
        version_label = "|".join(labels) if labels else "unlabelled"
        provenance_class = "|".join(provenance_classes) if provenance_classes else "unlabelled"

        logical_row = {
            "logical_identity": logical,
            "physical_sha_count": len(shas),
            "physical_sha256": shas,
            "sources": sources,
            "filenames": filenames,
            "contents_family": family,
            "contents_serialization_revision": revision_value,
            "version_label": version_label,
            "provenance_class": provenance_class,
        }
        logical_rows.append(logical_row)

        family_counts[family] += 1
        revision_counts[f"{family}:{revision}"] += 1
        version_label_counts[version_label] += 1
        for source in sources:
            source_logical_counts[source] += 1

        key = (family, revision, version_label)
        cell = cells.setdefault(
            key,
            {
                "contents_family": family,
                "contents_serialization_revision": revision_value,
                "version_label": version_label,
                "logical_identity_count": 0,
                "physical_sha_count": 0,
                "sources": set(),
                "provenance_classes": set(),
            },
        )
        cell["logical_identity_count"] += 1
        cell["physical_sha_count"] += len(shas)
        cell["sources"].update(sources)
        cell["provenance_classes"].update(provenance_classes)

    matrix = []
    for _, cell in sorted(cells.items()):
        row = dict(cell)
        row["sources"] = sorted(row["sources"])
        row["provenance_classes"] = sorted(row["provenance_classes"])
        matrix.append(row)

    physical_success = len(ok)
    logical_count = len(groups)
    surplus = physical_success - logical_count

    gap_queue = []
    unknown_family = family_counts.get("unknown", 0) + family_counts.get("missing", 0) + family_counts.get("too_short", 0)
    if unknown_family:
        gap_queue.append(
            {
                "priority": 1,
                "gap": "unknown_contents_family",
                "logical_identity_count": unknown_family,
                "next_discriminator": "rehydrate bounded /Contents prefix and classify exact family marker",
            }
        )
    unlabelled = version_label_counts.get("unlabelled", 0)
    if unlabelled:
        gap_queue.append(
            {
                "priority": 2,
                "gap": "missing_exact_version_provenance",
                "logical_identity_count": unlabelled,
                "next_discriminator": "join exact source/media/version provenance; do not infer marketing version from Contents family",
            }
        )
    for family in ("0x22", "0x2c"):
        count = family_counts.get(family, 0)
        if count:
            gap_queue.append(
                {
                    "priority": 3,
                    "gap": f"{family}_family_without_exact_release_breakdown",
                    "logical_identity_count": count,
                    "next_discriminator": "use independent source/version provenance or bounded file-level version fields",
                }
            )
    gap_queue.sort(key=lambda row: (row["priority"], -row["logical_identity_count"], row["gap"]))

    summary = {
        "schema": SCHEMA,
        "physical_sha_count": len(rows),
        "status": dict(status),
        "successful_physical_sha_count": physical_success,
        "logical_identity_count": logical_count,
        "surplus_physical_sha_count": surplus,
        "duplicate_logical_group_count": sum(len(members) > 1 for members in groups.values()),
        "physical_sha_in_duplicate_groups": sum(len(members) for members in groups.values() if len(members) > 1),
        "contents_family_logical_counts": dict(sorted(family_counts.items())),
        "contents_family_revision_logical_counts": dict(sorted(revision_counts.items())),
        "version_label_logical_counts": dict(sorted(version_label_counts.items())),
        "source_logical_membership_counts_nonexclusive": dict(sorted(source_logical_counts.items())),
        "matrix_cell_count": len(matrix),
        "weighting_law": "one evidence vote per exact logical stream-set identity; physical SHA remains provenance authority",
    }
    return {
        "summary": summary,
        "logical_identities": logical_rows,
        "matrix": matrix,
        "gap_queue": gap_queue,
    }


def write_outputs(result: dict, out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    for name, payload in (
        ("summary.json", result["summary"]),
        ("logical_identities.json", result["logical_identities"]),
        ("matrix.json", result["matrix"]),
        ("gap_queue.json", result["gap_queue"]),
    ):
        (out / name).write_text(
            json.dumps(payload, indent=2, ensure_ascii=False, sort_keys=True) + "\n",
            encoding="utf-8",
        )


def self_test() -> int:
    assert novelty.contents_family_projection(bytes([0xE8, 0xAC, 0x22, 0x00]))["contents_family"] == "0x22"
    mature = bytearray(14)
    mature[:4] = bytes([0xE8, 0xAC, 0x2C, 0x00])
    mature[12:14] = (0x1A).to_bytes(2, "little")
    projected = novelty.contents_family_projection(bytes(mature))
    assert projected == {
        "contents_family": "0x2c",
        "contents_serialization_revision": 0x1A,
    }
    assert novelty.contents_family_projection(bytes([0xE8, 0xAC, 0x2C]))["contents_family"] == "too_short"

    rows = [
        {
            "sha256": "1" * 64,
            "status": "ok",
            "content_topology_fingerprint_sha256": "a" * 64,
            "contents_family": "0x2c",
            "contents_serialization_revision": 26,
            "sources": ["media_a"],
            "filenames": ["SAME.PUB"],
        },
        {
            "sha256": "2" * 64,
            "status": "ok",
            "content_topology_fingerprint_sha256": "a" * 64,
            "contents_family": "0x2c",
            "contents_serialization_revision": 26,
            "sources": ["media_b"],
            "filenames": ["SAME.PUB"],
        },
        {
            "sha256": "3" * 64,
            "status": "ok",
            "content_topology_fingerprint_sha256": "b" * 64,
            "contents_family": "0x22",
            "contents_serialization_revision": 717,
            "sources": ["legacy"],
            "filenames": ["OLD.PUB"],
        },
        {"sha256": "4" * 64, "status": "probe_failed"},
    ]
    result = build(rows)
    s = result["summary"]
    assert s["physical_sha_count"] == 4
    assert s["successful_physical_sha_count"] == 3
    assert s["logical_identity_count"] == 2
    assert s["surplus_physical_sha_count"] == 1
    assert s["duplicate_logical_group_count"] == 1
    assert s["contents_family_logical_counts"] == {"0x22": 1, "0x2c": 1}
    assert sum(row["logical_identity_count"] for row in result["matrix"]) == 2

    conflicting = [dict(rows[0]), dict(rows[1])]
    conflicting[1]["contents_family"] = "0x22"
    try:
        build(conflicting)
    except ValueError as exc:
        assert "disagrees on contents_family" in str(exc)
    else:
        raise AssertionError("logical-duplicate family disagreement did not fail closed")

    print("version identity matrix self-test ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    build_cmd = sub.add_parser("build")
    build_cmd.add_argument("--input", type=Path, required=True)
    build_cmd.add_argument("--out", type=Path, required=True)
    build_cmd.add_argument("--provenance", type=Path)

    sub.add_parser("self-test")
    args = ap.parse_args()

    if args.cmd == "self-test":
        return self_test()

    rows = load_rows(args.input)
    result = build(rows, load_provenance(args.provenance))
    write_outputs(result, args.out)
    print(json.dumps(result["summary"], indent=2, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
