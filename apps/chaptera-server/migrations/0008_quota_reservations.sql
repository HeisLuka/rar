PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS quota_reservations (
    tenant_id             BLOB NOT NULL,
    reservation_id        BLOB NOT NULL,
    work_class            TEXT NOT NULL,
    amount                INTEGER NOT NULL,
    request_hash          BLOB NOT NULL,
    lease_generation      INTEGER NOT NULL,
    lease_expires_at_ms   INTEGER NOT NULL,
    released_at_ms        INTEGER,
    created_at_ms         INTEGER NOT NULL,
    updated_at_ms         INTEGER NOT NULL,
    PRIMARY KEY (tenant_id, reservation_id),
    CHECK (work_class IN ('interactive', 'export', 'background')),
    CHECK (amount > 0),
    CHECK (length(request_hash) = 64),
    CHECK (lease_generation > 0),
    CHECK (lease_expires_at_ms > created_at_ms),
    CHECK (created_at_ms >= 0),
    CHECK (updated_at_ms >= created_at_ms),
    CHECK (released_at_ms IS NULL OR released_at_ms >= created_at_ms)
);

CREATE INDEX IF NOT EXISTS quota_reservations_active_by_tenant
    ON quota_reservations(tenant_id, released_at_ms, lease_expires_at_ms, work_class);
