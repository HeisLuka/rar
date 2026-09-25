-- CLOUD-PROJECT-PERSISTENCE-01
-- Durable project/document lifecycle identity for atomic CreateProjectFromUpload.
--
-- upload_consumptions remains the authoritative idempotent source-consumption
-- receipt and source-authority bridge. These tables persist product lifecycle
-- identity without creating a second RevisionStream authority.

CREATE TABLE IF NOT EXISTS projects (
    project_id          BLOB PRIMARY KEY,
    tenant_id           BLOB NOT NULL,
    workspace_id        BLOB NOT NULL,
    name                TEXT NOT NULL,
    created_at_ms       INTEGER NOT NULL,

    CHECK (length(project_id) > 0),
    CHECK (length(tenant_id) > 0),
    CHECK (length(workspace_id) > 0),
    CHECK (length(name) > 0),
    CHECK (length(name) <= 512),
    CHECK (created_at_ms >= 0)
);

CREATE INDEX IF NOT EXISTS projects_tenant_workspace_idx
    ON projects (tenant_id, workspace_id, created_at_ms);

CREATE TABLE IF NOT EXISTS documents (
    document_id             BLOB PRIMARY KEY,
    tenant_id               BLOB NOT NULL,
    project_id              BLOB NOT NULL UNIQUE REFERENCES projects(project_id),
    source_upload_id        BLOB NOT NULL UNIQUE REFERENCES uploads(upload_id),
    durable_binding_id      BLOB NOT NULL,
    source_sha256           BLOB NOT NULL,
    genesis_revision_id     BLOB NOT NULL,
    created_at_ms           INTEGER NOT NULL,

    UNIQUE (tenant_id, genesis_revision_id),

    CHECK (length(document_id) > 0),
    CHECK (length(tenant_id) > 0),
    CHECK (length(durable_binding_id) > 0),
    CHECK (length(source_sha256) = 64),
    CHECK (length(genesis_revision_id) > 0),
    CHECK (created_at_ms >= 0)
);

CREATE INDEX IF NOT EXISTS documents_tenant_project_idx
    ON documents (tenant_id, project_id);
