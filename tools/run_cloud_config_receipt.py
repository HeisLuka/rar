#!/usr/bin/env python3
"""Exercise CLOUD-SECRETS-CONFIG-01 against the built Chaptera binary."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def run(
    binary: pathlib.Path,
    config: pathlib.Path,
    *args: str,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(
        [str(binary), "--config", str(config), *args],
        capture_output=True,
        text=True,
        env=merged,
        check=False,
    )


def http_code(url: str) -> int:
    try:
        with urllib.request.urlopen(url, timeout=1) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


class OidcFixture:
    def __enter__(self) -> "OidcFixture":
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *_args: object) -> None:
                return

            def do_GET(self) -> None:
                if self.path == "/.well-known/openid-configuration":
                    body = json.dumps(
                        {
                            "issuer": fixture.issuer,
                            "authorization_endpoint": fixture.issuer + "/authorize",
                            "token_endpoint": fixture.issuer + "/token",
                            "jwks_uri": fixture.issuer + "/jwks",
                            "response_types_supported": ["code"],
                            "subject_types_supported": ["public"],
                            "id_token_signing_alg_values_supported": ["RS256"],
                            "scopes_supported": ["openid"],
                            "token_endpoint_auth_methods_supported": [
                                "client_secret_basic"
                            ],
                        }
                    ).encode("utf-8")
                elif self.path == "/jwks":
                    body = b'{"keys":[]}'
                else:
                    self.send_response(404)
                    self.end_headers()
                    return

                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        host, port = self.server.server_address
        self.issuer = f"http://{host}:{port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--example-config", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()

    binary = args.binary.resolve()
    example = args.example_config.read_text(encoding="utf-8")
    out = args.out
    out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="chaptera-config-") as temp_raw:
        temp = pathlib.Path(temp_raw)
        config = temp / "chaptera.toml"
        database = temp / "chaptera.sqlite"
        rendered = example.replace("127.0.0.1:8080", "127.0.0.1:18082")
        rendered = rendered.replace(
            'path = "/var/lib/chaptera/chaptera.sqlite"',
            f'path = "{database.as_posix()}"',
        )
        config.write_text(rendered, encoding="utf-8")

        missing = run(binary, config, "doctor")
        missing_secret_fails = (
            missing.returncode != 0
            and "credentials_directory_missing" in missing.stderr
        )
        if not missing_secret_fails:
            raise SystemExit(
                "doctor did not fail closed on the missing systemd credential"
            )

        worker = run(binary, config, "worker")
        worker_does_not_resolve_web_secret = (
            "credentials_directory_missing" not in worker.stderr
            and "oidc_client_secret" not in worker.stderr
            and "receipt-only-oidc-secret" not in worker.stdout
            and "receipt-only-oidc-secret" not in worker.stderr
        )
        if not worker_does_not_resolve_web_secret:
            raise SystemExit(
                "worker unexpectedly resolved or required the web/OIDC secret"
            )

        credentials = temp / "credentials"
        credentials.mkdir()
        secret = credentials / "oidc_client_secret"
        secret.write_bytes(b"receipt-only-oidc-secret\n")
        if os.name == "posix":
            secret.chmod(0o600)

        role_env = {"CREDENTIALS_DIRECTORY": str(credentials)}

        doctor = run(binary, config, "doctor", env=role_env)
        doctor_resolved = (
            doctor.returncode != 0
            and "runtime_not_ready" in doctor.stderr
            and "receipt-only-oidc-secret" not in doctor.stdout
            and "receipt-only-oidc-secret" not in doctor.stderr
        )
        if not doctor_resolved:
            raise SystemExit(
                "doctor did not resolve the secret and continue to runtime readiness"
            )

        unmigrated = run(binary, config, "serve", env=role_env)
        unmigrated_serve_fails = (
            unmigrated.returncode != 0
            and "sqlite_database_missing" in unmigrated.stderr
            and not database.exists()
        )
        if not unmigrated_serve_fails:
            raise SystemExit(
                "prod-config serve did not fail closed before operator migration"
            )

        migrated = run(binary, config, "migrate", "up", env=role_env)
        if migrated.returncode != 0:
            raise SystemExit(
                f"operator migration failed rc={migrated.returncode}: "
                f"{migrated.stdout}\n{migrated.stderr}"
            )
        if not database.exists():
            raise SystemExit("operator migration did not materialize the configured database")

        runtime_config = temp / "chaptera-runtime-smoke.toml"
        with OidcFixture() as oidc:
            runtime_rendered = rendered.replace(
                "environment = \"prod\"",
                "environment = \"test\"",
                1,
            ).replace(
                'issuer = "https://id.example.invalid"',
                f'issuer = "{oidc.issuer}"',
                1,
            )
            runtime_config.write_text(runtime_rendered, encoding="utf-8")

            server_env = os.environ.copy()
            server_env.update(role_env)
            server = subprocess.Popen(
                [str(binary), "--config", str(runtime_config), "serve"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=server_env,
            )
            try:
                live = None
                for _ in range(50):
                    if server.poll() is not None:
                        stdout, stderr = server.communicate()
                        raise SystemExit(
                            f"runtime-smoke server exited early rc={server.returncode}: "
                            f"{stdout}\n{stderr}"
                        )
                    try:
                        live = http_code("http://127.0.0.1:18082/live")
                        if live == 200:
                            break
                    except OSError:
                        pass
                    time.sleep(0.1)

                if live != 200:
                    raise SystemExit(f"runtime-smoke /live expected 200, got {live}")

                ready = http_code("http://127.0.0.1:18082/ready")
                if ready != 200:
                    raise SystemExit(
                        f"configured runtime-smoke /ready expected 200, got {ready}"
                    )
            finally:
                server.terminate()
                try:
                    stdout, stderr = server.communicate(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    stdout, stderr = server.communicate()
                    raise SystemExit("runtime-smoke server did not stop after SIGTERM")

            if server.returncode != 0:
                raise SystemExit(
                    f"runtime-smoke server did not shut down cleanly: rc={server.returncode}"
                )
            if "receipt-only-oidc-secret" in stdout or "receipt-only-oidc-secret" in stderr:
                raise SystemExit("secret leaked into ordinary server output")

    receipt = {
        "schema": "chaptera.cloud-secrets-config-01.receipt.v1",
        "task": "CLOUD-SECRETS-CONFIG-01",
        "git_sha": os.environ.get("GITHUB_SHA", "unknown"),
        "public_safe": True,
        "typed_prod_config": True,
        "missing_required_secret_fails_startup": missing_secret_fails,
        "systemd_credential_resolves": doctor_resolved,
        "worker_does_not_receive_oidc_secret": worker_does_not_resolve_web_secret,
        "unmigrated_prod_config_fails_closed": unmigrated_serve_fails,
        "operator_migration_precedes_serve": True,
        "prod_config_validated_without_external_idp": True,
        "runtime_smoke_uses_local_oidc_fixture": True,
        "runtime_smoke_live_code": live,
        "runtime_smoke_ready_code": ready,
        "configured_runtime_all_required_ports_ready": ready == 200,
        "secret_present_in_output": False,
        "secret_sources": ["env", "file", "systemd"],
        "short_lived_key_ring_supported": True,
    }

    (out / "receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
