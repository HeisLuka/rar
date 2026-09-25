use std::{fmt, path::Path, str, time::Duration};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const MAX_ID_BYTES: usize = 160;
const MAX_CONNECTIONS: u32 = 16;
const MAX_BUSY_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_LEASE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaError {
    pub code: &'static str,
    pub message: String,
}

impl QuotaError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for QuotaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for QuotaError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaWorkClass {
    Interactive,
    Export,
    Background,
}

impl QuotaWorkClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Export => "export",
            Self::Background => "background",
        }
    }

    fn parse(value: &str) -> Result<Self, QuotaError> {
        match value {
            "interactive" => Ok(Self::Interactive),
            "export" => Ok(Self::Export),
            "background" => Ok(Self::Background),
            _ => Err(QuotaError::new(
                "quota_row_corrupt",
                "persisted quota work class is invalid",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaConfig {
    pub shared_capacity: i64,
    pub semantic_headroom: i64,
    pub export_cap: i64,
    pub background_cap: i64,
}

impl QuotaConfig {
    pub fn validate(&self) -> Result<(), QuotaError> {
        for (name, value) in [
            ("shared_capacity", self.shared_capacity),
            ("semantic_headroom", self.semantic_headroom),
            ("export_cap", self.export_cap),
            ("background_cap", self.background_cap),
        ] {
            if value < 0 {
                return Err(QuotaError::new(
                    "invalid_quota_config",
                    format!("{name} must be non-negative"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveRequest {
    pub tenant_id: String,
    pub reservation_id: String,
    pub work_class: QuotaWorkClass,
    pub amount: i64,
    pub request_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservationRecord {
    pub tenant_id: String,
    pub reservation_id: String,
    pub work_class: QuotaWorkClass,
    pub amount: i64,
    pub request_hash: String,
    pub lease_generation: i64,
    pub lease_expires_at_ms: i64,
    pub released_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveOutcome {
    Reserved(ReservationRecord),
    Existing(ReservationRecord),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuotaUsage {
    pub interactive: i64,
    pub export: i64,
    pub background: i64,
    pub shared_total: i64,
    pub protected_interactive: i64,
}

#[derive(Clone)]
pub struct SqliteQuotaAuthority {
    pool: SqlitePool,
    config: QuotaConfig,
}

impl SqliteQuotaAuthority {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
        config: QuotaConfig,
    ) -> Result<Self, QuotaError> {
        config.validate()?;
        if !(1..=MAX_CONNECTIONS).contains(&max_connections) {
            return Err(QuotaError::new(
                "invalid_pool_size",
                "quota pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > MAX_BUSY_TIMEOUT {
            return Err(QuotaError::new(
                "invalid_busy_timeout",
                "quota busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(QuotaError::new(
                "invalid_database_path",
                "quota database path must be non-empty",
            ));
        }
        if !path.exists() {
            return Err(QuotaError::new(
                "quota_database_missing",
                "quota database must be created by chaptera migrate up before runtime open",
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

        let authority = Self { pool, config };
        authority.require_schema().await?;
        authority.verify_profile().await?;
        Ok(authority)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn reserve(
        &self,
        request: ReserveRequest,
        now_ms: i64,
        lease: Duration,
    ) -> Result<ReserveOutcome, QuotaError> {
        validate_request(&request)?;
        let lease_ms = validate_lease(lease)?;
        validate_now(now_ms)?;

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = self
            .reserve_in_transaction(&mut conn, request, now_ms, lease_ms)
            .await;
        finish_transaction(&mut conn, result).await
    }

    async fn reserve_in_transaction(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
        request: ReserveRequest,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<ReserveOutcome, QuotaError> {
        reconcile_expired_for_tenant(conn, &request.tenant_id, now_ms).await?;

        if let Some(existing) =
            read_reservation(conn, &request.tenant_id, &request.reservation_id).await?
        {
            if existing.work_class != request.work_class
                || existing.amount != request.amount
                || existing.request_hash != request.request_hash
            {
                return Err(QuotaError::new(
                    "reservation_conflict",
                    "reservation identity was reused with a different request",
                ));
            }
            if existing.released_at_ms.is_some() {
                return Err(QuotaError::new(
                    "reservation_already_released",
                    "exact reservation retry arrived after release or expiry reconciliation",
                ));
            }
            return Ok(ReserveOutcome::Existing(existing));
        }

        let usage = usage_in_transaction(conn, &request.tenant_id, now_ms, &self.config).await?;
        self.admit(&usage, request.work_class, request.amount)?;

        let lease_expires_at_ms = now_ms
            .checked_add(lease_ms)
            .ok_or_else(|| QuotaError::new("quota_lease_overflow", "lease expiry overflow"))?;

        let done = sqlx::query(
            r#"
            INSERT INTO quota_reservations (
                tenant_id, reservation_id, work_class, amount, request_hash,
                lease_generation, lease_expires_at_ms, released_at_ms,
                created_at_ms, updated_at_ms
            ) VALUES (?, ?, ?, ?, ?, 1, ?, NULL, ?, ?)
            "#,
        )
        .bind(request.tenant_id.as_bytes())
        .bind(request.reservation_id.as_bytes())
        .bind(request.work_class.as_str())
        .bind(request.amount)
        .bind(request.request_hash.as_bytes())
        .bind(lease_expires_at_ms)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut **conn)
        .await
        .map_err(sqlite_error)?;

        if done.rows_affected() != 1 {
            return Err(QuotaError::new(
                "quota_reserve_no_effect",
                "reservation insert did not create exactly one row",
            ));
        }

        let record = read_reservation(conn, &request.tenant_id, &request.reservation_id)
            .await?
            .ok_or_else(|| {
                QuotaError::new(
                    "quota_reservation_missing",
                    "inserted reservation disappeared",
                )
            })?;

        Ok(ReserveOutcome::Reserved(record))
    }

    pub async fn renew(
        &self,
        tenant_id: &str,
        reservation_id: &str,
        expected_generation: i64,
        now_ms: i64,
        lease: Duration,
    ) -> Result<ReservationRecord, QuotaError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(reservation_id, "reservation_id")?;
        validate_generation(expected_generation)?;
        validate_now(now_ms)?;
        let lease_ms = validate_lease(lease)?;
        let next_expiry = now_ms
            .checked_add(lease_ms)
            .ok_or_else(|| QuotaError::new("quota_lease_overflow", "lease expiry overflow"))?;

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = async {
            let current = read_reservation(&mut conn, tenant_id, reservation_id)
                .await?
                .ok_or_else(|| {
                    QuotaError::new("reservation_not_found", "reservation does not exist")
                })?;
            if current.released_at_ms.is_some() {
                return Err(QuotaError::new(
                    "reservation_already_released",
                    "released reservation cannot be renewed",
                ));
            }
            if current.lease_generation != expected_generation {
                return Err(QuotaError::new(
                    "stale_lease_generation",
                    "reservation lease generation changed",
                ));
            }
            if current.lease_expires_at_ms <= now_ms {
                return Err(QuotaError::new(
                    "reservation_expired",
                    "reservation lease expired before renewal",
                ));
            }

            let next_generation = expected_generation
                .checked_add(1)
                .ok_or_else(|| {
                    QuotaError::new("quota_generation_overflow", "lease generation overflow")
                })?;

            let done = sqlx::query(
                r#"
                UPDATE quota_reservations
                SET lease_generation=?, lease_expires_at_ms=?, updated_at_ms=?
                WHERE tenant_id=? AND reservation_id=?
                  AND released_at_ms IS NULL
                  AND lease_generation=?
                  AND lease_expires_at_ms>?
                "#,
            )
            .bind(next_generation)
            .bind(next_expiry)
            .bind(now_ms)
            .bind(tenant_id.as_bytes())
            .bind(reservation_id.as_bytes())
            .bind(expected_generation)
            .bind(now_ms)
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            if done.rows_affected() != 1 {
                return Err(QuotaError::new(
                    "quota_renew_race",
                    "reservation changed before renewal",
                ));
            }

            read_reservation(&mut conn, tenant_id, reservation_id)
                .await?
                .ok_or_else(|| {
                    QuotaError::new("reservation_not_found", "renewed reservation disappeared")
                })
        }
        .await;

        finish_transaction(&mut conn, result).await
    }

    pub async fn release(
        &self,
        tenant_id: &str,
        reservation_id: &str,
        expected_generation: i64,
        now_ms: i64,
    ) -> Result<ReservationRecord, QuotaError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(reservation_id, "reservation_id")?;
        validate_generation(expected_generation)?;
        validate_now(now_ms)?;

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = async {
            let current = read_reservation(&mut conn, tenant_id, reservation_id)
                .await?
                .ok_or_else(|| {
                    QuotaError::new("reservation_not_found", "reservation does not exist")
                })?;
            if current.released_at_ms.is_some() {
                return Err(QuotaError::new(
                    "already_released",
                    "reservation was already released",
                ));
            }
            if current.lease_generation != expected_generation {
                return Err(QuotaError::new(
                    "stale_lease_generation",
                    "reservation lease generation changed",
                ));
            }
            if current.lease_expires_at_ms <= now_ms {
                return Err(QuotaError::new(
                    "reservation_expired",
                    "expired reservation must be reconciled instead of released by a stale owner",
                ));
            }

            let done = sqlx::query(
                r#"
                UPDATE quota_reservations
                SET released_at_ms=?, updated_at_ms=?
                WHERE tenant_id=? AND reservation_id=?
                  AND released_at_ms IS NULL
                  AND lease_generation=?
                  AND lease_expires_at_ms>?
                "#,
            )
            .bind(now_ms)
            .bind(now_ms)
            .bind(tenant_id.as_bytes())
            .bind(reservation_id.as_bytes())
            .bind(expected_generation)
            .bind(now_ms)
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            if done.rows_affected() != 1 {
                return Err(QuotaError::new(
                    "quota_release_race",
                    "reservation changed before release",
                ));
            }

            read_reservation(&mut conn, tenant_id, reservation_id)
                .await?
                .ok_or_else(|| {
                    QuotaError::new("reservation_not_found", "released reservation disappeared")
                })
        }
        .await;

        finish_transaction(&mut conn, result).await
    }

    pub async fn expire_stale(
        &self,
        tenant_id: &str,
        reservation_id: &str,
        observed_generation: i64,
        now_ms: i64,
    ) -> Result<ReservationRecord, QuotaError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(reservation_id, "reservation_id")?;
        validate_generation(observed_generation)?;
        validate_now(now_ms)?;

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = async {
            let current = read_reservation(&mut conn, tenant_id, reservation_id)
                .await?
                .ok_or_else(|| {
                    QuotaError::new("reservation_not_found", "reservation does not exist")
                })?;
            if current.released_at_ms.is_some() {
                return Ok(current);
            }
            if current.lease_generation != observed_generation {
                return Err(QuotaError::new(
                    "lease_advanced",
                    "reservation lease advanced after stale generation was observed",
                ));
            }
            if current.lease_expires_at_ms > now_ms {
                return Err(QuotaError::new(
                    "reservation_not_expired",
                    "reservation lease is still live",
                ));
            }

            let done = sqlx::query(
                r#"
                UPDATE quota_reservations
                SET released_at_ms=?, updated_at_ms=?
                WHERE tenant_id=? AND reservation_id=?
                  AND released_at_ms IS NULL
                  AND lease_generation=?
                  AND lease_expires_at_ms<=?
                "#,
            )
            .bind(now_ms)
            .bind(now_ms)
            .bind(tenant_id.as_bytes())
            .bind(reservation_id.as_bytes())
            .bind(observed_generation)
            .bind(now_ms)
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            if done.rows_affected() != 1 {
                return Err(QuotaError::new(
                    "quota_expiry_race",
                    "reservation changed before stale expiry",
                ));
            }

            read_reservation(&mut conn, tenant_id, reservation_id)
                .await?
                .ok_or_else(|| {
                    QuotaError::new("reservation_not_found", "expired reservation disappeared")
                })
        }
        .await;

        finish_transaction(&mut conn, result).await
    }

    pub async fn usage(&self, tenant_id: &str, now_ms: i64) -> Result<QuotaUsage, QuotaError> {
        require_ident(tenant_id, "tenant_id")?;
        validate_now(now_ms)?;
        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        usage_in_transaction(&mut conn, tenant_id, now_ms, &self.config).await
    }

    async fn require_schema(&self) -> Result<(), QuotaError> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='quota_reservations'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if exists != 1 {
            return Err(QuotaError::new(
                "quota_schema_missing",
                "quota_reservations is absent; run chaptera migrate up before runtime open",
            ));
        }
        Ok(())
    }

    async fn verify_profile(&self) -> Result<(), QuotaError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(QuotaError::new(
                "quota_profile_mismatch",
                format!("expected WAL journal mode, got {journal_mode}"),
            ));
        }

        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if synchronous != 2 {
            return Err(QuotaError::new(
                "quota_profile_mismatch",
                format!("expected synchronous=FULL(2), got {synchronous}"),
            ));
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if foreign_keys != 1 {
            return Err(QuotaError::new(
                "quota_profile_mismatch",
                "foreign_keys pragma is not enabled",
            ));
        }
        Ok(())
    }

    fn admit(
        &self,
        usage: &QuotaUsage,
        work_class: QuotaWorkClass,
        amount: i64,
    ) -> Result<(), QuotaError> {
        match work_class {
            QuotaWorkClass::Export => {
                if checked_add(usage.export, amount)? > self.config.export_cap {
                    return Err(QuotaError::new(
                        "export_concurrency_quota",
                        "export reservation exceeds the tenant export cap",
                    ));
                }
                if checked_add(usage.shared_total, amount)? > self.config.shared_capacity {
                    return Err(QuotaError::new(
                        "shared_capacity_exhausted",
                        "export reservation exceeds shared capacity",
                    ));
                }
            }
            QuotaWorkClass::Background => {
                if checked_add(usage.background, amount)? > self.config.background_cap
                    || checked_add(usage.shared_total, amount)? > self.config.shared_capacity
                {
                    return Err(QuotaError::new(
                        "background_budget_paused",
                        "background reservation is paused by class/shared capacity",
                    ));
                }
            }
            QuotaWorkClass::Interactive => {
                let lower = checked_add(usage.export, usage.background)?;
                let interactive_shared_available =
                    self.config.shared_capacity.saturating_sub(lower);
                let interactive_capacity =
                    checked_add(interactive_shared_available, self.config.semantic_headroom)?;
                if checked_add(usage.interactive, amount)? > interactive_capacity {
                    return Err(QuotaError::new(
                        "semantic_headroom_exhausted",
                        "interactive reservation exceeds shared plus protected semantic headroom",
                    ));
                }
            }
        }
        Ok(())
    }
}

async fn finish_transaction<T>(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    result: Result<T, QuotaError>,
) -> Result<T, QuotaError> {
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

async fn reconcile_expired_for_tenant(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    tenant_id: &str,
    now_ms: i64,
) -> Result<(), QuotaError> {
    sqlx::query(
        r#"
        UPDATE quota_reservations
        SET released_at_ms=lease_expires_at_ms, updated_at_ms=?
        WHERE tenant_id=? AND released_at_ms IS NULL AND lease_expires_at_ms<=?
        "#,
    )
    .bind(now_ms)
    .bind(tenant_id.as_bytes())
    .bind(now_ms)
    .execute(&mut **conn)
    .await
    .map_err(sqlite_error)?;
    Ok(())
}

async fn usage_in_transaction(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    tenant_id: &str,
    now_ms: i64,
    config: &QuotaConfig,
) -> Result<QuotaUsage, QuotaError> {
    let rows = sqlx::query(
        r#"
        SELECT work_class, COALESCE(SUM(amount), 0) AS total
        FROM quota_reservations
        WHERE tenant_id=? AND released_at_ms IS NULL AND lease_expires_at_ms>?
        GROUP BY work_class
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(now_ms)
    .fetch_all(&mut **conn)
    .await
    .map_err(sqlite_error)?;

    let mut usage = QuotaUsage::default();
    for row in rows {
        let class: String = row.try_get("work_class").map_err(sqlite_error)?;
        let total: i64 = row.try_get("total").map_err(sqlite_error)?;
        match QuotaWorkClass::parse(&class)? {
            QuotaWorkClass::Interactive => usage.interactive = total,
            QuotaWorkClass::Export => usage.export = total,
            QuotaWorkClass::Background => usage.background = total,
        }
    }

    let lower = checked_add(usage.export, usage.background)?;
    let interactive_shared_available = config.shared_capacity.saturating_sub(lower);
    let shared_interactive = usage.interactive.min(interactive_shared_available);

    usage.shared_total = checked_add(lower, shared_interactive)?;
    usage.protected_interactive = usage.interactive.saturating_sub(shared_interactive);
    Ok(usage)
}

async fn read_reservation(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    tenant_id: &str,
    reservation_id: &str,
) -> Result<Option<ReservationRecord>, QuotaError> {
    sqlx::query(
        r#"
        SELECT tenant_id, reservation_id, work_class, amount, request_hash,
               lease_generation, lease_expires_at_ms, released_at_ms,
               created_at_ms, updated_at_ms
        FROM quota_reservations
        WHERE tenant_id=? AND reservation_id=?
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(reservation_id.as_bytes())
    .fetch_optional(&mut **conn)
    .await
    .map_err(sqlite_error)?
    .map(decode_reservation)
    .transpose()
}

fn decode_reservation(row: sqlx::sqlite::SqliteRow) -> Result<ReservationRecord, QuotaError> {
    let work_class: String = row.try_get("work_class").map_err(sqlite_error)?;
    let record = ReservationRecord {
        tenant_id: blob_text(&row, "tenant_id")?,
        reservation_id: blob_text(&row, "reservation_id")?,
        work_class: QuotaWorkClass::parse(&work_class)?,
        amount: row.try_get("amount").map_err(sqlite_error)?,
        request_hash: blob_text(&row, "request_hash")?,
        lease_generation: row.try_get("lease_generation").map_err(sqlite_error)?,
        lease_expires_at_ms: row.try_get("lease_expires_at_ms").map_err(sqlite_error)?,
        released_at_ms: row.try_get("released_at_ms").map_err(sqlite_error)?,
        created_at_ms: row.try_get("created_at_ms").map_err(sqlite_error)?,
        updated_at_ms: row.try_get("updated_at_ms").map_err(sqlite_error)?,
    };

    if record.amount <= 0
        || record.lease_generation <= 0
        || record.lease_expires_at_ms <= record.created_at_ms
        || record.updated_at_ms < record.created_at_ms
    {
        return Err(QuotaError::new(
            "quota_row_corrupt",
            "persisted quota reservation violates bounds",
        ));
    }
    require_sha256(&record.request_hash)?;
    Ok(record)
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, QuotaError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| QuotaError::new("quota_row_corrupt", format!("{column} is not UTF-8")))
}

fn validate_request(request: &ReserveRequest) -> Result<(), QuotaError> {
    require_ident(&request.tenant_id, "tenant_id")?;
    require_ident(&request.reservation_id, "reservation_id")?;
    if request.amount <= 0 {
        return Err(QuotaError::new(
            "invalid_quota_amount",
            "reservation amount must be positive",
        ));
    }
    require_sha256(&request.request_hash)
}

fn require_ident(value: &str, label: &'static str) -> Result<(), QuotaError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(QuotaError::new(
            "invalid_quota_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_sha256(value: &str) -> Result<(), QuotaError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(QuotaError::new(
            "invalid_quota_request_hash",
            "request_hash must be lowercase SHA-256",
        ));
    }
    Ok(())
}

fn validate_generation(generation: i64) -> Result<(), QuotaError> {
    if generation <= 0 {
        return Err(QuotaError::new(
            "invalid_lease_generation",
            "lease generation must be positive",
        ));
    }
    Ok(())
}

fn validate_now(now_ms: i64) -> Result<(), QuotaError> {
    if now_ms < 0 {
        return Err(QuotaError::new(
            "invalid_quota_time",
            "quota timestamp must be non-negative",
        ));
    }
    Ok(())
}

fn validate_lease(lease: Duration) -> Result<i64, QuotaError> {
    if lease.is_zero() || lease > MAX_LEASE {
        return Err(QuotaError::new(
            "invalid_quota_lease",
            "quota lease must be >0 and <=24 hours",
        ));
    }
    i64::try_from(lease.as_millis())
        .map_err(|_| QuotaError::new("quota_lease_overflow", "quota lease does not fit i64 millis"))
}

fn checked_add(left: i64, right: i64) -> Result<i64, QuotaError> {
    left.checked_add(right)
        .ok_or_else(|| QuotaError::new("quota_capacity_overflow", "quota capacity arithmetic overflow"))
}

fn sqlite_error(error: impl fmt::Display) -> QuotaError {
    QuotaError::new(
        "quota_sqlite_error",
        error.to_string().chars().take(512).collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-quota-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn config() -> QuotaConfig {
        QuotaConfig {
            shared_capacity: 10,
            semantic_headroom: 5,
            export_cap: 4,
            background_cap: 4,
        }
    }

    fn request(id: &str, work_class: QuotaWorkClass, amount: i64) -> ReserveRequest {
        ReserveRequest {
            tenant_id: "tenant-a".into(),
            reservation_id: id.into(),
            work_class,
            amount,
            request_hash: "a".repeat(64),
        }
    }

    async fn authority(label: &str) -> (SqliteQuotaAuthority, PathBuf) {
        authority_with_config(label, config()).await
    }

    async fn authority_with_config(
        label: &str,
        config: QuotaConfig,
    ) -> (SqliteQuotaAuthority, PathBuf) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let authority =
            SqliteQuotaAuthority::open(&path, 4, Duration::from_secs(2), config)
                .await
                .unwrap();
        (authority, path)
    }

    #[tokio::test]
    async fn exact_retry_is_idempotent_and_changed_request_conflicts() {
        let (authority, path) = authority("retry").await;
        let first = authority
            .reserve(
                request("r1", QuotaWorkClass::Export, 2),
                10,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        let second = authority
            .reserve(
                request("r1", QuotaWorkClass::Export, 2),
                11,
                Duration::from_secs(10),
            )
            .await
            .unwrap();

        let first = match first {
            ReserveOutcome::Reserved(row) => row,
            ReserveOutcome::Existing(_) => panic!("first reserve must create"),
        };
        let second = match second {
            ReserveOutcome::Existing(row) => row,
            ReserveOutcome::Reserved(_) => panic!("exact retry must reuse"),
        };
        assert_eq!(first, second);

        let error = authority
            .reserve(
                request("r1", QuotaWorkClass::Export, 3),
                12,
                Duration::from_secs(10),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "reservation_conflict");

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn lower_classes_cannot_consume_protected_semantic_headroom() {
        let (authority, path) = authority_with_config(
            "headroom",
            QuotaConfig {
                shared_capacity: 10,
                semantic_headroom: 5,
                export_cap: 10,
                background_cap: 4,
            },
        )
        .await;

        authority
            .reserve(
                request("interactive-1", QuotaWorkClass::Interactive, 6),
                10,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        authority
            .reserve(
                request("export-1", QuotaWorkClass::Export, 4),
                11,
                Duration::from_secs(10),
            )
            .await
            .unwrap();

        let blocked = authority
            .reserve(
                request("export-2", QuotaWorkClass::Export, 1),
                12,
                Duration::from_secs(10),
            )
            .await
            .unwrap_err();
        assert_eq!(blocked.code, "shared_capacity_exhausted");

        authority
            .reserve(
                request("interactive-2", QuotaWorkClass::Interactive, 5),
                13,
                Duration::from_secs(10),
            )
            .await
            .unwrap();

        let usage = authority.usage("tenant-a", 14).await.unwrap();
        assert_eq!(usage.interactive, 11);
        assert_eq!(usage.export, 4);
        assert_eq!(usage.protected_interactive, 5);

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn renewal_fences_stale_generation_and_stale_expiry() {
        let (authority, path) = authority("renew").await;
        let reserved = authority
            .reserve(
                request("r1", QuotaWorkClass::Background, 1),
                10,
                Duration::from_millis(100),
            )
            .await
            .unwrap();
        let reserved = match reserved {
            ReserveOutcome::Reserved(row) => row,
            ReserveOutcome::Existing(_) => unreachable!(),
        };

        let renewed = authority
            .renew(
                "tenant-a",
                "r1",
                reserved.lease_generation,
                50,
                Duration::from_millis(100),
            )
            .await
            .unwrap();
        assert!(renewed.lease_generation > reserved.lease_generation);

        let stale = authority
            .expire_stale("tenant-a", "r1", reserved.lease_generation, 151)
            .await
            .unwrap_err();
        assert_eq!(stale.code, "lease_advanced");

        let expired = authority
            .expire_stale("tenant-a", "r1", renewed.lease_generation, 151)
            .await
            .unwrap();
        assert!(expired.released_at_ms.is_some());

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn release_is_exact_once() {
        let (authority, path) = authority("release").await;
        let reserved = authority
            .reserve(
                request("r1", QuotaWorkClass::Export, 1),
                10,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        let reserved = match reserved {
            ReserveOutcome::Reserved(row) => row,
            ReserveOutcome::Existing(_) => unreachable!(),
        };

        let released = authority
            .release("tenant-a", "r1", reserved.lease_generation, 20)
            .await
            .unwrap();
        assert_eq!(released.released_at_ms, Some(20));

        let error = authority
            .release("tenant-a", "r1", reserved.lease_generation, 21)
            .await
            .unwrap_err();
        assert_eq!(error.code, "already_released");

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn concurrent_export_reservations_cannot_exceed_class_cap() {
        let (authority, path) = authority("race").await;
        let left = authority.clone();
        let right = authority.clone();

        let (left, right) = tokio::join!(
            left.reserve(
                request("left", QuotaWorkClass::Export, 4),
                10,
                Duration::from_secs(10)
            ),
            right.reserve(
                request("right", QuotaWorkClass::Export, 4),
                10,
                Duration::from_secs(10)
            )
        );

        let accepted = usize::from(left.is_ok()) + usize::from(right.is_ok());
        assert_eq!(accepted, 1);
        let rejected = if let Err(error) = left {
            error
        } else {
            right.unwrap_err()
        };
        assert_eq!(rejected.code, "export_concurrency_quota");

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn restart_preserves_active_reservation_and_exact_retry() {
        let (authority, path) = authority("restart").await;
        let first = authority
            .reserve(
                request("r1", QuotaWorkClass::Background, 2),
                10,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        let first = match first {
            ReserveOutcome::Reserved(row) => row,
            ReserveOutcome::Existing(_) => unreachable!(),
        };
        authority.close().await;

        let reopened =
            SqliteQuotaAuthority::open(&path, 2, Duration::from_secs(2), config())
                .await
                .unwrap();
        let usage = reopened.usage("tenant-a", 11).await.unwrap();
        assert_eq!(usage.background, 2);

        let retry = reopened
            .reserve(
                request("r1", QuotaWorkClass::Background, 2),
                11,
                Duration::from_secs(10),
            )
            .await
            .unwrap();
        match retry {
            ReserveOutcome::Existing(row) => assert_eq!(row, first),
            ReserveOutcome::Reserved(_) => panic!("restart retry created duplicate reservation"),
        }

        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn open_requires_operator_migration_and_does_not_create_database() {
        let path = temp_db("missing");
        let result =
            SqliteQuotaAuthority::open(&path, 1, Duration::from_secs(1), config()).await;
        let error = result.err().expect("unmigrated quota open must fail");
        assert_eq!(error.code, "quota_database_missing");
        assert!(!path.exists());
        cleanup(&path);
    }
}
