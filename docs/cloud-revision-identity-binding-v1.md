# Cloud revision identity binding V1

`CLOUD-REVISION-IDENTITY-BINDING-01` persists one explicit relationship that
the Cloud runtime needs but is not allowed to infer:

```text
document + service/history RevisionId
  -> REVISION-MODEL-01 canonical AuthoringRevisionId
```

This is an identity bridge, not another revision model.

## Why the mapping is explicit

Chaptera intentionally has several distinct identities:

- durable service/history RevisionId in the Cloud RevisionStream;
- canonical Engine AuthoringRevisionId from REVISION-MODEL-01;
- AuthoringRootHash / state identity;
- canonical EditorProject hash;
- physical snapshot/artifact identities.

The merged derived-artifact fence contains both service/history revision identity
and canonical authoring revision identity. Therefore the runtime must not use a
service RevisionId, AuthoringRootHash, or project hash as a substitute merely
because every value happens to look hash-shaped.

## Canonical authority boundary

The canonical V1 implementation in
`pub-rs@067d3bf5c0698a5bb72f03a36d33ae13ec2a8018` defines:

- schema `chaptera.cdm.authoring-revision.v1`;
- AuthoringRevisionId as a transparent SHA-256 digest serialized as 64 lowercase
  hexadecimal characters;
- the actual derivation as an Engine-owned domain-separated hash over canonical
  authoring state and parent revision.

Cloud validates the schema tag and digest shape only. It does **not** copy the
Engine derivation law or attempt to rebuild the Engine graph in order to bind an
identity. The authoritative revision producer supplies the canonical id.

## Durable law

Migration `0009_revision_identity_bindings.sql` adds an immutable,
document-scoped mapping keyed by:

`(document_id, service_revision_id)`.

The laws are:

1. First valid mapping is durable.
2. Exact retry with the same canonical id is idempotent.
3. The retry returns the original historical row/timestamp.
4. A different canonical id for the same document + service revision fails
   closed with `revision_identity_conflict`.
5. The same service revision token in another document is independent.
6. Historical mappings remain readable after newer revisions are bound.
7. Restart preserves the exact mapping.
8. Missing mapping fails with `canonical_revision_unbound`.

## Materializer integration

`ExactRevisionMaterializer` still verifies the exact immutable source,
contiguous historical RevisionStream prefix, canonical EditorSession replay,
project hash and optional authoring root.

After those checks it now resolves the explicit identity binding for the exact
requested service/history revision. A successful receipt carries two clearly
named fields:

- `requested_revision_id` — durable service/history revision requested by the
  Cloud caller;
- `canonical_authoring_revision_id` — mapped REVISION-MODEL-01 identity;
- `canonical_revision_schema_version` — the versioned canonical schema tag.

If the mapping is absent, the materializer returns no publishable receipt. It
does not fall back to `child_revision`, `authoring_root_hash`, or
`project_sha256`.

## Export boundary

A future Cloud export executor must construct `DerivedArtifactFenceV1` with:

- `service_revision_id` from the durable Cloud history/job identity;
- `canonical_revision_id` only from this explicit mapping;
- state/root/project hashes only as independent verification or input
  fingerprints.

This prevents a semantically false exact-revision fence while keeping all
revision hash authorities independently versioned.
