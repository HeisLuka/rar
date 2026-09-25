# Cloud exact revision materializer V1

`CLOUD-REVISION-MATERIALIZER-01` turns one exact durable RevisionStream
revision into the authoritative canonical `EditorProject` for that revision.
It never substitutes the current/latest head.

## Authority split

The materializer composes existing Rar authorities instead of creating another
revision or edit model:

1. `DocumentSourceAuthority` resolves
   `tenant + document -> immutable binding + source SHA-256 + baseline revision`.
2. `BlobStoreExactSourceLoader` reuses `BlobStoreService::stream_binding_verified`
   to read the exact active tenant-safe immutable binding.
3. `SqliteRevisionStore::load_chain_to_revision` reads only the contiguous
   RevisionStream prefix from the authorized baseline through the exact requested
   revision.
4. The CHREV2 payload carries `EditorRevisionEventV1`, whose semantic operation is
   the existing `pub_editor::EditOperation`.
5. `PubEditorReplayEngine` opens the immutable PUB with the existing
   `pub_editor::open_mature_0x2c_editor` and replays the accumulated
   `EditorProject` through `EditorSession::apply_project`.

There is no new RevisionId law and no second edit-operation model.

## Exact historical prefix rule

For a request for historical revision `Rk`, the SQLite adapter first resolves
the exact child cursor of `Rk` and then loads only rows from the authorized
baseline through that cursor.

A later `Rn` is irrelevant. A corrupt event after `Rk` must not make
materialization of already-named `Rk` depend on the newer tail.

Gaps, wrong parent revisions or cursor discontinuities before `Rk` fail closed.

## Event and state binding

`chaptera.editor-revision-event.v1` contains:

- immutable source SHA-256;
- canonical project hash before the operation;
- canonical project hash after the operation;
- optional canonical authoring root hash;
- one existing `pub_editor::EditOperation`.

The durable edge independently stores `resulting_state_hash` and optional
`authoring_root_hash`. Materialization requires both copies to agree with
fresh canonical replay.

The project hash is not a new hashing convention. It is the raw 64-hex SHA-256
of the existing Rar canonical JSON project representation (the same bytes used
by the Python `RevisionKernel` project-hash law before its `sha256:` prefix).
The known SampleNewsletter baseline therefore remains:

`575fbcb664f2a6b672a05861a4d2aff6aca339204a50e3401d04e1920d946348`.

## V1 fail-closed boundary

Materialization fails without a state when any of these are observed:

- tenant/document/source binding mismatch;
- unavailable or hash/length-mismatched immutable source bytes;
- requested revision absent from this document;
- RevisionStream gap before the requested revision;
- corrupt CHREV2 envelope;
- unknown event schema or semantic schema version;
- event source mismatch;
- before-project hash mismatch;
- canonical `EditorSession` replay rejection;
- event after-project hash mismatch;
- durable `resulting_state_hash` mismatch;
- authoring-root mismatch.

`ReplaceImage` is intentionally not materialized by V1 because exact replay
requires immutable replacement-asset bytes. It fails closed rather than
silently replaying metadata without the asset authority.

## Hosted real-PUB proof

The repository does not commit the Publisher fixture bytes. Hosted acceptance
downloads the public Apache POI test fixture from an exact repository commit:

- repository: `apache/poi`;
- commit: `942d95d85b15d0dfdb3bc9ba1b4f273f277757c8`;
- path: `test-data/publisher/SampleNewsletter.pub`;
- expected length: `291840`;
- expected SHA-256:
  `6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf`.

CI refuses the fixture unless both length and SHA-256 match. It then proves with
the real `pub-editor` parser/session:

- exact baseline R0;
- real MoveNode R1;
- second MoveNode R2;
- historical R1 while R2 exists;
- byte-identical deterministic R1 materialization receipt on restart/replay;
- historical R1 remains materializable after the later R2 event bytes are
  deliberately corrupted, while requesting R2 fails closed.

The synthetic matrix remains useful only for negative contract cases; it is not
presented as proof that arbitrary PUB bytes open successfully.
