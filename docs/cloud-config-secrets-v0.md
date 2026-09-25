# CLOUD-SECRETS-CONFIG-01 — typed production config and role-scoped secrets

Chaptera V0 uses one typed TOML configuration root. Production services do not
assemble security-critical configuration from dozens of ad-hoc environment
variables.

## Entry point

Production:

```text
chaptera --config /etc/chaptera/chaptera.toml serve
chaptera --config /etc/chaptera/chaptera.toml worker
chaptera --config /etc/chaptera/chaptera.toml doctor
chaptera --config /etc/chaptera/chaptera.toml migrate status
chaptera --config /etc/chaptera/chaptera.toml migrate up
```

Development may omit `--config`; only that path retains bounded development
defaults and the historical `CHAPTERA_LISTEN` / `CHAPTERA_SQLITE_PATH`
overrides.

## Typed modes

`environment` is one of:

- `dev`
- `test`
- `prod`

The parser rejects unknown TOML fields. Production additionally requires:

- private/loopback application listener;
- HTTPS `public_origin`;
- absolute SQLite path;
- WAL + `synchronous=FULL`;
- bounded SQLite pool/timeouts;
- bounded worker concurrency;
- distinct quarantine/private storage namespaces;
- OIDC configuration with HTTPS issuer;
- a secret **reference**, never an inline OIDC client secret.

## Secret references

Supported V0 sources:

```toml
client_secret = { source = "env", name = "CHAPTERA_OIDC_CLIENT_SECRET" }
client_secret = { source = "file", path = "/run/secret/oidc" }
client_secret = { source = "systemd", name = "oidc_client_secret" }
```

`systemd` resolves only inside `$CREDENTIALS_DIRECTORY`. The canonical web
unit uses:

```ini
LoadCredential=oidc_client_secret:/etc/chaptera/credentials/oidc_client_secret
```

The worker unit intentionally does **not** receive that credential.

Production file-backed secrets must not be group/world accessible on Unix.
Secret files are bounded to 64 KiB and a single trailing CRLF/LF is stripped.

Process environment resolution is lazy: Chaptera reads only the explicitly
requested secret name. It does not copy the complete process environment into a
Rust map.

Resolved secret memory is wrapped in `zeroize::Zeroizing`; Debug output is
redacted and never renders secret bytes.

## Role boundary

- `serve`: resolves the OIDC client secret and active application key ring.
- `doctor`: resolves required secrets so an operator can detect missing
  credentials before admission.
- `worker`: parses/validates the same structural config but is not handed the
  OIDC client secret.
- `migrate`: consumes the typed SQLite path and busy timeout without requiring
  AuthN secrets.

This is deliberate least privilege, not four separate config formats.

## Short-lived key ring

When an owning protocol actually uses an application-managed symmetric key,
configuration may include:

```toml
[key_ring]
active = "grant-2026-09"
previous = ["grant-2026-08"]

[[key_ring.keys]]
id = "grant-2026-09"
secret = { source = "systemd", name = "grant_key_2026_09" }

[[key_ring.keys]]
id = "grant-2026-08"
secret = { source = "systemd", name = "grant_key_2026_08" }
```

V0 validates a unique active key, at most three previous overlap keys, and that
all referenced IDs exist.

This ring is **not** the long-lived migration-evidence signing authority.
Asymmetric long-lived issuer verification history remains a separate contract.

## Failure policy

Missing, empty, oversized, over-permissive, malformed or unresolved required
secrets fail the affected role before normal traffic admission. Secret values
must never appear in config dumps, Debug output, logs or receipts.

## Dependency choices

- `toml 1.1.6+spec-1.1.0` for Serde-compatible typed TOML parsing.
- `zeroize 1.9.0` for compiler-resistant in-memory secret clearing.
- `url 2.5.8` for structural origin/issuer validation.

No custom secret encryption format or Vault replacement is introduced.
