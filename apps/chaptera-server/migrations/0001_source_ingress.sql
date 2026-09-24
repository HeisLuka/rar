PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS uploads (
    upload_id               BLOB PRIMARY KEY,
    tenant_id               BLOB NOT NULL,
    principal_id            BLOB NOT NULL,
    purpose                 TEXT NOT NULL CHECK (purpose IN ('pub_source')),
    expected_byte_len       INTEGER NOT NULL CHECK (expected_byte_len > 0),
    declared_content_type   TEXT,
    physical_upload_ref     TEXT NOT NULL,
    state                   TEXT NOT NULL CHECK (
        state IN (
            'ISSUED',
            'STORED_UNVERIFIED',
            'VALIDATING',
            'VALIDATED_DURABLE',
            'CONSUMED',
            'REJECTED',
            'EXPIRED'
        )
    ),
    upload_generation       INTEGER NOT NULL CHECK (upload_generation >= 0),
    object_version          TEXT,
    object_etag             TEXT,
    observed_byte_len       INTEGER,
    canonical_sha256        BLOB,
    durable_binding_id      BLOB,
    created_at_ms           INTEGER NOT NULL,
    expires_at_ms           INTEGER NOT NULL,
    completed_at_ms         INTEGER,
    terminal_code           TEXT,
    idempotency_key         BLOB NOT NULL,
    request_hash            BLOB NOT NULL,
    UNIQUE (tenant_id, idempotency_key),
    CHECK (expires_at_ms > created_at_ms),
    CHECK (
        (state IN ('ISSUED') AND canonical_sha256 IS NULL AND durable_binding_id IS NULL)
        OR state <> 'ISSUED'
    ),
    CHECK (
        state NOT IN ('VALIDATED_DURABLE', 'CONSUMED')
        OR (
            canonical_sha256 IS NOT NULL
            AND durable_binding_id IS NOT NULL
            AND observed_byte_len IS NOT NULL
        )
    )
);

CREATE INDEX IF NOT EXISTS uploads_expiry_state_idx
    ON uploads (state, expires_at_ms);

CREATE INDEX IF NOT EXISTS uploads_tenant_state_idx
    ON uploads (tenant_id, state, created_at_ms);

-- CreateProjectFromUpload retry/response identity is durable separately from the
-- upload row so a crash after idempotent project creation but before the upload
-- state update can converge without manufacturing a second ProjectId.
CREATE TABLE IF NOT EXISTS upload_consumptions (
    upload_id               BLOB PRIMARY KEY REFERENCES uploads(upload_id),
    tenant_id               BLOB NOT NULL,
    idempotency_key         BLOB NOT NULL,
    request_hash            BLOB NOT NULL,
    project_id              BLOB NOT NULL,
    document_id             BLOB NOT NULL,
    genesis_revision_id     BLOB NOT NULL,
    committed_at_ms         INTEGER NOT NULL,
    UNIQUE (tenant_id, idempotency_key)
);
