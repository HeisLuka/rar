#!/usr/bin/env python3
"""Global Common Crawl discovery for Microsoft Publisher candidates.

Scans the public Common Crawl columnar URL index without seed-domain or country
filters. Emits provenance-rich rows compatible with harvest_pub.py.

Discovery is intentionally high-recall: URL paths ending in .pub and Publisher
MIME signals are candidates. Exact bytes are still recovered, classified,
deduplicated and completeness-gated by the shared Rar acquisition substrate.
"""
from __future__ import annotations

import argparse
import csv
import gzip
import json
import re
import sys
import time
from pathlib import Path
from typing import Any
from urllib.parse import unquote, urlparse
from urllib.request import Request, urlopen

COLLINFO = "https://index.commoncrawl.org/collinfo.json"
DATA = "https://data.commoncrawl.org/"
UA = "rar-pub-global-columnar/1.0 (public format research)"
YEAR_RE = re.compile(r"CC-MAIN-(\d{4})-")


def request_bytes(url: str, timeout: float, limit: int = 64 * 1024 * 1024) -> bytes:
    req = Request(url, headers={"User-Agent": UA, "Accept": "*/*"})
    with urlopen(req, timeout=timeout) as resp:
        data = resp.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"response exceeds {limit} bytes")
    return data


def crawl_year(crawl: str) -> int:
    m = YEAR_RE.search(crawl)
    return int(m.group(1)) if m else 0


def load_crawls(timeout: float, min_year: int) -> list[str]:
    rows = json.loads(request_bytes(COLLINFO, timeout).decode("utf-8"))
    ids = [str(r.get("id") or "") for r in rows if isinstance(r, dict)]
    return [x for x in ids if x and crawl_year(x) >= min_year]


def spread_select(values: list[str], count: int) -> list[str]:
    if count <= 0 or count >= len(values):
        return list(values)
    if count == 1:
        return values[:1]
    last = len(values) - 1
    indexes = sorted({round(i * last / (count - 1)) for i in range(count)})
    return [values[i] for i in indexes]


def parquet_urls(crawl: str, timeout: float, max_files: int) -> list[str]:
    url = f"{DATA}crawl-data/{crawl}/cc-index-table.paths.gz"
    lines = gzip.decompress(request_bytes(url, timeout)).decode("utf-8").splitlines()
    paths = [
        line.strip()
        for line in lines
        if line.strip()
        and f"crawl={crawl}/subset=warc/" in line
        and line.strip().endswith(".parquet")
    ]
    return [DATA + p for p in spread_select(paths, max_files)]


def build_sql(limit: int) -> str:
    return f"""
SELECT
  url,
  url_host_name,
  fetch_time,
  fetch_status,
  content_digest,
  content_mime_type,
  content_mime_detected,
  warc_filename,
  warc_record_offset,
  warc_record_length
FROM ccindex
WHERE fetch_status = 200
  AND (
    regexp_matches(lower(url_path), '[.]pub$')
    OR lower(coalesce(content_mime_type, '')) LIKE '%publisher%'
    OR lower(coalesce(content_mime_detected, '')) LIKE '%publisher%'
  )
LIMIT {int(limit)}
""".strip()


