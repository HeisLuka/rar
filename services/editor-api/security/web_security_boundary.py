#!/usr/bin/env python3
"""Public Web Editor security boundary primitives.

These helpers model source-neutral service contracts and fail-closed checks.
They do not parse PUB semantics and do not replace OS/container sandboxing.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
from pathlib import Path
import re
from typing import Any

HERE = Path(__file__).resolve().parent
PROFILE = json.loads((HERE / "web-security-profile-v1.json").read_text(encoding="utf-8"))
CFB_MAGIC = bytes.fromhex(PROFILE["upload"]["required_magic_hex"])
IDENT_RE = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
HASH_RE = re.compile(r"^[0-9a-f]{64}$")
FORBIDDEN_BROWSER_KEYS = {
    "raw_pub_bytes",
    "raw_bytes",
    "bytes",
    "source_path",
    "filesystem_path",
    "cfb_path",
    "stream_path",
    "stream_name",
    "parser_record",
    "carrier",
    "source_ref",
    "byte_range",
}
SENSITIVE_TELEMETRY_KEYS = {
    "text",
    "document_text",
    "story_text",
    "raw_pub_bytes",
    "raw_bytes",
    "source_path",
    "signed_url",
    "resource_token",
    "authorization",
    "cookie",
}


class SecurityBoundaryError(ValueError):
    pass


def _require_ident(value: str, label: str) -> str:
    if not isinstance(value, str) or not IDENT_RE.fullmatch(value):
        raise SecurityBoundaryError(f"invalid {label}")
    return value


def _require_hash(value: str, label: str) -> str:
    if not isinstance(value, str) or not HASH_RE.fullmatch(value):
        raise SecurityBoundaryError(f"invalid {label}")
    return value


def _b64u(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def _b64u_decode(value: str) -> bytes:
    padding = "=" * (-len(value) % 4)
    try:
        return base64.urlsafe_b64decode(value + padding)
    except Exception as exc:
        raise SecurityBoundaryError("invalid token encoding") from exc


def _canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def validate_upload_start(*, tenant_id: str, content_length: int, display_name: str | None = None) -> dict:
    _require_ident(tenant_id, "tenant_id")
    if not isinstance(content_length, int) or isinstance(content_length, bool) or content_length < 0:
        raise SecurityBoundaryError("invalid content length")
    if content_length > PROFILE["upload"]["max_content_length_bytes"]:
        raise SecurityBoundaryError("upload exceeds content-length limit")
    return {
        "policy_version": PROFILE["policy_version"],
        "tenant_id": tenant_id,
        "content_length": content_length,
        "display_name": display_name if isinstance(display_name, str) else None,
    }


def finalize_pub_upload(
    *,
    secret: bytes,
    tenant_id: str,
    content: bytes,
    declared_content_length: int,
    display_name: str | None = None,
) -> dict:
    validate_upload_start(
        tenant_id=tenant_id,
        content_length=declared_content_length,
        display_name=display_name,
    )
    if declared_content_length != len(content):
        raise SecurityBoundaryError("declared content length mismatch")
    if not content.startswith(CFB_MAGIC):
        raise SecurityBoundaryError("upload is not a supported CFB/PUB carrier")

    source_hash = hashlib.sha256(content).hexdigest()
    handle_payload = f"{tenant_id}\0{source_hash}".encode("utf-8")
    handle = "src_" + _b64u(hmac.new(secret, handle_payload, hashlib.sha256).digest()[:24])
    return {
        "policy_version": PROFILE["policy_version"],
        "tenant_id": tenant_id,
        "source_hash": source_hash,
        "source_blob_handle": handle,
        "content_length": len(content),
        "display_name": display_name if isinstance(display_name, str) else None,
        "immutable_source": True,
    }


def build_parse_job(upload_receipt: dict) -> dict:
    _require_ident(upload_receipt["tenant_id"], "tenant_id")
    _require_hash(upload_receipt["source_hash"], "source_hash")
    handle = upload_receipt.get("source_blob_handle")
    if not isinstance(handle, str) or not handle.startswith("src_"):
        raise SecurityBoundaryError("invalid source blob handle")

    limits = PROFILE["parse_worker"]
    return {
        "policy_version": PROFILE["policy_version"],
        "tenant_id": upload_receipt["tenant_id"],
        "source_hash": upload_receipt["source_hash"],
        "source_blob_handle": handle,
        "network_access": False,
        "temp_storage": limits["temp_storage"],
        "limits": {
            "wall_time_seconds": limits["wall_time_seconds"],
            "memory_bytes": limits["memory_bytes"],
            "max_output_bytes": limits["max_output_bytes"],
            "max_object_count": limits["max_object_count"],
            "max_stream_count": limits["max_stream_count"],
            "max_graph_depth": limits["max_graph_depth"],
        },
    }


def assert_browser_payload_source_neutral(value: Any, at: str = "$") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_BROWSER_KEYS:
                raise SecurityBoundaryError(f"forbidden browser payload key {key!r} at {at}")
            assert_browser_payload_source_neutral(child, f"{at}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            assert_browser_payload_source_neutral(child, f"{at}[{index}]")


def issue_resource_token(
    *,
    secret: bytes,
    tenant_id: str,
    document_id: str,
    resource_id: str,
    now_epoch: int,
    ttl_seconds: int | None = None,
) -> str:
    _require_ident(tenant_id, "tenant_id")
    _require_ident(document_id, "document_id")
    _require_ident(resource_id, "resource_id")
    if not isinstance(now_epoch, int) or isinstance(now_epoch, bool) or now_epoch < 0:
        raise SecurityBoundaryError("invalid token issue time")
    ttl = PROFILE["resource_delivery"]["token_ttl_seconds"] if ttl_seconds is None else ttl_seconds
    if not isinstance(ttl, int) or ttl <= 0 or ttl > PROFILE["resource_delivery"]["token_ttl_seconds"]:
        raise SecurityBoundaryError("resource token TTL exceeds policy")
    payload = {
        "v": "chaptera.resource-grant.v1",
        "tenant_id": tenant_id,
        "document_id": document_id,
        "resource_id": resource_id,
        "permission": "read",
        "iat": now_epoch,
        "exp": now_epoch + ttl,
    }
    encoded = _b64u(_canonical_json(payload))
    sig = _b64u(hmac.new(secret, encoded.encode("ascii"), hashlib.sha256).digest())
    return f"r1.{encoded}.{sig}"


def verify_resource_token(
    token: str,
    *,
    secret: bytes,
    tenant_id: str,
    document_id: str,
    resource_id: str,
    now_epoch: int,
) -> dict:
    try:
        prefix, encoded, supplied_sig = token.split(".", 2)
    except ValueError as exc:
        raise SecurityBoundaryError("malformed resource token") from exc
    if prefix != "r1":
        raise SecurityBoundaryError("unsupported resource token version")

    expected_sig = _b64u(hmac.new(secret, encoded.encode("ascii"), hashlib.sha256).digest())
    if not hmac.compare_digest(expected_sig, supplied_sig):
        raise SecurityBoundaryError("invalid resource token signature")

    try:
        payload = json.loads(_b64u_decode(encoded).decode("utf-8"))
    except Exception as exc:
        raise SecurityBoundaryError("invalid resource token payload") from exc

    expected = {
        "tenant_id": tenant_id,
        "document_id": document_id,
        "resource_id": resource_id,
        "permission": "read",
    }
    for key, value in expected.items():
        if payload.get(key) != value:
            raise SecurityBoundaryError(f"resource token {key} mismatch")
    if payload.get("v") != "chaptera.resource-grant.v1":
        raise SecurityBoundaryError("resource token payload version mismatch")
    if not isinstance(payload.get("exp"), int) or now_epoch >= payload["exp"]:
        raise SecurityBoundaryError("resource token expired")
    if not isinstance(payload.get("iat"), int) or payload["iat"] > now_epoch:
        raise SecurityBoundaryError("resource token issued in the future")
    return payload


def tenant_cache_key(*, tenant_id: str, content_hash: str, variant: str) -> str:
    _require_ident(tenant_id, "tenant_id")
    _require_hash(content_hash, "content_hash")
    _require_ident(variant, "variant")
    digest = hashlib.sha256(
        f"chaptera.cache.v1\0{tenant_id}\0{content_hash}\0{variant}".encode("utf-8")
    ).hexdigest()
    return "cache_" + digest


def authorize_external_fetch(url: str) -> None:
    # V0 deliberately has no document-controlled network fetch path.
    if not isinstance(url, str) or not url:
        raise SecurityBoundaryError("invalid external URL")
    raise SecurityBoundaryError("document-controlled external fetch is disabled by policy")


def build_telemetry_event(*, code: str, metadata: dict) -> dict:
    _require_ident(code, "telemetry code")
    allowed = set(PROFILE["telemetry"]["allowed_metadata_keys"])
    string_keys = {"stage", "status", "error_code"}
    integer_keys = {"duration_ms", "input_bytes", "output_bytes", "object_count", "page_count"}
    for key, value in metadata.items():
        if key in SENSITIVE_TELEMETRY_KEYS or key not in allowed:
            raise SecurityBoundaryError(f"telemetry metadata key is not allowed: {key}")
        if key in string_keys:
            if not isinstance(value, str) or not IDENT_RE.fullmatch(value):
                raise SecurityBoundaryError(f"telemetry {key} must be a bounded structural token")
        elif key in integer_keys:
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                raise SecurityBoundaryError(f"telemetry {key} must be a non-negative integer")
        else:
            raise SecurityBoundaryError(f"telemetry metadata key has no value policy: {key}")
    return {
        "policy_version": PROFILE["policy_version"],
        "code": code,
        "metadata": dict(metadata),
    }
