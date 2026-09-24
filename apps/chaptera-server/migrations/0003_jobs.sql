PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS jobs (
  job_id                 BLOB PRIMARY KEY,
  tenant_id              BLOB NOT NULL,
  job_kind               TEXT NOT NULL CHECK (
    job_kind IN ('parse','export','snapshot','projection','blob_gc')
  ),
  payload_schema_version INTEGER NOT NULL CHECK (payload_schema_version > 0),
  payload                BLOB NOT NULL,
  request_hash           BLOB NOT NULL,
  status                 TEXT NOT NULL CHECK (
    status IN ('queued','running','succeeded','failed','cancelled')
  ),
  available_at_ms        INTEGER NOT NULL,
  attempt                INTEGER NOT NULL CHECK (attempt >= 0),
  max_attempts            INTEGER NOT NULL CHECK (max_attempts > 0),
  lease_owner            TEXT,
  lease_generation       INTEGER NOT NULL CHECK (lease_generation >= 0),
  lease_expires_at_ms    INTEGER,
  cancel_requested_at_ms INTEGER,
  idempotency_key        BLOB NOT NULL,
  created_at_ms          INTEGER NOT NULL,
  started_at_ms          INTEGER,
  finished_at_ms         INTEGER,
  terminal_code          TEXT,
  UNIQUE (tenant_id, job_kind, idempotency_key)
);

CREATE INDEX IF NOT EXISTS jobs_ready
  ON jobs(status, available_at_ms, created_at_ms, job_id);

CREATE INDEX IF NOT EXISTS jobs_expired_lease
  ON jobs(status, lease_expires_at_ms);

CREATE TABLE IF NOT EXISTS job_effects (
  job_id            BLOB NOT NULL REFERENCES jobs(job_id),
  effect_key        BLOB NOT NULL,
  lease_generation  INTEGER NOT NULL,
  published_at_ms   INTEGER NOT NULL,
  PRIMARY KEY (job_id, effect_key)
);