def resilient_query(
    urls: list[str],
    sql: str,
    max_candidates: int,
    batch_size: int,
    retries: int,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    import duckdb  # type: ignore

    con = duckdb.connect()
    con.execute("INSTALL httpfs")
    con.execute("LOAD httpfs")
    con.execute("SET enable_http_metadata_cache=true")
    con.execute("SET http_retries=6")
    con.execute("SET threads=4")

    rows: list[dict[str, Any]] = []
    stats: dict[str, Any] = {
        "successful_parquet_files": 0,
        "failed_parquet_files": 0,
        "failed_files": [],
        "batch_failures": 0,
    }

    def scan(batch: list[str]) -> None:
        if not batch or len(rows) >= max_candidates:
            return
        error = ""
        for attempt in range(retries + 1):
            try:
                con.execute("DROP VIEW IF EXISTS ccindex")
                con.read_parquet(
                    batch, hive_partitioning=True, union_by_name=True
                ).create_view("ccindex")
                cur = con.execute(sql)
                names = [d[0] for d in cur.description]
                rows.extend(dict(zip(names, item)) for item in cur.fetchall())
                stats["successful_parquet_files"] += len(batch)
                return
            except Exception as exc:
                error = f"{type(exc).__name__}: {exc}"
                if attempt < retries:
                    time.sleep(1.0 * (attempt + 1))
        stats["batch_failures"] += 1
        if len(batch) == 1:
            stats["failed_parquet_files"] += 1
            stats["failed_files"].append({"url": batch[0], "error": error})
            return
        mid = len(batch) // 2
        scan(batch[:mid])
        scan(batch[mid:])

    try:
        for start in range(0, len(urls), batch_size):
            if len(rows) >= max_candidates:
                break
            scan(urls[start : start + batch_size])
    finally:
        con.close()
    return rows[:max_candidates], stats


def normalized_time(value: Any) -> str:
    return re.sub(r"\D", "", str(value or ""))[:14]


def candidate_name(url: str, digest: str) -> str:
    name = unquote(Path(urlparse(url).path).name).strip()
    if name.casefold().endswith(".pub"):
        return name[:180]
    token = re.sub(r"[^A-Za-z0-9_-]+", "", digest or "")[:32]
    return f"commoncrawl-{token or 'publisher-mime'}.pub"


def to_seed(record: dict[str, Any], crawl: str) -> dict[str, str] | None:
    url = str(record.get("url") or "").strip()
    filename = str(record.get("warc_filename") or "").strip()
    offset = str(record.get("warc_record_offset") or "").strip()
    length = str(record.get("warc_record_length") or "").strip()
    if not url or not filename or not offset or not length:
        return None
    digest = str(record.get("content_digest") or "").strip()
    host = str(record.get("url_host_name") or urlparse(url).hostname or "").strip()
    return {
        "source_page": "",
        "direct_url": url,
        "candidate_filename": candidate_name(url, digest),
        "quarantine": "",
        "source_class": "common_crawl_global",
        "notes": "global Common Crawl columnar discovery; identity requires exact-byte validation",
        "cc_crawl": crawl,
        "cc_timestamp": normalized_time(record.get("fetch_time")),
        "cc_digest": digest,
        "cc_warc_filename": filename,
        "cc_warc_offset": offset,
        "cc_warc_length": length,
        "cc_mime": str(record.get("content_mime_type") or ""),
        "cc_mime_detected": str(record.get("content_mime_detected") or ""),
        "cc_index_endpoint": "commoncrawl-columnar-index",
        "cc_source_host": host,
    }


def identity(row: dict[str, str]) -> str:
    if row.get("cc_digest"):
        return "digest:" + row["cc_digest"]
    return "capture:" + "|".join(
        [
            row.get("direct_url", "").casefold(),
            row.get("cc_timestamp", ""),
            row.get("cc_warc_filename", ""),
            row.get("cc_warc_offset", ""),
        ]
    )


def write_csv(rows: list[dict[str, str]], path: Path) -> None:
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
        writer.writerows(rows)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--crawl", action="append", default=[])
    ap.add_argument("--crawl-count", type=int, default=6)
    ap.add_argument("--min-year", type=int, default=2015)
    ap.add_argument("--max-parquet-files", type=int, default=60)
    ap.add_argument("--max-candidates-per-crawl", type=int, default=500)
    ap.add_argument("--batch-size", type=int, default=12)
    ap.add_argument("--retries", type=int, default=1)
    ap.add_argument("--timeout", type=float, default=45.0)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--summary", type=Path)
    args = ap.parse_args()

    if args.crawl:
        crawls = args.crawl
    else:
        crawls = spread_select(
            load_crawls(args.timeout, args.min_year), args.crawl_count
        )
    if not crawls:
        raise SystemExit("no Common Crawl collections selected")

    all_rows: list[dict[str, str]] = []
    crawl_stats: list[dict[str, Any]] = []
    for crawl in crawls:
        urls = parquet_urls(crawl, args.timeout, args.max_parquet_files)
        sql = build_sql(args.max_candidates_per_crawl)
        records, stats = resilient_query(
            urls,
            sql,
            args.max_candidates_per_crawl,
            args.batch_size,
            args.retries,
        )
        converted = [x for x in (to_seed(r, crawl) for r in records) if x]
        all_rows.extend(converted)
        crawl_stats.append(
            {
                "crawl": crawl,
                "parquet_files_selected": len(urls),
                "candidate_records": len(converted),
                **stats,
            }
        )
        print(f"{crawl}: candidates={len(converted)} parquets={len(urls)}", file=sys.stderr)

    dedup: dict[str, dict[str, str]] = {}
    for row in all_rows:
        dedup.setdefault(identity(row), row)
    rows = sorted(
        dedup.values(),
        key=lambda r: (
            r.get("cc_crawl", ""),
            r.get("cc_timestamp", ""),
            r.get("direct_url", ""),
        ),
    )
    write_csv(rows, args.out)
    summary = {
        "schema": "rar-commoncrawl-global-v1",
        "crawls": crawls,
        "raw_candidates": len(all_rows),
        "deduplicated_candidates": len(rows),
        "unique_source_hosts": len({r.get("cc_source_host", "") for r in rows if r.get("cc_source_host")}),
        "crawl_stats": crawl_stats,
    }
    out = args.summary or args.out.with_suffix(".summary.json")
    out.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
