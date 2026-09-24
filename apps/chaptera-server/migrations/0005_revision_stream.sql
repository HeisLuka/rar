-- CLOUD-MIGRATION-01 / CLOUD-SQLITE-V0-01
-- Operator-applied physical RevisionStream V2 schema.
-- The adapter-local schema_migrations table remains for compatibility with the
-- first SQLite RevisionStore implementation; the global migration authority is
-- chaptera_schema_migrations managed by schema_migration.rs.

CREATE TABLE IF NOT EXISTS schema_migrations (
    version       INTEGER PRIMARY KEY,
    applied_at_ms INTEGER NOT NULL
);

INSERT OR IGNORE INTO schema_migrations(version, applied_at_ms)
VALUES (1, 0);

CREATE TABLE IF NOT EXISTS revision_edges (
    document_id              BLOB NOT NULL,
    parent_revision          BLOB NOT NULL,
    parent_cursor            INTEGER NOT NULL,
    operation_id             BLOB NOT NULL,
    request_hash             BLOB NOT NULL,
    canonical_event          BLOB NOT NULL,
    child_revision           BLOB NOT NULL,
    child_cursor             INTEGER NOT NULL,
    resulting_state_hash     BLOB NOT NULL,
    authoring_root_hash      BLOB,
    semantic_schema_version  INTEGER NOT NULL,
    committed_at_ms          INTEGER NOT NULL,
    PRIMARY KEY (document_id, parent_revision),
    UNIQUE (document_id, child_revision),
    UNIQUE (document_id, child_cursor),
    CHECK (child_cursor = parent_cursor + 1),
    CHECK (semantic_schema_version > 0),
    CHECK (committed_at_ms >= 0)
);
