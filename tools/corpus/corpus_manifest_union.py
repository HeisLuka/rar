#!/usr/bin/env python3
"""Compute cumulative exact-byte Publisher corpus knowledge across harvest manifests.

The tool deliberately separates two different questions:

1. What exact Publisher payload SHA-256 values were acquired in each snapshot?
2. What exact Publisher payload SHA-256 values have ever been byte-confirmed across
   the supplied snapshots, even if a later public fetch is transiently unavailable?

Input specs may repeat a label (for example four shard manifests from one run):

    run188=shard0/manifest.csv run188=shard1/manifest.csv ...

Repeated labels are merged into one snapshot before per-snapshot counts are made.
"""
from __future__ import annotations

import argparse
import csv
import json
from collections import defaultdict
from pathlib import Path
from typing import Iterable

PUBLISHER_CLASSIFICATION = "cfb_publisher_hint"
TRUNCATED_PUBLISHER_CLASSIFICATION = "cfb_publisher_truncated"

def intrinsic_completeness(row: dict[str, object]) -> str:
    classification = str(row.get("classification") or "").strip()
    if classification == TRUNCATED_PUBLISHER_CLASSIFICATION:
        return "partial"
    if str(row.get("cc_warc_truncated") or "").strip():
        return "partial"

    filename = str(row.get("cc_warc_filename") or "").strip().casefold()
    if filename.endswith(".arc.gz"):
        try:
            size = int(str(row.get("size_bytes") or "0").strip())
            content_length = int(str(row.get("content_length_header") or "0").strip())
        except ValueError:
            return "unknown"
        transfer = str(row.get("cc_http_transfer_encoding") or "").strip().casefold()
        encoding = str(row.get("cc_http_content_encoding") or "").strip().casefold()
        if content_length > 0 and not transfer and not encoding:
            if size < content_length:
                return "partial"
            if size == content_length:
                return "complete"
        return "unknown"

    return "complete"

QUARANTINE_BUCKET = "quarantine_active_content"


def load_rows(path: Path) -> list[dict[str, object]]:
    suffix = path.suffix.casefold()
    if suffix == ".json":
        value = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(value, list):
            raise ValueError(f"{path}: expected top-level JSON array")
        return [dict(row) for row in value if isinstance(row, dict)]
    if suffix == ".csv":
        with path.open("r", encoding="utf-8-sig", newline="") as handle:
            return [dict(row) for row in csv.DictReader(handle)]
    raise ValueError(f"{path}: expected .csv or .json manifest")


def load_completeness_statuses(path: Path) -> dict[str, set[str]]:
    rows = load_rows(path)
    out: dict[str, set[str]] = defaultdict(set)
    for row in rows:
        sha = str(row.get("sha256") or row.get("actual_sha256") or "").strip().casefold()
        if not sha:
            continue
        status = str(row.get("status") or row.get("completeness") or "").strip().casefold()
        truncated = str(row.get("warc_truncated") or row.get("cc_warc_truncated") or "").strip()
        if status == "complete":
            out[sha].add("complete")
        elif status in {"partial", "partial_with_audit_failures"} or truncated:
            out[sha].add("partial")
    return out


def parse_spec(spec: str) -> tuple[str, Path]:
    if "=" in spec:
        label, raw_path = spec.split("=", 1)
        label = label.strip()
        if not label:
            raise ValueError(f"invalid empty label in {spec!r}")
    else:
        raw_path = spec
        label = Path(raw_path).stem
    path = Path(raw_path)
    if not path.exists():
        raise FileNotFoundError(path)
    return label, path


def truthy(value: object) -> bool:
    return str(value or "").strip().casefold() in {"1", "true", "yes"}


def publisher_row(row: dict[str, object]) -> bool:
    classification = str(row.get("classification") or "")
    return classification in {
        PUBLISHER_CLASSIFICATION,
        TRUNCATED_PUBLISHER_CLASSIFICATION,
    } and bool(str(row.get("sha256") or "").strip())


def row_stratum(row: dict[str, object]) -> str:
    bucket = str(row.get("bucket") or "")
    if bucket == QUARANTINE_BUCKET or truthy(row.get("quarantine")):
        return "quarantine"
    return "natural"


def row_observation(
    label: str, source_path: Path, row: dict[str, object]
) -> dict[str, object]:
    return {
        "snapshot": label,
        "manifest": str(source_path),
        "candidate_filename": str(row.get("candidate_filename") or ""),
        "source_page": str(row.get("source_page") or ""),
        "sha256": str(row.get("sha256") or ""),
        "size_bytes": str(row.get("size_bytes") or ""),
        "bucket": str(row.get("bucket") or ""),
        "stratum": row_stratum(row),
        "fetch_status": str(row.get("fetch_status") or ""),
        "harvested_at_utc": str(row.get("harvested_at_utc") or ""),
        "completeness": intrinsic_completeness(row),
    }


