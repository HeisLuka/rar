#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import mimetypes
import pathlib
import re
import socket
import sys
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from dataclasses import dataclass
from datetime import datetime, timezone
from io import BytesIO

MANIFEST_VERSION = "chaptera.hosted-public-acquisition.v1"
RECEIPT_VERSION = "chaptera.hosted-public-acquisition-receipt.v1"
PUB_RECEIPT_VERSION = "chaptera.pub-research-receipt.v1"
TASK_KEY_RE = re.compile(r"^[A-Z0-9][A-Z0-9._-]{2,127}$")
PUB_TASK_RE = re.compile(r"^PUB-[A-Z0-9-]{2,64}$")
RUN_RE = re.compile(r"^PUB-RUN-[0-9]{1,8}$")
SOURCE_ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
SENSITIVE_QUERY_KEY_RE = re.compile(
    r"(?:token|secret|password|passwd|credential|authorization|auth|signature|sig|api[_-]?key|access[_-]?key|x-amz-)",
    re.IGNORECASE,
)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def fail(message: str) -> "NoReturn":
    raise ValueError(message)


def validate_public_url(url: str, *, allow_local_http: bool = False) -> urllib.parse.ParseResult:
    parsed = urllib.parse.urlparse(url)
    allowed = {"https"}
    if allow_local_http:
        allowed.add("http")
    if parsed.scheme not in allowed:
        fail(f"unsupported URL scheme: {parsed.scheme!r}")
    if parsed.username or parsed.password:
        fail("credentials in URL are forbidden")
    if not parsed.hostname:
        fail("URL hostname is required")
    for key, _value in urllib.parse.parse_qsl(parsed.query, keep_blank_values=True):
        if SENSITIVE_QUERY_KEY_RE.search(key):
            fail(f"credential-like URL query parameter is forbidden: {key}")
    if allow_local_http and parsed.hostname in {"127.0.0.1", "localhost", "::1"}:
        return parsed
    try:
        infos = socket.getaddrinfo(parsed.hostname, parsed.port or 443, type=socket.SOCK_STREAM)
    except socket.gaierror as exc:
        fail(f"cannot resolve host {parsed.hostname}: {exc}")
    addresses = {info[4][0] for info in infos}
    if not addresses:
        fail(f"host {parsed.hostname} resolved to no addresses")
    for address in addresses:
        ip = ipaddress.ip_address(address.split("%", 1)[0])
        if not ip.is_global:
            fail(f"non-public destination forbidden: {address}")
    return parsed


class SafeRedirectHandler(urllib.request.HTTPRedirectHandler):
    def __init__(self, redirect_chain: list[str], allow_local_http: bool):
        super().__init__()
        self.redirect_chain = redirect_chain
        self.allow_local_http = allow_local_http

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        validate_public_url(newurl, allow_local_http=self.allow_local_http)
        self.redirect_chain.append(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def sniff_magic(data: bytes) -> str:
    if data.startswith(b"PK\x03\x04"):
        return "zip"
    if data.startswith(b"MZ"):
        return "pe"
    if data.startswith(bytes.fromhex("d0cf11e0a1b11ae1")):
        return "cfb"
    if data.startswith(b"%PDF-"):
        return "pdf"
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png"
    if data.startswith(b"\xff\xd8\xff"):
        return "jpeg"
    prefix = data[:256].lstrip().lower()
    if prefix.startswith((b"<!doctype html", b"<html")):
        return "html"
    if prefix.startswith((b"{", b"[")):
        return "json-or-text"
    return "unknown"


@dataclass
class FetchResult:
    body: bytes
    final_url: str
    redirect_chain: list[str]
    status: int
    content_type: str


def fetch_url(
    url: str,
    *,
    max_bytes: int,
    timeout_seconds: int,
    allow_local_http: bool = False,
) -> FetchResult:
    validate_public_url(url, allow_local_http=allow_local_http)
    redirects: list[str] = []
    opener = urllib.request.build_opener(SafeRedirectHandler(redirects, allow_local_http))
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": "ChapteraHostedResearch/1.0 (+public acquisition receipt)",
            "Accept": "*/*",
        },
        method="GET",
    )
    with opener.open(request, timeout=timeout_seconds) as response:
        final_url = response.geturl()
        validate_public_url(final_url, allow_local_http=allow_local_http)
        raw_len = response.headers.get("Content-Length")
        if raw_len:
            try:
                if int(raw_len) > max_bytes:
                    fail(f"Content-Length {raw_len} exceeds max_bytes {max_bytes}")
            except ValueError:
                pass
        body = response.read(max_bytes + 1)
        if len(body) > max_bytes:
            fail(f"response exceeded max_bytes {max_bytes}")
        return FetchResult(
            body=body,
            final_url=final_url,
            redirect_chain=redirects,
            status=int(response.getcode() or 0),
            content_type=(response.headers.get_content_type() or "application/octet-stream"),
        )


