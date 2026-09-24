# CLOUD-SERVER-RUNTIME-01 — Rust runtime shell

This is the first production-shaped Chaptera Cloud process shell in the active
`HeisLuka/rar` Rust workspace.

It is intentionally small. It does **not** implement document semantics,
authentication policy, durable RevisionStream storage, the job state machine,
object storage, schema migrations, or TLS.

## Binary surface

```text
chaptera serve
chaptera worker
chaptera migrate status
chaptera migrate up
chaptera doctor
chaptera --version
```

The package is `chaptera-server`; the installed binary name is `chaptera`.

## Network boundary

`serve` defaults to:

```text
127.0.0.1:8080
```

`CHAPTERA_LISTEN` may override the address, but the runtime rejects public or
unspecified listeners such as `0.0.0.0:8080`, `[::]:8080`, and public IPs.

Allowed addresses are loopback, RFC1918 IPv4, or IPv6 unique-local addresses.
The normal first-host deployment keeps the process on loopback behind Caddy.

## Routes

- `GET /live` — process/event-loop liveness.
- `GET /ready` — dependency readiness.
- `GET /version` — package version + build git identity.

The shell deliberately starts **live but not ready** while its required
producer adapters are absent.

## Dependency injection seams

`RuntimePorts` has explicit injected fields for:

- AuthN;
- AuthZ;
- RevisionStream durability;
- durable jobs;
- BlobStore;
- observability.

AuthN/AuthZ/RevisionStream/jobs/BlobStore are required for readiness.
Observability is reported but is not itself a traffic-readiness dependency.

The initial binary wires `RuntimePorts::unconfigured()`. This is deliberate:
merging the process shell must not pretend that CLOUD-AUTHN-01,
CLOUD-SQLITE-V0-01, CLOUD-ASYNC-RUNTIME-01, or CLOUD-BLOB-STORE-V0-01 already
exist.

## Delegated commands

`worker` delegates to a `WorkerRuntime` seam owned by
`CLOUD-ASYNC-RUNTIME-01`.

`migrate status|up` delegates to a `MigrationRuntime` seam owned by
`CLOUD-MIGRATION-01`.

Until those producers are connected, both fail closed with a bounded error.

`doctor` prints the read-only dependency report and exits non-zero while a
required producer is absent.

## Build identity

The binary reports:

```text
<package-version>+<git-sha>
```

The build script prefers an explicitly supplied `CHAPTERA_BUILD_GIT_SHA`; if
it is absent it reads the current Git HEAD. CI therefore ties the tested binary
to a concrete source revision.

## Graceful shutdown

On Unix, `serve` listens for SIGTERM or Ctrl-C and passes the signal into
Axum graceful shutdown. Acceptance tests also exercise the same
`run_with_listener` path with an injected shutdown future.

Later runtime producers remain responsible for their own drain barriers:
durable append resolution, worker lease disposition, authority passivation,
and realtime close semantics.

## Current limitation

This task proves the process/network/composition shell only. The expected V0
state after this task is:

```text
/live  -> 200
/ready -> 503 (until required producer adapters are connected)
```

A 503 readiness response is a correctness signal, not an implementation
failure.
