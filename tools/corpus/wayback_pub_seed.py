#!/usr/bin/env python3
"""Discover/recover Microsoft Publisher locators through Wayback CDX.

Input seed rows provide exact public URLs and known-positive domains. The output
is compatible with harvest_pub.py and uses Wayback replay URLs as inert byte
sources. Source/archive provenance is retained separately from payload identity.
"""
from __future__ import annotations

import argparse
import csv
import json
import re
import time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode, urlparse
from urllib.request import Request, urlopen

CDX = "https://web.archive.org/cdx/search/cdx"
UA = "rar-pub-wayback/1.0 (public format research)"
PUB_RE = re.compile(r"[.]pub(?:[?#].*)?$", re.I)
GENERIC = {
    "archive.org", "github.com", "githubusercontent.com", "google.com",
    "amazonaws.com", "cloudfront.net", "dropbox.com", "microsoft.com",
}


def host(url: str) -> str:
    h = (urlparse(url).hostname or "").casefold().strip(".")
    return h[4:] if h.startswith("www.") else h


def generic_domain(h: str) -> bool:
    return any(h == x or h.endswith("." + x) for x in GENERIC)


def request_json(params: list[tuple[str, str]], timeout: float, retries: int) -> list[dict[str, str]]:
    url = CDX + "?" + urlencode(params)
    last = None
    for attempt in range(retries + 1):
        try:
            req = Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
            with urlopen(req, timeout=timeout) as resp:
                raw = resp.read(32 * 1024 * 1024 + 1)
            if len(raw) > 32 * 1024 * 1024:
                raise ValueError("CDX response exceeded 32 MiB")
            data = json.loads(raw.decode("utf-8"))
            if not data:
                return []
            header = data[0]
            if not isinstance(header, list):
                raise ValueError("invalid CDX header")
            out = []
            for row in data[1:]:
                if isinstance(row, list):
                    out.append({str(k): str(v) for k, v in zip(header, row)})
            return out
        except HTTPError as exc:
            last = exc
            if attempt < retries and exc.code in {429, 500, 502, 503, 504}:
                time.sleep(2.0 * (attempt + 1))
                continue
            raise
        except (URLError, TimeoutError, OSError, ValueError, json.JSONDecodeError) as exc:
            last = exc
            if attempt < retries:
                time.sleep(2.0 * (attempt + 1))
                continue
            raise
    raise last  # type: ignore[misc]


def load_seed(paths: list[Path]) -> tuple[list[str], list[str]]:
    exact: set[str] = set()
    domains: set[str] = set()
    for path in paths:
        with path.open("r", encoding="utf-8-sig", newline="") as fh:
            for row in csv.DictReader(fh):
                for key in ("direct_url", "source_page"):
                    value = (row.get(key) or "").strip()
                    if not value:
                        continue
                    h = host(value)
                    if h and not generic_domain(h):
                        domains.add(h)
                    if key == "direct_url" and PUB_RE.search(value):
                        exact.add(value)
    return sorted(exact), sorted(domains)


def exact_query(url: str, limit: int, timeout: float, retries: int) -> list[dict[str, str]]:
    return request_json([
        ("url", url),
        ("matchType", "exact"),
        ("output", "json"),
        ("fl", "timestamp,original,mimetype,statuscode,digest,length"),
        ("filter", "statuscode:200"),
        ("collapse", "digest"),
        ("limit", str(limit)),
        ("gzip", "false"),
    ], timeout, retries)


def domain_query(domain: str, limit: int, timeout: float, retries: int) -> list[dict[str, str]]:
    return request_json([
        ("url", domain),
        ("matchType", "domain"),
        ("output", "json"),
        ("fl", "timestamp,original,mimetype,statuscode,digest,length"),
        ("filter", "statuscode:200"),
        ("filter", r"original:.*[.]pub(?:[?#].*)?$"),
        ("collapse", "urlkey"),
        ("limit", str(limit)),
        ("gzip", "false"),
    ], timeout, retries)


def replay_url(timestamp: str, original: str) -> str:
    encoded = quote(original, safe=":/?&=%+,$;~*'()!")
    return f"https://web.archive.org/web/{timestamp}id_/{encoded}"