def build_cdx_url(source: dict) -> str:
    target = source.get("target_url")
    if not isinstance(target, str) or not target:
        fail("wayback_cdx source requires target_url")
    limit = int(source.get("limit", 50))
    if not (1 <= limit <= 500):
        fail("wayback_cdx limit must be 1..500")
    params: list[tuple[str, str]] = [
        ("url", target),
        ("output", "json"),
        ("fl", "timestamp,original,statuscode,mimetype,digest"),
        ("filter", "statuscode:200"),
        ("limit", str(limit)),
    ]
    for key in ("from", "to"):
        if source.get(key) is not None:
            value = str(source[key])
            if not re.fullmatch(r"[0-9]{4,14}", value):
                fail(f"wayback_cdx {key} must be YYYY..YYYYMMDDhhmmss digits")
            params.append((key, value))
    return "https://web.archive.org/cdx/search/cdx?" + urllib.parse.urlencode(params)


def inspect_zip(data: bytes, *, max_members: int, max_uncompressed_bytes: int) -> list[dict]:
    if max_members < 1 or max_uncompressed_bytes < 1:
        fail("zip limits must be positive")
    members: list[dict] = []
    total = 0
    with zipfile.ZipFile(BytesIO(data)) as archive:
        infos = archive.infolist()
        if len(infos) > max_members:
            fail(f"zip member count {len(infos)} exceeds max_members {max_members}")
        for info in infos:
            path = pathlib.PurePosixPath(info.filename)
            if path.is_absolute() or ".." in path.parts:
                fail(f"unsafe zip member path: {info.filename}")
            if info.is_dir():
                continue
            total += int(info.file_size)
            if total > max_uncompressed_bytes:
                fail("zip uncompressed byte ceiling exceeded")
            payload = archive.read(info)
            if len(payload) != info.file_size:
                fail(f"zip member size mismatch: {info.filename}")
            members.append(
                {
                    "path": info.filename,
                    "size": len(payload),
                    "sha256": sha256_bytes(payload),
                    "magic": sniff_magic(payload),
                }
            )
    return members


def validate_manifest(doc: dict) -> dict:
    if not isinstance(doc, dict):
        fail("manifest root must be object")
    if doc.get("manifest_version") != MANIFEST_VERSION:
        fail("wrong manifest_version")
    task_key = doc.get("task_key")
    pub_task_id = doc.get("pub_task_id")
    run_id = doc.get("run_id")
    scope = doc.get("scope")
    sources = doc.get("sources")
    if not isinstance(task_key, str) or not TASK_KEY_RE.fullmatch(task_key):
        fail("task_key must be a bounded ASCII task key")
    if pub_task_id is not None and (
        not isinstance(pub_task_id, str) or not PUB_TASK_RE.fullmatch(pub_task_id)
    ):
        fail("pub_task_id must match PUB-[A-Z0-9-]+ when supplied")
    if not isinstance(run_id, str) or not RUN_RE.fullmatch(run_id):
        fail("run_id must match PUB-RUN-<1..8 digits>")
    if not isinstance(scope, str) or not (3 <= len(scope) <= 1000):
        fail("scope must be a bounded string")
    if not isinstance(sources, list) or not (1 <= len(sources) <= 100):
        fail("sources must contain 1..100 entries")
    seen: set[str] = set()
    for source in sources:
        if not isinstance(source, dict):
            fail("source must be object")
        sid = source.get("id")
        if not isinstance(sid, str) or not SOURCE_ID_RE.fullmatch(sid):
            fail(f"unsafe source id: {sid!r}")
        if sid in seen:
            fail(f"duplicate source id: {sid}")
        seen.add(sid)
        kind = source.get("kind")
        if kind not in {"https", "wayback_cdx"}:
            fail(f"unsupported source kind: {kind!r}")
        if kind == "https" and not isinstance(source.get("url"), str):
            fail(f"https source {sid} requires url")
        if "expected_sha256" in source and source["expected_sha256"] is not None:
            if not isinstance(source["expected_sha256"], str) or not SHA_RE.fullmatch(source["expected_sha256"]):
                fail(f"invalid expected_sha256 for {sid}")
        max_bytes = int(source.get("max_bytes", 10_000_000))
        if not (1 <= max_bytes <= 1_000_000_000):
            fail(f"max_bytes out of bounds for {sid}")
    return doc


