CREATE TABLE activation_issuer_authorities (
    entitlement_id       BLOB PRIMARY KEY NOT NULL,
    owner_principal_id   BLOB NOT NULL,
    product_id           BLOB NOT NULL,
    grants_json          BLOB NOT NULL,
    max_active_slots     INTEGER NOT NULL CHECK (max_active_slots BETWEEN 1 AND 64),
    generation           INTEGER NOT NULL CHECK (generation >= 0),
    created_at_ms        INTEGER NOT NULL,
    updated_at_ms        INTEGER NOT NULL,
    CHECK (length(entitlement_id) BETWEEN 1 AND 160),
    CHECK (length(owner_principal_id) BETWEEN 1 AND 160),
    CHECK (length(product_id) BETWEEN 1 AND 160),
    CHECK (length(grants_json) BETWEEN 2 AND 16384)
);

CREATE INDEX activation_issuer_authorities_owner_product
ON activation_issuer_authorities(owner_principal_id, product_id);

CREATE TABLE activation_slots (
    entitlement_id       BLOB NOT NULL,
    slot_id              BLOB NOT NULL,
    activation_id        BLOB NOT NULL,
    device_key_id        BLOB NOT NULL CHECK (length(device_key_id) = 32),
    state                TEXT NOT NULL CHECK (state IN ('active', 'released', 'revoked')),
    created_at_ms        INTEGER NOT NULL,
    updated_at_ms        INTEGER NOT NULL,
    PRIMARY KEY (entitlement_id, slot_id),
    UNIQUE (activation_id),
    FOREIGN KEY (entitlement_id)
        REFERENCES activation_issuer_authorities(entitlement_id)
        ON DELETE RESTRICT
);

CREATE UNIQUE INDEX activation_slots_one_active_device
ON activation_slots(entitlement_id, device_key_id)
WHERE state = 'active';

CREATE INDEX activation_slots_active_capacity
ON activation_slots(entitlement_id, state);

CREATE TABLE activation_request_receipts (
    owner_principal_id   BLOB NOT NULL,
    request_id           BLOB NOT NULL,
    request_commitment   BLOB NOT NULL CHECK (length(request_commitment) = 32),
    entitlement_id       BLOB NOT NULL,
    slot_id              BLOB NOT NULL,
    activation_id        BLOB NOT NULL,
    device_key_id        BLOB NOT NULL CHECK (length(device_key_id) = 32),
    requested_major      INTEGER NOT NULL CHECK (requested_major BETWEEN 0 AND 4294967295),
    disposition          TEXT NOT NULL CHECK (disposition IN ('allocated_new_slot', 'reused_existing_slot')),
    product_id           BLOB NOT NULL,
    grants_json          BLOB NOT NULL,
    committed_at_ms      INTEGER NOT NULL,
    PRIMARY KEY (owner_principal_id, request_id),
    FOREIGN KEY (entitlement_id, slot_id)
        REFERENCES activation_slots(entitlement_id, slot_id)
        ON DELETE RESTRICT,
    CHECK (length(owner_principal_id) BETWEEN 1 AND 160),
    CHECK (length(request_id) BETWEEN 1 AND 160),
    CHECK (length(entitlement_id) BETWEEN 1 AND 160),
    CHECK (length(slot_id) BETWEEN 1 AND 160),
    CHECK (length(activation_id) BETWEEN 1 AND 160),
    CHECK (length(product_id) BETWEEN 1 AND 160),
    CHECK (length(grants_json) BETWEEN 2 AND 16384)
);

CREATE INDEX activation_request_receipts_entitlement
ON activation_request_receipts(entitlement_id, committed_at_ms);
