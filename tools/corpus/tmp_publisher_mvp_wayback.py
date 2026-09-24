#!/usr/bin/env python3
"""Targeted Wayback sweep for historical Microsoft Publisher MVP/template sites.

This is intentionally narrower than the generic Wayback vacuum: it searches
specific historical path prefixes that were independently evidenced as hosting
Publisher templates/sample files.
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import wayback_pub_seed as wb

SCOPES = [
    "users.bigpond.com/auslandline/publisher/",
    "www.users.bigpond.com/auslandline/publisher/",
    "users.bigpond.com/ausirion/",
    "www.users.bigpond.com/ausirion/",
    "mvps.org/publisher/",
    "www.mvps.org/publisher/",
    "kvalheim.org/",
    "www.kvalheim.org/",
    "publishermvps.com/",
    "www.publishermvps.com/",
    "ed.mvps.org/Publisher/",
]

def prefix_query(prefix: str, limit: int, timeout: float, retries: int):
    return wb.request_json([
        ("url", prefix),
        ("matchType", "prefix"),
        ("output", "json"),
        ("fl", "timestamp,original,mimetype,statuscode,digest,length"),
        ("filter", "statuscode:200"),
        ("filter", r"original:.*[.]pub(?:[?#].*)?$"),
        ("collapse", "digest"),
        ("limit", str(limit)),
        ("gzip", "false"),
    ], timeout, retries)

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--summary", type=Path)
    ap.add_argument("--limit", type=int, default=5000)
    ap.add_argument("--timeout", type=float, default=60)
    ap.add_argument("--retries", type=int, default=2)
    ap.add_argument("--delay", type=float, default=0.5)
    args = ap.parse_args()

    rows = []
    errors = []
    per_scope = {}
    for scope in SCOPES:
        try:
            found = prefix_query(scope, args.limit, args.timeout, args.retries)
            converted = [
                x for x in (wb.convert(r, "prefix", scope) for r in found)
                if x is not None
            ]
            rows.extend(converted)
            per_scope[scope] = len(converted)
            print(scope, len(converted))
        except Exception as exc:
            errors.append({
                "scope": scope,
                "error": f"{type(exc).__name__}: {exc}",
            })
            per_scope[scope] = 0
        time.sleep(args.delay)

    # Preserve earliest capture per original URL + archive digest.
    dedup = {}
    for row in sorted(rows, key=lambda r: r["wayback_timestamp"]):
        key = (row["wayback_original_url"].casefold(), row["wayback_digest"])
        dedup.setdefault(key, row)
    final = sorted(
        dedup.values(),
        key=lambda r: (r["wayback_timestamp"], r["wayback_original_url"]),
    )
    wb.write_csv(final, args.out)

    summary = {
        "schema": "rar-publisher-mvp-wayback-v1",
        "scopes": SCOPES,
        "per_scope_rows": per_scope,
        "raw_capture_rows": len(rows),
        "deduplicated_locator_rows": len(final),
        "unique_original_urls": len({r["wayback_original_url"] for r in final}),
        "errors": errors,
    }
    p = args.summary or args.out.with_suffix(".summary.json")
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
