use std::{fmt, path::Path, time::Duration};

use serde::{Deserialize, Serialize};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const MAX_ID_BYTES: usize = 160;
const MAX_PRODUCT_BYTES: usize = 160;
const MAX_GRANTS: usize = 64;
const MAX_GRANT_BYTES: usize = 64;
const MAX_CONNECTIONS: u32 = 16;
const MAX_BUSY_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ACTIVE_SLOTS: i64 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationStoreError {
    pub code: &'static str,
    pub message: String,
}

impl ActivationStoreError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ActivationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ActivationStoreError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationAuthorityConfig {
    pub owner_principal_id: String,
    pub entitlement_id: String,
    pub product_id: String,
    pub grants: Vec<String>,
    pub max_active_slots: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationAuthorityRecord {
    pub config: ActivationAuthorityConfig,
    pub generation: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionAuthorityOutcome {
    Created(ActivationAuthorityRecord),
    Existing(ActivationAuthorityRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuanceDisposition {
    AllocatedNewSlot,
    ReusedExistingSlot,
}

impl IssuanceDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::AllocatedNewSlot => "allocated_new_slot",
            Self::ReusedExistingSlot => "reused_existing_slot",
        }
    }

    fn parse(value: &str) -> Result<Self, ActivationStoreError> {
        match value {
            "allocated_new_slot" => Ok(Self::AllocatedNewSlot),
            "reused_existing_slot" => Ok(Self::ReusedExistingSlot),
            _ => Err(ActivationStoreError::new(
                "activation_row_corrupt",
                "persisted issuance disposition is invalid",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIssueRequest {
    pub owner_principal_id: String,
    pub entitlement_id: String,
    pub request_id: String,
    pub request_commitment: [u8; 32],
    pub expected_generation: i64,
    pub device_key_id: [u8; 32],
    pub requested_major: u32,
    pub proposed_slot_id: String,
    pub proposed_activation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationIssuanceRecord {
    pub owner_principal_id: String,
    pub request_id: String,
    pub request_commitment: [u8; 32],
    pub entitlement_id: String,
    pub slot_id: String,
    pub activation_id: String,
    pub device_key_id: [u8; 32],
    pub requested_major: u32,
    pub disposition: IssuanceDisposition,
    pub product_id: String,
    pub grants: Vec<String>,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationIssueOutcome {
    AllocatedNewSlot(ActivationIssuanceRecord),
    ReusedExistingSlot(ActivationIssuanceRecord),
    Existing(ActivationIssuanceRecord),
}

#[derive(Debug, Clone)]
struct ActiveSlot {
    slot_id: String,
    activation_id: String,
    device_key_id: [u8; 32],
}

#[derive(Clone)]
pub struct SqliteActivationIssuerStore {
    pool: SqlitePool,
}

impl SqliteActivationIssuerStore {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, ActivationStoreError> {
        if !(1..=MAX_CONNECTIONS).contains(&max_connections) {
            return Err(ActivationStoreError::new(
                "invalid_pool_size",
                "activation issuer pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > MAX_BUSY_TIMEOUT {
            return Err(ActivationStoreError::new(
                "invalid_busy_timeout",
                "activation issuer busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(ActivationStoreError::new(
                "invalid_database_path",
                "activation issuer database path must be non-empty",
            ));
        }
        if !path.exists() {
            return Err(ActivationStoreError::new(
                "activation_database_missing",
                "activation issuer database must be created by chaptera migrate up before runtime open",
            ));
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(busy_timeout);

        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .min_connections(1)
            .connect_with(options)
            .await
            .map_err(sqlite_error)?;

        let store = Self { pool };
        store.require_schema().await?;
        store.verify_profile().await?;
        Ok(store)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn provision_authority(
        &self,
        config: ActivationAuthorityConfig,
        now_ms: i64,
    ) -> Result<ProvisionAuthorityOutcome, ActivationStoreError> {
        validate_authority_config(&config)?;
        validate_now(now_ms)?;
        let grants_json = encode_grants(&config.grants)?;

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = async {
            if let Some(existing) = read_authority(&mut conn, &config.entitlement_id).await? {
                if existing.config != config {
                    return Err(ActivationStoreError::new(
                        "activation_authority_conflict",
                        "entitlement authority already exists with different owner/product/grants/capacity",
                    ));
                }
                return Ok(ProvisionAuthorityOutcome::Existing(existing));
            }

            let done = sqlx::query(
                r#"
                INSERT INTO activation_issuer_authorities (
                    entitlement_id, owner_principal_id, product_id, grants_json,
                    max_active_slots, generation, created_at_ms, updated_at_ms
                ) VALUES (?, ?, ?, ?, ?, 0, ?, ?)
                "#,
            )
            .bind(config.entitlement_id.as_bytes())
            .bind(config.owner_principal_id.as_bytes())
            .bind(config.product_id.as_bytes())
            .bind(grants_json.as_bytes())
            .bind(config.max_active_slots)
            .bind(now_ms)
            .bind(now_ms)
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            if done.rows_affected() != 1 {
                return Err(ActivationStoreError::new(
                    "activation_authority_create_no_effect",
                    "authority insert did not create exactly one row",
                ));
            }

            let created = read_authority(&mut conn, &config.entitlement_id)
                .await?
                .ok_or_else(|| {
                    ActivationStoreError::new(
                        "activation_authority_missing",
                        "inserted authority disappeared",
                    )
                })?;
            Ok(ProvisionAuthorityOutcome::Created(created))
        }
        .await;

        finish_transaction(&mut conn, result).await
    }

    pub async fn issue(
        &self,
        request: ActivationIssueRequest,
        now_ms: i64,
    ) -> Result<ActivationIssueOutcome, ActivationStoreError> {
        validate_issue_request(&request)?;
        validate_now(now_ms)?;

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = self.issue_in_transaction(&mut conn, request, now_ms).await;
        finish_transaction(&mut conn, result).await
    }

    async fn issue_in_transaction(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
        request: ActivationIssueRequest,
        now_ms: i64,
    ) -> Result<ActivationIssueOutcome, ActivationStoreError> {
        // Idempotency is account scoped because the signed offline request does not
        // contain EntitlementId. One valid request must not be replayable into a
        // second entitlement owned by the same account.
        if let Some(existing) =
            read_receipt(conn, &request.owner_principal_id, &request.request_id).await?
        {
            if existing.request_commitment != request.request_commitment {
                return Err(ActivationStoreError::new(
                    "activation_request_conflict",
                    "request_id was already committed with different verified request facts",
                ));
            }
            return Ok(ActivationIssueOutcome::Existing(existing));
        }

        let authority = read_authority(conn, &request.entitlement_id)
            .await?
            .ok_or_else(|| {
                ActivationStoreError::new(
                    "activation_authority_not_found",
                    "entitlement activation authority does not exist",
                )
            })?;

        if authority.config.owner_principal_id != request.owner_principal_id {
            return Err(ActivationStoreError::new(
                "activation_owner_mismatch",
                "entitlement activation authority belongs to a different principal",
            ));
        }
        if authority.generation != request.expected_generation {
            return Err(ActivationStoreError::new(
                "stale_activation_generation",
                "activation authority generation changed",
            ));
        }

        if let Some(existing_slot) =
            read_active_slot_for_device(conn, &request.entitlement_id, &request.device_key_id)
                .await?
        {
            let record = make_record(
                &authority,
                &request,
                &existing_slot,
                IssuanceDisposition::ReusedExistingSlot,
                now_ms,
            );
            advance_generation(conn, &authority, now_ms).await?;
            insert_receipt(conn, &record).await?;
            return Ok(ActivationIssueOutcome::ReusedExistingSlot(record));
        }

        validate_ident(&request.proposed_slot_id, "proposed_slot_id")?;
        validate_ident(&request.proposed_activation_id, "proposed_activation_id")?;

        let active_count = active_slot_count_in_transaction(conn, &request.entitlement_id).await?;
        if active_count >= authority.config.max_active_slots {
            return Err(ActivationStoreError::new(
                "activation_capacity_exhausted",
                "entitlement has no free active ActivationSlot",
            ));
        }

        if slot_id_exists(conn, &request.entitlement_id, &request.proposed_slot_id).await? {
            return Err(ActivationStoreError::new(
                "activation_slot_id_conflict",
                "proposed ActivationSlot id already exists",
            ));
        }
        if activation_id_exists(conn, &request.proposed_activation_id).await? {
            return Err(ActivationStoreError::new(
                "activation_id_conflict",
                "proposed ActivationId already exists",
            ));
        }

        let slot = ActiveSlot {
            slot_id: request.proposed_slot_id.clone(),
            activation_id: request.proposed_activation_id.clone(),
            device_key_id: request.device_key_id,
        };

        let done = sqlx::query(
            r#"
            INSERT INTO activation_slots (
                entitlement_id, slot_id, activation_id, device_key_id,
                state, created_at_ms, updated_at_ms
            ) VALUES (?, ?, ?, ?, 'active', ?, ?)
            "#,
        )
        .bind(request.entitlement_id.as_bytes())
        .bind(slot.slot_id.as_bytes())
        .bind(slot.activation_id.as_bytes())
        .bind(slot.device_key_id.as_slice())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut **conn)
        .await
        .map_err(sqlite_error)?;
        if done.rows_affected() != 1 {
            return Err(ActivationStoreError::new(
                "activation_slot_create_no_effect",
                "slot insert did not create exactly one row",
            ));
        }

        let record = make_record(
            &authority,
            &request,
            &slot,
            IssuanceDisposition::AllocatedNewSlot,
            now_ms,
        );
        advance_generation(conn, &authority, now_ms).await?;
        insert_receipt(conn, &record).await?;
        Ok(ActivationIssueOutcome::AllocatedNewSlot(record))
    }

    pub async fn active_slot_count(
        &self,
        entitlement_id: &str,
    ) -> Result<i64, ActivationStoreError> {
        validate_ident(entitlement_id, "entitlement_id")?;
        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        active_slot_count_in_transaction(&mut conn, entitlement_id).await
    }

    async fn require_schema(&self) -> Result<(), ActivationStoreError> {
        for table in [
            "activation_issuer_authorities",
            "activation_slots",
            "activation_request_receipts",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if exists != 1 {
                return Err(ActivationStoreError::new(
                    "activation_schema_missing",
                    format!("{table} is absent; run chaptera migrate up before runtime open"),
                ));
            }
        }
        Ok(())
    }

    async fn verify_profile(&self) -> Result<(), ActivationStoreError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(ActivationStoreError::new(
                "activation_profile_mismatch",
                format!("expected WAL journal mode, got {journal_mode}"),
            ));
        }

        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if synchronous != 2 {
            return Err(ActivationStoreError::new(
                "activation_profile_mismatch",
                format!("expected synchronous=FULL(2), got {synchronous}"),
            ));
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if foreign_keys != 1 {
            return Err(ActivationStoreError::new(
                "activation_profile_mismatch",
                "foreign_keys must be enabled",
            ));
        }
        Ok(())
    }
}

fn make_record(
    authority: &ActivationAuthorityRecord,
    request: &ActivationIssueRequest,
    slot: &ActiveSlot,
    disposition: IssuanceDisposition,
    committed_at_ms: i64,
) -> ActivationIssuanceRecord {
    ActivationIssuanceRecord {
        owner_principal_id: request.owner_principal_id.clone(),
        request_id: request.request_id.clone(),
        request_commitment: request.request_commitment,
        entitlement_id: authority.config.entitlement_id.clone(),
        slot_id: slot.slot_id.clone(),
        activation_id: slot.activation_id.clone(),
        device_key_id: slot.device_key_id,
        requested_major: request.requested_major,
        disposition,
        product_id: authority.config.product_id.clone(),
        grants: authority.config.grants.clone(),
        committed_at_ms,
    }
}

async fn advance_generation(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    authority: &ActivationAuthorityRecord,
    now_ms: i64,
) -> Result<(), ActivationStoreError> {
    let next = authority
        .generation
        .checked_add(1)
        .ok_or_else(|| ActivationStoreError::new("activation_generation_overflow", "generation overflow"))?;
    let done = sqlx::query(
        r#"
        UPDATE activation_issuer_authorities
        SET generation=?, updated_at_ms=?
        WHERE entitlement_id=? AND generation=?
        "#,
    )
    .bind(next)
    .bind(now_ms)
    .bind(authority.config.entitlement_id.as_bytes())
    .bind(authority.generation)
    .execute(&mut **conn)
    .await
    .map_err(sqlite_error)?;

    if done.rows_affected() != 1 {
        return Err(ActivationStoreError::new(
            "activation_generation_race",
            "activation authority changed before commit",
        ));
    }
    Ok(())
}

async fn insert_receipt(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    record: &ActivationIssuanceRecord,
) -> Result<(), ActivationStoreError> {
    let grants_json = encode_grants(&record.grants)?;
    let done = sqlx::query(
        r#"
        INSERT INTO activation_request_receipts (
            owner_principal_id, request_id, request_commitment,
            entitlement_id, slot_id, activation_id, device_key_id,
            requested_major, disposition, product_id, grants_json, committed_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(record.owner_principal_id.as_bytes())
    .bind(record.request_id.as_bytes())
    .bind(record.request_commitment.as_slice())
    .bind(record.entitlement_id.as_bytes())
    .bind(record.slot_id.as_bytes())
    .bind(record.activation_id.as_bytes())
    .bind(record.device_key_id.as_slice())
    .bind(i64::from(record.requested_major))
    .bind(record.disposition.as_str())
    .bind(record.product_id.as_bytes())
    .bind(grants_json.as_bytes())
    .bind(record.committed_at_ms)
    .execute(&mut **conn)
    .await
    .map_err(sqlite_error)?;

    if done.rows_affected() != 1 {
        return Err(ActivationStoreError::new(
            "activation_receipt_create_no_effect",
            "request receipt insert did not create exactly one row",
        ));
    }
    Ok(())
}

async fn read_authority(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    entitlement_id: &str,
) -> Result<Option<ActivationAuthorityRecord>, ActivationStoreError> {
    let row = sqlx::query(
        r#"
        SELECT owner_principal_id, entitlement_id, product_id, grants_json,
               max_active_slots, generation, created_at_ms, updated_at_ms
        FROM activation_issuer_authorities
        WHERE entitlement_id=?
        "#,
    )
    .bind(entitlement_id.as_bytes())
    .fetch_optional(&mut **conn)
    .await
    .map_err(sqlite_error)?;

    row.map(parse_authority_row).transpose()
}

fn parse_authority_row(row: sqlx::sqlite::SqliteRow) -> Result<ActivationAuthorityRecord, ActivationStoreError> {
    let owner = blob_string(&row, "owner_principal_id")?;
    let entitlement = blob_string(&row, "entitlement_id")?;
    let product = blob_string(&row, "product_id")?;
    let grants_json = blob_string(&row, "grants_json")?;
    let grants = decode_grants(&grants_json)?;
    let max_active_slots: i64 = row.try_get("max_active_slots").map_err(sqlite_error)?;
    let generation: i64 = row.try_get("generation").map_err(sqlite_error)?;
    let created_at_ms: i64 = row.try_get("created_at_ms").map_err(sqlite_error)?;
    let updated_at_ms: i64 = row.try_get("updated_at_ms").map_err(sqlite_error)?;

    let config = ActivationAuthorityConfig {
        owner_principal_id: owner,
        entitlement_id: entitlement,
        product_id: product,
        grants,
        max_active_slots,
    };
    validate_authority_config(&config)?;
    if generation < 0 || created_at_ms < 0 || updated_at_ms < 0 {
        return Err(ActivationStoreError::new(
            "activation_row_corrupt",
            "persisted authority counters/timestamps are invalid",
        ));
    }

    Ok(ActivationAuthorityRecord {
        config,
        generation,
        created_at_ms,
        updated_at_ms,
    })
}

async fn read_receipt(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    owner_principal_id: &str,
    request_id: &str,
) -> Result<Option<ActivationIssuanceRecord>, ActivationStoreError> {
    let row = sqlx::query(
        r#"
        SELECT owner_principal_id, request_id, request_commitment,
               entitlement_id, slot_id, activation_id, device_key_id,
               requested_major, disposition, product_id, grants_json, committed_at_ms
        FROM activation_request_receipts
        WHERE owner_principal_id=? AND request_id=?
        "#,
    )
    .bind(owner_principal_id.as_bytes())
    .bind(request_id.as_bytes())
    .fetch_optional(&mut **conn)
    .await
    .map_err(sqlite_error)?;

    row.map(parse_receipt_row).transpose()
}

fn parse_receipt_row(row: sqlx::sqlite::SqliteRow) -> Result<ActivationIssuanceRecord, ActivationStoreError> {
    let commitment = blob_32(&row, "request_commitment")?;
    let device = blob_32(&row, "device_key_id")?;
    let major: i64 = row.try_get("requested_major").map_err(sqlite_error)?;
    let requested_major = u32::try_from(major).map_err(|_| {
        ActivationStoreError::new("activation_row_corrupt", "requested_major does not fit u32")
    })?;
    let disposition_raw: String = row.try_get("disposition").map_err(sqlite_error)?;
    let grants_json = blob_string(&row, "grants_json")?;
    let committed_at_ms: i64 = row.try_get("committed_at_ms").map_err(sqlite_error)?;
    if committed_at_ms < 0 {
        return Err(ActivationStoreError::new(
            "activation_row_corrupt",
            "receipt committed_at_ms is negative",
        ));
    }

    Ok(ActivationIssuanceRecord {
        owner_principal_id: blob_string(&row, "owner_principal_id")?,
        request_id: blob_string(&row, "request_id")?,
        request_commitment: commitment,
        entitlement_id: blob_string(&row, "entitlement_id")?,
        slot_id: blob_string(&row, "slot_id")?,
        activation_id: blob_string(&row, "activation_id")?,
        device_key_id: device,
        requested_major,
        disposition: IssuanceDisposition::parse(&disposition_raw)?,
        product_id: blob_string(&row, "product_id")?,
        grants: decode_grants(&grants_json)?,
        committed_at_ms,
    })
}

async fn read_active_slot_for_device(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    entitlement_id: &str,
    device_key_id: &[u8; 32],
) -> Result<Option<ActiveSlot>, ActivationStoreError> {
    let row = sqlx::query(
        r#"
        SELECT slot_id, activation_id, device_key_id
        FROM activation_slots
        WHERE entitlement_id=? AND device_key_id=? AND state='active'
        "#,
    )
    .bind(entitlement_id.as_bytes())
    .bind(device_key_id.as_slice())
    .fetch_optional(&mut **conn)
    .await
    .map_err(sqlite_error)?;

    row.map(|row| {
        Ok(ActiveSlot {
            slot_id: blob_string(&row, "slot_id")?,
            activation_id: blob_string(&row, "activation_id")?,
            device_key_id: blob_32(&row, "device_key_id")?,
        })
    })
    .transpose()
}

async fn active_slot_count_in_transaction(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    entitlement_id: &str,
) -> Result<i64, ActivationStoreError> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM activation_slots WHERE entitlement_id=? AND state='active'",
    )
    .bind(entitlement_id.as_bytes())
    .fetch_one(&mut **conn)
    .await
    .map_err(sqlite_error)
}

async fn slot_id_exists(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    entitlement_id: &str,
    slot_id: &str,
) -> Result<bool, ActivationStoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM activation_slots WHERE entitlement_id=? AND slot_id=?",
    )
    .bind(entitlement_id.as_bytes())
    .bind(slot_id.as_bytes())
    .fetch_one(&mut **conn)
    .await
    .map_err(sqlite_error)?;
    Ok(count != 0)
}

async fn activation_id_exists(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    activation_id: &str,
) -> Result<bool, ActivationStoreError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM activation_slots WHERE activation_id=?")
            .bind(activation_id.as_bytes())
            .fetch_one(&mut **conn)
            .await
            .map_err(sqlite_error)?;
    Ok(count != 0)
}

async fn finish_transaction<T>(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    result: Result<T, ActivationStoreError>,
) -> Result<T, ActivationStoreError> {
    match result {
        Ok(value) => {
            sqlx::query("COMMIT")
                .execute(&mut **conn)
                .await
                .map_err(sqlite_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut **conn).await;
            Err(error)
        }
    }
}

fn validate_authority_config(config: &ActivationAuthorityConfig) -> Result<(), ActivationStoreError> {
    validate_ident(&config.owner_principal_id, "owner_principal_id")?;
    validate_ident(&config.entitlement_id, "entitlement_id")?;
    validate_product(&config.product_id)?;
    validate_grants(&config.grants)?;
    if !(1..=MAX_ACTIVE_SLOTS).contains(&config.max_active_slots) {
        return Err(ActivationStoreError::new(
            "invalid_activation_capacity",
            "max_active_slots must be 1..=64",
        ));
    }
    Ok(())
}

fn validate_issue_request(request: &ActivationIssueRequest) -> Result<(), ActivationStoreError> {
    validate_ident(&request.owner_principal_id, "owner_principal_id")?;
    validate_ident(&request.entitlement_id, "entitlement_id")?;
    validate_ident(&request.request_id, "request_id")?;
    if request.expected_generation < 0 {
        return Err(ActivationStoreError::new(
            "invalid_activation_generation",
            "expected_generation must be non-negative",
        ));
    }
    Ok(())
}

fn validate_ident(value: &str, field: &str) -> Result<(), ActivationStoreError> {
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(ActivationStoreError::new(
            "invalid_activation_identifier",
            format!("{field} must be 1..={MAX_ID_BYTES} bytes"),
        ));
    }
    Ok(())
}

fn validate_product(value: &str) -> Result<(), ActivationStoreError> {
    if value.is_empty() || value.len() > MAX_PRODUCT_BYTES {
        return Err(ActivationStoreError::new(
            "invalid_activation_product",
            "product_id is empty or too long",
        ));
    }
    Ok(())
}

fn validate_grants(grants: &[String]) -> Result<(), ActivationStoreError> {
    if grants.is_empty() || grants.len() > MAX_GRANTS {
        return Err(ActivationStoreError::new(
            "invalid_activation_grants",
            "grants must contain 1..=64 entries",
        ));
    }
    for (index, grant) in grants.iter().enumerate() {
        if grant.is_empty() || grant.len() > MAX_GRANT_BYTES {
            return Err(ActivationStoreError::new(
                "invalid_activation_grants",
                "grant is empty or too long",
            ));
        }
        if grants[..index].iter().any(|seen| seen == grant) {
            return Err(ActivationStoreError::new(
                "invalid_activation_grants",
                "duplicate grants are forbidden",
            ));
        }
    }
    Ok(())
}

fn validate_now(now_ms: i64) -> Result<(), ActivationStoreError> {
    if now_ms < 0 {
        Err(ActivationStoreError::new(
            "invalid_activation_time",
            "now_ms must be non-negative",
        ))
    } else {
        Ok(())
    }
}

fn encode_grants(grants: &[String]) -> Result<String, ActivationStoreError> {
    validate_grants(grants)?;
    serde_json::to_string(grants).map_err(|error| {
        ActivationStoreError::new(
            "activation_grants_encode_failed",
            bounded_message(&error.to_string()),
        )
    })
}

fn decode_grants(value: &str) -> Result<Vec<String>, ActivationStoreError> {
    let grants: Vec<String> = serde_json::from_str(value).map_err(|error| {
        ActivationStoreError::new(
            "activation_row_corrupt",
            format!("persisted grants JSON is invalid: {}", bounded_message(&error.to_string())),
        )
    })?;
    validate_grants(&grants)?;
    Ok(grants)
}

fn blob_string(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, ActivationStoreError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        ActivationStoreError::new(
            "activation_row_corrupt",
            format!("{column} is not valid UTF-8"),
        )
    })
}

fn blob_32(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<[u8; 32], ActivationStoreError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    bytes.try_into().map_err(|_| {
        ActivationStoreError::new(
            "activation_row_corrupt",
            format!("{column} must be exactly 32 bytes"),
        )
    })
}

fn sqlite_error(error: impl fmt::Display) -> ActivationStoreError {
    ActivationStoreError::new(
        "activation_sqlite_error",
        bounded_message(&error.to_string()),
    )
}

fn bounded_message(message: &str) -> String {
    const MAX: usize = 512;
    if message.len() <= MAX {
        message.to_owned()
    } else {
        format!("{}...", &message[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use super::*;
    use crate::schema_migration::SqliteMigrationRuntime;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let n = NEXT_DB.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "chaptera-activation-issuer-{label}-{}-{n}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for candidate in [
            path.to_path_buf(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            let _ = fs::remove_file(candidate);
        }
    }

    async fn migrated_store(label: &str) -> (PathBuf, SqliteActivationIssuerStore) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let store = SqliteActivationIssuerStore::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (path, store)
    }

    fn config(entitlement: &str, capacity: i64) -> ActivationAuthorityConfig {
        ActivationAuthorityConfig {
            owner_principal_id: "principal-1".into(),
            entitlement_id: entitlement.into(),
            product_id: "chaptera.editor".into(),
            grants: vec!["edit".into(), "export".into()],
            max_active_slots: capacity,
        }
    }

    fn request(
        entitlement: &str,
        request_id: &str,
        commitment: u8,
        generation: i64,
        device: u8,
        slot: &str,
        activation: &str,
    ) -> ActivationIssueRequest {
        ActivationIssueRequest {
            owner_principal_id: "principal-1".into(),
            entitlement_id: entitlement.into(),
            request_id: request_id.into(),
            request_commitment: [commitment; 32],
            expected_generation: generation,
            device_key_id: [device; 32],
            requested_major: 2,
            proposed_slot_id: slot.into(),
            proposed_activation_id: activation.into(),
        }
    }

    #[tokio::test]
    async fn authority_provision_is_idempotent_and_changed_policy_conflicts() {
        let (path, store) = migrated_store("provision").await;
        let cfg = config("ent-1", 2);
        let first = store.provision_authority(cfg.clone(), 10).await.unwrap();
        assert!(matches!(first, ProvisionAuthorityOutcome::Created(_)));
        let second = store.provision_authority(cfg.clone(), 11).await.unwrap();
        assert!(matches!(second, ProvisionAuthorityOutcome::Existing(_)));

        let mut changed = cfg;
        changed.max_active_slots = 3;
        let error = store.provision_authority(changed, 12).await.unwrap_err();
        assert_eq!(error.code, "activation_authority_conflict");

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn exact_request_replay_returns_original_slot_before_generation_or_candidate_checks() {
        let (path, store) = migrated_store("retry").await;
        store.provision_authority(config("ent-1", 1), 10).await.unwrap();

        let first = store
            .issue(request("ent-1", "req-1", 0x11, 0, 0xA1, "slot-1", "act-1"), 20)
            .await
            .unwrap();
        let original = match first {
            ActivationIssueOutcome::AllocatedNewSlot(row) => row,
            other => panic!("unexpected first outcome: {other:?}"),
        };

        let replay = store
            .issue(
                request(
                    "ent-1",
                    "req-1",
                    0x11,
                    0,
                    0xA1,
                    "slot-ignored",
                    "act-ignored",
                ),
                30,
            )
            .await
            .unwrap();
        let recovered = match replay {
            ActivationIssueOutcome::Existing(row) => row,
            other => panic!("unexpected replay outcome: {other:?}"),
        };
        assert_eq!(recovered, original);
        assert_eq!(store.active_slot_count("ent-1").await.unwrap(), 1);

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn changed_commitment_under_same_account_request_id_fails_closed() {
        let (path, store) = migrated_store("conflict").await;
        store.provision_authority(config("ent-1", 2), 10).await.unwrap();
        store
            .issue(request("ent-1", "req-1", 0x11, 0, 0xA1, "slot-1", "act-1"), 20)
            .await
            .unwrap();

        let error = store
            .issue(request("ent-1", "req-1", 0x22, 1, 0xA1, "slot-2", "act-2"), 30)
            .await
            .unwrap_err();
        assert_eq!(error.code, "activation_request_conflict");
        assert_eq!(store.active_slot_count("ent-1").await.unwrap(), 1);

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn account_scoped_request_replay_cannot_be_redirected_to_second_entitlement() {
        let (path, store) = migrated_store("cross-entitlement").await;
        store.provision_authority(config("ent-1", 1), 10).await.unwrap();
        store.provision_authority(config("ent-2", 1), 11).await.unwrap();

        let first = store
            .issue(request("ent-1", "req-shared", 0x41, 0, 0xA1, "slot-1", "act-1"), 20)
            .await
            .unwrap();
        let first = match first {
            ActivationIssueOutcome::AllocatedNewSlot(row) => row,
            _ => panic!("expected allocation"),
        };

        let replay = store
            .issue(request("ent-2", "req-shared", 0x41, 0, 0xA1, "slot-2", "act-2"), 30)
            .await
            .unwrap();
        let replay = match replay {
            ActivationIssueOutcome::Existing(row) => row,
            _ => panic!("expected existing receipt"),
        };
        assert_eq!(replay.entitlement_id, "ent-1");
        assert_eq!(replay.activation_id, first.activation_id);
        assert_eq!(store.active_slot_count("ent-2").await.unwrap(), 0);

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn same_device_new_request_reuses_active_slot_without_consuming_capacity() {
        let (path, store) = migrated_store("reuse-device").await;
        store.provision_authority(config("ent-1", 1), 10).await.unwrap();
        store
            .issue(request("ent-1", "req-1", 0x11, 0, 0xA1, "slot-1", "act-1"), 20)
            .await
            .unwrap();

        let second = store
            .issue(
                request("ent-1", "req-2", 0x22, 1, 0xA1, "slot-unused", "act-unused"),
                30,
            )
            .await
            .unwrap();
        let row = match second {
            ActivationIssueOutcome::ReusedExistingSlot(row) => row,
            other => panic!("unexpected reuse outcome: {other:?}"),
        };
        assert_eq!(row.slot_id, "slot-1");
        assert_eq!(row.activation_id, "act-1");
        assert_eq!(store.active_slot_count("ent-1").await.unwrap(), 1);

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn concurrent_capacity_one_requests_have_exactly_one_winner() {
        let (path, store) = migrated_store("race").await;
        store.provision_authority(config("ent-1", 1), 10).await.unwrap();

        let left = store.clone();
        let right = store.clone();
        let (a, b) = tokio::join!(
            left.issue(request("ent-1", "req-a", 0x11, 0, 0xA1, "slot-a", "act-a"), 20),
            right.issue(request("ent-1", "req-b", 0x22, 0, 0xB2, "slot-b", "act-b"), 20)
        );

        let successes = [a.as_ref().ok(), b.as_ref().ok()]
            .into_iter()
            .flatten()
            .count();
        assert_eq!(successes, 1);

        let loser = if a.is_err() { a.unwrap_err() } else { b.unwrap_err() };
        assert_eq!(loser.code, "stale_activation_generation");
        assert_eq!(store.active_slot_count("ent-1").await.unwrap(), 1);

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn reopen_preserves_exact_receipt_and_active_slot() {
        let (path, store) = migrated_store("restart").await;
        store.provision_authority(config("ent-1", 1), 10).await.unwrap();
        let first = store
            .issue(request("ent-1", "req-1", 0x11, 0, 0xA1, "slot-1", "act-1"), 20)
            .await
            .unwrap();
        let first = match first {
            ActivationIssueOutcome::AllocatedNewSlot(row) => row,
            _ => panic!("expected allocation"),
        };
        store.close().await;

        let reopened = SqliteActivationIssuerStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        let replay = reopened
            .issue(
                request("ent-1", "req-1", 0x11, 0, 0xA1, "ignored", "ignored"),
                99,
            )
            .await
            .unwrap();
        let replay = match replay {
            ActivationIssueOutcome::Existing(row) => row,
            _ => panic!("expected existing receipt"),
        };
        assert_eq!(replay, first);
        assert_eq!(reopened.active_slot_count("ent-1").await.unwrap(), 1);

        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn open_requires_operator_migrated_schema_and_does_not_create_tables() {
        let path = temp_db("unmigrated");
        fs::File::create(&path).unwrap();

        let error = SqliteActivationIssuerStore::open(&path, 2, Duration::from_secs(2))
            .await
            .err()
            .expect("unmigrated store must fail");
        assert_eq!(error.code, "activation_schema_missing");

        cleanup(&path);
    }
}
