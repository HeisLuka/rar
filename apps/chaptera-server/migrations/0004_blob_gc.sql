PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS gc_candidates (
  candidate_id          BLOB PRIMARY KEY,
  tenant_id             BLOB NOT NULL,
  object_kind           TEXT NOT NULL CHECK (
    object_kind IN (
      'physical_blob','quarantine_upload','temp_object',
      'derived_artifact','export_artifact'
    )
  ),
  object_id             BLOB NOT NULL,
  reason_code           TEXT NOT NULL,
  not_before_ms         INTEGER NOT NULL,
  observed_generation   TEXT,
  state                 TEXT NOT NULL CHECK (
    state IN ('pending','running','completed','cancelled')
  ),
  attempt               INTEGER NOT NULL CHECK (attempt >= 0),
  lease_owner           TEXT,
  lease_generation      INTEGER NOT NULL CHECK (lease_generation >= 0),
  lease_expires_at_ms   INTEGER,
  last_error_code       TEXT,
  created_at_ms         INTEGER NOT NULL,
  completed_at_ms       INTEGER,
  UNIQUE (tenant_id, object_kind, object_id, reason_code)
);

CREATE INDEX IF NOT EXISTS gc_due
  ON gc_candidates(state, not_before_ms, lease_expires_at_ms);
