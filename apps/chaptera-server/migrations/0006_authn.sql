PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS principals (
    principal_id   BLOB PRIMARY KEY,
    created_at_ms  INTEGER NOT NULL,
    disabled_at_ms INTEGER,
    CHECK (created_at_ms >= 0),
    CHECK (disabled_at_ms IS NULL OR disabled_at_ms >= created_at_ms)
);

CREATE TABLE IF NOT EXISTS principal_identities (
    issuer         TEXT NOT NULL,
    subject        TEXT NOT NULL,
    principal_id   BLOB NOT NULL,
    email_snapshot TEXT,
    linked_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (issuer, subject),
    FOREIGN KEY (principal_id) REFERENCES principals(principal_id),
    CHECK (length(issuer) > 0),
    CHECK (length(subject) > 0),
    CHECK (linked_at_ms >= 0)
);

CREATE TABLE IF NOT EXISTS sessions (
    session_id_hash        BLOB PRIMARY KEY,
    principal_id           BLOB NOT NULL,
    csrf_token_hash        BLOB NOT NULL,
    session_generation     INTEGER NOT NULL,
    created_at_ms           INTEGER NOT NULL,
    last_seen_at_ms         INTEGER NOT NULL,
    idle_expires_at_ms      INTEGER NOT NULL,
    absolute_expires_at_ms  INTEGER NOT NULL,
    revoked_at_ms           INTEGER,
    FOREIGN KEY (principal_id) REFERENCES principals(principal_id),
    CHECK (length(session_id_hash) = 32),
    CHECK (length(csrf_token_hash) = 32),
    CHECK (session_generation > 0),
    CHECK (created_at_ms >= 0),
    CHECK (last_seen_at_ms >= created_at_ms),
    CHECK (idle_expires_at_ms > created_at_ms),
    CHECK (absolute_expires_at_ms >= idle_expires_at_ms),
    CHECK (revoked_at_ms IS NULL OR revoked_at_ms >= created_at_ms)
);

CREATE INDEX IF NOT EXISTS sessions_by_principal
    ON sessions(principal_id, revoked_at_ms, absolute_expires_at_ms);
