# LOCAL-PRODUCT-API-HTTP-01

Coordination bootstrap for the draft PR. Replace or remove this file before the PR is marked ready if the implementation no longer needs it.

Source of truth: Notion task LOCAL-PRODUCT-API-HTTP-01.

## Goal

Add one bounded Rust product router over the landed authority/storage layers. The HTTP layer is composition only and must not become a new source of truth.

## Required acceptance

- Mount through the existing serve::router_with_edge_auth_local_and_product seam.
- Expose authenticated current/open for one document.
- Expose authenticated + CSRF MoveNode commit using the existing chaptera.commit-request.v1 intent shape.
- Resolve document -> tenant through existing durable document authority.
- Derive current head from baseline plus verified RevisionStream. Do not add a mutable current-head table.
- Materialize the exact base through ExactRevisionMaterializer.
- Let canonical EditorSession derive MoveNode before/after from node_id/x_emu/y_emu.
- Derive service/canonical revision identities using existing laws.
- ACK only after SqliteAuthorizedRevisionCommitter durably commits the exact edge plus identity binding under AuthZ.
- Preserve idempotent retry.
- Reject stale parent.
- Reject unauthorized and missing/invalid CSRF.
- Add HTTP acceptance covering open, accepted MoveNode, exact retry replay, stale-parent rejection, unauthorized rejection, and CSRF rejection.

## Client must not provide authority

The mutation body must not be trusted for:

- before-state / before geometry;
- tenant_id or workspace identity;
- child revision;
- resulting state hash;
- authz_version.

## Non-goals

- A second document model.
- A second replay engine.
- A second current-head authority.
- Export/job expansion.
- Synthetic fake-ready routes.

## Proven prerequisite

- #901 AuthZ <-> RevisionStream transaction barrier: landed.
