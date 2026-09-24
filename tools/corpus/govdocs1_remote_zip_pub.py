#!/usr/bin/env python3
"""Discover and extract GovDocs1 .pub members from the official 000.zip..999.zip bundles.

The current GovDocs1 distribution documents 1000 ZIP files, one per numbered
directory. This tool avoids downloading the multi-hundred-MiB ZIPs wholesale:
it uses HTTP Range to read the EOCD + central directory, selects .pub members,
then fetches only each selected member's local header + compressed data.

No archive member is executed. All recovered bytes remain quarantine evidence.
"""
from __future__ import annotations

import argparse
import binascii
import csv
import hashlib
import json
import struct
import sys
import time
import zlib
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Iterable
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

sys.path.insert(0, str(Path(__file__).resolve().parent))
import harvest_pub

UA = "rar-govdocs1-remote-zip/1.0 (public format research)"
BASE = "https://downloads.digitalcorpora.org/corpora/files/govdocs1/zipfiles"
EOCD_SIG = b"PK\x05\x06"
CD_SIG = b"PK\x01\x02"
LOCAL_SIG = b"PK\x03\x04"
MAX_TAIL = 256 * 1024
MAX_CD = 4 * 1024 * 1024


@dataclass(frozen=True)
class Entry:
    name: str
    flags: int
    method: int
    crc32: int
    compressed_size: int
    uncompressed_size: int
    local_offset: int


def zip_url(index: int) -> str:
    return f"{BASE}/{index:03d}.zip"


def safe_member(name: str) -> bool:
    clean = name.replace("\\", "/")
    p = PurePosixPath(clean)
    return (
        bool(clean)
        and not clean.startswith("/")
        and ".." not in p.parts
        and "\x00" not in clean
    )


def request(
    url: str,
    *,
    method: str = "GET",
    range_header: str = "",
    timeout: float = 30.0,
    max_bytes: int = 8 * 1024 * 1024,
    retries: int = 2,
) -> tuple[bytes, dict[str, str]]:
    last: Exception | None = None
    for attempt in range(retries + 1):
        try:
            headers = {"User-Agent": UA, "Accept": "*/*", "Accept-Encoding": "identity"}
            if range_header:
                headers["Range"] = range_header
            req = Request(url, headers=headers, method=method)
            with urlopen(req, timeout=timeout) as resp:
                meta = {
                    "status": str(getattr(resp, "status", 200)),
                    "content_length": resp.headers.get("Content-Length", ""),
                    "content_range": resp.headers.get("Content-Range", ""),
                    "final_url": resp.geturl(),
                    "etag": resp.headers.get("ETag", ""),
                    "last_modified": resp.headers.get("Last-Modified", ""),
                }
                if method == "HEAD":
                    return b"", meta
                data = resp.read(max_bytes + 1)
            if len(data) > max_bytes:
                raise ValueError(f"response exceeded bounded {max_bytes} bytes")
            return data, meta
        except (HTTPError, URLError, TimeoutError, OSError, ValueError) as exc:
            last = exc
            if attempt < retries:
                time.sleep(1.0 * (attempt + 1))
    assert last is not None
    raise last


def remote_size(url: str, timeout: float) -> tuple[int, dict[str, str]]:
    try:
        _, meta = request(url, method="HEAD", timeout=timeout, max_bytes=0)
        length = int(meta.get("content_length") or 0)
        if length > 0:
            return length, meta
    except Exception:
        pass

    data, meta = request(
        url,
        range_header="bytes=0-0",
        timeout=timeout,
        max_bytes=2,
    )
    cr = meta.get("content_range", "")
    if "/" in cr:
        total = cr.rsplit("/", 1)[1]
        if total.isdigit():
            return int(total), meta
    if meta.get("status") == "200":
        length = int(meta.get("content_length") or len(data))
        if length > 0:
            return length, meta
    raise ValueError("could not determine remote ZIP size")


