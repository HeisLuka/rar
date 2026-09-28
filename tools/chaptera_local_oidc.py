#!/usr/bin/env python3
"""Standards-valid loopback OIDC provider for Chaptera test/acceptance flows.

Test-only. Implements discovery, JWKS, Authorization Code + PKCE S256 and a
single-use token endpoint with RS256 ID tokens. No external identity provider
or non-stdlib Python dependency is required.
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import secrets
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

_TEST_RSA_N = 21384838327386467725398402427843693812669403272430590231979626485329315279130267061859102219075401808720369359868792536070615142151990453704649409612611542417973848562415786067270243261128976195315609042830220787545770434657216445602242393778246118144384409727682791493439742408220965992765245865939840580050528933352338390366430408600967328800676447737645976287631184523493233883501756692020388725049209051564393431840800425578488760513535516035930678590827563134122591542994185292289568287006259627895047421525174470207488429921242955892070138089835878833749896244653994377525326407081586810669297540252865932264913
_TEST_RSA_E = 65537
_TEST_RSA_D = 4453203245083034290284491159711259461196052324038275786058897301654513179912573351339275486135031298114213968274552483879697272571513644459377188728323176604807942051903038594276067639749419451997646740192951129866043639577404541577377711355243814904183686036262735506762880273985620846029932602276789818213155959315787587495298450578156999405915698115048498401842536915730714804822952696541550607723982679454364769554902397133779719664112375048234550250356070794183964319789321769349339800614164885491541722798033030102057715969614277043620856219142063466148467332153102544828373817723351695200316548887344888263463
_TEST_KID = "chaptera-local-rs256-v1"
_DIGEST_INFO_SHA256 = bytes.fromhex("3031300d060960864801650304020105000420")


def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def _int_b64url(value: int) -> str:
    size = max(1, (value.bit_length() + 7) // 8)
    return _b64url(value.to_bytes(size, "big"))


def _jwt_rs256(claims: dict[str, object]) -> str:
    header = {"alg": "RS256", "kid": _TEST_KID, "typ": "JWT"}
    encoded_header = _b64url(
        json.dumps(header, separators=(",", ":"), sort_keys=True).encode()
    )
    encoded_claims = _b64url(
        json.dumps(claims, separators=(",", ":"), sort_keys=True).encode()
    )
    signing_input = f"{encoded_header}.{encoded_claims}".encode("ascii")
    digest_info = _DIGEST_INFO_SHA256 + hashlib.sha256(signing_input).digest()
    modulus_size = (_TEST_RSA_N.bit_length() + 7) // 8
    padding_len = modulus_size - len(digest_info) - 3
    if padding_len < 8:
        raise RuntimeError("test RSA key too small for RS256 encoding")
    encoded_message = (
        b"\x00\x01" + (b"\xff" * padding_len) + b"\x00" + digest_info
    )
    signature_int = pow(
        int.from_bytes(encoded_message, "big"), _TEST_RSA_D, _TEST_RSA_N
    )
    signature = signature_int.to_bytes(modulus_size, "big")
    return f"{encoded_header}.{encoded_claims}.{_b64url(signature)}"


def _parse_basic(value: str | None) -> tuple[str, str] | None:
    if not value or not value.startswith("Basic "):
        return None
    try:
        raw = base64.b64decode(value[6:], validate=True).decode("utf-8")
        client_id, secret = raw.split(":", 1)
        return urllib.parse.unquote(client_id), urllib.parse.unquote(secret)
    except Exception:
        return None


class OidcFixture:
    def __init__(
        self,
        *,
        client_id: str = "chaptera-cloud",
        client_secret: str = "chaptera-local-only-secret",
        redirect_uri: str = "http://127.0.0.1:18082/v1/auth/callback",
        subject: str = "chaptera-local-user",
        email: str = "local-user@chaptera.invalid",
    ) -> None:
        self.client_id = client_id
        self.client_secret = client_secret
        self.redirect_uri = redirect_uri
        self.subject = subject
        self.email = email
        self._codes: dict[str, dict[str, str]] = {}
        self._lock = threading.Lock()

    def __enter__(self) -> "OidcFixture":
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, _format: str, *_args: object) -> None:
                return

            def _json(self, status: int, value: object) -> None:
                body = json.dumps(
                    value, separators=(",", ":"), sort_keys=True
                ).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self) -> None:
                parsed = urllib.parse.urlsplit(self.path)
                if parsed.path == "/.well-known/openid-configuration":
                    self._json(200, fixture.discovery_document())
                    return
                if parsed.path == "/jwks":
                    self._json(200, fixture.jwks())
                    return
                if parsed.path == "/authorize":
                    params = urllib.parse.parse_qs(
                        parsed.query, keep_blank_values=True
                    )
                    try:
                        target = fixture.authorize(params)
                    except ValueError as error:
                        self._json(400, {"error": str(error)})
                        return
                    self.send_response(302)
                    self.send_header("Location", target)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                self.send_response(404)
                self.send_header("Content-Length", "0")
                self.end_headers()

            def do_POST(self) -> None:
                parsed = urllib.parse.urlsplit(self.path)
                if parsed.path != "/token":
                    self.send_response(404)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                except ValueError:
                    length = 0
                if length <= 0 or length > 16384:
                    self._json(400, {"error": "invalid_request"})
                    return
                form = urllib.parse.parse_qs(
                    self.rfile.read(length).decode("utf-8"),
                    keep_blank_values=True,
                )
                try:
                    value = fixture.token(
                        form, self.headers.get("Authorization")
                    )
                except ValueError as error:
                    self._json(400, {"error": str(error)})
                    return
                self._json(200, value)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        host, port = self.server.server_address
        self.issuer = f"http://{host}:{port}"
        self.thread = threading.Thread(
            target=self.server.serve_forever, daemon=True
        )
        self.thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def discovery_document(self) -> dict[str, object]:
        return {
            "issuer": self.issuer,
            "authorization_endpoint": self.issuer + "/authorize",
            "token_endpoint": self.issuer + "/token",
            "jwks_uri": self.issuer + "/jwks",
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "scopes_supported": ["openid", "email"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic"],
        }

    def jwks(self) -> dict[str, object]:
        return {
            "keys": [
                {
                    "kty": "RSA",
                    "use": "sig",
                    "alg": "RS256",
                    "kid": _TEST_KID,
                    "n": _int_b64url(_TEST_RSA_N),
                    "e": _int_b64url(_TEST_RSA_E),
                }
            ]
        }

    @staticmethod
    def _one(params: dict[str, list[str]], name: str) -> str:
        values = params.get(name, [])
        if len(values) != 1 or not values[0]:
            raise ValueError("invalid_request")
        return values[0]

    def authorize(self, params: dict[str, list[str]]) -> str:
        if self._one(params, "response_type") != "code":
            raise ValueError("unsupported_response_type")
        if self._one(params, "client_id") != self.client_id:
            raise ValueError("unauthorized_client")
        redirect_uri = self._one(params, "redirect_uri")
        if redirect_uri != self.redirect_uri:
            raise ValueError("invalid_redirect_uri")
        state = self._one(params, "state")
        nonce = self._one(params, "nonce")
        challenge = self._one(params, "code_challenge")
        if self._one(params, "code_challenge_method") != "S256":
            raise ValueError("invalid_request")
        scopes = set(self._one(params, "scope").split())
        if "openid" not in scopes:
            raise ValueError("invalid_scope")

        code = secrets.token_urlsafe(32)
        with self._lock:
            self._codes[code] = {
                "nonce": nonce,
                "challenge": challenge,
                "redirect_uri": redirect_uri,
            }
        query = urllib.parse.urlencode({"code": code, "state": state})
        separator = "&" if "?" in redirect_uri else "?"
        return redirect_uri + separator + query

    def token(
        self,
        form: dict[str, list[str]],
        authorization: str | None,
    ) -> dict[str, object]:
        credentials = _parse_basic(authorization)
        if credentials is None:
            raise ValueError("invalid_client")
        client_id, client_secret = credentials
        if client_id != self.client_id or not hmac.compare_digest(
            client_secret, self.client_secret
        ):
            raise ValueError("invalid_client")
        if self._one(form, "grant_type") != "authorization_code":
            raise ValueError("unsupported_grant_type")
        code = self._one(form, "code")
        verifier = self._one(form, "code_verifier")
        redirect_uri = self._one(form, "redirect_uri")
        with self._lock:
            record = self._codes.pop(code, None)
        if record is None:
            raise ValueError("invalid_grant")
        if redirect_uri != record["redirect_uri"]:
            raise ValueError("invalid_grant")
        actual_challenge = _b64url(
            hashlib.sha256(verifier.encode("ascii")).digest()
        )
        if not hmac.compare_digest(
            actual_challenge, record["challenge"]
        ):
            raise ValueError("invalid_grant")

        now = int(time.time())
        id_token = _jwt_rs256(
            {
                "iss": self.issuer,
                "sub": self.subject,
                "aud": self.client_id,
                "iat": now,
                "exp": now + 300,
                "nonce": record["nonce"],
                "email": self.email,
                "email_verified": True,
            }
        )
        return {
            "access_token": "chaptera-local-access-token",
            "token_type": "Bearer",
            "expires_in": 300,
            "id_token": id_token,
        }
