#!/usr/bin/env python3
"""WEB-SECURITY-01 integration seam over real Rar security primitives.

This module does not invent a second parser/sanitizer/decoder. It composes:
- the proven active-content sanitizer ported from CLOUD-SANITIZE-01;
- tenant/document authorization + tenant-bound artifact grants;
- the real Linux per-file isolation harness from rar#143;
- the source-neutral browser payload fence from the migrated web boundary.

Production chaptera-server routing remains separate and must consume this policy.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass
import pathlib
import sys
from typing import Any, Literal

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from migration_pdf_worker_isolation import WorkerLimits, run_isolated_worker

try:
    from .authz_v1 import AuthzKernel, CAP_VIEW
    from .sanitize_active_content import SanitizationError, sanitize_html, sanitize_svg
    from .tenant_isolation import verify_artifact_grant
    from .web_security_boundary import (
        SecurityBoundaryError,
        assert_browser_payload_source_neutral,
        authorize_external_fetch,
    )
except ImportError:
    from authz_v1 import AuthzKernel, CAP_VIEW
    from sanitize_active_content import SanitizationError, sanitize_html, sanitize_svg
    from tenant_isolation import verify_artifact_grant
    from web_security_boundary import (
        SecurityBoundaryError,
        assert_browser_payload_source_neutral,
        authorize_external_fetch,
    )


class WebSecurityIntegrationError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class BrowserActiveContentV1:
    kind: Literal["svg", "html"]
    body: bytes
    receipt: dict


@dataclass(frozen=True)
class IsolatedParseReceiptV1:
    protocol_version: Literal["chaptera.web-isolated-parse-receipt.v1"]
    status: str
    network_policy: str
    timed_out: bool
    outputs: tuple[str, ...]
    staging_cleaned: bool
    limits: dict


CSP_V1 = (
    "default-src 'self'; "
    "script-src 'self'; "
    "style-src 'self'; "
    "img-src 'self' blob:; "
    "font-src 'self'; "
    "connect-src 'self'; "
    "object-src 'none'; "
    "frame-src 'none'; "
    "base-uri 'none'; "
    "frame-ancestors 'none'; "
    "form-action 'none'"
)


def browser_security_headers_v1() -> dict[str, str]:
    return {
        "content-security-policy": CSP_V1,
        "x-content-type-options": "nosniff",
        "referrer-policy": "no-referrer",
        "cross-origin-resource-policy": "same-origin",
        "permissions-policy": (
            "camera=(), microphone=(), geolocation=(), payment=(), "
            "usb=(), serial=(), bluetooth=()"
        ),
    }


def guard_browser_payload_v1(value: Any) -> None:
    try:
        assert_browser_payload_source_neutral(value)
    except SecurityBoundaryError as exc:
        raise WebSecurityIntegrationError("browser_payload_rejected", str(exc)) from exc


def sanitize_browser_active_content_v1(
    *,
    kind: Literal["svg", "html"],
    body: bytes,
) -> BrowserActiveContentV1:
    try:
        sanitized, receipt = (
            sanitize_svg(body) if kind == "svg" else sanitize_html(body)
        )
    except SanitizationError as exc:
        raise WebSecurityIntegrationError("active_content_rejected", str(exc)) from exc
    if receipt.get("network_fetch_allowed") is not False:
        raise WebSecurityIntegrationError(
            "sanitizer_policy_mismatch",
            "active-content sanitizer must deny document-controlled network fetch",
        )
    if receipt.get("active_content_allowed") is not False:
        raise WebSecurityIntegrationError(
            "sanitizer_policy_mismatch",
            "active-content sanitizer must deny executable active content",
        )
    return BrowserActiveContentV1(kind=kind, body=sanitized, receipt=receipt)


def assert_external_fetch_denied_v1(url: str) -> None:
    try:
        authorize_external_fetch(url)
    except SecurityBoundaryError:
        return
    raise WebSecurityIntegrationError(
        "external_fetch_policy_bypass",
        "document-controlled external fetch unexpectedly admitted",
    )


def authorize_browser_resource_v1(
    *,
    authz: AuthzKernel,
    tenant_secret: bytes,
    tenant_id: str,
    document_id: str,
    principal_id: str,
    artifact_grant: str,
    artifact_id: str,
    now_epoch: int,
) -> dict:
    """Require both document view authorization and tenant-bound resource grant."""
    authz.authorize(
        tenant_id=tenant_id,
        document_id=document_id,
        principal_id=principal_id,
        capability=CAP_VIEW,
    )
    return verify_artifact_grant(
        artifact_grant,
        secret=tenant_secret,
        tenant_id=tenant_id,
        artifact_id=artifact_id,
        now_epoch=now_epoch,
    )


def run_isolated_parse_worker_v1(
    *,
    command: list[str],
    input_path: pathlib.Path,
    final_output_dir: pathlib.Path,
    timeout_seconds: float = 10.0,
    limits: WorkerLimits | None = None,
) -> IsolatedParseReceiptV1:
    """Run one parse-like worker through the actual Linux no-network harness."""
    if not isinstance(command, list) or not command or any(
        not isinstance(part, str) or not part for part in command
    ):
        raise WebSecurityIntegrationError("invalid_parse_command", "non-empty argv is required")
    if timeout_seconds <= 0 or timeout_seconds > 120:
        raise WebSecurityIntegrationError(
            "invalid_parse_timeout",
            "parse wall timeout must be >0 and <=120 seconds",
        )
    chosen = limits or WorkerLimits(
        address_space_bytes=512 * 1024 * 1024,
        cpu_seconds=10,
        open_files=64,
        output_file_bytes=32 * 1024 * 1024,
    )
    result = run_isolated_worker(
        command,
        final_output_dir=final_output_dir,
        timeout_seconds=timeout_seconds,
        limits=chosen,
        input_path=input_path,
    )
    receipt = IsolatedParseReceiptV1(
        protocol_version="chaptera.web-isolated-parse-receipt.v1",
        status=result.status,
        network_policy=result.network_policy,
        timed_out=result.timed_out,
        outputs=result.outputs,
        staging_cleaned=result.staging_cleaned,
        limits=asdict(result.limits),
    )
    if receipt.network_policy != "seccomp_default_deny":
        raise WebSecurityIntegrationError(
            "parse_network_policy_missing",
            "parse worker did not execute under the required no-network policy",
        )
    return receipt