def range_bytes(url: str, start: int, end: int, timeout: float, cap: int) -> tuple[bytes, dict[str, str]]:
    if start < 0 or end < start:
        raise ValueError("invalid byte range")
    expected = end - start + 1
    if expected > cap:
        raise ValueError(f"range {expected} exceeds cap {cap}")
    data, meta = request(
        url,
        range_header=f"bytes={start}-{end}",
        timeout=timeout,
        max_bytes=expected,
    )
    status = int(meta.get("status") or 0)
    if status != 206:
        # Never accept a multi-hundred-MiB full-body response when a range was requested.
        raise ValueError(f"server ignored Range request: HTTP {status}")
    if len(data) != expected:
        raise ValueError(f"short range: expected {expected}, got {len(data)}")
    return data, meta


def parse_eocd(tail: bytes, tail_start: int) -> tuple[int, int, int]:
    pos = tail.rfind(EOCD_SIG)
    if pos < 0 or len(tail) - pos < 22:
        raise ValueError("EOCD not found")
    (
        sig,
        disk_no,
        cd_disk,
        entries_disk,
        entries_total,
        cd_size,
        cd_offset,
        comment_len,
    ) = struct.unpack_from("<4s4H2LH", tail, pos)
    if sig != EOCD_SIG:
        raise ValueError("bad EOCD signature")
    if disk_no != 0 or cd_disk != 0 or entries_disk != entries_total:
        raise ValueError("multi-disk ZIP unsupported")
    if entries_total == 0xFFFF or cd_size == 0xFFFFFFFF or cd_offset == 0xFFFFFFFF:
        raise ValueError("ZIP64 archive unsupported by bounded GovDocs1 indexer")
    if pos + 22 + comment_len > len(tail):
        raise ValueError("truncated EOCD/comment")
    absolute_eocd = tail_start + pos
    if cd_offset + cd_size > absolute_eocd:
        raise ValueError("central directory overlaps EOCD")
    return entries_total, cd_offset, cd_size


def parse_central_directory(data: bytes, expected_entries: int) -> list[Entry]:
    out: list[Entry] = []
    pos = 0
    while pos < len(data):
        if len(data) - pos < 46 or data[pos : pos + 4] != CD_SIG:
            raise ValueError(f"invalid central-directory entry at {pos}")
        fields = struct.unpack_from("<4s6H3L5H2L", data, pos)
        (
            _sig,
            _made,
            _needed,
            flags,
            method,
            _mtime,
            _mdate,
            crc32,
            comp_size,
            uncomp_size,
            name_len,
            extra_len,
            comment_len,
            disk_start,
            _int_attr,
            _ext_attr,
            local_offset,
        ) = fields
        end = pos + 46 + name_len + extra_len + comment_len
        if end > len(data):
            raise ValueError("truncated central-directory record")
        name_raw = data[pos + 46 : pos + 46 + name_len]
        encoding = "utf-8" if flags & 0x800 else "cp437"
        name = name_raw.decode(encoding, errors="replace")
        if disk_start != 0:
            raise ValueError("multi-disk member unsupported")
        out.append(
            Entry(
                name=name,
                flags=flags,
                method=method,
                crc32=crc32,
                compressed_size=comp_size,
                uncompressed_size=uncomp_size,
                local_offset=local_offset,
            )
        )
        pos = end
    if len(out) != expected_entries:
        raise ValueError(f"central entry count mismatch: {len(out)} != {expected_entries}")
    return out


def list_remote_zip(url: str, timeout: float) -> tuple[list[Entry], dict[str, str]]:
    size, meta = remote_size(url, timeout)
    tail_len = min(size, MAX_TAIL)
    tail_start = size - tail_len
    tail, range_meta = range_bytes(url, tail_start, size - 1, timeout, MAX_TAIL)
    entries, cd_offset, cd_size = parse_eocd(tail, tail_start)
    if cd_size > MAX_CD:
        raise ValueError(f"central directory too large: {cd_size}")
    cd, _ = range_bytes(url, cd_offset, cd_offset + cd_size - 1, timeout, MAX_CD)
    parsed = parse_central_directory(cd, entries)
    meta.update(
        {
            "final_url": range_meta.get("final_url", meta.get("final_url", url)),
            "zip_size": str(size),
            "zip_entry_count": str(entries),
            "zip_cd_offset": str(cd_offset),
            "zip_cd_size": str(cd_size),
        }
    )
    return parsed, meta