def acquire_source(source: dict, *, allow_local_http: bool = False) -> dict:
    sid = source["id"]
    kind = source["kind"]
    max_bytes = int(source.get("max_bytes", 10_000_000))
    timeout_seconds = int(source.get("timeout_seconds", 60))
    if not (1 <= timeout_seconds <= 1800):
        fail(f"timeout_seconds out of bounds for {sid}")
    url = source["url"] if kind == "https" else build_cdx_url(source)
    base = {
        "id": sid,
        "kind": kind,
        "requested_url": url,
        "required": bool(source.get("required", True)),
    }
    try:
        fetched = fetch_url(
            url,
            max_bytes=max_bytes,
            timeout_seconds=timeout_seconds,
            allow_local_http=allow_local_http,
        )
        digest = sha256_bytes(fetched.body)
        expected = source.get("expected_sha256")
        if expected and digest != expected:
            fail(f"sha256 mismatch for {sid}: expected {expected}, got {digest}")
        record = {
            **base,
            "result": "ok",
            "fetched_at": utc_now(),
            "http_status": fetched.status,
            "final_url": fetched.final_url,
            "redirect_chain": fetched.redirect_chain,
            "size": len(fetched.body),
            "sha256": digest,
            "content_type": fetched.content_type,
            "magic": sniff_magic(fetched.body),
        }
        if source.get("inspect_zip"):
            if record["magic"] != "zip":
                fail(f"inspect_zip requested but {sid} is not ZIP")
            record["archive_members"] = inspect_zip(
                fetched.body,
                max_members=int(source.get("zip_max_members", 5000)),
                max_uncompressed_bytes=int(source.get("zip_max_uncompressed_bytes", 500_000_000)),
            )
        if source.get("retain_body"):
            if source.get("disclosure_safe") is not True:
                fail(f"retain_body for {sid} requires disclosure_safe=true")
            record["_body"] = fetched.body
        return record
    except (ValueError, urllib.error.URLError, urllib.error.HTTPError, TimeoutError, OSError, zipfile.BadZipFile) as exc:
        return {
            **base,
            "result": "error",
            "fetched_at": utc_now(),
            "error_type": type(exc).__name__,
            "error": str(exc)[:1000],
        }


def canonical_receipt(manifest: dict, manifest_bytes: bytes, acquisition_bytes: bytes, records: list[dict]) -> dict:
    ok = sum(1 for record in records if record["result"] == "ok")
    failed = len(records) - ok
    limitations = list(manifest.get("limitations") or [])
    if not limitations:
        limitations = ["Acquisition/provenance evidence only; no format-semantic promotion."]
    return {
        "receipt_version": PUB_RECEIPT_VERSION,
        "task_id": manifest["pub_task_id"],
        "run_id": manifest["run_id"],
        "scope": manifest["scope"],
        "inputs": [{"kind": "acquisition_manifest", "sha256": sha256_bytes(manifest_bytes)}],
        "outputs": [{"kind": "acquisition_receipt", "sha256": sha256_bytes(acquisition_bytes)}],
        "counts": {
            "observations": len(records),
            "supporting": ok,
            "counterexamples": failed,
        },
        "canonical_impact": "none",
        "limitations": limitations,
        "privacy": {
            "pub_bytes_in_receipt": False,
            "document_text_in_receipt": False,
            "local_path_in_receipt": False,
            "credentials_in_receipt": False,
        },
    }


def run_manifest(
    manifest_path: pathlib.Path,
    output_dir: pathlib.Path,
    *,
    allow_local_http: bool = False,
) -> int:
    manifest_bytes = manifest_path.read_bytes()
    manifest = validate_manifest(json.loads(manifest_bytes.decode("utf-8-sig")))
    output_dir.mkdir(parents=True, exist_ok=True)
    payload_dir = output_dir / "payloads"
    records = [acquire_source(source, allow_local_http=allow_local_http) for source in manifest["sources"]]
    for record in records:
        body = record.pop("_body", None)
        if body is not None:
            payload_dir.mkdir(parents=True, exist_ok=True)
            (payload_dir / f"{record['id']}.bin").write_bytes(body)
    acquisition = {
        "receipt_version": RECEIPT_VERSION,
        "manifest_version": MANIFEST_VERSION,
        "task_key": manifest["task_key"],
        "pub_task_id": manifest.get("pub_task_id"),
        "run_id": manifest["run_id"],
        "generated_at": utc_now(),
        "scope": manifest["scope"],
        "sources": records,
    }
    acquisition_bytes = (json.dumps(acquisition, indent=2, ensure_ascii=False, sort_keys=True) + "\n").encode("utf-8")
    (output_dir / "acquisition-receipt.json").write_bytes(acquisition_bytes)
    if manifest.get("pub_task_id"):
        canonical = canonical_receipt(manifest, manifest_bytes, acquisition_bytes, records)
        (output_dir / "pub-research-receipt.json").write_text(
            json.dumps(canonical, indent=2, ensure_ascii=False, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    lines = [
        f"# Hosted public acquisition — {manifest['task_key']}",
        "",
        f"- run: {manifest['run_id']}",
        f"- sources: {len(records)}",
        f"- ok: {sum(r['result'] == 'ok' for r in records)}",
        f"- error: {sum(r['result'] == 'error' for r in records)}",
        "",
    ]
    for record in records:
        suffix = record.get("sha256", record.get("error", ""))
        lines.append(f"- {record['id']}: {record['result']} {suffix}")
    (output_dir / "SUMMARY.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    required_failures = [r for r in records if r["required"] and r["result"] != "ok"]
    return 2 if required_failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Bounded public acquisition + provenance receipt runner")
    parser.add_argument("--manifest", type=pathlib.Path, required=True)
    parser.add_argument("--output-dir", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        return run_manifest(args.manifest, args.output_dir)
    except (ValueError, json.JSONDecodeError, OSError) as exc:
        print(f"hosted research runner failed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
