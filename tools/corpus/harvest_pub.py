#!/usr/bin/env python3
"""Harvest public Microsoft Publisher (.pub) candidates from a seed CSV.

Stdlib-only. Never executes downloaded documents. Keeps provenance for
successes, failures, negatives, duplicates, and quarantine samples.
"""
from __future__ import annotations

import argparse
import csv
import gzip
import hashlib
import html
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile
import zlib
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from html.parser import HTMLParser
from pathlib import Path
from typing import Iterable
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, quote, unquote, urljoin, urlparse
from urllib.request import Request, urlopen

UA = "pub-rs-corpus-harvester/1.0 (research corpus collector)"
CFB_MAGIC = bytes.fromhex("D0CF11E0A1B11AE1")
MAX_PAGE_BYTES = 8 * 1024 * 1024

@dataclass
class Link:
    href: str
    text: str

class LinkParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.links: list[Link] = []
        self._href: str | None = None
        self._text: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag.lower() != "a":
            return
        href = dict(attrs).get("href")
        if href:
            self._href = href
            self._text = []

    def handle_data(self, data: str) -> None:
        if self._href is not None:
            self._text.append(data)

    def handle_endtag(self, tag: str) -> None:
        if tag.lower() == "a" and self._href is not None:
            self.links.append(Link(self._href, " ".join(self._text).strip()))
            self._href = None
            self._text = []

def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")

def norm(s: str) -> str:
    s = html.unescape(unquote(s or "")).replace("\\", "/").strip().casefold()
    return re.sub(r"\s+", " ", s)

def safe_name(name: str) -> str:
    name = unquote(name or "candidate.pub").replace("\\", "_").replace("/", "_")
    name = re.sub(r'[\x00-\x1f<>:"|?*]+', "_", name)
    name = re.sub(r"\s+", " ", name).strip(" .") or "candidate.pub"
    return name[:180]

def url_filename(url: str) -> str:
    return unquote(Path(urlparse(url).path).name)

def pubish(text: str) -> bool:
    return ".pub" in norm(text)

