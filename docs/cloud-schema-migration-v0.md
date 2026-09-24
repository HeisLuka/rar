# CLOUD-MIGRATION-01 — single-host schema migration contract

This is the first operator-controlled schema migration runtime for the Chaptera
Cloud single-host SQLite lane.

## Commands

```text
CHAPTERA_SQLITE_PATH=/var/lib/chaptera/chaptera.sqlite chaptera migrate status
CHAPTERA_SQLITE_PATH=/var/lib/chaptera/chaptera.sqlite chaptera migrate up
```

`CHAPTERA_SQLITE_PATH` is mandatory for both commands. Ordinary `chaptera
serve` does not invoke `migrate up`.

## Current ordered schema

The current binary knows these checksummed migrations:

1. `0001_source_ingress.sql`
2. `0002_blob_store.sql`
3. `0003_jobs.sql`
4. `0004_blob_gc.sql`
5. `0005_revision_stream.sql`

The global operator ledger is:

```sql
chaptera_schema_migrations(
  version INTEGER PRIMARY KEY,
  name TEXT UNIQUE NOT NULL,
  checksum_sha256 TEXT NOT NULL,
  applied_at_ms INTEGER NOT NULL
)
```

Each applied row is bound to the exact SQL bytes embedded in the binary. If a
historical migration file is edited after application, `status` and `up`
fail closed with a checksum mismatch.

## Locking

`migrate up` acquires a SQLite `BEGIN IMMEDIATE` transaction before creating
or advancing the migration ledger. This gives the V0 single-host migration
runner one write owner. A second runner waits under the bounded SQLite busy
timeout and then observes the already-applied history.

All currently known V0 migrations are small DDL/bootstrap migrations, so one
transaction is intentionally preferred to a partially-applied schema.

## Status is read-only

If the DB file does not exist, `migrate status` reports schema 0 with all
versions pending and **does not create the database**.

If known Chaptera tables exist but the global migration ledger does not,
Chaptera refuses implicit adoption. An unversioned production database needs a
separate explicit adoption/reconciliation procedure; the migration command must
not silently bless unknown DDL.

## Compatibility and rollback

The V0 migration receipt reports the current and target schema versions.

There is currently **no declared cross-schema previous-binary compatibility
window**. Therefore a deployment that changes schema must retain a
pre-migration database backup and must not assume that moving the application
symlink back is a safe database rollback.

This matches CLOUD-DEPLOY-01: binary rollback is allowed only inside a declared
schema compatibility window. Otherwise recovery restores the compatible
database state under the recovery runbook.

Future changes that remove/rename data must use expand/migrate/contract (or an
equivalent staged policy). Destructive downgrade is not implemented.

## RevisionStream compatibility shim

`0005_revision_stream.sql` also creates the small adapter-local
`schema_migrations` row used by the first `SqliteRevisionStore`. This is a
transition shim so the operator migration can fully pre-provision the existing
adapter. Global schema authority is `chaptera_schema_migrations`.

## Acceptance

Tests cover:

- absent DB status is non-mutating;
- complete forward migration;
- idempotent rerun;
- checksum tamper rejection;
- unknown/gapped future history rejection;
- restart after every migration boundary followed by convergence to current;
- concurrent `migrate up` serialization;
- refusal of known unversioned Chaptera tables.

CI additionally drives the built `chaptera` binary against a temporary
production-shaped SQLite file and proves the public operator command surface.
