# Chaptera Cloud Rust MSRV

Canonical workspace minimum Rust version: **1.94.1**.

This floor is intentional. The production Cloud BlobStore provider is required to use the current maintained official AWS Rust SDK line rather than a stale compatibility pin. At the time this floor was adopted, current `aws-sdk-s3` and `aws-config` both declared `rust-version = 1.94.1`.

Rules:

- root `Cargo.toml` is the canonical MSRV declaration;
- authoritative Cloud workflows must test the same minimum, not only latest stable;
- do not lower the workspace MSRV by pinning an older AWS SDK generation;
- raising this floor again requires an explicit dependency reason and full Cloud regression;
- SQLx/SQLite, ingress, queue, BlobStore and server semantics are not changed by an MSRV bump.

The provider adapter and real S3/IAM/CORS receipt remain separate gates under `CLOUD-BLOB-STORE-V0-01`.
