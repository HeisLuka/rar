#!/usr/bin/env python3
"""End-to-end CLOUD-EDGE-01 receipt using the production-shaped Caddy config."""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.server
import json
import pathlib
import socket
import ssl
import subprocess
import threading
import time
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGET = ROOT / "target" / "cloud-edge-01"
SOURCE_CADDY = ROOT / "deploy" / "caddy" / "Caddyfile.example"
SOURCE_CONFIG = ROOT / "deploy" / "config" / "chaptera.prod.example.toml"

EDGE_HOST = "edge.test"
HTTP_PORT = 18080
HTTPS_PORT = 18443
UPSTREAM_PORT = 18081
CONTAINER_NAME = "chaptera-cloud-edge-01"
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
WS_KEY = "dGhlIHNhbXBsZSBub25jZQ=="


class FixtureServer(http.server.ThreadingHTTPServer):
    daemon_threads = True


class FixtureHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "chaptera-edge-fixture"
    sys_version = ""

    def log_message(self, _format: str, *_args: Any) -> None:
        return

    def _json_response(self, body_bytes: int = 0) -> None:
        payload = {
            "method": self.command,
            "path": self.path,
            "host": self.headers.get("Host"),
            "x_forwarded_for": self.headers.get("X-Forwarded-For"),
            "x_forwarded_host": self.headers.get("X-Forwarded-Host"),
            "x_forwarded_proto": self.headers.get("X-Forwarded-Proto"),
            "body_bytes": body_bytes,
        }
        raw = (json.dumps(payload, sort_keys=True) + "\n").encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(raw)

    def _websocket_upgrade(self) -> bool:
        if self.headers.get("Upgrade", "").lower() != "websocket":
            return False
        key = self.headers.get("Sec-WebSocket-Key")
        if not key:
            self.send_error(400, "missing websocket key")
            return True
        accept = base64.b64encode(
            hashlib.sha1((key + WS_GUID).encode("ascii")).digest()
        ).decode("ascii")
        self.send_response(101, "Switching Protocols")
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        self.wfile.flush()
        self.close_connection = True
        return True

    def do_GET(self) -> None:
        if self._websocket_upgrade():
            return
        self._json_response()

    def do_POST(self) -> None:
        remaining = int(self.headers.get("Content-Length", "0"))
        observed = 0
        while remaining:
            chunk = self.rfile.read(min(64 * 1024, remaining))
            if not chunk:
                break
            observed += len(chunk)
            remaining -= len(chunk)
        self._json_response(observed)


