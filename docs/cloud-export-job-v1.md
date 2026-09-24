# CLOUD-EXPORT-JOB-01 — durable exact-revision export job V1

User-requested export is durable work pinned to one exact canonical revision. It is not a latest-wins rebuildable projection.

## Identity and idempotency

CreateExport binds tenant, document, exact RevisionId, target/profile and Layout Environment. The client request id is idempotent: exact retry returns the same job; same id with changed semantic input fails closed.

## Durable lifecycle

`queued → running → succeeded | failed | cancelled`.

Workers claim via a lease generation. A crashed/stale worker lease may expire and the same durable job may return to queued/reclaim. Stale workers cannot publish after another lease generation takes ownership.

## Authorization barriers

Authorization is rechecked at create, worker claim, publication and download.

A revoke before publish prevents an artifact binding from becoming visible. A revoke after success prevents later download. Physical content hash is never authorization.

## Cancellation

Queued cancellation is immediate. Running cancellation records `cancel_requested`; worker observes it at the pre-publication barrier. Cancellation after succeeded is too late and does not rewrite historical success.

## Artifact identity

Succeeded job points to a logical immutable artifact binding and an exact LossReport hash. Physical deterministic bytes may be reused across retries/tenants only behind separate authorized logical bindings.

Artifact expiry disables download but keeps the durable job history as succeeded.

## Limits

The public Rar implementation is an executable service contract. It does not select a queue/blob provider, run the real Chaptera exporter, measure AuthZ propagation latency or define production retention/SLO/tier limits.
