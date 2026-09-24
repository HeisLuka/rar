#!/usr/bin/env python3
"""Discover .pub-labelled GovDocs1 objects from the public DigitalCorpora S3 bucket.

GovDocs1 is exposed through the AWS Open Data bucket without credentials. This
tool enumerates the numbered GovDocs1 directories through S3 ListObjectsV2 and
emits seed rows compatible with harvest_pub.py. Extension is treated only as a
candidate signal; the existing harvester still performs exact-byte CFB/PUB
classification and SHA-256 deduplication.

Stdlib-only.
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
import time
import xml.etree.ElementTree as ET
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen

BUCKET_HOST = "digitalcorpora.s3.us-west-2.amazonaws.com"
BUCKET_BASE = f"https://{BUCKET_HOST}"
PREFIX_ROOT = "corpora/files/govdocs1"
UA = "rar-pub-govdocs1/1.0 (public-format research; https://github.com/HeisLuka/rar)"
S3_NS = {"s3": "http://s3.amazonaws.com/doc/2006-03-01/"}


def request_bytes(url: str, timeout: float, retries: int = 3) -> bytes:
    last: Exception | None = None
    for attempt in range(retries + 1):
        try:
            req = Request(url, headers={"User-Agent": UA, "Accept": "application/xml,*/*"})
            with urlopen(req, timeout=timeout) as resp:
                data = resp.read(16 * 1024 * 1024 + 1)
                if len(data) > 16 * 1024 * 1024:
                    raise ValueError("S3 listing exceeded 16 MiB")
                return data
        except (HTTPError, URLError, TimeoutError, OSError, ValueError) as exc:
            last = exc
            if attempt < retries:
                time.sleep(min(10.0, 1.5 * (attempt + 1)))
    assert last is not None
    raise last


def parse_list_response(data: bytes) -> tuple[list[dict[str, str]], str]:
    root = ET.fromstring(data)
    rows: list[dict[str, str]] = []
    for item in root.findall("s3:Contents", S3_NS):
        key = (item.findtext("s3:Key", default="", namespaces=S3_NS) or "").strip()
        if not key:
            continue
        rows.append(
            {
                "key": key,
                "size": (item.findtext("s3:Size", default="", namespaces=S3_NS) or "").strip(),
                "etag": (item.findtext("s3:ETag", default="", namespaces=S3_NS) or "").strip().strip('"'),
                "last_modified": (
                    item.findtext("s3:LastModified", default="", namespaces=S3_NS) or ""
                ).strip(),
            }
        )
    next_token = (
        root.findtext("s3:NextContinuationToken", default="", namespaces=S3_NS) or ""
    ).strip()
    return rows, next_token


def list_page(prefix: str, timeout: float, continuation: str = "") -> tuple[list[dict[str, str]], str]:
    params = {
        "list-type": "2",
        "prefix": prefix,
        "max-keys": "1000",
    }
    if continuation:
        params["continuation-token"] = continuation
    url = BUCKET_BASE + "/?" + urlencode(params)
    return parse_list_response(request_bytes(url, timeout))


def list_directory(index: int, timeout: float) -> tuple[int, list[dict[str, str]]]:
    prefix = f"{PREFIX_ROOT}/{index:03d}/"
    all_rows: list[dict[str, str]] = []
    token = ""
    while True:
        rows, token = list_page(prefix, timeout, token)
        all_rows.extend(rows)
        if not token:
            break
    return index, all_rows


def is_pub_key(key: str) -> bool:
    return key.casefold().endswith(".pub")


def object_url(key: str) -> str:
    return BUCKET_BASE + "/" + quote(key, safe="/")


def candidate_filename(key: str) -> str:
    return Path(key).name


def to_seed(row: dict[str, str]) -> dict[str, str]:
    key = row["key"]
    directory = key.split("/")[-2] if "/" in key else ""
    return {
        "source_page": "https://digitalcorpora.org/corpora/file-corpora/files/",
        "direct_url": object_url(key),
        "candidate_filename": candidate_filename(key),
        "quarantine": "yes",
        "source_class": "govdocs1",
        "notes": (
            "public GovDocs1 candidate from DigitalCorpora AWS Open Data; "
            "extension is not accepted as Publisher identity until byte validation; "
            "source corpus is quarantined because GovDocs1 may contain active/malicious content"
        ),
        "govdocs1_key": key,
        "govdocs1_directory": directory,
        "govdocs1_size": row.get("size", ""),
        "govdocs1_etag": row.get("etag", ""),
        "govdocs1_last_modified": row.get("last_modified", ""),
    }


def write_csv(rows: list[dict[str, str]], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fields: list[str] = []
    seen: set[str] = set()
    for row in rows:
        for key in row:
            if key not in seen:
                seen.add(key)
                fields.append(key)
    with path.open("w", encoding="utf-8", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--summary", type=Path)
    ap.add_argument("--start-dir", type=int, default=0)
    ap.add_argument("--end-dir", type=int, default=999)
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument("--max-candidates", type=int, default=1000)
    args = ap.parse_args()

    if not (0 <= args.start_dir <= args.end_dir <= 999):
        raise SystemExit("directory range must satisfy 0 <= start <= end <= 999")
    if args.workers < 1 or args.workers > 32:
        raise SystemExit("workers must be between 1 and 32")
    if args.max_candidates < 1:
        raise SystemExit("max-candidates must be positive")

    candidates: list[dict[str, str]] = []
    errors: list[dict[str, str]] = []
    directories_ok = 0
    objects_seen = 0

    indexes = list(range(args.start_dir, args.end_dir + 1))
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {
            pool.submit(list_directory, index, args.timeout): index for index in indexes
        }
        for future in as_completed(futures):
            index = futures[future]
            try:
                _, rows = future.result()
            except Exception as exc:
                errors.append(
                    {"directory": f"{index:03d}", "error": f"{type(exc).__name__}: {exc}"}
                )
                print(f"WARN {index:03d}: {exc}", file=sys.stderr)
                continue
            directories_ok += 1
            objects_seen += len(rows)
            for row in rows:
                if is_pub_key(row["key"]):
                    candidates.append(to_seed(row))

    candidates.sort(key=lambda row: row["govdocs1_key"])
    truncated = len(candidates) > args.max_candidates
    candidates = candidates[: args.max_candidates]
    write_csv(candidates, args.out)

    summary = {
        "source": "GovDocs1 / DigitalCorpora AWS Open Data",
        "directory_range": [args.start_dir, args.end_dir],
        "directories_requested": len(indexes),
        "directories_ok": directories_ok,
        "objects_seen": objects_seen,
        "pub_extension_candidates": len(candidates),
        "truncated": truncated,
        "query_errors": errors,
    }
    summary_path = args.summary or args.out.with_suffix(".summary.json")
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
