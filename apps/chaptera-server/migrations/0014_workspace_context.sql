-- CLOUD-WORKSPACE-CONTEXT-01
-- Minimal V0 workspace membership authority.
--
-- AuthN proves principal identity. This schema proves which active workspace
-- that principal may act in and binds the workspace to exactly one tenant.

CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id       BLOB PRIMARY KEY,
    tenant_id          BLOB NOT NULL,
    lifecycle_state    TEXT NOT NULL CHECK (lifecycle_state IN ('active', 'deleted')),
    created_at_ms      INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL,

    CHECK (length(workspace_id) > 0),
    CHECK (length(tenant_id) > 0),
    CHECK (created_at_ms >= 0),
    CHECK (updated_at_ms >= created_at_ms)
);

CREATE INDEX IF NOT EXISTS workspaces_tenant_idx
    ON workspaces (tenant_id, lifecycle_state);

CREATE TABLE IF NOT EXISTS workspace_memberships (
    workspace_id       BLOB NOT NULL REFERENCES workspaces(workspace_id),
    principal_id       BLOB NOT NULL REFERENCES principals(principal_id),
    state              TEXT NOT NULL CHECK (state IN ('active', 'revoked')),
    created_at_ms      INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL,
    revoked_at_ms      INTEGER,

    PRIMARY KEY (workspace_id, principal_id),

    CHECK (created_at_ms >= 0),
    CHECK (updated_at_ms >= created_at_ms),
    CHECK (
        (state = 'active' AND revoked_at_ms IS NULL)
        OR
        (state = 'revoked' AND revoked_at_ms IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS workspace_memberships_principal_idx
    ON workspace_memberships (principal_id, state, workspace_id);
