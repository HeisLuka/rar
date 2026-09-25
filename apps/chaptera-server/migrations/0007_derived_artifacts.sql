-- CLOUD-REVISION-01
-- Durable exact derived-artifact fences for Layout/Scene/preview/export.
-- Historical rows are immutable: a changed revision/environment/stage/input
-- creates a different fence instead of invalidating older rows in place.

CREATE TABLE IF NOT EXISTS derived_artifacts (
    fence_id                  TEXT PRIMARY KEY,
    document_id               BLOB NOT NULL,
    service_revision_id       BLOB NOT NULL,
    canonical_revision_id     TEXT NOT NULL,
    stage                     TEXT NOT NULL,
    stage_version             TEXT NOT NULL,
    environment_fingerprint   TEXT NOT NULL,
    input_fingerprint         TEXT NOT NULL,
    content_hash              TEXT NOT NULL,
    created_at_ms             INTEGER NOT NULL,

    UNIQUE (
        document_id,
        service_revision_id,
        canonical_revision_id,
        stage,
        stage_version,
        environment_fingerprint,
        input_fingerprint
    ),

    CHECK (length(fence_id) = 71 AND substr(fence_id, 1, 7) = 'sha256:'),
    CHECK (length(canonical_revision_id) = 64),
    CHECK (stage IN ('layout', 'scene', 'preview', 'export')),
    CHECK (length(stage_version) > 0),
    CHECK (length(environment_fingerprint) = 71 AND substr(environment_fingerprint, 1, 7) = 'sha256:'),
    CHECK (length(input_fingerprint) = 71 AND substr(input_fingerprint, 1, 7) = 'sha256:'),
    CHECK (length(content_hash) = 71 AND substr(content_hash, 1, 7) = 'sha256:'),
    CHECK (created_at_ms >= 0)
);

CREATE INDEX IF NOT EXISTS derived_artifacts_by_document_revision
    ON derived_artifacts(document_id, canonical_revision_id, stage);
