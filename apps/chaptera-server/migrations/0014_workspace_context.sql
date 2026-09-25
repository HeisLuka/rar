-- CLOUD-WORKSPACE-CONTEXT-01
-- Minimal V0 principal -> workspace -> tenant authority for authenticated routes.
-- WorkspaceId is a selector; TenantId is always resolved from durable server state.

CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id         BLOB PRIMARY KEY,
    tenant_id            BLOB NOT NULL,
    lifecycle_state      TEXT NOT NULL CHECK (lifecycle_state IN ('active', 'deleted')),
    lifecycle_generation INTEGER NOT NULL CHECK (lifecycle_generation >= 0),
    metadata_version     INTEGER NOT NULL CHECK (metadata_version >= 0),
    created_at_ms        INTEGER NOT NULL,
    CHECK (length(workspace_id) > 0),
    CHECK (length(tenant_id) > 0),
    CHECK (created_at_ms >= 0)
);

CREATE TABLE IF NOT EXISTS workspace_memberships (
    workspace_id       BLOB NOT NULL REFERENCES workspaces(workspace_id),
    principal_id       BLOB NOT NULL REFERENCES principals(principal_id),
    role               TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    membership_state   TEXT NOT NULL CHECK (membership_state IN ('active', 'revoked')),
    membership_version INTEGER NOT NULL CHECK (membership_version >= 0),
    created_at_ms      INTEGER NOT NULL,
    revoked_at_ms      INTEGER,
    PRIMARY KEY (workspace_id, principal_id),
    CHECK (created_at_ms >= 0),
    CHECK (
        (membership_state = 'active' AND revoked_at_ms IS NULL)
        OR
        (membership_state = 'revoked' AND revoked_at_ms IS NOT NULL AND revoked_at_ms >= created_at_ms)
    )
);

CREATE INDEX IF NOT EXISTS workspace_memberships_by_principal
    ON workspace_memberships (principal_id, membership_state, workspace_id);
