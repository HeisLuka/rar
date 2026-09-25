-- CLOUD-REVISION-IDENTITY-BINDING-01
-- Explicit durable mapping between Chaptera service/history RevisionId and
-- canonical REVISION-MODEL-01 AuthoringRevisionId.
--
-- The canonical id is supplied by the authoritative Engine revision producer.
-- Cloud persists the relationship and never derives one identity from another.

CREATE TABLE IF NOT EXISTS revision_identity_bindings (
    document_id                BLOB NOT NULL,
    service_revision_id        BLOB NOT NULL,
    canonical_schema_version   TEXT NOT NULL,
    canonical_revision_id      TEXT NOT NULL,
    bound_at_ms                INTEGER NOT NULL,

    PRIMARY KEY (document_id, service_revision_id),

    CHECK (canonical_schema_version = 'chaptera.cdm.authoring-revision.v1'),
    CHECK (length(canonical_revision_id) = 64),
    CHECK (canonical_revision_id NOT GLOB '*[^0-9a-f]*'),
    CHECK (bound_at_ms >= 0)
);

CREATE INDEX IF NOT EXISTS revision_identity_bindings_by_canonical
    ON revision_identity_bindings(document_id, canonical_revision_id);
