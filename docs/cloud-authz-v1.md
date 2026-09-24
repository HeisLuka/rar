# CLOUD-AUTHZ-01 — document authorization V1

This bounded Rar slice defines the service-side authorization contract used by
document commands and realtime subscriptions.

## Authority split

- `RevisionKernel` owns semantic revision and state identity.
- `AuthzKernel` owns access grants, capability resolution, live revocation,
  scoped share grants, and payload-free access audit.
- `authz_version` is intentionally **not** part of RevisionId or StateId.
- Tenant isolation remains a lower storage/job boundary; this AuthZ slice uses
  tenant identity but does not claim deployment-level object/IAM closure.

## Roles

Product presets map to explicit capabilities:

- Viewer: view.
- Commenter: view + comment read/write.
- Editor: commenter capabilities + semantic edit, text/geometry edit, asset
  upload and export.
- Owner: editor capabilities + share/member management and destructive delete.

Services authorize capabilities rather than branching on role names.

## Revocation barrier

Authorization changes and durable authorized actions share a per-document
barrier. If a commit acquired the barrier first, it may linearize before a
concurrent revoke. Once `revoke()` returns with
`active_session_barrier_complete=true`, no action admitted under the previous
access generation can still be executing behind that barrier.

This explicitly separates:

1. grant mutation becoming durable; and
2. the stronger active-session revoke barrier completing.

## Revision integration

`AuthorizedRevisionGateway` checks the required capability while holding the
AuthZ document barrier, then calls the existing `RevisionKernel`. Browser
requests still cannot supply canonical before-state. AuthZ failure occurs before
the authoritative semantic executor and therefore cannot advance revision
state.

## Realtime integration

Subscriptions are opened only with `document.view`. Grant revoke/downgrade
re-evaluates active subscriptions under the same document barrier; denied
subscriptions are marked inactive and excluded from fanout.

## Share grants

V1 share grants are tenant/document/grant bound, HMAC authenticated, expiring,
server-state backed, and revocable. Only viewer/commenter share roles are
admitted in this bounded slice.

## Audit

Audit events contain structural identity, action, result, capability,
`authz_version`, and bounded error code. They do not contain Story text, raw
PUB bytes, asset bytes, semantic commands, signed URLs, tokens, or document
payload.

## What this does not close

This is the command/realtime service integration slice for `CLOUD-AUTHZ-01`.
The umbrella task still depends on the remaining physical tenant-isolation
receipts (object/IAM/signed-delivery and disposable-temp cleanup). Do not mark
the full security gate DONE until those producers close.
