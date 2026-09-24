#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import hashlib
import json
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

import harvest_pub


def load_manifest(path: Path) -> list[dict[str, str]]:
    with path.open("r", encoding="utf-8-sig", newline="") as fh:
        return list(csv.DictReader(fh))


def has_warc_coordinates(row: dict[str, str]) -> bool:
    return all((row.get(k) or "").strip() for k in (
        "cc_warc_filename", "cc_warc_offset", "cc_warc_length"
    ))


def is_publisher_capture(row: dict[str, str]) -> bool:
    classification = (row.get("classification") or "").strip()
    return classification.startswith("cfb_publisher")


def audit_row(
    row: dict[str, str],
    timeout: float,
    max_bytes: int,
) -> dict[str, Any]:
    out: dict[str, Any] = {
        "sha256": (row.get("sha256") or "").strip(),
        "candidate_filename": (row.get("candidate_filename") or "").strip(),
        "direct_url": (row.get("direct_url") or "").strip(),
        "cc_crawl": (row.get("cc_crawl") or "").strip(),
        "cc_warc_filename": (row.get("cc_warc_filename") or "").strip(),
        "cc_warc_offset": (row.get("cc_warc_offset") or "").strip(),
        "cc_warc_length": (row.get("cc_warc_length") or "").strip(),
        "prior_classification": (row.get("classification") or "").strip(),
        "stratum": (row.get("stratum") or "").strip(),
        "source_run": (row.get("source_run") or "").strip(),
        "country_code": (row.get("country_code") or "").strip(),
        "status": "",
        "warc_truncated": "",
        "observed_sha256": "",
        "error": "",
    }
    try:
        data, meta = harvest_pub.common_crawl_fetch(
            row,
            timeout=timeout,
            max_bytes=max_bytes,
            retries=2,
        )
    except Exception as exc:
        out["status"] = "audit_failed"
        out["error"] = f"{type(exc).__name__}: {exc}"
        return out

    observed = hashlib.sha256(data).hexdigest()
    out["observed_sha256"] = observed
    expected = out["sha256"]
    if expected and observed != expected:
        out["status"] = "sha_mismatch"
        return out

    truncated = str(meta.get("cc_warc_truncated") or "").strip()
    out["warc_truncated"] = truncated
    archive_format = str(meta.get("cc_archive_format") or "").casefold()
    if not archive_format:
        filename = str(out.get("cc_warc_filename") or "").casefold()
        archive_format = "arc" if filename.endswith(".arc.gz") else "warc"
    out["archive_format"] = archive_format
    out["content_length_header"] = str(meta.get("content_length_header") or "")

    if archive_format == "warc":
        out["status"] = "partial" if truncated else "complete"
        return out

    if archive_format == "arc":
        raw_length = str(meta.get("content_length_header") or "").strip()
        try:
            declared = int(raw_length) if raw_length else None
        except ValueError:
            declared = None
        if declared is None:
            out["status"] = "legacy_arc_no_http_length"
        elif declared == len(data):
            out["status"] = "complete"
        else:
            out["status"] = "partial"
            out["error"] = (
                f"ARC HTTP Content-Length mismatch: declared={declared} "
                f"payload={len(data)}"
            )
        return out

    out["status"] = "audit_failed"
    out["error"] = f"unsupported archive format: {archive_format!r}"
    return out


def aggregate_by_sha(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        key = row.get("sha256") or (
            f"capture:{row.get('cc_warc_filename')}:{row.get('cc_warc_offset')}"
        )
        grouped[str(key)].append(row)

    out: list[dict[str, Any]] = []
    for sha, captures in grouped.items():
        statuses = {str(row.get("status") or "") for row in captures}
        if "complete" in statuses:
            status = "complete"
        elif "sha_mismatch" in statuses:
            status = "sha_mismatch"
        elif statuses == {"partial"}:
            status = "partial"
        elif statuses == {"legacy_arc_no_http_length"}:
            status = "legacy_arc_no_http_length"
        elif "partial" in statuses and statuses <= {"partial", "audit_failed"}:
            status = "partial_with_audit_failures"
        else:
            status = "audit_failed"
        out.append({
            "sha256": sha,
            "status": status,
            "capture_count": len(captures),
            "complete_capture_count": sum(r["status"] == "complete" for r in captures),
            "partial_capture_count": sum(r["status"] == "partial" for r in captures),
            "failed_capture_count": sum(r["status"] in {"audit_failed", "sha_mismatch"} for r in captures),
            "unknown_capture_count": sum(r["status"] == "legacy_arc_no_http_length" for r in captures),
            "candidate_filenames": sorted({str(r.get("candidate_filename") or "") for r in captures if r.get("candidate_filename")}),
        })
    return sorted(out, key=lambda row: (row["status"], row["sha256"]))


def write_csv(rows: list[dict[str, Any]], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    keys: list[str] = []
    seen: set[str] = set()
    for row in rows:
        for key in row:
            if key not in seen:
                seen.add(key)
                keys.append(key)
    with path.open("w", encoding="utf-8", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=keys)
        writer.writeheader()
        for row in rows:
            normalized = dict(row)
            if isinstance(normalized.get("candidate_filenames"), list):
                normalized["candidate_filenames"] = ";".join(normalized["candidate_filenames"])
            writer.writerow(normalized)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", type=Path, action="append", required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--timeout", type=float, default=25.0)
    ap.add_argument("--max-bytes", type=int, default=100 * 1024 * 1024)
    ap.add_argument("--max-rows", type=int, default=-1)
    ap.add_argument("--workers", type=int, default=1)
    args = ap.parse_args()
    if args.workers < 1:
        raise SystemExit("--workers must be >= 1")

    selected: list[dict[str, str]] = []
    skipped_non_publisher = 0
    skipped_no_coordinates = 0
    for path in args.manifest:
        for row in load_manifest(path):
            if not is_publisher_capture(row):
                skipped_non_publisher += 1
                continue
            if not has_warc_coordinates(row):
                skipped_no_coordinates += 1
                continue
            row = dict(row)
            row["_source_manifest"] = str(path)
            selected.append(row)

    if args.max_rows >= 0:
        selected = selected[:args.max_rows]

    if args.workers == 1:
        audited = [
            audit_row(row, timeout=args.timeout, max_bytes=args.max_bytes)
            for row in selected
        ]
    else:
        audited = []
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            futures = {
                pool.submit(
                    audit_row,
                    row,
                    timeout=args.timeout,
                    max_bytes=args.max_bytes,
                ): row
                for row in selected
            }
            for future in as_completed(futures):
                audited.append(future.result())
    by_sha = aggregate_by_sha(audited)

    args.out.mkdir(parents=True, exist_ok=True)
    write_csv(audited, args.out / "warc-captures.csv")
    write_csv(by_sha, args.out / "warc-sha.csv")
    (args.out / "warc-captures.json").write_text(
        json.dumps(audited, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    (args.out / "warc-sha.json").write_text(
        json.dumps(by_sha, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    summary = {
        "schema": "pub-warc-completeness-audit-v1",
        "manifests": [str(path) for path in args.manifest],
        "capture_rows_audited": len(audited),
        "unique_sha_audited": len(by_sha),
        "capture_statuses": dict(Counter(row["status"] for row in audited)),
        "sha_statuses": dict(Counter(row["status"] for row in by_sha)),
        "skipped_non_publisher_rows": skipped_non_publisher,
        "skipped_rows_without_warc_coordinates": skipped_no_coordinates,
    }
    (args.out / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0 if not summary["sha_statuses"].get("sha_mismatch", 0) else 2


if __name__ == "__main__":
    raise SystemExit(main())
