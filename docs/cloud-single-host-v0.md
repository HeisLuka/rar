# Chaptera Cloud single-host V0 deployment contract

Task: `CLOUD-DEPLOY-01`

This directory materializes the public-safe deployment shell for the first
Chaptera Cloud host. It deliberately does **not** pretend that the production
Rust server already exists in this repository.

## Boundary

The templates are consumers of these task contracts:

- `CLOUD-SERVER-RUNTIME-01`
- `CLOUD-SQLITE-V0-01`
- `CLOUD-MIGRATION-01`
- `CLOUD-AUTHN-01`
- `CLOUD-SECRETS-CONFIG-01`
- `CLOUD-EDGE-01`
- `CLOUD-ASYNC-RUNTIME-01`
- `CLOUD-BLOB-STORE-V0-01`
- `CLOUD-SOURCE-INGRESS-01`
- `CLOUD-BLOB-GC-01`
- `CLOUD-HOST-GOVERNOR-01`

Do not use these templates as evidence that those producers are implemented.

## Physical V0

```text
Internet
  |
  v
Caddy :443
  |
  v
127.0.0.1:8080 chaptera serve

chaptera.slice
  +-- chaptera-web.service
  +-- chaptera-worker.service

serve + worker
  +-- local-block SQLite
  +-- private S3-compatible object storage

worker
  +-- bounded spool
  +-- sandboxed raw-PUB child (no network, no storage credentials)
```

The application listener is private by bind address, not merely by firewall.

## 2 GiB starting envelope

These are deliberately provisional deployment defaults:

| cgroup | MemoryHigh | MemoryMax |
| --- | ---: | ---: |
| `chaptera.slice` | 1280 MiB | 1536 MiB |
| web child | 960 MiB | 1280 MiB |
| worker child | 384 MiB | 640 MiB |

The parent slice caps aggregate web + worker use. Exact production values are
measurement outputs, not architecture constants.

Heavy parse/export concurrency starts at **1**.

## Disk/state

Canonical SQLite state:

```text
/var/lib/chaptera/chaptera.sqlite
```

Disposable/reconcilable worker spill:

```text
/var/spool/chaptera/
```

Do not put SQLite WAL on a network filesystem. Do not put large parser/export
spill in `/run` tmpfs on a 2 GiB VM.

## Health

- `/live`: process/event-loop liveness only.
- `/ready`: safe normal admission; false during incompatible schema/config,
  required durable dependency failure, startup, or drain.
- Detailed dependency diagnostics belong in `chaptera doctor`, not public
  health output.

## Caddy

The example Caddyfile:

- terminates HTTPS outside Chaptera;
- proxies only to `127.0.0.1:8080`;
- uses a finite 8 MiB ordinary API body bound;
- uses a separate finite 256 MiB streamed-upload fallback bound;
- relies on Caddy's native WebSocket proxy support.

Final Host/Origin/CORS/CSRF/CSP/HSTS policy remains owned by
`CLOUD-EDGE-01`.

## Raw PUB

The general worker may have object-store network access. The raw PUB parser
child must not inherit object-store credentials or network authority.

Conceptually:

```text
worker -> exact authorized object -> bounded file/fd -> sandboxed parser child
```

## Deploy transaction

1. Install an immutable versioned artifact under
   `/opt/chaptera/releases/<version>/chaptera`.
2. Verify artifact identity/signing policy.
3. Run `chaptera --version`, `chaptera doctor`, and migration status.
4. Verify the previous release is still within the declared schema rollback
   window.
5. Run operator-controlled migration if required.
6. Move `/opt/chaptera/current` to the new immutable release.
7. Restart worker/web.
8. Wait for readiness and run loopback/public smoke.
9. Prove one acknowledged canonical edit survives restart.
10. Keep the previous artifact until the rollback window closes.

Never overwrite the running binary in place.

## Drain

Web SIGTERM:

1. readiness false;
2. stop new expensive and semantic admission;
3. resolve any in-flight durable append;
4. stop new authority activation;
5. close/reject realtime sessions according to protocol;
6. passivate/release without a checkpoint storm;
7. exit inside the bounded stop deadline.

Worker SIGTERM:

1. stop new claims;
2. complete only at a defined safe barrier;
3. otherwise leave/release a reclaimable lease;
4. never turn ambiguous publication into success;
5. exit.

## Rollback

Rollback is only a binary/symlink operation while the durable schema remains
inside the declared backward-compatibility window. If migration crossed that
boundary, use the migration/recovery procedure instead of pretending that a
symlink reversal is safe.

## Acceptance

Final host closure must prove at least:

- direct external app-port access fails;
- HTTPS + WebSocket through the edge pass;
- incompatible schema produces not-ready;
- source upload -> quarantine -> validation -> durable binding -> project passes;
- ACKed edit survives web restart;
- killed worker lease is reclaimed without double publication;
- soft memory pressure sheds derived state before the hard parent limit;
- worker memory failure does not destroy web/durable truth;
- spool exhaustion rejects new heavy work while SQLite remains usable;
- SIGTERM during append produces a reconciled outcome;
- deliberately bad app release rolls back inside the compatibility window;
- normal logs contain no document payload, secret or signed URL.

## Public-safe status

This packet can land before the Rust runtime because it is a deployment
contract plus deterministic static validation. Do not mark `CLOUD-DEPLOY-01`
DONE until a real 2 GiB Linux acceptance host exercises the assembled runtime.
