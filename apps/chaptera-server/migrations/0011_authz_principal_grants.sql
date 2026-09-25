CREATE TABLE IF NOT EXISTS authz_documents (
    tenant_id       BLOB NOT NULL,
    document_id     BLOB NOT NULL,
    authz_version   INTEGER NOT NULL CHECK (authz_version >= 0),
    PRIMARY KEY (tenant_id, document_id)
);

CREATE TABLE IF NOT EXISTS authz_principal_grants (
    tenant_id       BLOB NOT NULL,
    document_id     BLOB NOT NULL,
    principal_id    BLOB NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('viewer', 'commenter', 'editor', 'owner')),
    expires_at_ms   INTEGER CHECK (expires_at_ms IS NULL OR expires_at_ms >= 0),
    updated_at_ms   INTEGER NOT NULL CHECK (updated_at_ms >= 0),
    PRIMARY KEY (tenant_id, document_id, principal_id),
    FOREIGN KEY (tenant_id, document_id)
        REFERENCES authz_documents(tenant_id, document_id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS authz_audit_events (
    event_id         INTEGER PRIMARY KEY AUTOINCREMENT,
    tenant_id        BLOB NOT NULL,
    document_id      BLOB NOT NULL,
    principal_id     BLOB NOT NULL,
    operation_id     BLOB NOT NULL,
    action           TEXT NOT NULL,
    result           TEXT NOT NULL CHECK (result IN ('allowed', 'denied')),
    capability       TEXT NOT NULL,
    authz_version    INTEGER NOT NULL CHECK (authz_version >= 0),
    error_code       TEXT,
    created_at_ms    INTEGER NOT NULL CHECK (created_at_ms >= 0)
);

CREATE INDEX IF NOT EXISTS authz_grants_principal_idx
    ON authz_principal_grants (tenant_id, principal_id, document_id);

CREATE INDEX IF NOT EXISTS authz_audit_document_idx
    ON authz_audit_events (tenant_id, document_id, event_id);
