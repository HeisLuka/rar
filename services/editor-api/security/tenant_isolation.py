#!/usr/bin/env python3
"""Tenant-isolation contract primitives for Chaptera cloud execution.

This module is intentionally storage-vendor neutral. It proves that handles,
jobs, caches, temporary scopes, grants, and audit metadata carry tenant context.
Actual bucket/IAM/container enforcement still requires deployment receipts.
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
POLICY = json.loads((HERE / "tenant-policy-v1.json").read_text(encoding="utf-8"))
IDENT_RE = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
HASH_RE = re.compile(r"^[0-9a-f]{64}$")
HANDLE_PREFIX = "ta1"
GRANT_PREFIX = "tg1"

FORBIDDEN_AUDIT_FIELDS = {
    "text",
    "document_text",
    "story_text",
    "raw_bytes",
    "raw_pub_bytes",
    "payload",
    "content",
    "source_path",
    "filesystem_path",
    "signed_url",
    "token",
    "authorization",
}


class TenantIsolationError(ValueError):
    pass


def _require_ident(value: str, label: str) -> str:
    if not isinstance(value, str) or not IDENT_RE.fullmatch(value):
        raise TenantIsolationError(f"invalid {label}")
    return value


def _require_hash(value: str, label: str) -> str:
    if not isinstance(value, str) or not HASH_RE.fullmatch(value):
        raise TenantIsolationError(f"invalid {label}")
    return value


def _canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _b64u(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def _b64u_decode(value: str) -> bytes:
    try:
        return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))
    except Exception as exc:
        raise TenantIsolationError("invalid opaque token encoding") from exc


def _sign(prefix: str, payload: dict, secret: bytes) -> str:
    encoded = _b64u(_canonical_json(payload))
    signature = _b64u(hmac.new(secret, f"{prefix}.{encoded}".encode("ascii"), hashlib.sha256).digest())
    return f"{prefix}.{encoded}.{signature}"


def _verify(token: str, prefix: str, secret: bytes) -> dict:
    try:
        actual_prefix, encoded, supplied_signature = token.split(".", 2)
    except ValueError as exc:
        raise TenantIsolationError("malformed opaque token") from exc
    if actual_prefix != prefix:
        raise TenantIsolationError("opaque token version mismatch")
    expected_signature = _b64u(
        hmac.new(secret, f"{prefix}.{encoded}".encode("ascii"), hashlib.sha256).digest()
    )
    if not hmac.compare_digest(expected_signature, supplied_signature):
        raise TenantIsolationError("opaque token signature mismatch")
    try:
        payload = json.loads(_b64u_decode(encoded).decode("utf-8"))
    except Exception as exc:
        raise TenantIsolationError("invalid opaque token payload") from exc
    return payload


def issue_artifact_handle(
    *,
    secret: bytes,
    tenant_id: str,
    artifact_id: str,
    artifact_kind: str,
    content_hash: str,
) -> str:
    _require_ident(tenant_id, "tenant_id")
    _require_ident(artifact_id, "artifact_id")
    _require_hash(content_hash, "content_hash")
    if artifact_kind not in POLICY["allowed_artifact_kinds"]:
        raise TenantIsolationError("artifact kind is not allowed")
    return _sign(
        HANDLE_PREFIX,
        {
            "v": POLICY["policy_version"],
            "tenant_id": tenant_id,
            "artifact_id": artifact_id,
            "artifact_kind": artifact_kind,
            "content_hash": content_hash,
            "permission": "read",
        },
        secret,
    )


def verify_artifact_handle(
    handle: str,
    *,
    secret: bytes,
    tenant_id: str,
    allowed_kinds: set[str] | None = None,
) -> dict:
    _require_ident(tenant_id, "tenant_id")
    payload = _verify(handle, HANDLE_PREFIX, secret)
    if payload.get("v") != POLICY["policy_version"]:
        raise TenantIsolationError("artifact handle policy mismatch")
    if payload.get("tenant_id") != tenant_id:
        raise TenantIsolationError("cross-tenant artifact handle")
    if payload.get("permission") != "read":
        raise TenantIsolationError("artifact handle is not read-only")
    kind = payload.get("artifact_kind")
    if kind not in POLICY["allowed_artifact_kinds"]:
        raise TenantIsolationError("artifact handle kind is invalid")
    if allowed_kinds is not None and kind not in allowed_kinds:
        raise TenantIsolationError("artifact handle kind is not permitted for this operation")
    _require_ident(payload.get("artifact_id"), "artifact_id")
    _require_hash(payload.get("content_hash"), "content_hash")
    return payload


def build_worker_job(
    *,
    secret: bytes,
    tenant_id: str,
    job_id: str,
    input_handles: list[str],
) -> dict:
    _require_ident(tenant_id, "tenant_id")
    _require_ident(job_id, "job_id")
    if len(input_handles) > POLICY["max_job_inputs"]:
        raise TenantIsolationError("too many job inputs")

    verified = [
        verify_artifact_handle(handle, secret=secret, tenant_id=tenant_id)
        for handle in input_handles
    ]
    if len({item["artifact_id"] for item in verified}) != len(verified):
        raise TenantIsolationError("duplicate job input artifact")

    namespace_seed = f"{POLICY['policy_version']}\0{tenant_id}\0{job_id}".encode("utf-8")
    namespace = hashlib.sha256(namespace_seed).hexdigest()
    return {
        "policy_version": POLICY["policy_version"],
        "tenant_id": tenant_id,
        "job_id": job_id,
        "inputs": list(input_handles),
        "input_permissions": "read-only",
        "output_namespace": "out_" + namespace,
        "temp_namespace": "tmp_" + namespace,
        "temp_cleanup_required": True,
    }


def authorize_worker_input(
    job: dict,
    handle: str,
    *,
    secret: bytes,
    tenant_id: str,
) -> dict:
    if job.get("tenant_id") != tenant_id:
        raise TenantIsolationError("job tenant mismatch")
    if handle not in job.get("inputs", []):
        raise TenantIsolationError("artifact handle was not granted to this job")
    return verify_artifact_handle(handle, secret=secret, tenant_id=tenant_id)


def issue_artifact_grant(
    *,
    secret: bytes,
    tenant_id: str,
    artifact_handle: str,
    now_epoch: int,
    ttl_seconds: int | None = None,
) -> str:
    artifact = verify_artifact_handle(
        artifact_handle,
        secret=secret,
        tenant_id=tenant_id,
    )
    if not isinstance(now_epoch, int) or isinstance(now_epoch, bool) or now_epoch < 0:
        raise TenantIsolationError("invalid grant issue time")
    max_ttl = POLICY["artifact_grant_ttl_seconds"]
    ttl = max_ttl if ttl_seconds is None else ttl_seconds
    if not isinstance(ttl, int) or isinstance(ttl, bool) or ttl <= 0 or ttl > max_ttl:
        raise TenantIsolationError("invalid artifact grant TTL")

    return _sign(
        GRANT_PREFIX,
        {
            "v": POLICY["policy_version"],
            "tenant_id": tenant_id,
            "artifact_id": artifact["artifact_id"],
            "artifact_kind": artifact["artifact_kind"],
            "content_hash": artifact["content_hash"],
            "permission": "read",
            "iat": now_epoch,
            "exp": now_epoch + ttl,
        },
        secret,
    )


def verify_artifact_grant(
    grant: str,
    *,
    secret: bytes,
    tenant_id: str,
    artifact_id: str,
    now_epoch: int,
) -> dict:
    _require_ident(tenant_id, "tenant_id")
    _require_ident(artifact_id, "artifact_id")
    payload = _verify(grant, GRANT_PREFIX, secret)
    if payload.get("v") != POLICY["policy_version"]:
        raise TenantIsolationError("artifact grant policy mismatch")
    if payload.get("tenant_id") != tenant_id:
        raise TenantIsolationError("cross-tenant artifact grant")
    if payload.get("artifact_id") != artifact_id:
        raise TenantIsolationError("artifact grant resource mismatch")
    if payload.get("permission") != "read":
        raise TenantIsolationError("artifact grant is not read-only")
    if not isinstance(now_epoch, int) or isinstance(now_epoch, bool) or now_epoch < 0:
        raise TenantIsolationError("invalid grant verification time")
    if not isinstance(payload.get("iat"), int) or payload["iat"] > now_epoch:
        raise TenantIsolationError("artifact grant issued in the future")
    if not isinstance(payload.get("exp"), int) or now_epoch >= payload["exp"]:
        raise TenantIsolationError("artifact grant expired")
    return payload


class TempScopeRegistry:
    """Process-local lifecycle model for tenant/job temp isolation.

    Deployment must map each namespace to an actually isolated disposable
    filesystem/container location and prove deletion independently.
    """

    def __init__(self) -> None:
        self._active: dict[str, str] = {}
        self._closed: set[str] = set()

    def open_job(self, *, tenant_id: str, job_id: str) -> str:
        _require_ident(tenant_id, "tenant_id")
        _require_ident(job_id, "job_id")
        key = f"{tenant_id}\0{job_id}"
        if key in self._active or key in self._closed:
            raise TenantIsolationError("job temp scope cannot be reused")
        namespace = "tmp_" + hashlib.sha256(
            f"{POLICY['policy_version']}\0{key}".encode("utf-8")
        ).hexdigest()
        self._active[key] = namespace
        return namespace

    def assert_access(self, *, tenant_id: str, job_id: str, namespace: str) -> None:
        key = f"{tenant_id}\0{job_id}"
        if self._active.get(key) != namespace:
            raise TenantIsolationError("temp namespace is not active for this tenant/job")

    def close_job(self, *, tenant_id: str, job_id: str) -> str:
        key = f"{tenant_id}\0{job_id}"
        namespace = self._active.pop(key, None)
        if namespace is None:
            raise TenantIsolationError("temp scope is not active")
        self._closed.add(key)
        return namespace


def tenant_job_cache_key(
    *,
    tenant_id: str,
    content_hash: str,
    variant: str,
) -> str:
    """Opaque tenant-scoped application cache identity.

    Physical content dedupe may happen below this layer, but the application
    cache key itself can never authorize cross-tenant reuse.
    """
    _require_ident(tenant_id, "tenant_id")
    _require_hash(content_hash, "content_hash")
    _require_ident(variant, "variant")
    payload = (
        f"{POLICY['policy_version']}\0{tenant_id}\0{content_hash}\0{variant}"
    ).encode("utf-8")
    return "tc1." + hashlib.sha256(payload).hexdigest()


def build_tenant_audit_event(
    *,
    tenant_id: str,
    action: str,
    result: str,
    metadata: dict,
) -> dict:
    _require_ident(tenant_id, "tenant_id")
    _require_ident(action, "action")
    _require_ident(result, "result")
    allowed = set(POLICY["audit_allowed_fields"])
    for key, value in metadata.items():
        if key in FORBIDDEN_AUDIT_FIELDS or key not in allowed:
            raise TenantIsolationError(f"audit field is not allowed: {key}")
        if isinstance(value, str):
            _require_ident(value, f"audit {key}")
        elif not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise TenantIsolationError(f"audit {key} must be a bounded token or non-negative integer")
    return {
        "policy_version": POLICY["policy_version"],
        "tenant_id": tenant_id,
        "action": action,
        "result": result,
        "metadata": dict(metadata),
    }