def normalize_request_url(url: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme.casefold() not in {"http", "https"}:
        return url
    path = quote(parsed.path, safe="/%:@!$&'()*+,;=-._~")
    query = quote(parsed.query, safe="=&;%:+,/?@!$'()*[]-._~")
    return parsed._replace(path=path, query=query).geturl()


def request_bytes(
    url: str,
    timeout: float,
    max_bytes: int | None = None,
    extra_headers: dict[str, str] | None = None,
) -> tuple[bytes, dict]:
    headers = {"User-Agent": UA, "Accept": "*/*", "Accept-Language": "en-US,en;q=0.8"}
    if extra_headers:
        headers.update(extra_headers)
    request_url = normalize_request_url(url)
    req = Request(request_url, headers=headers)
    with urlopen(req, timeout=timeout) as resp:
        content_length = resp.headers.get("Content-Length")
        if content_length and max_bytes is not None and int(content_length) > max_bytes:
            raise ValueError(f"content-length {content_length} exceeds max {max_bytes}")
        data = resp.read((max_bytes + 1) if max_bytes is not None else -1)
        if max_bytes is not None and len(data) > max_bytes:
            raise ValueError(f"download exceeded max {max_bytes}")
        return data, {
            "final_url": resp.geturl(),
            "http_status": getattr(resp, "status", 200),
            "content_type": resp.headers.get("Content-Type", ""),
            "content_length_header": content_length or "",
        }

def fetch_with_retries(
    url: str,
    timeout: float,
    max_bytes: int | None,
    retries: int = 2,
    extra_headers: dict[str, str] | None = None,
) -> tuple[bytes, dict]:
    last: Exception | None = None
    for attempt in range(retries + 1):
        try:
            return request_bytes(url, timeout, max_bytes, extra_headers=extra_headers)
        except (HTTPError, URLError, TimeoutError, ValueError, OSError) as exc:
            last = exc
            if attempt < retries:
                time.sleep(1.5 * (attempt + 1))
    assert last is not None
    raise last

def _split_header_block(data: bytes, label: str) -> tuple[str, bytes]:
    marker = b"\r\n\r\n"
    pos = data.find(marker)
    step = 4
    if pos < 0:
        marker = b"\n\n"
        pos = data.find(marker)
        step = 2
    if pos < 0:
        raise ValueError(f"{label} headers not terminated")
    head = data[:pos].decode("iso-8859-1", errors="replace")
    return head, data[pos + step:]


def _header_map(lines: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for line in lines:
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        out[key.strip().casefold()] = value.strip()
    return out


def _decode_chunked(data: bytes, max_bytes: int) -> bytes:
    out = bytearray()
    pos = 0
    while True:
        line_end = data.find(b"\r\n", pos)
        if line_end < 0:
            raise ValueError("invalid chunked response: missing size terminator")
        size_text = data[pos:line_end].split(b";", 1)[0].strip()
        try:
            size = int(size_text, 16)
        except ValueError as exc:
            raise ValueError("invalid chunked response: bad chunk size") from exc
        pos = line_end + 2
        if size == 0:
            break
        end = pos + size
        if end + 2 > len(data) or data[end:end + 2] != b"\r\n":
            raise ValueError("invalid chunked response: truncated chunk")
        out.extend(data[pos:end])
        if len(out) > max_bytes:
            raise ValueError(
                f"decoded Common Crawl payload exceeds max {max_bytes}"
            )
        pos = end + 2
    return bytes(out)


def decode_common_crawl_warc_member(
    compressed: bytes,
    max_bytes: int,
) -> tuple[bytes, dict]:
    try:
        record = gzip.decompress(compressed)
    except (OSError, EOFError) as exc:
        raise ValueError(
            f"invalid Common Crawl gzip member: {exc}"
        ) from exc

    warc_head, remainder = _split_header_block(record, "WARC")
    warc_lines = warc_head.splitlines()
    warc_headers = _header_map(warc_lines[1:])
    warc_type = warc_headers.get("warc-type", "")
    if warc_type.casefold() != "response":
        raise ValueError(
            f"Common Crawl WARC-Type is {warc_type!r}, expected response"
        )
    declared = warc_headers.get("content-length")
    if declared:
        try:
            remainder = remainder[:int(declared)]
        except ValueError as exc:
            raise ValueError("invalid WARC Content-Length") from exc

    http_head, body = _split_header_block(remainder, "HTTP")
    http_lines = http_head.splitlines()
    if not http_lines or not http_lines[0].startswith("HTTP/"):
        raise ValueError(
            "WARC response does not contain an HTTP status line"
        )
    parts = http_lines[0].split(None, 2)
    status = (
        int(parts[1])
        if len(parts) > 1 and parts[1].isdigit()
        else 0
    )
    headers = _header_map(http_lines[1:])

    transfer = headers.get("transfer-encoding", "").casefold()
    if "chunked" in transfer:
        body = _decode_chunked(body, max_bytes)

    encoding = headers.get("content-encoding", "").casefold().strip()
    if encoding in {"gzip", "x-gzip"}:
        body = gzip.decompress(body)
    elif encoding == "deflate":
        try:
            body = zlib.decompress(body)
        except zlib.error:
            body = zlib.decompress(body, -zlib.MAX_WBITS)
    elif encoding and encoding != "identity":
        raise ValueError(
            "unsupported HTTP Content-Encoding in Common Crawl record: "
            + encoding
        )

    if len(body) > max_bytes:
        raise ValueError(
            f"decoded Common Crawl payload exceeds max {max_bytes}"
        )
    return body, {
        "http_status": status,
        "content_type": headers.get("content-type", ""),
        "content_length_header": headers.get("content-length", ""),
        "cc_http_content_encoding": encoding,
        "cc_http_transfer_encoding": transfer,
        "cc_warc_truncated": warc_headers.get("warc-truncated", ""),
    }


def decode_common_crawl_arc_member(
    compressed: bytes,
    max_bytes: int,
) -> tuple[bytes, dict]:
    """Decode one individually-gzipped legacy Common Crawl ARC response record."""
    try:
        record = gzip.decompress(compressed)
    except (OSError, EOFError) as exc:
        raise ValueError(
            f"invalid Common Crawl ARC gzip member: {exc}"
        ) from exc

    line_end = record.find(b"\n")
    if line_end < 0:
        raise ValueError("Common Crawl ARC record has no metadata line terminator")
    metadata_line = record[:line_end].rstrip(b"\r").decode(
        "iso-8859-1",
        errors="replace",
    )
    parts = metadata_line.split()
    if len(parts) < 5 or parts[0].startswith("filedesc://"):
        raise ValueError(
            f"invalid Common Crawl ARC response metadata: {metadata_line[:240]!r}"
        )

    archive_url = parts[0]
    archive_date = parts[2]
    archive_mime = parts[3]
    try:
        declared = int(parts[-1])
    except ValueError as exc:
        raise ValueError("invalid ARC archive length") from exc

    remainder = record[line_end + 1:]
    if declared > 0:
        if declared > len(remainder):
            raise ValueError(
                "ARC archive length exceeds decompressed record: "
                f"declared {declared}, got {len(remainder)}"
            )
        remainder = remainder[:declared]

    http_head, body = _split_header_block(remainder, "HTTP")
    http_lines = http_head.splitlines()
    if not http_lines or not http_lines[0].startswith("HTTP/"):
        raise ValueError(
            "ARC response does not contain an HTTP status line"
        )
    status_parts = http_lines[0].split(None, 2)
    status = (
        int(status_parts[1])
        if len(status_parts) > 1 and status_parts[1].isdigit()
        else 0
    )
    headers = _header_map(http_lines[1:])

    transfer = headers.get("transfer-encoding", "").casefold()
    if "chunked" in transfer:
        body = _decode_chunked(body, max_bytes)

    encoding = headers.get("content-encoding", "").casefold().strip()
    if encoding in {"gzip", "x-gzip"}:
        body = gzip.decompress(body)
    elif encoding == "deflate":
        try:
            body = zlib.decompress(body)
        except zlib.error:
            body = zlib.decompress(body, -zlib.MAX_WBITS)
    elif encoding and encoding != "identity":
        raise ValueError(
            "unsupported HTTP Content-Encoding in Common Crawl ARC record: "
            + encoding
        )

    if len(body) > max_bytes:
        raise ValueError(
            f"decoded Common Crawl payload exceeds max {max_bytes}"
        )
    return body, {
        "http_status": status,
        "content_type": headers.get("content-type", "") or archive_mime,
        "content_length_header": headers.get("content-length", ""),
        "cc_http_content_encoding": encoding,
        "cc_http_transfer_encoding": transfer,
        "cc_arc_url": archive_url,
        "cc_arc_date": archive_date,
        "cc_arc_declared_length": declared,
    }


def common_crawl_fetch(
    row: dict[str, str],
    timeout: float,
    max_bytes: int,
    retries: int = 2,
) -> tuple[bytes, dict]:
    filename = (row.get("cc_warc_filename") or "").strip().lstrip("/")
    offset_text = (row.get("cc_warc_offset") or "").strip()
    length_text = (row.get("cc_warc_length") or "").strip()
    if not filename or not offset_text or not length_text:
        raise ValueError("Common Crawl archive coordinates are incomplete")
    try:
        offset = int(offset_text)
        length = int(length_text)
    except ValueError as exc:
        raise ValueError("Common Crawl archive coordinates are invalid") from exc
    if offset < 0 or length <= 0:
        raise ValueError("Common Crawl archive coordinates are out of range")
    # ARC/WARC framing and HTTP headers add only a small amount around a payload.
    if length > max_bytes + 4 * 1024 * 1024:
        raise ValueError(
            f"Common Crawl record length {length} exceeds bounded payload limit"
        )

    archive_url = "https://data.commoncrawl.org/" + filename
    end = offset + length - 1
    last: Exception | None = None
    for attempt in range(retries + 1):
        try:
            req = Request(
                archive_url,
                headers={
                    "User-Agent": UA,
                    "Accept": "*/*",
                    "Accept-Encoding": "identity",
                    "Range": f"bytes={offset}-{end}",
                },
            )
            with urlopen(req, timeout=timeout) as resp:
                status = getattr(resp, "status", 0)
                if status != 206:
                    raise ValueError(
                        f"Common Crawl range request returned HTTP {status}"
                    )
                compressed = resp.read(length + 1)
                if len(compressed) != length:
                    raise ValueError(
                        "Common Crawl range length mismatch: "
                        f"expected {length}, got {len(compressed)}"
                    )
                legacy_arc = filename.casefold().endswith(".arc.gz")
                decoder = (
                    decode_common_crawl_arc_member
                    if legacy_arc
                    else decode_common_crawl_warc_member
                )
                body, meta = decoder(compressed, max_bytes)
                meta.update({
                    "final_url": (row.get("direct_url") or "").strip(),
                    "cc_archive_url": archive_url,
                    "cc_range": f"bytes={offset}-{end}",
                    "cc_archive_format": "arc" if legacy_arc else "warc",
                    "cc_fetch_source": "arc_range" if legacy_arc else "warc_range",
                })
                return body, meta
        except (
            HTTPError,
            URLError,
            TimeoutError,
            ValueError,
            OSError,
            gzip.BadGzipFile,
            zlib.error,
        ) as exc:
            last = exc
            if attempt < retries:
                time.sleep(1.5 * (attempt + 1))
    assert last is not None
    raise last


def extract_links(page_url: str, page_bytes: bytes) -> list[Link]:
    text = page_bytes.decode("utf-8", errors="replace")
    parser = LinkParser()
    parser.feed(text)
    return [Link(urljoin(page_url, x.href), x.text) for x in parser.links]

def score_link(link: Link, wanted: str) -> int:
    wh, href, text, base = norm(wanted), norm(link.href), norm(link.text), norm(url_filename(link.href))
    wanted_stem = norm(Path(wanted).stem)
    score = 0
    if wh and base == wh: score += 100
    if wh and wh in href: score += 80
    if wh and wh in text: score += 70
    # Some school CMSes render uploaded Publisher attachments as
    # "<filename stem> PUB File" while the opaque href carries no extension.
    if wanted_stem and wanted_stem in text and ("pub file" in text or pubish(text)): score += 70
    if pubish(base): score += 20
    if pubish(href): score += 10
    if link.href.lower().startswith("https://"): score += 2
    return score

def aws_presign_expiry(url: str) -> datetime | None:
    """Return the UTC expiry of an AWS SigV4 presigned URL when available."""
    try:
        query = parse_qs(urlparse(url).query)
        raw_date = (query.get("X-Amz-Date") or query.get("x-amz-date") or [""])[0]
        raw_expires = (query.get("X-Amz-Expires") or query.get("x-amz-expires") or [""])[0]
        if not raw_date or not raw_expires:
            return None
        issued = datetime.strptime(raw_date, "%Y%m%dT%H%M%SZ").replace(tzinfo=timezone.utc)
        return issued + timedelta(seconds=int(raw_expires))
    except (TypeError, ValueError, OverflowError):
        return None


def aws_presign_is_expired(url: str, now: datetime | None = None) -> bool:
    expiry = aws_presign_expiry(url)
    if expiry is None:
        return False
    current = now or datetime.now(timezone.utc)
    if current.tzinfo is None:
        current = current.replace(tzinfo=timezone.utc)
    return current >= expiry


def inspect_angular_transfer_state(raw_text: str, wanted: str, base_page: str) -> tuple[str | None, str]:
    """Inspect Angular SSR TransferState for public file metadata.

    Angular hydration serializes cached HttpClient responses into a JSON
    <script id="ng-state">. School Jotter 3 SSR pages can contain legacy file
    rows that disappear from the live DOM after hydration; the transfer state
    is therefore a better source of stable IDs/URLs than browser clicking.
    """
    script_body = None
    for m in re.finditer(r"<script\b([^>]*)>(.*?)</script\s*>", raw_text, flags=re.I | re.S):
        attrs, body = m.group(1), m.group(2)
        if re.search(r'''\bid\s*=\s*["']ng-state["']''', attrs, flags=re.I):
            script_body = body.strip()
            break
    if not script_body:
        return None, ""

    try:
        state = json.loads(html.unescape(script_body))
    except Exception as exc:
        return None, f"ng_state_json_error:{type(exc).__name__}:{exc}"

    wanted_cf = norm(wanted)
    matches: list[tuple[list[str], object]] = []

    def walk(node: object, path: list[str]) -> None:
        if len(matches) >= 12:
            return
        if isinstance(node, dict):
            direct_hit = any(
                isinstance(v, str) and wanted_cf and wanted_cf in norm(v)
                for v in node.values()
            )
            if direct_hit:
                matches.append((path[:], node))
            for k, v in node.items():
                walk(v, path + [str(k)])
        elif isinstance(node, list):
            for i, v in enumerate(node):
                if isinstance(v, str) and wanted_cf and wanted_cf in norm(v):
                    matches.append((path + [str(i)], node))
                walk(v, path + [str(i)])

    walk(state, [])
    if not matches:
        return None, "ng_state_present:no_filename_match"

    url_candidates: list[tuple[int, str]] = []
    debug_parts: list[str] = []

    def consider_string(value: str, key_hint: str = "") -> None:
        value = html.unescape(value).strip()
        if not value:
            return
        low = value.casefold()
        is_urlish = value.startswith(("http://", "https://", "/"))
        if not is_urlish:
            return
        score = 2
        if wanted_cf and wanted_cf in norm(value):
            score += 100
        if ".pub" in low:
            score += 35
        if "download" in low:
            score += 30
        if any(tok in low for tok in ("attachment", "/file", "document", "media", "asset", "upload")):
            score += 15
        kh = key_hint.casefold()
        if any(tok in kh for tok in ("download", "url", "href", "path", "uri", "src", "origin")):
            score += 20
        # School Jotter 3 serializes the original file as a short-lived
        # presigned S3 URL under url.origin, alongside a PDF preview URL.
        # Cached SSR pages can outlive the one-hour AWS signature. Never
        # promote an already-expired presign into the fetch path.
        if "amazonaws.com" in low and ("x-amz-" in low or "sj3-bucket-media" in low):
            if aws_presign_is_expired(value):
                expiry = aws_presign_expiry(value)
                debug_parts.append(
                    "expired_aws_presign:"
                    + (expiry.isoformat() if expiry is not None else "unknown")
                )
                return
            score += 45
        url_candidates.append((score, urljoin(base_page, value)))

    def collect(node: object, depth: int = 0, key_hint: str = "") -> None:
        if depth > 4:
            return
        if isinstance(node, dict):
            for k, v in node.items():
                if isinstance(v, str):
                    consider_string(v, str(k))
                elif isinstance(v, (dict, list)):
                    collect(v, depth + 1, str(k))
        elif isinstance(node, list):
            for v in node[:50]:
                if isinstance(v, str):
                    consider_string(v, key_hint)
                elif isinstance(v, (dict, list)):
                    collect(v, depth + 1, key_hint)

    for path, obj in matches[:4]:
        # API/URL-like TransferState keys are useful context too.
        for key in path:
            consider_string(key, "transfer_state_key")
        collect(obj)
        try:
            compact = json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
        except Exception:
            compact = repr(obj)
        debug_parts.append(f"path={'/'.join(path[-5:])};object={compact[:1800]}")

    url_candidates.sort(key=lambda x: x[0], reverse=True)
    best_url = None
    if url_candidates and url_candidates[0][0] >= 35:
        best_url = url_candidates[0][1]

    debug = "ng_state_match:" + " || ".join(debug_parts)
    if url_candidates:
        debug += " || urls=" + json.dumps(url_candidates[:8], ensure_ascii=False)
    return best_url, debug[:6000]


def resolve_yandex_public_url(public_url: str, timeout: float) -> str:
    """Resolve a public Yandex Disk share URL to its current download href."""
    api = (
        "https://cloud-api.yandex.net/v1/disk/public/resources/download"
        "?public_key=" + quote(public_url, safe="")
    )
    payload, _ = request_bytes(api, timeout, MAX_PAGE_BYTES)
    data = json.loads(payload.decode("utf-8"))
    href = (data.get("href") or "").strip()
    if not href:
        raise ValueError("Yandex public API response did not include href")
    return href


def resolve_candidate_url(row: dict[str, str], timeout: float) -> tuple[str | None, str, str]:
    direct = (row.get("direct_url") or "").strip()
    if direct:
        host = urlparse(direct).netloc.casefold()
        if host in {"disk.yandex.ru", "yadi.sk"}:
            try:
                return resolve_yandex_public_url(direct, timeout), "yandex_public_api", ""
            except Exception as exc:
                return None, f"yandex_public_api_failed:{type(exc).__name__}:{exc}", ""
        return direct, "seed_direct_url", ""
    source_page = (row.get("source_page") or "").strip()
    wanted = (row.get("candidate_filename") or "").strip()
    if not source_page:
        return None, "no_source_page", ""
    try:
        page_bytes, meta = fetch_with_retries(
            source_page,
            timeout,
            MAX_PAGE_BYTES,
            retries=1,
            extra_headers={"Cache-Control": "no-cache", "Pragma": "no-cache"},
        )
    except Exception as exc:
        return None, f"source_page_fetch_failed:{type(exc).__name__}:{exc}", ""
    ctype = (meta.get("content_type") or "").lower()
    if "html" not in ctype and b"<html" not in page_bytes[:4096].lower():
        return None, f"source_page_not_html:{ctype}", ""
    base_page = meta.get("final_url") or source_page
    raw_text = page_bytes.decode("utf-8", errors="replace")
    links = extract_links(base_page, page_bytes)

    # Prefer a stable public attachment href when the exact filename is
    # rendered next to one. School Jotter pages can expose both a durable
    # /downloadfile/<id> link and a short-lived TransferState S3 presign;
    # the durable public link is the better acquisition locator.
    #
    # If no such href exists (for example JS-only Jotter3 rows), fall back
    # to the Angular TransferState original-file metadata below.
    wanted_cf = html.unescape(wanted).casefold()
    raw_cf = html.unescape(raw_text).casefold()
    pos = raw_cf.find(wanted_cf) if wanted_cf else -1
    if pos >= 0:
        lo, hi = max(0, pos - 1800), min(len(raw_text), pos + len(wanted) + 1800)
        window = raw_text[lo:hi]
        local_pos = pos - lo
        candidates = []
        for m in re.finditer(r'''href\s*=\s*["']([^"']+)["']''', window, flags=re.I):
            href = html.unescape(m.group(1))
            href_cf = href.casefold()
            attachmentish = any(
                token in href_cf
                for token in (
                    "/downloadfile/",
                    "attachments/documents.asp",
                    "/attachments/",
                    "/uploads/",
                    "/upload/",
                    "/app/download/",
                    ".pub",
                )
            )
            if not attachmentish:
                continue
            distance = abs(m.start() - local_pos)
            after_penalty = 0 if m.start() >= local_pos else 250
            candidates.append((distance + after_penalty, urljoin(base_page, href)))
        if candidates:
            candidates.sort(key=lambda x: x[0])
            return candidates[0][1], f"near_filename_attachment:{candidates[0][0]}", ""

    # Angular SSR may carry the underlying public file metadata inside
    # TransferState even when hydration later removes the legacy row.
    state_url, state_debug = inspect_angular_transfer_state(raw_text, wanted, base_page)
    if state_url:
        return state_url, "angular_transfer_state", state_debug

    debug_context = state_debug or ""
    if pos >= 0:
        context_lo = max(0, pos - 900)
        context_hi = min(len(raw_text), pos + len(wanted) + 1600)
        html_context = re.sub(r"\s+", " ", raw_text[context_lo:context_hi]).strip()[:2400]
        debug_context = (debug_context + " || html=" + html_context).strip(" |")[:6000]

    # Opaque CMS attachment URLs often do not contain ".pub" at all
    # (for example attachments/documents.asp?id=123). Score every anchor
    # against the expected filename and keep extension presence only as
    # an additional signal.
    ranked = sorted(
        ((score_link(link, wanted), link) for link in links),
        key=lambda x: x[0], reverse=True,
    )
    if not ranked:
        return None, "no_pub_link_found", debug_context
    best_score, best = ranked[0]
    if best_score < 20:
        ranked_debug = json.dumps(
            [
                {"score": score, "href": link.href, "text": link.text[:300]}
                for score, link in ranked[:8]
            ],
            ensure_ascii=False,
        )
        debug_context = (debug_context + " || ranked_links=" + ranked_debug).strip(" |")[:6000]
        return None, "no_confident_pub_link", debug_context
    return best.href, f"source_page_link_score:{best_score}", ""

def browser_fetch(
    helper: Path,
    source_page: str,
    wanted: str,
    timeout: float,
    max_bytes: int,
) -> tuple[bytes, dict]:
    """Use a real browser only as a fallback for public JS-only download buttons."""
    with tempfile.TemporaryDirectory(prefix="pub-jotter3-") as td:
        td_path = Path(td)
        payload = td_path / "payload.bin"
        meta_path = td_path / "meta.json"
        cmd = [
            "node", str(helper),
            "--url", source_page,
            "--filename", wanted,
            "--out", str(payload),
            "--meta", str(meta_path),
        ]
        p = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
            timeout=max(60.0, timeout * 4),
        )
        if p.returncode != 0:
            detail = (p.stderr or p.stdout or "").strip()[-1200:]
            raise RuntimeError(f"browser helper failed ({p.returncode}): {detail}")
        if not payload.exists():
            raise RuntimeError("browser helper returned success without payload")
        data = payload.read_bytes()
        if len(data) > max_bytes:
            raise ValueError(f"browser download exceeded max {max_bytes}")
        meta = {}
        if meta_path.exists():
            try:
                meta = json.loads(meta_path.read_text(encoding="utf-8"))
            except Exception:
                meta = {}
        meta.setdefault("final_url", meta.get("download_url", source_page))
        meta.setdefault("http_status", "browser")
        meta.setdefault("content_type", "")
        meta.setdefault("content_length_header", "")
        return data, meta

def looks_text(data: bytes) -> bool:
    sample = data[:65536]
    if not sample or sample.count(b"\x00") > len(sample) * 0.02:
        return False
    printable = sum((32 <= b <= 126) or b in (9, 10, 13) for b in sample)
    return printable / len(sample) > 0.90

def file_command(path: Path) -> str:
    exe = shutil.which("file")
    if not exe:
        return ""
    try:
        p = subprocess.run([exe, "-b", "--mime-type", str(path)], check=False, capture_output=True, text=True, timeout=10)
        return p.stdout.strip()
    except Exception:
        return ""

def classify(data: bytes) -> tuple[str, list[str]]:
    hints: list[str] = []
    lower = data[:4 * 1024 * 1024].lower()
    if data.startswith(CFB_MAGIC):
        for label, needle in (
            ("publisher_string", "microsoft publisher"),
            ("mspublisher_string", "mspublisher"),
            ("contents_stream", "contents"),
            ("quill_storage", "quill"),
            ("escher_name", "escher"),
        ):
            if needle.encode("utf-16le") in lower or needle.encode() in lower:
                hints.append(label)
        pub_hint = any(x in hints for x in ("publisher_string", "mspublisher_string", "quill_storage"))
        return ("cfb_publisher_hint" if pub_hint else "cfb_candidate"), hints
    stripped = data[:8192].lstrip().lower()
    html_probe = stripped[:2048]
    if (
        stripped.startswith((b"<!doctype html", b"<html", b"<head", b"<body"))
        or (
            stripped.startswith(b"<")
            and (b"<!doctype html" in html_probe or b"<html" in html_probe)
        )
    ):
        return "html_mislabel", hints
    if data.startswith(b"PK\x03\x04"): return "zip_mislabel", hints
    if data.startswith(b"%PDF-"): return "pdf_mislabel", hints
    if stripped.startswith(b"{\\rtf"): return "rtf_mislabel", hints
    if looks_text(data): return "text_mislabel", hints
    if len(data) < 512: return "tiny_or_truncated", hints
    return "unknown_binary", hints

def classify_harvest_payload(
    data: bytes,
    meta: dict | None = None,
) -> tuple[str, list[str]]:
    classification, hints = classify(data)
    warc_truncated = str((meta or {}).get("cc_warc_truncated") or "").strip()
    if warc_truncated and classification == "cfb_publisher_hint":
        classification = "cfb_publisher_truncated"
    elif warc_truncated and classification == "cfb_candidate":
        classification = "cfb_candidate_truncated"
    return classification, hints


def bucket_for(classification: str, quarantine: bool) -> str:
    if quarantine:
        return "quarantine_active_content"
    if classification == "zip_container":
        return "source_container"
    if classification.endswith("_truncated"):
        return "truncated_partial"
    if classification.endswith("_mislabel") or classification == "tiny_or_truncated":
        return "mislabel_negative"
    return "natural_wild"

def extract_pub_archive_members(
    archive_data: bytes,
    parent_rec: dict,
    seed_row: dict[str, str],
    out_dir: Path,
    seed_index: int,
    shard_index: int,
    quarantine: bool,
    seen_sha: dict[str, str],
    max_member_bytes: int,
    max_total_bytes: int,
    max_members: int,
) -> list[dict]:
    """Extract only .pub members from a ZIP, without trusting archive paths."""
    children: list[dict] = []
    total = 0
    try:
        with zipfile.ZipFile(io.BytesIO(archive_data)) as zf:
            pub_infos = [
                info for info in zf.infolist()
                if not info.is_dir() and info.filename.casefold().endswith(".pub")
            ]
            parent_rec["archive_pub_member_candidates"] = len(pub_infos)

            for member_index, info in enumerate(pub_infos[:max_members], start=1):
                child: dict = dict(seed_row)
                child.update({
                    "row_kind": "archive_member",
                    "harvested_at_utc": utc_now(),
                    "seed_index": seed_index,
                    "shard_index": shard_index,
                    "archive_member_index": member_index,
                    "archive_member": info.filename,
                    "candidate_filename": Path(info.filename.replace("\\", "/")).name or f"member-{member_index}.pub",
                    "parent_archive_sha256": parent_rec.get("sha256", ""),
                    "parent_archive_filename": seed_row.get("candidate_filename", ""),
                    "parent_archive_url": parent_rec.get("final_url", "") or parent_rec.get("resolved_url", ""),
                    "url_resolution": "archive_member",
                })

                normalized = info.filename.replace("\\", "/")
                if normalized.startswith("/") or any(part == ".." for part in normalized.split("/")):
                    child["fetch_status"] = "archive_member_skipped_unsafe_path"
                    children.append(child)
                    continue
                if info.flag_bits & 0x1:
                    child["fetch_status"] = "archive_member_skipped_encrypted"
                    children.append(child)
                    continue

                unix_mode = (info.external_attr >> 16) & 0o170000
                if unix_mode == 0o120000:
                    child["fetch_status"] = "archive_member_skipped_symlink"
                    children.append(child)
                    continue
                if info.file_size > max_member_bytes:
                    child["fetch_status"] = "archive_member_skipped_too_large"
                    child["declared_size_bytes"] = info.file_size
                    children.append(child)
                    continue
                if total + info.file_size > max_total_bytes:
                    child["fetch_status"] = "archive_member_skipped_total_limit"
                    children.append(child)
                    continue

                try:
                    member_data = zf.read(info)
                except (RuntimeError, OSError, zipfile.BadZipFile) as exc:
                    child["fetch_status"] = "archive_member_read_failed"
                    child["error"] = f"{type(exc).__name__}: {exc}"
                    children.append(child)
                    continue

                total += len(member_data)
                child["fetch_status"] = "ok_archive_member"
                child["size_bytes"] = len(member_data)
                child["zip_compressed_size"] = info.compress_size
                child["zip_crc32"] = f"{info.CRC:08x}"
                sha = hashlib.sha256(member_data).hexdigest()
                child["sha256"] = sha
                classification, hints = classify(member_data)
                child["classification"] = classification
                child["publisher_hints"] = ";".join(hints)
                child["bucket"] = bucket_for(classification, quarantine)

                duplicate_of = seen_sha.get(sha)
                if duplicate_of:
                    child["duplicate_of_sha256"] = sha
                    child["duplicate_of_file"] = duplicate_of
                    children.append(child)
                    continue

                target = (
                    out_dir / child["bucket"]
                    / f"s{shard_index:02d}_{seed_index:04d}__a{member_index:03d}__{safe_name(child['candidate_filename'])}"
                )
                target.write_bytes(member_data)
                child["stored_path"] = str(target.relative_to(out_dir))
                child["file_mime"] = file_command(target)
                seen_sha[sha] = child["stored_path"]
                children.append(child)

            if len(pub_infos) > max_members:
                parent_rec["archive_pub_members_truncated"] = len(pub_infos) - max_members
            parent_rec["archive_pub_members_processed"] = len(children)
            parent_rec["archive_pub_uncompressed_bytes_processed"] = total
    except zipfile.BadZipFile as exc:
        parent_rec["archive_error"] = f"{type(exc).__name__}: {exc}"
    return children

def write_manifest(rows: list[dict], out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "manifest.json").write_text(json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf-8")
    keys: list[str] = []
    seen: set[str] = set()
    for row in rows:
        for key in row:
            if key not in seen:
                seen.add(key); keys.append(key)
    with (out_dir / "manifest.csv").open("w", encoding="utf-8", newline="") as f:
        w = csv.DictWriter(f, fieldnames=keys); w.writeheader(); w.writerows(rows)

def append_step_summary(rows: list[dict]) -> None:
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not path:
        return
    statuses = Counter(r.get("fetch_status", "") for r in rows)
    classes = Counter(r.get("classification", "") for r in rows if r.get("classification"))
    buckets = Counter(r.get("bucket", "") for r in rows if r.get("bucket"))
    unique = {r["sha256"] for r in rows if r.get("sha256") and not r.get("duplicate_of_sha256")}
    seed_count = sum(r.get("row_kind", "seed") != "archive_member" for r in rows)
    archive_ok = statuses.get("ok_archive_member", 0)
    lines = [
        "## PUB corpus harvest", "",
        f"- Seed rows processed: **{seed_count}**",
        f"- Seed resources fetched: **{statuses.get('ok', 0)}**",
        f"- PUB archive members extracted: **{archive_ok}**",
        f"- Unique downloaded payloads: **{len(unique)}**", "",
        "### Buckets",
    ]
    lines += [f"- `{k}`: {v}" for k, v in sorted(buckets.items())]
    lines += ["", "### Classifications"]
    lines += [f"- `{k}`: {v}" for k, v in sorted(classes.items())]
    Path(path).write_text("\n".join(lines) + "\n", encoding="utf-8")

def iter_seed(path: Path) -> Iterable[dict[str, str]]:
    with path.open("r", encoding="utf-8-sig", newline="") as f:
        yield from csv.DictReader(f)

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--max-files", type=int, default=250)
    ap.add_argument("--timeout", type=float, default=25.0)
    ap.add_argument("--max-bytes", type=int, default=100 * 1024 * 1024)
    ap.add_argument("--max-archive-total-bytes", type=int, default=300 * 1024 * 1024)
    ap.add_argument("--max-archive-members", type=int, default=200)
    ap.add_argument("--per-host-delay", type=float, default=0.75)
    ap.add_argument("--include-quarantine", action="store_true")
    ap.add_argument("--browser-helper", type=Path)
    ap.add_argument("--shard-index", type=int, default=0)
    ap.add_argument("--shard-count", type=int, default=1)
    args = ap.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    for bucket in (
        "natural_wild",
        "mislabel_negative",
        "truncated_partial",
        "quarantine_active_content",
        "source_container",
    ):
        (args.out / bucket).mkdir(parents=True, exist_ok=True)

    rows_out: list[dict] = []
    seen_sha: dict[str, str] = {}
    last_host_time: dict[str, float] = {}
    seed_rows = list(iter_seed(args.seed))
    if args.max_files >= 0:
        seed_rows = seed_rows[:args.max_files]
    if args.shard_count < 1 or not (0 <= args.shard_index < args.shard_count):
        raise SystemExit("invalid shard index/count")
    seed_rows = seed_rows[args.shard_index::args.shard_count]

    for idx, row in enumerate(seed_rows, start=1):
        rec: dict = dict(row)
        rec["row_kind"] = "seed"
        rec["harvested_at_utc"] = utc_now(); rec["seed_index"] = idx; rec["shard_index"] = args.shard_index; rec["shard_count"] = args.shard_count
        quarantine = norm(row.get("quarantine", "")) in {"yes", "true", "1"}
        if quarantine and not args.include_quarantine:
            rec["fetch_status"] = "skipped_quarantine"; rows_out.append(rec); continue

        prefetched: tuple[bytes, dict] | None = None
        resolved_url: str | None = None
        resolution = ""
        resolver_debug = ""

        has_cc_capture = all(
            (row.get(key) or "").strip()
            for key in (
                "cc_warc_filename",
                "cc_warc_offset",
                "cc_warc_length",
            )
        )
        if has_cc_capture:
            try:
                prefetched = common_crawl_fetch(
                    row,
                    args.timeout,
                    args.max_bytes,
                )
                resolved_url = (row.get("direct_url") or "").strip() or None
                resolution = "common_crawl_warc"
                print(
                    f"[{idx}/{len(seed_rows)}] CC-WARC "
                    f"{row.get('candidate_filename')}"
                )
            except Exception as exc:
                rec["cc_fetch_error"] = f"{type(exc).__name__}: {exc}"

        if prefetched is None:
            resolved_url, resolution, resolver_debug = resolve_candidate_url(
                row,
                args.timeout,
            )

        rec["resolved_url"] = resolved_url or ""
        rec["url_resolution"] = resolution
        rec["resolver_debug"] = resolver_debug

        browser_eligible = (
            not resolved_url
            and args.browser_helper is not None
            and resolver_debug
            and 'title="Download"' in resolver_debug
            and (row.get("source_page") or "").strip()
            and (row.get("candidate_filename") or "").strip()
        )
        if browser_eligible:
            try:
                prefetched = browser_fetch(
                    args.browser_helper,
                    (row.get("source_page") or "").strip(),
                    (row.get("candidate_filename") or "").strip(),
                    args.timeout,
                    args.max_bytes,
                )
                rec["url_resolution"] = "browser_click_download"
                rec["resolved_url"] = prefetched[1].get("download_url", "") or prefetched[1].get("final_url", "")
                print(f"[{idx}/{len(seed_rows)}] BROWSER {row.get('candidate_filename')}")
            except Exception as exc:
                rec["browser_error"] = f"{type(exc).__name__}: {exc}"

        if not resolved_url and prefetched is None:
            rec["fetch_status"] = "unresolved"; rows_out.append(rec); write_manifest(rows_out, args.out); continue

        if prefetched is not None:
            data, meta = prefetched
        else:
            assert resolved_url is not None
            host = urlparse(resolved_url).netloc.casefold()
            prev = last_host_time.get(host)
            if prev is not None:
                delay = args.per_host_delay - (time.monotonic() - prev)
                if delay > 0: time.sleep(delay)
            try:
                data, meta = fetch_with_retries(resolved_url, args.timeout, args.max_bytes, retries=2)
                last_host_time[host] = time.monotonic()
            except Exception as exc:
                # Jotter3 SSR pages can themselves be CDN-cached longer than the
                # one-hour S3 presign. A stale/invalid presign is observed as
                # HTTP 400 or 403, so refresh the public source page with a
                # cache-busting query and resolve one fresh origin URL.
                refreshed = False
                if rec.get("url_resolution") == "angular_transfer_state" and isinstance(exc, HTTPError) and exc.code in {400, 403}:
                    source_page = (row.get("source_page") or "").strip()
                    if source_page:
                        sep = "&" if "?" in source_page else "?"
                        fresh_row = dict(row)
                        fresh_row["source_page"] = f"{source_page}{sep}_pub_harvest={int(time.time())}"
                        try:
                            fresh_url, fresh_resolution, fresh_debug = resolve_candidate_url(fresh_row, args.timeout)
                            if fresh_url and fresh_url != resolved_url:
                                data, meta = fetch_with_retries(fresh_url, args.timeout, args.max_bytes, retries=1)
                                rec["resolved_url"] = fresh_url
                                rec["url_resolution"] = "angular_transfer_state_refresh"
                                rec["resolver_debug"] = fresh_debug
                                refreshed = True
                                print(f"[{idx}/{len(seed_rows)}] REFRESH {row.get('candidate_filename')}")
                        except Exception as refresh_exc:
                            rec["refresh_error"] = f"{type(refresh_exc).__name__}: {refresh_exc}"
                if not refreshed:
                    rec["fetch_status"] = "fetch_failed"; rec["error"] = f"{type(exc).__name__}: {exc}"
                    rows_out.append(rec); write_manifest(rows_out, args.out)
                    print(f"[{idx}/{len(seed_rows)}] FAIL {row.get('candidate_filename')}: {exc}")
                    continue

        rec.update(meta); rec["fetch_status"] = "ok"; rec["size_bytes"] = len(data)
        sha = hashlib.sha256(data).hexdigest(); rec["sha256"] = sha
        original = row.get("candidate_filename") or url_filename(meta.get("final_url", "")) or "candidate.pub"
        classification, hints = classify_harvest_payload(data, meta)
        if classification == "zip_mislabel" and original.casefold().endswith(".zip"):
            classification = "zip_container"
        rec["classification"] = classification; rec["publisher_hints"] = ";".join(hints)
        rec["bucket"] = bucket_for(classification, quarantine)

        duplicate_of = seen_sha.get(sha)
        if duplicate_of:
            rec["duplicate_of_sha256"] = sha; rec["duplicate_of_file"] = duplicate_of
            rows_out.append(rec); write_manifest(rows_out, args.out)
            print(f"[{idx}/{len(seed_rows)}] DUP  {row.get('candidate_filename')} -> {duplicate_of}")
            continue

        target = args.out / rec["bucket"] / f"s{args.shard_index:02d}_{idx:04d}__{safe_name(original)}"
        target.write_bytes(data)
        rec["stored_path"] = str(target.relative_to(args.out)); rec["file_mime"] = file_command(target)
        seen_sha[sha] = rec["stored_path"]
        rows_out.append(rec)

        archive_children: list[dict] = []
        if classification == "zip_container":
            archive_children = extract_pub_archive_members(
                data,
                rec,
                row,
                args.out,
                idx,
                args.shard_index,
                quarantine,
                seen_sha,
                args.max_bytes,
                args.max_archive_total_bytes,
                args.max_archive_members,
            )
            rows_out.extend(archive_children)

        write_manifest(rows_out, args.out)
        suffix = f" +{sum(x.get('fetch_status') == 'ok_archive_member' for x in archive_children)} PUB members" if archive_children else ""
        print(f"[{idx}/{len(seed_rows)}] OK   {original} {len(data)} bytes {classification} {sha[:12]}{suffix}")

    write_manifest(rows_out, args.out); append_step_summary(rows_out)
    fetched = sum(r.get("fetch_status") in {"ok", "ok_archive_member"} for r in rows_out)
    unique = sum(
        r.get("fetch_status") in {"ok", "ok_archive_member"} and not r.get("duplicate_of_sha256")
        for r in rows_out
    )
    print(f"harvest complete: rows={len(rows_out)} fetched={fetched} unique={unique}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
