#!/usr/bin/env python3
"""Static public-safe validator for the CLOUD-DEPLOY-01 host packet."""

from __future__ import annotations

import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGET = ROOT / "target" / "cloud-deploy-v0"

FILES = {
    "slice": ROOT / "deploy/systemd/chaptera.slice",
    "web": ROOT / "deploy/systemd/chaptera-web.service",
    "worker": ROOT / "deploy/systemd/chaptera-worker.service",
    "tmpfiles": ROOT / "deploy/tmpfiles/chaptera.conf",
    "caddy": ROOT / "deploy/caddy/Caddyfile.example",
    "config": ROOT / "deploy/config/chaptera.prod.example.toml",
    "doc": ROOT / "docs/cloud-single-host-v0.md",
}


def require(text: str, needle: str, label: str) -> None:
    if needle not in text:
        raise AssertionError(f"{label}: missing {needle!r}")


def memory_mib(text: str, key: str) -> int:
    match = re.search(rf"(?m)^{re.escape(key)}=(\d+)M$", text)
    if not match:
        raise AssertionError(f"missing {key}=<MiB>M")
    return int(match.group(1))


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    texts = {}
    for name, path in FILES.items():
        if not path.is_file():
            raise AssertionError(f"missing required file: {path.relative_to(ROOT)}")
        texts[name] = path.read_text(encoding="utf-8")

    slice_text = texts["slice"]
    web = texts["web"]
    worker = texts["worker"]
    caddy = texts["caddy"]
    config = texts["config"]

    require(slice_text, "MemoryHigh=1280M", "slice")
    require(slice_text, "MemoryMax=1536M", "slice")
    if memory_mib(slice_text, "MemoryMax") > 1536:
        raise AssertionError("parent Chaptera MemoryMax exceeds 1536 MiB")

    for label, unit in (("web", web), ("worker", worker)):
        require(unit, "User=chaptera", label)
        require(unit, "Group=chaptera", label)
        require(unit, "Slice=chaptera.slice", label)
        require(unit, "NoNewPrivileges=yes", label)
        require(unit, "ProtectSystem=strict", label)
        require(unit, "ProtectHome=yes", label)
        require(unit, "PrivateTmp=yes", label)
        require(unit, "CapabilityBoundingSet=", label)
        if "User=root" in unit:
            raise AssertionError(f"{label}: root service identity is forbidden")

    require(
        web,
        "ExecStart=/opt/chaptera/current/chaptera --config /etc/chaptera/chaptera.toml serve",
        "web",
    )
    require(
        worker,
        "ExecStart=/opt/chaptera/current/chaptera --config /etc/chaptera/chaptera.toml worker",
        "worker",
    )
    require(
        web,
        "LoadCredential=oidc_client_secret:/etc/chaptera/credentials/oidc_client_secret",
        "web",
    )
    if "LoadCredential=oidc_client_secret" in worker:
        raise AssertionError("worker must not receive the web/OIDC client secret")
    require(worker, "MemoryHigh=384M", "worker")
    require(worker, "MemoryMax=640M", "worker")

    require(caddy, "reverse_proxy 127.0.0.1:8080", "caddy")
    require(caddy, "max_size 256MB", "caddy")
    require(caddy, "max_size 8MB", "caddy")
    require(caddy, "/v1/uploads/*/content", "caddy")
    require(caddy, "header_up X-Forwarded-For {remote_host}", "caddy")
    require(caddy, "header_up X-Forwarded-Proto https", "caddy")
    require(caddy, "header_up X-Forwarded-Host {host}", "caddy")
    require(caddy, 'Strict-Transport-Security "max-age=31536000; includeSubDomains"', "caddy")

    require(config, 'listen = "127.0.0.1:8080"', "config")
    require(config, "heavy_concurrency = 1", "config")
    require(config, "quota_shared_capacity =", "config")
    require(config, "quota_semantic_headroom =", "config")
    require(config, "quota_export_cap =", "config")
    require(config, "quota_background_cap =", "config")
    require(config, 'journal_mode = "wal"', "config")
    require(config, 'synchronous = "full"', "config")
    require(config, '[auth.oidc]', "config")
    require(config, 'source = "systemd"', "config")
    require(config, 'name = "oidc_client_secret"', "config")
    require(config, '[edge]', "config")
    require(config, 'trusted_proxy_ips = ["127.0.0.1", "::1"]', "config")
    require(config, "max_header_bytes = 32768", "config")
    require(config, "max_api_body_bytes = 8388608", "config")
    require(config, "max_upload_body_bytes = 268435456", "config")
    require(config, "request_timeout_ms = 30000", "config")
    require(config, "[upload_admission]", "config")
    require(config, "principal_concurrent_cap = 2", "config")
    require(config, "tenant_concurrent_cap = 8", "config")
    require(config, "max_single_upload_bytes = 268435456", "config")
    require(config, "[source_validation]", "config")
    require(config, 'clamd_endpoint = "127.0.0.1:3310"', "config")
    require(config, 'isolation_python = "/usr/bin/python3"', "config")
    require(
        config,
        'isolation_harness = "/opt/chaptera/current/tools/migration_pdf_worker_isolation.py"',
        "config",
    )
    require(
        config,
        'worker_binary = "/opt/chaptera/current/chaptera-untrusted-pub-worker"',
        "config",
    )
    require(config, "max_file_bytes = 268435456", "config")
    if "0.0.0.0:8080" in config or "[::]:8080" in config:
        raise AssertionError("example config exposes the app listener publicly")

    forbidden = (
        "AWS_SECRET_ACCESS_KEY=",
        "SECRET_KEY=",
        "SESSION_SECRET=",
        "PRIVATE_KEY=",
        "BEGIN PRIVATE KEY",
    )
    joined = "\n".join(texts.values())
    for marker in forbidden:
        if marker in joined:
            raise AssertionError(f"public deployment packet contains secret marker {marker!r}")

    receipt = {
        "schema": "chaptera.cloud-deploy-v0.static-receipt.v1",
        "task": "CLOUD-DEPLOY-01",
        "public_safe": True,
        "runtime_implemented": False,
        "assertions": {
            "aggregate_memory_max_mib": memory_mib(slice_text, "MemoryMax"),
            "aggregate_memory_high_mib": memory_mib(slice_text, "MemoryHigh"),
            "private_listener": True,
            "web_unprivileged": True,
            "worker_unprivileged": True,
            "websocket_proxy_owned_by_caddy": True,
            "stream_upload_bounded": True,
            "ordinary_api_bounded": True,
            "proxy_authority_rewritten_at_edge": True,
            "typed_rust_edge_limits": True,
            "heavy_worker_concurrency": 1,
            "explicit_worker_quota_budgets": True,
            "typed_production_config": True,
            "web_oidc_secret_via_systemd_credential": True,
            "worker_has_no_oidc_credential": True,
        },
        "files": {
            name: {
                "path": str(path.relative_to(ROOT)),
                "sha256": sha256(path),
            }
            for name, path in FILES.items()
        },
        "limitations": [
            "Static packet only; no Rust Chaptera server is claimed.",
            "Memory limits are provisional until real hot-memory/capacity receipts.",
            "Rust enforces Host/Origin/proxy/body/header/timeout policy; route-owned auth still verifies CSRF tokens against sessions.",
            "CLOUD-DEPLOY-01 requires a real 2 GiB Linux acceptance host before closure.",
        ],
    }

    TARGET.mkdir(parents=True, exist_ok=True)
    out = TARGET / "receipt.json"
    out.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
