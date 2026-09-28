#!/usr/bin/env python3
"""Hosted receipt for the real Chaptera OIDC login path using loopback IdP."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

from chaptera_local_oidc import OidcFixture


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def no_redirect(url: str, *, headers: dict[str, str] | None = None):
    request = urllib.request.Request(url, headers=headers or {})
    try:
        return urllib.request.build_opener(NoRedirect).open(request, timeout=5)
    except urllib.error.HTTPError as error:
        return error


def wait_live(process: subprocess.Popen[str]) -> None:
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if process.poll() is not None:
            stdout, stderr = process.communicate()
            raise SystemExit(
                f"chaptera serve exited early rc={process.returncode}: {stdout}\n{stderr}"
            )
        try:
            with urllib.request.urlopen("http://127.0.0.1:18082/live", timeout=1) as response:
                if response.status == 200:
                    return
        except OSError:
            pass
        time.sleep(0.2)
    raise SystemExit("chaptera serve did not reach /live=200")


def provider_self_test(provider: OidcFixture) -> tuple[bool, bool]:
    verifier = "chaptera-local-verifier-" + ("x" * 32)
    challenge = base64.urlsafe_b64encode(
        hashlib.sha256(verifier.encode("ascii")).digest()
    ).rstrip(b"=").decode("ascii")
    base = {
        "response_type": ["code"],
        "client_id": [provider.client_id],
        "redirect_uri": [provider.redirect_uri],
        "scope": ["openid email"],
        "state": ["self-test-state"],
        "nonce": ["self-test-nonce"],
        "code_challenge": [challenge],
        "code_challenge_method": ["S256"],
    }
    bad_target = provider.authorize(base)
    bad_code = urllib.parse.parse_qs(
        urllib.parse.urlsplit(bad_target).query
    )["code"][0]
    basic = "Basic " + base64.b64encode(
        f"{provider.client_id}:{provider.client_secret}".encode("utf-8")
    ).decode("ascii")
    try:
        provider.token(
            {
                "grant_type": ["authorization_code"],
                "code": [bad_code],
                "redirect_uri": [provider.redirect_uri],
                "code_verifier": ["wrong-verifier"],
            },
            basic,
        )
        wrong_pkce_rejected = False
    except ValueError as error:
        wrong_pkce_rejected = str(error) == "invalid_grant"

    good_target = provider.authorize(base)
    good_code = urllib.parse.parse_qs(
        urllib.parse.urlsplit(good_target).query
    )["code"][0]
    token = provider.token(
        {
            "grant_type": ["authorization_code"],
            "code": [good_code],
            "redirect_uri": [provider.redirect_uri],
            "code_verifier": [verifier],
        },
        basic,
    )
    if token.get("id_token", "").count(".") != 2:
        raise SystemExit("provider did not issue a signed ID token")
    try:
        provider.token(
            {
                "grant_type": ["authorization_code"],
                "code": [good_code],
                "redirect_uri": [provider.redirect_uri],
                "code_verifier": [verifier],
            },
            basic,
        )
        replay_rejected = False
    except ValueError as error:
        replay_rejected = str(error) == "invalid_grant"
    return wrong_pkce_rejected, replay_rejected


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--example-config", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    example = args.example_config.read_text(encoding="utf-8")

    with tempfile.TemporaryDirectory(prefix="chaptera-oidc-login-") as temp_raw:
        temp = pathlib.Path(temp_raw)
        database = temp / "chaptera.sqlite"
        credentials = temp / "credentials"
        credentials.mkdir()
        (credentials / "oidc_client_secret").write_text(
            "chaptera-local-only-secret\n", encoding="utf-8"
        )
        if os.name == "posix":
            (credentials / "oidc_client_secret").chmod(0o600)

        with OidcFixture() as provider:
            wrong_pkce_rejected, replay_rejected = provider_self_test(provider)
            config = temp / "chaptera.toml"
            rendered = example
            rendered = rendered.replace(
                'environment = "prod"', 'environment = "test"', 1
            )
            rendered = rendered.replace(
                'listen = "127.0.0.1:8080"',
                'listen = "127.0.0.1:18082"',
                1,
            )
            rendered = rendered.replace(
                'public_origin = "https://cloud.example.invalid"',
                'public_origin = "http://127.0.0.1:18082"',
                1,
            )
            rendered = rendered.replace(
                'path = "/var/lib/chaptera/chaptera.sqlite"',
                f'path = "{database.as_posix()}"',
                1,
            )
            rendered = rendered.replace(
                'issuer = "https://id.example.invalid"',
                f'issuer = "{provider.issuer}"',
                1,
            )
            config.write_text(rendered, encoding="utf-8")

            env = os.environ.copy()
            env.update(
                {
                    "CREDENTIALS_DIRECTORY": str(credentials),
                    "AWS_ACCESS_KEY_ID": "chaptera-local",
                    "AWS_SECRET_ACCESS_KEY": "chaptera-local",
                    "AWS_REGION": "us-east-1",
                    "AWS_EC2_METADATA_DISABLED": "true",
                }
            )
            migrated = subprocess.run(
                [str(args.binary), "--config", str(config), "migrate", "up"],
                capture_output=True,
                text=True,
                env=env,
                check=False,
            )
            if migrated.returncode != 0:
                raise SystemExit(
                    f"migration failed: {migrated.stdout}\n{migrated.stderr}"
                )

            server = subprocess.Popen(
                [str(args.binary), "--config", str(config), "serve"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=env,
            )
            try:
                wait_live(server)
                login = no_redirect(
                    "http://127.0.0.1:18082/v1/auth/login?return_path=%2Flocal"
                )
                if login.code not in (302, 303, 307, 308):
                    raise SystemExit(f"login did not redirect: {login.code}")
                authorize_url = login.headers.get("Location")
                if not authorize_url or not authorize_url.startswith(provider.issuer):
                    raise SystemExit("login redirect did not target local provider")

                authorize = no_redirect(authorize_url)
                if authorize.code not in (302, 303):
                    raise SystemExit(
                        f"provider authorize did not redirect: {authorize.code}"
                    )
                callback_url = authorize.headers.get("Location")
                if not callback_url or not callback_url.startswith(
                    "http://127.0.0.1:18082/v1/auth/callback?"
                ):
                    raise SystemExit("provider did not preserve Chaptera callback")

                callback = no_redirect(callback_url)
                if callback.code not in (302, 303):
                    body = callback.read().decode("utf-8", errors="replace")
                    raise SystemExit(
                        f"Chaptera callback failed: {callback.code} {body}"
                    )
                if callback.headers.get("Location") != "/local":
                    raise SystemExit("Chaptera callback did not preserve return_path")
                set_cookie = callback.headers.get("Set-Cookie", "")
                required_cookie_parts = [
                    "__Host-chaptera_session=",
                    "Secure",
                    "HttpOnly",
                    "SameSite=Lax",
                    "Path=/",
                ]
                cookie_issued = all(part in set_cookie for part in required_cookie_parts)
                if not cookie_issued:
                    raise SystemExit(
                        f"verified login did not issue hardened session cookie: {set_cookie}"
                    )

                replay = no_redirect(callback_url)
                callback_replay_rejected = replay.code in (400, 401, 403)
                if not callback_replay_rejected:
                    raise SystemExit(
                        f"callback replay was not rejected: {replay.code}"
                    )
            finally:
                server.terminate()
                try:
                    stdout, stderr = server.communicate(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    stdout, stderr = server.communicate()
                if "chaptera-local-only-secret" in stdout or "chaptera-local-only-secret" in stderr:
                    raise SystemExit("OIDC client secret leaked into server output")

    receipt = {
        "schema": "chaptera.local-oidc-login.receipt.v1",
        "task": "LOCAL-WEB-JOURNEY-01 / Phase C-C prep",
        "git_sha": os.environ.get("GITHUB_SHA", "unknown"),
        "public_safe": True,
        "provider_authorization_code_pkce_s256": True,
        "provider_rs256_jwks": True,
        "wrong_pkce_rejected": wrong_pkce_rejected,
        "authorization_code_replay_rejected": replay_rejected,
        "chaptera_real_login_route_used": True,
        "chaptera_real_callback_route_used": True,
        "chaptera_hardened_session_cookie_issued": cookie_issued,
        "chaptera_callback_replay_rejected": callback_replay_rejected,
        "synthetic_principal_header_used": False,
        "auth_http_api_test_used": False,
        "browser_secure_origin_claim": False,
        "s3_source_ingress_claim": False,
    }
    (args.out / "receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
