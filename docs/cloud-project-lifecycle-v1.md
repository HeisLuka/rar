# CLOUD-PROJECT-LIFECYCLE-01 — saved project lifecycle V1

This public reference implementation defines the ordinary Cloud Project control-plane lifecycle above RevisionStream.

## Identity layers

A saved project has a user-facing `ProjectId` and exactly one canonical authoring `DocumentId` in V1. Metadata operations do not become semantic document revisions.

## Split concurrency fences

Two counters are deliberately independent:

- `lifecycle_generation` fences admission/lifecycle state such as trash, restore and hard delete.
- `metadata_version` fences rename and same-tenant directory/workspace metadata races.

Rename/move do not advance `lifecycle_generation` and do not change `DocumentId` or semantic revision identity. This preserves active edit-session lifecycle tokens while still rejecting stale metadata writes.

## Operations

- create/import → new ProjectId + DocumentId;
- rename → metadata only;
- same-tenant workspace move → metadata only;
- cross-tenant identity-preserving move → not V0, fail closed;
- trash/restore → same ProjectId/DocumentId, lifecycle generation advances;
- hard delete → terminal, identity is never reusable;
- fork/save-a-copy → new ProjectId + DocumentId + genesis revision.

Fork does not inherit grants/comments by default. Asset bytes may be physically reusable, but the fork receives new logical bindings.

## Idempotency

Every retryable command has an explicit request id.

- same id + same semantic request → same result;
- same id + different semantic request → `idempotency_conflict`.

## Semantic revision fence

Project metadata/lifecycle mutations never synthesize a document semantic RevisionId. Fork is different: it creates a new document and a new genesis revision derived from the selected source revision.

## Limits

This closes the public control-plane contract/reference implementation only. It does not prove deployed AuthZ, distributed storage transactions, cross-region lifecycle ownership, asset binding authorization, job cancellation, comments retention or hard-delete execution.