def decode_member(entry: Entry, compressed: bytes, max_member_bytes: int) -> bytes:
    if entry.flags & 0x1:
        raise ValueError("encrypted member")
    if entry.uncompressed_size > max_member_bytes:
        raise ValueError("member exceeds uncompressed cap")
    if entry.method == 0:
        data = compressed
    elif entry.method == 8:
        data = zlib.decompress(compressed, -zlib.MAX_WBITS)
    else:
        raise ValueError(f"unsupported ZIP method {entry.method}")
    if len(data) != entry.uncompressed_size:
        raise ValueError(
            f"uncompressed size mismatch: {len(data)} != {entry.uncompressed_size}"
        )
    if len(data) > max_member_bytes:
        raise ValueError("decoded member exceeds cap")
    crc = binascii.crc32(data) & 0xFFFFFFFF
    if crc != entry.crc32:
        raise ValueError(f"CRC mismatch: {crc:08x} != {entry.crc32:08x}")
    return data


def fetch_member(url: str, entry: Entry, timeout: float, max_member_bytes: int) -> bytes:
    header, _ = range_bytes(
        url, entry.local_offset, entry.local_offset + 29, timeout, 30
    )
    if header[:4] != LOCAL_SIG:
        raise ValueError("bad local-header signature")
    (
        _sig,
        _needed,
        flags,
        method,
        _mtime,
        _mdate,
        _crc,
        _comp,
        _uncomp,
        name_len,
        extra_len,
    ) = struct.unpack("<4s5H3L2H", header)
    if flags & 0x1:
        raise ValueError("encrypted local member")
    if method != entry.method:
        raise ValueError("local/central compression method mismatch")
    data_start = entry.local_offset + 30 + name_len + extra_len
    if entry.compressed_size > max_member_bytes + 1024 * 1024:
        raise ValueError("compressed member exceeds bounded cap")
    compressed, _ = range_bytes(
        url,
        data_start,
        data_start + entry.compressed_size - 1,
        timeout,
        max_member_bytes + 1024 * 1024,
    )
    return decode_member(entry, compressed, max_member_bytes)


def scan_one(index: int, timeout: float, max_member_bytes: int) -> tuple[dict, list[dict]]:
    url = zip_url(index)
    entries, meta = list_remote_zip(url, timeout)
    pub_entries = [
        e for e in entries if safe_member(e.name) and e.name.casefold().endswith(".pub")
    ]
    rows: list[dict] = []
    for member_index, entry in enumerate(pub_entries):
        base = {
            "row_kind": "govdocs1_zip_member",
            "quarantine": "yes",
            "source_class": "govdocs1",
            "source_page": "https://digitalcorpora.org/corpora/file-corpora/files/",
            "container_url": url,
            "container_final_url": meta.get("final_url", url),
            "govdocs1_zip_index": f"{index:03d}",
            "archive_member": entry.name,
            "candidate_filename": Path(entry.name).name,
            "archive_member_index": member_index,
            "declared_size_bytes": entry.uncompressed_size,
            "compressed_size_bytes": entry.compressed_size,
            "zip_crc32": f"{entry.crc32:08x}",
            "container_etag": meta.get("etag", ""),
            "container_last_modified": meta.get("last_modified", ""),
            "notes": "GovDocs1 public ZIP member; quarantine/static classification only",
        }
        try:
            data = fetch_member(url, entry, timeout, max_member_bytes)
            sha = hashlib.sha256(data).hexdigest()
            classification, hints = harvest_pub.classify(data)
            base.update(
                {
                    "fetch_status": "ok_archive_member",
                    "size_bytes": len(data),
                    "sha256": sha,
                    "classification": classification,
                    "publisher_hints": ";".join(hints),
                }
            )
        except Exception as exc:
            base.update(
                {
                    "fetch_status": "archive_member_fetch_failed",
                    "error": f"{type(exc).__name__}: {exc}",
                }
            )
        rows.append(base)
    summary = {
        "zip_index": f"{index:03d}",
        "url": url,
        "zip_size": int(meta.get("zip_size") or 0),
        "entry_count": len(entries),
        "pub_members": len(pub_entries),
    }
    return summary, rows


