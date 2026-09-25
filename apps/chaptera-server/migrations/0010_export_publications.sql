-- CLOUD-EXPORT-EXECUTOR-01
-- Durable logical export publication identity.
--
-- Physical BlobStore dedupe/bindings and exact derived-artifact fences remain
-- separate authorities. This row is the stable tenant/job publication effect
-- that ties them together for crash/retry-safe WorkerLoop completion.

CREATE TABLE IF NOT EXISTS export_publications (
    publication_id            TEXT PRIMARY KEY,
    effect_key                BLOB NOT NULL UNIQUE,
    tenant_id                 BLOB NOT NULL,
    job_id                    BLOB NOT NULL,
    document_id               BLOB NOT NULL,
    exact_revision_id         BLOB NOT NULL,
    canonical_revision_id     TEXT NOT NULL,
    target_profile            TEXT NOT NULL,
    layout_environment_id     TEXT NOT NULL,
    fence_id                  TEXT NOT NULL,
    artifact_binding_id       BLOB NOT NULL,
    artifact_content_hash     TEXT NOT NULL,
    loss_binding_id           BLOB NOT NULL,
    loss_report_hash          TEXT NOT NULL,
    created_at_ms             INTEGER NOT NULL,

    UNIQUE (tenant_id, job_id),

    CHECK (length(publication_id) = 71 AND substr(publication_id, 1, 7) = 'sha256:'),
    CHECK (length(effect_key) = 71),
    CHECK (length(canonical_revision_id) = 64),
    CHECK (length(layout_environment_id) = 71 AND substr(layout_environment_id, 1, 7) = 'sha256:'),
    CHECK (length(fence_id) = 71 AND substr(fence_id, 1, 7) = 'sha256:'),
    CHECK (length(artifact_content_hash) = 71 AND substr(artifact_content_hash, 1, 7) = 'sha256:'),
    CHECK (length(loss_report_hash) = 71 AND substr(loss_report_hash, 1, 7) = 'sha256:'),
    CHECK (created_at_ms >= 0)
);

CREATE INDEX IF NOT EXISTS export_publications_by_revision
    ON export_publications(tenant_id, document_id, canonical_revision_id);
