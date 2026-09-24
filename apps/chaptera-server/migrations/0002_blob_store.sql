PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS physical_blobs (
    physical_blob_id       BLOB PRIMARY KEY,
    tenant_id              BLOB NOT NULL,
    content_sha256         BLOB NOT NULL,
    byte_len               INTEGER NOT NULL CHECK (byte_len > 0),
    canonical_mime         TEXT,
    object_namespace       TEXT NOT NULL CHECK (
        object_namespace IN ('quarantine', 'canonical', 'checkpoint', 'derived', 'export')
    ),
    object_locator         TEXT NOT NULL UNIQUE,
    storage_generation     TEXT NOT NULL,
    created_at_ms          INTEGER NOT NULL,
    delete_eligible_at_ms  INTEGER,
    deleted                INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    UNIQUE (tenant_id, content_sha256, byte_len)
);

CREATE INDEX IF NOT EXISTS physical_blobs_delete_eligible_idx
    ON physical_blobs (deleted, delete_eligible_at_ms);

CREATE TABLE IF NOT EXISTS resource_bindings (
    binding_id             BLOB PRIMARY KEY,
    tenant_id              BLOB NOT NULL,
    project_id             BLOB,
    document_id            BLOB,
    physical_blob_id       BLOB NOT NULL,
    content_sha256         BLOB NOT NULL,
    byte_len               INTEGER NOT NULL CHECK (byte_len > 0),
    resource_kind          TEXT NOT NULL CHECK (
        resource_kind IN ('pub_source', 'asset', 'checkpoint', 'derived_artifact', 'export_artifact')
    ),
    validation_profile     TEXT NOT NULL,
    lifecycle_state        TEXT NOT NULL CHECK (
        lifecycle_state IN ('active', 'retired', 'purge_eligible')
    ),
    created_at_ms          INTEGER NOT NULL,
    retired_at_ms          INTEGER,
    FOREIGN KEY (physical_blob_id) REFERENCES physical_blobs(physical_blob_id)
);

CREATE INDEX IF NOT EXISTS resource_bindings_tenant_project_idx
    ON resource_bindings (tenant_id, project_id, lifecycle_state);

CREATE INDEX IF NOT EXISTS resource_bindings_physical_idx
    ON resource_bindings (physical_blob_id, lifecycle_state);
