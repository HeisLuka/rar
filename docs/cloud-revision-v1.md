# CLOUD-REVISION-01 — canonical revision bridge V1

Rar has two legitimate revision identities and must not conflate them.

- The Web/service `RevisionKernel` identifies immutable service history nodes.
- `REVISION-MODEL-01` identifies canonical authoring revisions from the canonical graph and parent RevisionId.

`services/editor-api/cloud_revision_v1.py` binds these identities explicitly. A service revision is mapped to exactly one canonical authoring RevisionId. The hashes are intentionally allowed to differ.

## SemanticDiffV1 admission

The bridge consumes the serialized public contract from `pub-model/revision.rs`:

- schema `chaptera.cdm.authoring-revision.v1`;
- exact canonical base RevisionId;
- canonical operation ordering;
- duplicate target rejection;
- `update_node_bounds` and `update_story_text` mutation classes;
- add/remove/rebind remain fail-closed until the canonical Engine supports them.

The Rar base service revision must already be bound to the SemanticDiffV1 canonical base. The authoritative executor returns a canonical child receipt whose parent must equal that base. Only after all existing `RevisionKernel` validation succeeds is the new service revision bound to the canonical child.

This does not reimplement `apply_source_graph_diff_v1`; the executor remains the canonical mutation authority.

## Derived-artifact fence

Derived layout, scene, preview and export material is keyed by:

`document + service revision + canonical revision + stage + stage version + environment fingerprint + input fingerprint`.

A changed exact fence is a cache miss/rebuild requirement, not destructive global invalidation. Historical exact-fence artifacts remain addressable. Publishing different bytes under the same exact fence is a deterministic-build conflict.

## Export binding

The contract receipt passes the canonical child RevisionId into the existing durable Cloud export job as its `exact_revision_id`, and passes the same layout environment fingerprint into `layout_environment_id`. Export is therefore pinned to the canonical authoring state rather than an ambiguous latest service head.

## Scope

This slice is executable integration of the current Rar service contracts. It is not deployment proof and does not claim a durable production Scene/preview artifact provider. Final CLOUD-REVISION-01 closure still requires the same identity/fence binding in the production Chaptera persistence and worker routes.