def run(command: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(
            f"command failed rc={completed.returncode}: {' '.join(command)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render_caddy() -> pathlib.Path:
    source = SOURCE_CADDY.read_text(encoding="utf-8")
    for marker in ("cloud.example.invalid", "127.0.0.1:8080", "reverse_proxy"):
        if marker not in source:
            raise AssertionError(f"production Caddyfile missing marker {marker!r}")

    rendered = source.replace("cloud.example.invalid", EDGE_HOST)
    rendered = rendered.replace(
        f"{EDGE_HOST} {{",
        f"{EDGE_HOST} {{\n    tls internal",
        1,
    )
    rendered = rendered.replace(
        "127.0.0.1:8080",
        f"127.0.0.1:{UPSTREAM_PORT}",
        1,
    )
    rendered = (
        "{\n"
        f"    http_port {HTTP_PORT}\n"
        f"    https_port {HTTPS_PORT}\n"
        "}\n\n"
        + rendered
    )
    path = TARGET / "Caddyfile"
    path.write_text(rendered, encoding="utf-8")
    return path


def wait_for_tls(caddy: subprocess.Popen[str], log_path: pathlib.Path) -> None:
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    last_error: Exception | None = None
    for _ in range(120):
        if caddy.poll() is not None:
            detail = log_path.read_text(encoding="utf-8", errors="replace") if log_path.exists() else ""
            raise RuntimeError(
                f"Caddy exited before TLS became ready rc={caddy.returncode}\n{detail}"
            )
        try:
            with socket.create_connection(("127.0.0.1", HTTPS_PORT), timeout=0.5) as raw:
                with context.wrap_socket(raw, server_hostname=EDGE_HOST):
                    return
        except Exception as error:  # noqa: BLE001 - bounded readiness probe
            last_error = error
            time.sleep(0.25)
    raise RuntimeError(f"Caddy TLS did not become ready: {last_error}")


def curl(*arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return run(
        [
            "curl",
            "--noproxy",
            "*",
            "--silent",
            "--show-error",
            *arguments,
        ],
        check=check,
    )


def parse_headers(raw: str) -> tuple[int, dict[str, str]]:
    blocks = [block for block in raw.replace("\r\n", "\n").split("\n\n") if block.strip()]
    if not blocks:
        raise AssertionError("HTTP response contained no header block")
    block = blocks[-1]
    lines = block.splitlines()
    parts = lines[0].split()
    if len(parts) < 2 or not parts[1].isdigit():
        raise AssertionError(f"invalid HTTP status line: {lines[0]!r}")
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if ":" in line:
            name, value = line.split(":", 1)
            headers[name.strip().lower()] = value.strip()
    return int(parts[1]), headers


def prove_redirect() -> tuple[int, str]:
    response = curl(
        "--dump-header",
        "-",
        "--output",
        "/dev/null",
        "--resolve",
        f"{EDGE_HOST}:{HTTP_PORT}:127.0.0.1",
        f"http://{EDGE_HOST}:{HTTP_PORT}/inspect",
    )
    status, headers = parse_headers(response.stdout)
    location = headers.get("location", "")
    # The production site address is the canonical HTTPS origin without an
    # explicit port. The nonstandard HTTPS_PORT exists only inside this CI
    # fixture so the redirect must not leak that harness detail.
    expected = f"https://{EDGE_HOST}/inspect"
    if status not in (301, 302, 307, 308) or location != expected:
        raise AssertionError(
            f"unexpected HTTPS redirect status={status} location={location!r} expected={expected!r}"
        )
    return status, location


def prove_tls_and_forwarded_headers() -> tuple[dict[str, Any], dict[str, str], str]:
    header_path = TARGET / "https-headers.txt"
    body_path = TARGET / "inspect.json"
    curl(
        "--insecure",
        "--dump-header",
        str(header_path),
        "--output",
        str(body_path),
        "--resolve",
        f"{EDGE_HOST}:{HTTPS_PORT}:127.0.0.1",
        "--header",
        "X-Forwarded-For: 198.51.100.77",
        "--header",
        "X-Forwarded-Host: attacker.invalid",
        "--header",
        "X-Forwarded-Proto: http",
        f"https://{EDGE_HOST}:{HTTPS_PORT}/inspect",
    )
    status, headers = parse_headers(header_path.read_text(encoding="utf-8"))
    if status != 200:
        raise AssertionError(f"HTTPS proxy request returned {status}")

    observed = json.loads(body_path.read_text(encoding="utf-8"))
    if observed["x_forwarded_for"] != "127.0.0.1":
        raise AssertionError(f"proxy did not overwrite X-Forwarded-For: {observed}")
    if observed["x_forwarded_proto"] != "https":
        raise AssertionError(f"proxy did not overwrite X-Forwarded-Proto: {observed}")
    if observed["x_forwarded_host"] not in (EDGE_HOST, f"{EDGE_HOST}:{HTTPS_PORT}"):
        raise AssertionError(f"proxy did not overwrite X-Forwarded-Host: {observed}")
    if "attacker.invalid" in json.dumps(observed):
        raise AssertionError("attacker-supplied forwarded authority reached upstream")

    required = {
        "strict-transport-security",
        "x-content-type-options",
        "referrer-policy",
        "permissions-policy",
    }
    missing = sorted(required - headers.keys())
    if missing:
        raise AssertionError(f"HTTPS response missing security headers: {missing}")
    if "server" in headers:
        raise AssertionError(f"edge leaked Server header: {headers['server']!r}")

    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    with socket.create_connection(("127.0.0.1", HTTPS_PORT), timeout=2) as raw:
        with context.wrap_socket(raw, server_hostname=EDGE_HOST) as tls:
            certificate = tls.getpeercert(binary_form=True)
    fingerprint = hashlib.sha256(certificate).hexdigest()
    return observed, headers, fingerprint


def prove_websocket_upgrade() -> str:
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    expected_accept = base64.b64encode(
        hashlib.sha1((WS_KEY + WS_GUID).encode("ascii")).digest()
    ).decode("ascii")

    with socket.create_connection(("127.0.0.1", HTTPS_PORT), timeout=3) as raw:
        with context.wrap_socket(raw, server_hostname=EDGE_HOST) as tls:
            request = (
                "GET /ws HTTP/1.1\r\n"
                f"Host: {EDGE_HOST}:{HTTPS_PORT}\r\n"
                "Connection: Upgrade\r\n"
                "Upgrade: websocket\r\n"
                "Sec-WebSocket-Version: 13\r\n"
                f"Sec-WebSocket-Key: {WS_KEY}\r\n"
                "\r\n"
            )
            tls.sendall(request.encode("ascii"))
            response = bytearray()
            while b"\r\n\r\n" not in response and len(response) < 16 * 1024:
                chunk = tls.recv(4096)
                if not chunk:
                    break
                response.extend(chunk)

    text = response.decode("iso-8859-1")
    status, headers = parse_headers(text)
    if status != 101:
        raise AssertionError(f"WebSocket upgrade returned {status}: {text}")
    if headers.get("upgrade", "").lower() != "websocket":
        raise AssertionError(f"WebSocket Upgrade header missing: {headers}")
    if headers.get("sec-websocket-accept") != expected_accept:
        raise AssertionError(f"WebSocket accept mismatch: {headers}")
    return headers["sec-websocket-accept"]


def prove_body_classes() -> tuple[int, int, int]:
    payload = TARGET / "payload-9m.bin"
    with payload.open("wb") as handle:
        handle.truncate(9 * 1024 * 1024)

    ordinary = curl(
        "--insecure",
        "--output",
        "/dev/null",
        "--write-out",
        "%{http_code}",
        "--resolve",
        f"{EDGE_HOST}:{HTTPS_PORT}:127.0.0.1",
        "--data-binary",
        f"@{payload}",
        f"https://{EDGE_HOST}:{HTTPS_PORT}/v1/ordinary",
        check=False,
    )
    if ordinary.returncode not in (0, 22):
        raise RuntimeError(f"ordinary body curl failed: {ordinary.stderr}")
    ordinary_code = int(ordinary.stdout[-3:])
    if ordinary_code != 413:
        raise AssertionError(f"ordinary 9 MiB body expected 413, got {ordinary_code}")

    upload_body = TARGET / "upload-response.json"
    upload = curl(
        "--insecure",
        "--output",
        str(upload_body),
        "--write-out",
        "%{http_code}",
        "--resolve",
        f"{EDGE_HOST}:{HTTPS_PORT}:127.0.0.1",
        "--data-binary",
        f"@{payload}",
        f"https://{EDGE_HOST}:{HTTPS_PORT}/v1/uploads/test/content",
    )
    upload_code = int(upload.stdout[-3:])
    if upload_code != 200:
        raise AssertionError(f"9 MiB upload-class body expected 200, got {upload_code}")
    observed = json.loads(upload_body.read_text(encoding="utf-8"))
    observed_bytes = int(observed["body_bytes"])
    if observed_bytes != 9 * 1024 * 1024:
        raise AssertionError(f"upload fixture observed {observed_bytes} bytes")

    payload.unlink(missing_ok=True)
    return ordinary_code, upload_code, observed_bytes


def prove_direct_app_port_is_loopback_only(image: str) -> bool:
    attempt = run(
        [
            "docker",
            "run",
            "--rm",
            "--entrypoint",
            "sh",
            "--add-host",
            "host.docker.internal:host-gateway",
            image,
            "-c",
            (
                "wget -q -T 2 -O - "
                f"http://host.docker.internal:{UPSTREAM_PORT}/inspect"
            ),
        ],
        check=False,
    )
    if attempt.returncode == 0:
        raise AssertionError(
            "bridge-network container reached loopback-only application fixture directly"
        )
    return True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--caddy-image", default="caddy:2.10.2-alpine")
    args = parser.parse_args()

    TARGET.mkdir(parents=True, exist_ok=True)
    if 'listen = "127.0.0.1:8080"' not in SOURCE_CONFIG.read_text(encoding="utf-8"):
        raise AssertionError("production config no longer pins the application listener to loopback")

    rendered = render_caddy()
    fixture = FixtureServer(("127.0.0.1", UPSTREAM_PORT), FixtureHandler)
    fixture_thread = threading.Thread(target=fixture.serve_forever, daemon=True)
    fixture_thread.start()

    run(["docker", "rm", "-f", CONTAINER_NAME], check=False)
    log_path = TARGET / "caddy.log"
    log_handle = log_path.open("w", encoding="utf-8")
    caddy = subprocess.Popen(
        [
            "docker",
            "run",
            "--rm",
            "--name",
            CONTAINER_NAME,
            "--network",
            "host",
            "-v",
            f"{rendered.resolve()}:/etc/caddy/Caddyfile:ro",
            "--entrypoint",
            "caddy",
            args.caddy_image,
            "run",
            "--config",
            "/etc/caddy/Caddyfile",
            "--adapter",
            "caddyfile",
        ],
        cwd=ROOT,
        text=True,
        stdout=log_handle,
        stderr=subprocess.STDOUT,
    )

    try:
        wait_for_tls(caddy, log_path)
        redirect_status, redirect_location = prove_redirect()
        observed, security_headers, certificate_sha256 = prove_tls_and_forwarded_headers()
        websocket_accept = prove_websocket_upgrade()
        ordinary_code, upload_code, upload_bytes = prove_body_classes()
        direct_denied = prove_direct_app_port_is_loopback_only(args.caddy_image)

        receipt = {
            "schema": "chaptera.cloud-edge-01.integration-receipt.v1",
            "task": "CLOUD-EDGE-01",
            "caddy_image": args.caddy_image,
            "fixture": {
                "host": EDGE_HOST,
                "http_port": HTTP_PORT,
                "https_port": HTTPS_PORT,
                "upstream_port": UPSTREAM_PORT,
                "tls": "caddy_internal_generated_test_certificate",
            },
            "assertions": {
                "https_redirect": True,
                "tls_termination": True,
                "generated_test_certificate": True,
                "forwarded_authority_overwritten": True,
                "security_headers": True,
                "websocket_upgrade_101": True,
                "ordinary_body_limit_8m": True,
                "upload_class_allows_9m_below_256m_limit": True,
                "upload_streamed_to_fixture": True,
                "direct_app_port_denied_from_bridge_network": direct_denied,
                "production_listener_loopback": True,
            },
            "observed": {
                "redirect_status": redirect_status,
                "redirect_location": redirect_location,
                "certificate_sha256": certificate_sha256,
                "forwarded": {
                    "for": observed["x_forwarded_for"],
                    "host": observed["x_forwarded_host"],
                    "proto": observed["x_forwarded_proto"],
                },
                "security_headers": {
                    key: security_headers[key]
                    for key in (
                        "strict-transport-security",
                        "x-content-type-options",
                        "referrer-policy",
                        "permissions-policy",
                    )
                },
                "websocket_accept": websocket_accept,
                "ordinary_9m_status": ordinary_code,
                "upload_9m_status": upload_code,
                "upload_9m_observed_bytes": upload_bytes,
            },
            "files": {
                "source_caddy": {
                    "path": str(SOURCE_CADDY.relative_to(ROOT)),
                    "sha256": sha256(SOURCE_CADDY),
                },
                "rendered_caddy": {
                    "path": str(rendered.relative_to(ROOT)),
                    "sha256": sha256(rendered),
                },
            },
        }
        (TARGET / "receipt.json").write_text(
            json.dumps(receipt, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(json.dumps(receipt, indent=2, sort_keys=True))
    finally:
        run(["docker", "rm", "-f", CONTAINER_NAME], check=False)
        if caddy.poll() is None:
            caddy.terminate()
            try:
                caddy.wait(timeout=5)
            except subprocess.TimeoutExpired:
                caddy.kill()
                caddy.wait(timeout=5)
        log_handle.close()
        fixture.shutdown()
        fixture.server_close()
        fixture_thread.join(timeout=2)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
