#!/usr/bin/env python3
"""Compose Chaptera AuthN + Caddy TLS + Chromium for secure-origin acceptance."""

from __future__ import annotations

import argparse
import os
import pathlib
import socket
import ssl
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

from chaptera_local_oidc import OidcFixture

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE_CADDY = ROOT / "deploy" / "caddy" / "Caddyfile.example"
SOURCE_CONFIG = ROOT / "deploy" / "config" / "chaptera.prod.example.toml"
TARGET = ROOT / "target" / "secure-origin-oidc"

EDGE_HOST = "edge.test"
HTTP_PORT = 18080
HTTPS_PORT = 18443
UPSTREAM_PORT = 18082
CONTAINER_NAME = "chaptera-secure-origin-oidc"


def run_checked(
    args: list[str],
    *,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        args,
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed rc={completed.returncode}: {' '.join(args)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed


def render_caddy() -> pathlib.Path:
    source = SOURCE_CADDY.read_text(encoding="utf-8")
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


def render_config(
    example: str,
    *,
    database: pathlib.Path,
    issuer: str,
) -> str:
    public_origin = f"https://{EDGE_HOST}:{HTTPS_PORT}"
    rendered = example
    rendered = rendered.replace(
        'environment = "prod"',
        'environment = "test"',
        1,
    )
    rendered = rendered.replace(
        'listen = "127.0.0.1:8080"',
        f'listen = "127.0.0.1:{UPSTREAM_PORT}"',
        1,
    )
    rendered = rendered.replace(
        'public_origin = "https://cloud.example.invalid"',
        f'public_origin = "{public_origin}"',
        1,
    )
    rendered = rendered.replace(
        'path = "/var/lib/chaptera/chaptera.sqlite"',
        f'path = "{database.as_posix()}"',
        1,
    )
    rendered = rendered.replace(
        'issuer = "https://id.example.invalid"',
        f'issuer = "{issuer}"',
        1,
    )
    return rendered


def wait_live(server: subprocess.Popen[str]) -> None:
    deadline = time.monotonic() + 20
    last: object = None
    while time.monotonic() < deadline:
        if server.poll() is not None:
            raise RuntimeError(
                f"Chaptera server exited before /live became ready: {server.returncode}"
            )
        try:
            with urllib.request.urlopen(
                f"http://127.0.0.1:{UPSTREAM_PORT}/live",
                timeout=1,
            ) as response:
                if response.status == 200:
                    return
                last = response.status
        except (OSError, urllib.error.HTTPError) as error:
            last = error
        time.sleep(0.2)
    raise RuntimeError(f"Chaptera /live did not become ready: {last}")


def wait_tls(caddy: subprocess.Popen[str]) -> None:
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    deadline = time.monotonic() + 30
    last: object = None
    while time.monotonic() < deadline:
        if caddy.poll() is not None:
            raise RuntimeError(
                f"Caddy exited before TLS became ready: {caddy.returncode}"
            )
        try:
            with socket.create_connection(
                ("127.0.0.1", HTTPS_PORT),
                timeout=1,
            ) as raw:
                with context.wrap_socket(raw, server_hostname=EDGE_HOST):
                    return
        except OSError as error:
            last = error
        time.sleep(0.25)
    raise RuntimeError(f"Caddy TLS did not become ready: {last}")


def terminate(process: subprocess.Popen[str] | None) -> None:
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument(
        "--browser-script",
        type=pathlib.Path,
        default=ROOT / "tools" / "run_secure_origin_oidc_browser.mjs",
    )
    parser.add_argument("--caddy-image", default="caddy:2.10.2-alpine")
    args = parser.parse_args()

    binary = args.binary.resolve()
    browser_script = args.browser_script.resolve()
    if not binary.is_file():
        raise SystemExit(f"Chaptera binary not found: {binary}")
    if not browser_script.is_file():
        raise SystemExit(f"browser script not found: {browser_script}")

    TARGET.mkdir(parents=True, exist_ok=True)
    caddy_config = render_caddy()
    subprocess.run(
        ["docker", "rm", "-f", CONTAINER_NAME],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )

    server: subprocess.Popen[str] | None = None
    caddy: subprocess.Popen[str] | None = None
    server_log = TARGET / "server.log"
    caddy_log = TARGET / "caddy.log"
    browser_log = TARGET / "browser.log"
    server_handle = None
    caddy_handle = None

    try:
        with tempfile.TemporaryDirectory(prefix="chaptera-secure-origin-") as temp_raw:
            temp = pathlib.Path(temp_raw)
            database = temp / "chaptera.sqlite"
            credentials = temp / "credentials"
            credentials.mkdir()
            secret = credentials / "oidc_client_secret"
            secret.write_text("chaptera-local-only-secret\n", encoding="utf-8")
            if os.name == "posix":
                secret.chmod(0o600)

            redirect_uri = (
                f"https://{EDGE_HOST}:{HTTPS_PORT}/v1/auth/callback"
            )
            with OidcFixture(redirect_uri=redirect_uri) as provider:
                config = temp / "chaptera.toml"
                config.write_text(
                    render_config(
                        SOURCE_CONFIG.read_text(encoding="utf-8"),
                        database=database,
                        issuer=provider.issuer,
                    ),
                    encoding="utf-8",
                )

                env = os.environ.copy()
                env.update(
                    {
                        "CREDENTIALS_DIRECTORY": str(credentials),
                        "AWS_ACCESS_KEY_ID": "chaptera-secure-origin",
                        "AWS_SECRET_ACCESS_KEY": "chaptera-secure-origin",
                        "AWS_REGION": "us-east-1",
                        "AWS_EC2_METADATA_DISABLED": "true",
                    }
                )

                run_checked(
                    [str(binary), "--config", str(config), "migrate", "up"],
                    env=env,
                )

                server_handle = server_log.open("w", encoding="utf-8")
                server = subprocess.Popen(
                    [str(binary), "--config", str(config), "serve"],
                    cwd=ROOT,
                    env=env,
                    stdout=server_handle,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                wait_live(server)

                caddy_handle = caddy_log.open("w", encoding="utf-8")
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
                        f"{caddy_config.resolve()}:/etc/caddy/Caddyfile:ro",
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
                    stdout=caddy_handle,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                wait_tls(caddy)

                browser_env = env.copy()
                browser_env.update(
                    {
                        "CHAPTERA_SECURE_ORIGIN": (
                            f"https://{EDGE_HOST}:{HTTPS_PORT}"
                        ),
                        "CHAPTERA_OIDC_PROVIDER_ORIGIN": provider.issuer,
                        "CHAPTERA_SECURE_RECEIPT": str(
                            (TARGET / "receipt.json").resolve()
                        ),
                        "CHAPTERA_SECURE_SCREENSHOT": str(
                            (TARGET / "session.png").resolve()
                        ),
                    }
                )
                browser = subprocess.run(
                    ["node", str(browser_script)],
                    cwd=ROOT,
                    env=browser_env,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                browser_log.write_text(
                    browser.stdout + "\n--- stderr ---\n" + browser.stderr,
                    encoding="utf-8",
                )
                if browser.returncode != 0:
                    raise RuntimeError(
                        "secure-origin Chromium acceptance failed; "
                        f"see {browser_log}"
                    )
                if not (TARGET / "receipt.json").is_file():
                    raise RuntimeError(
                        "secure-origin Chromium acceptance emitted no receipt"
                    )
    finally:
        terminate(server)
        subprocess.run(
            ["docker", "rm", "-f", CONTAINER_NAME],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        terminate(caddy)
        if server_handle is not None:
            server_handle.close()
        if caddy_handle is not None:
            caddy_handle.close()

    print((TARGET / "receipt.json").read_text(encoding="utf-8"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
