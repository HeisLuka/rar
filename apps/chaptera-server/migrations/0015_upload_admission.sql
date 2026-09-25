-- CLOUD-UPLOAD-ADMISSION-01
-- Restart-safe reservation authority for HTTP source-upload admission.
--
-- This is intentionally distinct from quota_reservations: those rows protect
-- semantic/job work classes, while these rows fence principal/tenant upload
-- concurrency and declared ingress bytes before object-store admission.

CREATE TABLE IF NOT EXISTS upload_admission_reservations (
    reservation_id       BLOB PRIMARY KEY,
    tenant_id            BLOB NOT NULL,
    principal_id         BLOB NOT NULL REFERENCES principals(principal_id),
    expected_bytes       INTEGER NOT NULL CHECK (expected_bytes > 0),
    request_hash         BLOB NOT NULL,
    lease_generation     INTEGER NOT NULL CHECK (lease_generation >= 0),
    lease_expires_at_ms  INTEGER NOT NULL CHECK (lease_expires_at_ms >= 0),
    released_at_ms       INTEGER,
    created_at_ms        INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms        INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),

    CHECK (length(reservation_id) > 0),
    CHECK (length(tenant_id) > 0),
    CHECK (length(principal_id) > 0),
    CHECK (length(request_hash) = 64),
    CHECK (released_at_ms IS NULL OR released_at_ms >= created_at_ms)
);

CREATE INDEX IF NOT EXISTS upload_admission_tenant_active_idx
    ON upload_admission_reservations (
        tenant_id, released_at_ms, lease_expires_at_ms
    );

CREATE INDEX IF NOT EXISTS upload_admission_principal_active_idx
    ON upload_admission_reservations (
        principal_id, released_at_ms, lease_expires_at_ms
    );

CREATE INDEX IF NOT EXISTS upload_admission_cleanup_idx
    ON upload_admission_reservations (
        released_at_ms, lease_expires_at_ms, updated_at_ms
    );