def summarize_sha_set(
    shas: Iterable[str],
    observations: dict[str, list[dict[str, object]]],
    snapshot: str | None = None,
) -> dict[str, int]:
    sha_set = set(shas)
    natural = 0
    quarantine = 0
    mixed = 0
    for sha in sha_set:
        sha_observations = observations.get(sha, [])
        if snapshot is not None:
            sha_observations = [
                obs for obs in sha_observations if obs.get("snapshot") == snapshot
            ]
        strata = {str(obs["stratum"]) for obs in sha_observations}
        if strata == {"natural"}:
            natural += 1
        elif strata == {"quarantine"}:
            quarantine += 1
        else:
            mixed += 1
    return {
        "publisher_sha256": len(sha_set),
        "natural_sha256": natural,
        "quarantine_sha256": quarantine,
        "mixed_stratum_sha256": mixed,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "manifests",
        nargs="+",
        help="manifest path or LABEL=manifest path; repeat LABEL for shards",
    )
    ap.add_argument(
        "--latest-label",
        help="snapshot label to compare against cumulative byte-confirmed union",
    )
    ap.add_argument(
        "--completeness-audit",
        action="append",
        default=[],
        type=Path,
        help=(
            "CSV/JSON audit rows. SHA with status=partial or "
            "partial_with_audit_failures are excluded from complete-file counts."
        ),
    )
    ap.add_argument("--pretty", action="store_true")
    args = ap.parse_args()

    audit_statuses: dict[str, set[str]] = defaultdict(set)
    for audit_path in args.completeness_audit:
        if not audit_path.exists():
            raise FileNotFoundError(audit_path)
        for sha, statuses in load_completeness_statuses(audit_path).items():
            audit_statuses[sha].update(statuses)

    snapshot_shas: dict[str, set[str]] = defaultdict(set)
    snapshot_paths: dict[str, list[str]] = defaultdict(list)
    observations: dict[str, list[dict[str, object]]] = defaultdict(list)
    seen_observation_keys: set[tuple[str, str, str, str, str, str]] = set()

    label_order: list[str] = []
    seen_labels: set[str] = set()

    for spec in args.manifests:
        label, path = parse_spec(spec)
        if label not in seen_labels:
            seen_labels.add(label)
            label_order.append(label)
        snapshot_paths[label].append(str(path))
        snapshot_shas[label]  # retain snapshots even when every Publisher observation is partial

        for row in load_rows(path):
            if not publisher_row(row):
                continue
            sha = str(row.get("sha256") or "").strip().casefold()
            obs = row_observation(label, path, row)
            statuses = audit_statuses.get(sha, set())
            if "complete" in statuses:
                obs["completeness"] = "complete"
            elif "partial" in statuses:
                obs["completeness"] = "partial"
            if obs["completeness"] == "complete":
                snapshot_shas[label].add(sha)
            key = (
                label,
                sha,
                str(obs["candidate_filename"]),
                str(obs["source_page"]),
                str(obs["stratum"]),
                str(obs["size_bytes"]),
            )
            if key not in seen_observation_keys:
                seen_observation_keys.add(key)
                observations[sha].append(obs)

    if not label_order:
        raise SystemExit("no manifests")

    latest_label = args.latest_label or label_order[-1]
    if latest_label not in snapshot_shas:
        raise SystemExit(f"unknown --latest-label {latest_label!r}")

    cumulative = set().union(*(snapshot_shas[label] for label in label_order))
    latest = snapshot_shas[latest_label]
    prior_union = (
        set().union(
            *(snapshot_shas[label] for label in label_order if label != latest_label)
        )
        if len(label_order) > 1
        else set()
    )
    missing_latest = sorted(cumulative - latest)
    new_latest = sorted(latest - prior_union)

    snapshots = []
    for label in label_order:
        shas = snapshot_shas[label]
        snapshots.append(
            {
                "label": label,
                "manifest_paths": snapshot_paths[label],
                **summarize_sha_set(shas, observations, label),
                "missing_vs_cumulative": len(cumulative - shas),
            }
        )

    stratum_conflicts = []
    size_conflicts = []
    for sha, obs in sorted(observations.items()):
        strata = sorted({str(x["stratum"]) for x in obs})
        if len(strata) > 1:
            stratum_conflicts.append({"sha256": sha, "strata": strata})
        sizes = sorted(
            {
                str(x["size_bytes"])
                for x in obs
                if x.get("size_bytes") not in (None, "")
            }
        )
        if len(sizes) > 1:
            size_conflicts.append({"sha256": sha, "sizes": sizes})

    def describe(sha: str) -> dict[str, object]:
        obs = observations.get(sha, [])
        names = sorted(
            {str(x["candidate_filename"]) for x in obs if x["candidate_filename"]}
        )
        pages = sorted({str(x["source_page"]) for x in obs if x["source_page"]})
        strata = sorted({str(x["stratum"]) for x in obs})
        appearances = sorted({str(x["snapshot"]) for x in obs})
        return {
            "sha256": sha,
            "candidate_filenames": names,
            "source_pages": pages,
            "strata": strata,
            "observed_in_snapshots": appearances,
        }

    output = {
        "schema": "pub-corpus-manifest-union-v2",
        "publisher_classifications": [
            PUBLISHER_CLASSIFICATION,
            TRUNCATED_PUBLISHER_CLASSIFICATION,
        ],
        "latest_label": latest_label,
        "completeness_audit_paths": [
            str(path) for path in args.completeness_audit
        ],
        "partial_sha256": sorted(
            sha
            for sha, obs in observations.items()
            if obs and all(str(x.get("completeness")) != "complete" for x in obs)
        ),
        "partial_sha256_count": sum(
            1
            for obs in observations.values()
            if obs and all(str(x.get("completeness")) != "complete" for x in obs)
        ),
        "cumulative": summarize_sha_set(cumulative, observations),
        "latest": summarize_sha_set(latest, observations, latest_label),
        "snapshots": snapshots,
        "known_but_absent_latest_count": len(missing_latest),
        "known_but_absent_latest": [describe(sha) for sha in missing_latest],
        "new_in_latest_count": len(new_latest),
        "new_in_latest": [describe(sha) for sha in new_latest],
        "stratum_conflicts": stratum_conflicts,
        "size_conflicts": size_conflicts,
    }
    print(
        json.dumps(
            output,
            indent=2 if args.pretty else None,
            ensure_ascii=False,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
