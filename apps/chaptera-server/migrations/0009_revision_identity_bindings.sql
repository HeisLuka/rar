-- CLOUD-REVISION-01
-- Durable binding between immutable service-history revision identity and the
-- canonical REVISION-MODEL-01 AuthoringRevisionId. The two identities are
-- intentionally distinct and must never be inferred from one another.

CREATE TABLE revision_identity_bindings (
    document_id                    BLOB NOT NULL,
    service_revision_id            BLOB NOT NULL,
    canonical_revision_id          TEXT NOT NULL,
    service_parent_revision_id     BLOB,
    canonical_parent_revision_id   TEXT,
    created_at_ms                  INTEGER NOT NULL,

    PRIMARY KEY (document_id, service_revision_id),
    UNIQUE (document_id, canonical_revision_id),

    CHECK (length(document_id) > 0),
    CHECK (length(service_revision_id) > 0),
    CHECK (length(canonical_revision_id) = 64),
    CHECK (created_at_ms >= 0),
    CHECK (
        (service_parent_revision_id IS NULL AND canonical_parent_revision_id IS NULL)
        OR
        (service_parent_revision_id IS NOT NULL AND canonical_parent_revision_id IS NOT NULL)
    ),
    CHECK (
        canonical_parent_revision_id IS NULL
        OR length(canonical_parent_revision_id) = 64
    )
);

CREATE INDEX revision_identity_by_canonical
    ON revision_identity_bindings(document_id, canonical_revision_id);