def filename(original: str, digest: str) -> str:
    name = Path(urlparse(original).path).name
    if name.casefold().endswith(".pub"):
        return name[:180]
    token = re.sub(r"[^A-Za-z0-9_-]+", "", digest)[:32]
    return f"wayback-{token or 'candidate'}.pub"


def convert(row: dict[str, str], query_kind: str, query_value: str) -> dict[str, str] | None:
    ts = row.get("timestamp", "").strip()
    original = row.get("original", "").strip()
    if not ts or not original:
        return None
    digest = row.get("digest", "").strip()
    return {
        "source_page": f"https://web.archive.org/web/{ts}/{quote(original, safe=':/?&=%')}",
        "direct_url": replay_url(ts, original),
        "candidate_filename": filename(original, digest),
        "quarantine": "",
        "source_class": "wayback_cdx",
        "notes": "Wayback CDX public capture; archive timestamp is provenance, not Publisher writer-version proof",
        "wayback_timestamp": ts,
        "wayback_original_url": original,
        "wayback_digest": digest,
        "wayback_mimetype": row.get("mimetype", ""),
        "wayback_length": row.get("length", ""),
        "wayback_query_kind": query_kind,
        "wayback_query_value": query_value,
    }


def write_csv(rows: list[dict[str, str]], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    keys: list[str] = []
    seen = set()
    for row in rows:
        for key in row:
            if key not in seen:
                seen.add(key)
                keys.append(key)
    with path.open("w", encoding="utf-8", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=keys)
        w.writeheader()
        w.writerows(rows)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=Path, action="append", default=[])
    ap.add_argument("--url", action="append", default=[])
    ap.add_argument("--domain", action="append", default=[])
    ap.add_argument("--max-domains", type=int, default=100)
    ap.add_argument("--exact-limit", type=int, default=20)
    ap.add_argument("--domain-limit", type=int, default=500)
    ap.add_argument("--delay", type=float, default=1.0)
    ap.add_argument("--timeout", type=float, default=45)
    ap.add_argument("--retries", type=int, default=1)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--summary", type=Path)
    args = ap.parse_args()

    exact, domains = load_seed(args.seed)
    exact = sorted(set(exact) | {x.strip() for x in args.url if x.strip()})
    domains = sorted(set(domains) | {x.strip().casefold() for x in args.domain if x.strip()})
    domains = [x for x in domains if x and not generic_domain(x)][: args.max_domains]

    rows: list[dict[str, str]] = []
    errors: list[dict[str, str]] = []
    for kind, values in (("exact", exact), ("domain", domains)):
        for value in values:
            try:
                found = (
                    exact_query(value, args.exact_limit, args.timeout, args.retries)
                    if kind == "exact"
                    else domain_query(value, args.domain_limit, args.timeout, args.retries)
                )
                rows.extend(
                    x for x in (convert(r, kind, value) for r in found) if x is not None
                )
            except Exception as exc:
                errors.append({"kind": kind, "value": value, "error": f"{type(exc).__name__}: {exc}"})
            time.sleep(args.delay)

    # Preserve one earliest archived observation per original URL + digest.
    dedup: dict[tuple[str, str], dict[str, str]] = {}
    for row in sorted(rows, key=lambda r: r["wayback_timestamp"]):
        key = (row["wayback_original_url"].casefold(), row["wayback_digest"])
        dedup.setdefault(key, row)
    final = sorted(dedup.values(), key=lambda r: (r["wayback_timestamp"], r["wayback_original_url"]))
    write_csv(final, args.out)

    summary = {
        "schema": "rar-wayback-cdx-v1",
        "exact_queries": len(exact),
        "domain_queries": len(domains),
        "raw_capture_rows": len(rows),
        "deduplicated_locator_rows": len(final),
        "unique_original_urls": len({r["wayback_original_url"] for r in final}),
        "unique_domains": len({host(r["wayback_original_url"]) for r in final}),
        "errors": errors,
    }
    p = args.summary or args.out.with_suffix(".summary.json")
    p.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