def write_manifest(rows: list[dict], out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf-8")
    fields: list[str] = []
    seen: set[str] = set()
    for row in rows:
        for key in row:
            if key not in seen:
                seen.add(key)
                fields.append(key)
    with out.with_suffix(".csv").open("w", encoding="utf-8", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--start-dir", type=int, default=0)
    ap.add_argument("--end-dir", type=int, default=999)
    ap.add_argument("--workers", type=int, default=12)
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument("--max-member-bytes", type=int, default=100 * 1024 * 1024)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--summary", type=Path)
    args = ap.parse_args()

    if not (0 <= args.start_dir <= args.end_dir <= 999):
        raise SystemExit("range must satisfy 0 <= start <= end <= 999")
    if not (1 <= args.workers <= 32):
        raise SystemExit("workers must be 1..32")

    rows: list[dict] = []
    zip_summaries: list[dict] = []
    errors: list[dict] = []
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {
            pool.submit(scan_one, i, args.timeout, args.max_member_bytes): i
            for i in range(args.start_dir, args.end_dir + 1)
        }
        for future in as_completed(futures):
            i = futures[future]
            try:
                summary, member_rows = future.result()
                zip_summaries.append(summary)
                rows.extend(member_rows)
                print(
                    f"{i:03d}: entries={summary['entry_count']} pub={summary['pub_members']}",
                    file=sys.stderr,
                )
            except Exception as exc:
                errors.append(
                    {"zip_index": f"{i:03d}", "error": f"{type(exc).__name__}: {exc}"}
                )
                print(f"WARN {i:03d}: {exc}", file=sys.stderr)

    # Global exact-byte duplicate annotation.
    first: dict[str, str] = {}
    for row in sorted(rows, key=lambda r: (r.get("govdocs1_zip_index", ""), r.get("archive_member", ""))):
        sha = row.get("sha256")
        if not sha:
            continue
        if sha in first:
            row["duplicate_of_sha256"] = sha
            row["duplicate_of_file"] = first[sha]
        else:
            first[sha] = f"{row.get('govdocs1_zip_index')}:{row.get('archive_member')}"

    write_manifest(rows, args.out)
    summary = {
        "schema": "rar-govdocs1-remote-zip-v1",
        "directory_range": [args.start_dir, args.end_dir],
        "zip_archives_requested": args.end_dir - args.start_dir + 1,
        "zip_archives_ok": len(zip_summaries),
        "zip_archives_failed": len(errors),
        "archive_entries_seen": sum(x["entry_count"] for x in zip_summaries),
        "pub_member_candidates": sum(x["pub_members"] for x in zip_summaries),
        "manifest_rows": len(rows),
        "fetched_members": sum(r.get("fetch_status") == "ok_archive_member" for r in rows),
        "unique_sha256": len({r.get("sha256") for r in rows if r.get("sha256")}),
        "publisher_cfb_hints": sum(r.get("classification") == "cfb_publisher_hint" for r in rows),
        "classifications": {
            key: sum(r.get("classification") == key for r in rows)
            for key in sorted({r.get("classification") for r in rows if r.get("classification")})
        },
        "errors": errors,
    }
    sp = args.summary or args.out.with_name(args.out.stem + ".summary.json")
    sp.write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
