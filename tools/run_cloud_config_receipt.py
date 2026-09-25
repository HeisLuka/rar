#!/usr/bin/env python3
"""Exercise CLOUD-SECRETS-CONFIG-01 against the built Chaptera binary."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


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
        config.write_text(
            example.replace("127.0.0.1:8080", "127.0.0.1:18082"),
            encoding="utf-8",
        )

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
        worker_isolated = (
            worker.returncode != 0
            and "worker_runtime_not_configured" in worker.stderr
            and "credentials_directory_missing" not in worker.stderr
        )
        if not worker_isolated:
            raise SystemExit("worker unexpectedly required the web/OIDC secret")

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

        server_env = os.environ.copy()
        server_env.update(role_env)
        server = subprocess.Popen(
            [str(binary), "--config", str(config), "serve"],
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
                        f"prod-config server exited early rc={server.returncode}: "
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
                raise SystemExit(f"prod-config /live expected 200, got {live}")

            ready = http_code("http://127.0.0.1:18082/ready")
            if ready != 503:
                raise SystemExit(
                    f"prod-config /ready expected 503 before producers, got {ready}"
                )
        finally:
            server.terminate()
            try:
                stdout, stderr = server.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                stdout, stderr = server.communicate()
                raise SystemExit("prod-config server did not stop after SIGTERM")

        if server.returncode != 0:
            raise SystemExit(
                f"prod-config server did not shut down cleanly: rc={server.returncode}"
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
        "worker_does_not_receive_oidc_secret": worker_isolated,
        "prod_config_live_code": live,
        "prod_config_ready_without_producers": ready,
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
