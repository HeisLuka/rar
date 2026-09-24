# Chaptera SQLite single-host V0

```text
SQLite lane = first 2 GiB single-host durable adapter
             ≠ permanent global database decision
```

## Supported deployment

The canonical SQLite database MUST live on local block storage owned by the Chaptera host.

Supported V0 profile:

- `journal_mode = WAL`
- `synchronous = FULL`
- `foreign_keys = ON`
- bounded `busy_timeout`
- intentionally small connection pool
- one SQLite writer at a time; write-heavy background work remains bounded

Do not deploy the canonical WAL database on NFS, SMB, FUSE object-store mounts, network home directories or other filesystems whose locking/durability semantics have not been accepted explicitly. If the operator cannot establish local-block semantics, this adapter is not an approved deployment mode.

## Durability meaning

An accepted Chaptera revision means the revision edge is durably committed before the server returns the accepted result. The adapter must not silently weaken production durability to `synchronous=NORMAL`.

There is no mutable canonical HEAD row. RevisionStream V2 remains an immutable create-once edge set keyed by `(document_id, parent_revision)`.

On a same-parent collision:

1. read the exact existing edge;
2. exact operation/request/canonical identity → `AlreadyCommitted`;
3. any changed retry → `Conflict`;
4. never guess a newer head.

## Canonical event codec

`canonical_event` is stored as a versioned binary envelope (`CHREV2`) containing:

- explicit payload length;
- SHA-256 of exact payload bytes;
- exact canonical payload bytes.

Truncation, trailing bytes, hash mismatch or unsupported magic fail closed during replay.

## Replaceability

The Chaptera semantic contract is the repository/RevisionStream contract, not SQLite. PostgreSQL, YDB or another network database may implement the same append/read semantics later without changing document or operation meaning.
