use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::{
    Connection, Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAdmissionError {
    pub code: &'static str,
    pub message: String,
}

impl UploadAdmissionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for UploadAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for UploadAdmissionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAdmissionConfig {
    pub principal_concurrent_cap: i64,
    pub tenant_concurrent_cap: i64,
    pub principal_bytes_cap: i64,
    pub tenant_bytes_cap: i64,
    pub max_single_upload_bytes: i64,
    pub lease_duration: Duration,
    pub retention: Duration,
}

impl UploadAdmissionConfig {
    pub fn validate(&self) -> Result<(), UploadAdmissionError> {
        for (name, value) in [
            ("principal_concurrent_cap", self.principal_concurrent_cap),
            ("tenant_concurrent_cap", self.tenant_concurrent_cap),
            ("principal_bytes_cap", self.principal_bytes_cap),
            ("tenant_bytes_cap", self.tenant_bytes_cap),
            ("max_single_upload_bytes", self.max_single_upload_bytes),
        ] {
            if value <= 0 {
                return Err(UploadAdmissionError::new(
                    "upload_admission_config_invalid",
                    format!("{name} must be positive"),
                ));
            }
        }
        if self.principal_concurrent_cap > self.tenant_concurrent_cap {
            return Err(UploadAdmissionError::new(
                "upload_admission_config_invalid",
                "principal concurrent cap cannot exceed tenant concurrent cap",
            ));
        }
        if self.principal_bytes_cap > self.tenant_bytes_cap {
            return Err(UploadAdmissionError::new(
                "upload_admission_config_invalid",
                "principal byte cap cannot exceed tenant byte cap",
            ));
        }
        if self.max_single_upload_bytes > self.principal_bytes_cap {
            return Err(UploadAdmissionError::new(
                "upload_admission_config_invalid",
                "single-upload byte cap cannot exceed principal byte cap",
            ));
        }
        if self.lease_duration.is_zero() || self.lease_duration > Duration::from_secs(24 * 60 * 60)
        {
            return Err(UploadAdmissionError::new(
                "upload_admission_config_invalid",
                "upload admission lease must be >0 and <=24 hours",
            ));
        }
        if self.retention.is_zero() || self.retention > Duration::from_secs(30 * 24 * 60 * 60) {
            return Err(UploadAdmissionError::new(
                "upload_admission_config_invalid",
                "upload admission retention must be >0 and <=30 days",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAdmissionRequest {
    pub reservation_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub expected_bytes: i64,
    pub request_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAdmissionReservation {
    pub reservation_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub expected_bytes: i64,
    pub request_hash: String,
    pub lease_generation: i64,
    pub lease_expires_at_ms: i64,
    pub released_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveUploadOutcome {
    Reserved(UploadAdmissionReservation),
    Existing(UploadAdmissionReservation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseUploadOutcome {
    Released,
    AlreadyReleased,
}

#[derive(Clone)]
pub struct SqliteUploadAdmissionAuthority {
    path: PathBuf,
    pool: SqlitePool,
    config: UploadAdmissionConfig,
}

impl SqliteUploadAdmissionAuthority {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
        config: UploadAdmissionConfig,
    ) -> Result<Self, UploadAdmissionError> {
        config.validate()?;
        if !(1..=16).contains(&max_connections) {
            return Err(UploadAdmissionError::new(
                "upload_admission_pool_size_invalid",
                "upload admission SQLite pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(UploadAdmissionError::new(
                "upload_admission_busy_timeout_invalid",
                "upload admission SQLite busy timeout must be >0 and <=30 seconds",
            ));
        }
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(UploadAdmissionError::new(
                "upload_admission_database_missing",
                "run chaptera migrate up before opening upload admission",
            ));
        }

        let options = SqliteConnectOptions::new()
            .filename(&path)
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

        let authority = Self { path, pool, config };
        authority.require_schema().await?;
        Ok(authority)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn reserve(
        &self,
        request: UploadAdmissionRequest,
        now_ms: i64,
    ) -> Result<ReserveUploadOutcome, UploadAdmissionError> {
        validate_request(&request)?;
        if now_ms < 0 {
            return Err(UploadAdmissionError::new(
                "upload_admission_time_invalid",
                "now_ms must be non-negative",
            ));
        }
        if request.expected_bytes > self.config.max_single_upload_bytes {
            return Err(UploadAdmissionError::new(
                "upload_bytes_too_large",
                "declared upload exceeds configured single-upload byte cap",
            ));
        }
        let lease_ms = duration_ms(self.config.lease_duration, "lease_duration")?;
        let lease_expires_at_ms = now_ms.checked_add(lease_ms).ok_or_else(|| {
            UploadAdmissionError::new(
                "upload_admission_time_overflow",
                "upload admission lease expiry overflows i64 milliseconds",
            )
        })?;

        let mut connection = self.pool.acquire().await.map_err(sqlite_error)?;
        let mut tx = (&mut *connection)
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlite_error)?;
        cleanup_old_rows(&mut tx, now_ms, self.config.retention).await?;

        if let Some(existing) = fetch_reservation(&mut tx, &request.reservation_id).await? {
            if existing.tenant_id != request.tenant_id
                || existing.principal_id != request.principal_id
                || existing.expected_bytes != request.expected_bytes
                || existing.request_hash != request.request_hash
            {
                return Err(UploadAdmissionError::new(
                    "upload_admission_idempotency_conflict",
                    "upload reservation id was reused with different input",
                ));
            }
            if existing.released_at_ms.is_some() {
                return Err(UploadAdmissionError::new(
                    "upload_admission_already_released",
                    "released upload reservation cannot be reserved again",
                ));
            }
            if existing.lease_expires_at_ms <= now_ms {
                return Err(UploadAdmissionError::new(
                    "upload_admission_lease_expired",
                    "expired upload reservation must use a new reservation identity",
                ));
            }
            tx.commit().await.map_err(sqlite_error)?;
            return Ok(ReserveUploadOutcome::Existing(existing));
        }

        require_active_principal(&mut tx, &request.principal_id).await?;

        let tenant_usage = active_usage(&mut *tx, "tenant_id", &request.tenant_id, now_ms).await?;
        let principal_usage =
            active_usage(&mut *tx, "principal_id", &request.principal_id, now_ms).await?;

        enforce_capacity(
            principal_usage,
            request.expected_bytes,
            self.config.principal_concurrent_cap,
            self.config.principal_bytes_cap,
            "upload_principal_capacity",
        )?;
        enforce_capacity(
            tenant_usage,
            request.expected_bytes,
            self.config.tenant_concurrent_cap,
            self.config.tenant_bytes_cap,
            "upload_tenant_capacity",
        )?;

        sqlx::query(
            r#"
            INSERT INTO upload_admission_reservations (
                reservation_id, tenant_id, principal_id, expected_bytes, request_hash,
                lease_generation, lease_expires_at_ms, released_at_ms,
                created_at_ms, updated_at_ms
            ) VALUES (?, ?, ?, ?, ?, 0, ?, NULL, ?, ?)
            "#,
        )
        .bind(request.reservation_id.as_bytes())
        .bind(request.tenant_id.as_bytes())
        .bind(request.principal_id.as_bytes())
        .bind(request.expected_bytes)
        .bind(request.request_hash.as_bytes())
        .bind(lease_expires_at_ms)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            if is_unique_violation(&error) {
                UploadAdmissionError::new(
                    "upload_admission_race_conflict",
                    "concurrent reservation creation raced on the same identity",
                )
            } else {
                sqlite_error(error)
            }
        })?;

        tx.commit().await.map_err(sqlite_error)?;
        Ok(ReserveUploadOutcome::Reserved(UploadAdmissionReservation {
            reservation_id: request.reservation_id,
            tenant_id: request.tenant_id,
            principal_id: request.principal_id,
            expected_bytes: request.expected_bytes,
            request_hash: request.request_hash,
            lease_generation: 0,
            lease_expires_at_ms,
            released_at_ms: None,
        }))
    }

    pub async fn renew(
        &self,
        tenant_id: &str,
        reservation_id: &str,
        expected_generation: i64,
        now_ms: i64,
    ) -> Result<UploadAdmissionReservation, UploadAdmissionError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(reservation_id, "reservation_id")?;
        if expected_generation < 0 || now_ms < 0 {
            return Err(UploadAdmissionError::new(
                "upload_admission_renew_invalid",
                "generation and now_ms must be non-negative",
            ));
        }
        let lease_ms = duration_ms(self.config.lease_duration, "lease_duration")?;
        let next_expiry = now_ms.checked_add(lease_ms).ok_or_else(|| {
            UploadAdmissionError::new(
                "upload_admission_time_overflow",
                "upload admission lease expiry overflows i64 milliseconds",
            )
        })?;

        let result = sqlx::query(
            r#"
            UPDATE upload_admission_reservations
            SET lease_generation = lease_generation + 1,
                lease_expires_at_ms = ?,
                updated_at_ms = ?
            WHERE reservation_id = ?
              AND tenant_id = ?
              AND lease_generation = ?
              AND released_at_ms IS NULL
              AND lease_expires_at_ms > ?
            "#,
        )
        .bind(next_expiry)
        .bind(now_ms)
        .bind(reservation_id.as_bytes())
        .bind(tenant_id.as_bytes())
        .bind(expected_generation)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if result.rows_affected() != 1 {
            return Err(UploadAdmissionError::new(
                "upload_admission_renew_conflict",
                "upload reservation was released, expired, missing or has a stale generation",
            ));
        }

        fetch_reservation_pool(&self.pool, reservation_id)
            .await?
            .ok_or_else(|| {
                UploadAdmissionError::new(
                    "upload_admission_row_missing",
                    "renewed upload reservation disappeared",
                )
            })
    }

    pub async fn release(
        &self,
        tenant_id: &str,
        reservation_id: &str,
        expected_generation: i64,
        now_ms: i64,
    ) -> Result<ReleaseUploadOutcome, UploadAdmissionError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(reservation_id, "reservation_id")?;
        if expected_generation < 0 || now_ms < 0 {
            return Err(UploadAdmissionError::new(
                "upload_admission_release_invalid",
                "generation and now_ms must be non-negative",
            ));
        }

        let result = sqlx::query(
            r#"
            UPDATE upload_admission_reservations
            SET released_at_ms = ?, updated_at_ms = ?
            WHERE reservation_id = ?
              AND tenant_id = ?
              AND lease_generation = ?
              AND released_at_ms IS NULL
            "#,
        )
        .bind(now_ms)
        .bind(now_ms)
        .bind(reservation_id.as_bytes())
        .bind(tenant_id.as_bytes())
        .bind(expected_generation)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if result.rows_affected() == 1 {
            return Ok(ReleaseUploadOutcome::Released);
        }

        let Some(existing) = fetch_reservation_pool(&self.pool, reservation_id).await? else {
            return Err(UploadAdmissionError::new(
                "upload_admission_not_found",
                "upload reservation does not exist",
            ));
        };
        if existing.tenant_id != tenant_id || existing.lease_generation != expected_generation {
            return Err(UploadAdmissionError::new(
                "upload_admission_release_conflict",
                "upload reservation tenant or lease generation does not match",
            ));
        }
        if existing.released_at_ms.is_some() {
            return Ok(ReleaseUploadOutcome::AlreadyReleased);
        }
        Err(UploadAdmissionError::new(
            "upload_admission_release_conflict",
            "upload reservation could not be released",
        ))
    }

    pub async fn usage(
        &self,
        tenant_id: &str,
        principal_id: &str,
        now_ms: i64,
    ) -> Result<(i64, i64, i64, i64), UploadAdmissionError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(principal_id, "principal_id")?;
        if now_ms < 0 {
            return Err(UploadAdmissionError::new(
                "upload_admission_time_invalid",
                "now_ms must be non-negative",
            ));
        }
        let mut connection = self.pool.acquire().await.map_err(sqlite_error)?;
        let tenant = active_usage(&mut *connection, "tenant_id", tenant_id, now_ms).await?;
        let principal =
            active_usage(&mut *connection, "principal_id", principal_id, now_ms).await?;
        Ok((tenant.0, tenant.1, principal.0, principal.1))
    }

    async fn require_schema(&self) -> Result<(), UploadAdmissionError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='upload_admission_reservations'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;
        if count != 1 {
            return Err(UploadAdmissionError::new(
                "upload_admission_schema_missing",
                "upload_admission_reservations table is absent; run chaptera migrate up",
            ));
        }
        Ok(())
    }
}

async fn require_active_principal(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    principal_id: &str,
) -> Result<(), UploadAdmissionError> {
    let rows = sqlx::query("SELECT disabled_at_ms FROM principals WHERE principal_id=? LIMIT 2")
        .bind(principal_id.as_bytes())
        .fetch_all(&mut **tx)
        .await
        .map_err(sqlite_error)?;
    match rows.as_slice() {
        [] => Err(UploadAdmissionError::new(
            "upload_admission_principal_missing",
            "authenticated principal does not exist",
        )),
        [row] => {
            let disabled: Option<i64> = row.try_get("disabled_at_ms").map_err(sqlite_error)?;
            if disabled.is_some() {
                Err(UploadAdmissionError::new(
                    "upload_admission_principal_disabled",
                    "disabled principal cannot reserve upload capacity",
                ))
            } else {
                Ok(())
            }
        }
        _ => Err(UploadAdmissionError::new(
            "upload_admission_principal_ambiguous",
            "principal identity resolved to multiple rows",
        )),
    }
}

async fn active_usage<'e, E>(
    executor: E,
    column: &'static str,
    value: &str,
    now_ms: i64,
) -> Result<(i64, i64), UploadAdmissionError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let query = match column {
        "tenant_id" => {
            "SELECT COUNT(*), COALESCE(SUM(expected_bytes), 0) FROM upload_admission_reservations WHERE tenant_id=? AND released_at_ms IS NULL AND lease_expires_at_ms>?"
        }
        "principal_id" => {
            "SELECT COUNT(*), COALESCE(SUM(expected_bytes), 0) FROM upload_admission_reservations WHERE principal_id=? AND released_at_ms IS NULL AND lease_expires_at_ms>?"
        }
        _ => {
            return Err(UploadAdmissionError::new(
                "upload_admission_internal_error",
                "unsupported upload usage dimension",
            ));
        }
    };
    let row = sqlx::query(query)
        .bind(value.as_bytes())
        .bind(now_ms)
        .fetch_one(executor)
        .await
        .map_err(sqlite_error)?;
    Ok((
        row.try_get(0).map_err(sqlite_error)?,
        row.try_get(1).map_err(sqlite_error)?,
    ))
}

fn enforce_capacity(
    usage: (i64, i64),
    requested_bytes: i64,
    concurrent_cap: i64,
    byte_cap: i64,
    code: &'static str,
) -> Result<(), UploadAdmissionError> {
    let next_count = usage.0.checked_add(1).ok_or_else(|| {
        UploadAdmissionError::new(
            "upload_admission_capacity_overflow",
            "upload count overflow",
        )
    })?;
    let next_bytes = usage.1.checked_add(requested_bytes).ok_or_else(|| {
        UploadAdmissionError::new(
            "upload_admission_capacity_overflow",
            "upload byte usage overflow",
        )
    })?;
    if next_count > concurrent_cap || next_bytes > byte_cap {
        return Err(UploadAdmissionError::new(
            code,
            "upload admission capacity is exhausted",
        ));
    }
    Ok(())
}

async fn cleanup_old_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    now_ms: i64,
    retention: Duration,
) -> Result<(), UploadAdmissionError> {
    let retention_ms = duration_ms(retention, "retention")?;
    let cutoff = now_ms.saturating_sub(retention_ms);
    sqlx::query(
        r#"
        DELETE FROM upload_admission_reservations
        WHERE updated_at_ms < ?
          AND (
            released_at_ms IS NOT NULL
            OR lease_expires_at_ms <= ?
          )
        "#,
    )
    .bind(cutoff)
    .bind(cutoff)
    .execute(&mut **tx)
    .await
    .map_err(sqlite_error)?;
    Ok(())
}

async fn fetch_reservation(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    reservation_id: &str,
) -> Result<Option<UploadAdmissionReservation>, UploadAdmissionError> {
    let rows = sqlx::query(
        r#"
        SELECT reservation_id, tenant_id, principal_id, expected_bytes, request_hash,
               lease_generation, lease_expires_at_ms, released_at_ms
        FROM upload_admission_reservations
        WHERE reservation_id=?
        LIMIT 2
        "#,
    )
    .bind(reservation_id.as_bytes())
    .fetch_all(&mut **tx)
    .await
    .map_err(sqlite_error)?;
    parse_single_reservation(rows)
}

async fn fetch_reservation_pool(
    pool: &SqlitePool,
    reservation_id: &str,
) -> Result<Option<UploadAdmissionReservation>, UploadAdmissionError> {
    let rows = sqlx::query(
        r#"
        SELECT reservation_id, tenant_id, principal_id, expected_bytes, request_hash,
               lease_generation, lease_expires_at_ms, released_at_ms
        FROM upload_admission_reservations
        WHERE reservation_id=?
        LIMIT 2
        "#,
    )
    .bind(reservation_id.as_bytes())
    .fetch_all(pool)
    .await
    .map_err(sqlite_error)?;
    parse_single_reservation(rows)
}

fn parse_single_reservation(
    rows: Vec<sqlx::sqlite::SqliteRow>,
) -> Result<Option<UploadAdmissionReservation>, UploadAdmissionError> {
    if rows.len() > 1 {
        return Err(UploadAdmissionError::new(
            "upload_admission_row_ambiguous",
            "upload reservation resolved to multiple rows",
        ));
    }
    rows.first().map(parse_reservation).transpose()
}

fn parse_reservation(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<UploadAdmissionReservation, UploadAdmissionError> {
    Ok(UploadAdmissionReservation {
        reservation_id: blob_text(row, "reservation_id")?,
        tenant_id: blob_text(row, "tenant_id")?,
        principal_id: blob_text(row, "principal_id")?,
        expected_bytes: row.try_get("expected_bytes").map_err(sqlite_error)?,
        request_hash: blob_text(row, "request_hash")?,
        lease_generation: row.try_get("lease_generation").map_err(sqlite_error)?,
        lease_expires_at_ms: row.try_get("lease_expires_at_ms").map_err(sqlite_error)?,
        released_at_ms: row.try_get("released_at_ms").map_err(sqlite_error)?,
    })
}

fn validate_request(request: &UploadAdmissionRequest) -> Result<(), UploadAdmissionError> {
    require_ident(&request.reservation_id, "reservation_id")?;
    require_ident(&request.tenant_id, "tenant_id")?;
    require_ident(&request.principal_id, "principal_id")?;
    if request.expected_bytes <= 0 {
        return Err(UploadAdmissionError::new(
            "upload_admission_bytes_invalid",
            "expected upload bytes must be positive",
        ));
    }
    require_hash(&request.request_hash)
}

fn require_ident(value: &str, label: &'static str) -> Result<(), UploadAdmissionError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(UploadAdmissionError::new(
            "upload_admission_identity_invalid",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_hash(value: &str) -> Result<(), UploadAdmissionError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(UploadAdmissionError::new(
            "upload_admission_request_hash_invalid",
            "request hash must be 64 lowercase hexadecimal characters",
        ));
    }
    Ok(())
}

fn duration_ms(value: Duration, label: &'static str) -> Result<i64, UploadAdmissionError> {
    i64::try_from(value.as_millis()).map_err(|_| {
        UploadAdmissionError::new(
            "upload_admission_duration_overflow",
            format!("{label} does not fit i64 milliseconds"),
        )
    })
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, UploadAdmissionError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        UploadAdmissionError::new(
            "upload_admission_row_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code == "2067" || code == "1555")
}

fn sqlite_error(error: impl fmt::Display) -> UploadAdmissionError {
    UploadAdmissionError::new(
        "sqlite_upload_admission_error",
        error.to_string().chars().take(512).collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-upload-admission-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn config() -> UploadAdmissionConfig {
        UploadAdmissionConfig {
            principal_concurrent_cap: 2,
            tenant_concurrent_cap: 3,
            principal_bytes_cap: 1000,
            tenant_bytes_cap: 1800,
            max_single_upload_bytes: 900,
            lease_duration: Duration::from_millis(100),
            retention: Duration::from_millis(200),
        }
    }

    async fn setup(label: &str) -> (PathBuf, SqliteUploadAdmissionAuthority) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(false)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO principals(principal_id, created_at_ms, disabled_at_ms) VALUES (?, 1, NULL)",
        )
        .bind(b"principal-a".as_slice())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO principals(principal_id, created_at_ms, disabled_at_ms) VALUES (?, 1, NULL)",
        )
        .bind(b"principal-b".as_slice())
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let authority =
            SqliteUploadAdmissionAuthority::open(&path, 4, Duration::from_secs(2), config())
                .await
                .unwrap();
        (path, authority)
    }

    fn request(id: &str, principal: &str, bytes: i64) -> UploadAdmissionRequest {
        UploadAdmissionRequest {
            reservation_id: id.into(),
            tenant_id: "tenant-a".into(),
            principal_id: principal.into(),
            expected_bytes: bytes,
            request_hash: format!("{:064x}", id.bytes().fold(1_u64, |a, b| a + u64::from(b))),
        }
    }

    #[tokio::test]
    async fn exact_retry_survives_reopen_and_changed_input_conflicts() {
        let (path, authority) = setup("retry").await;
        let req = request("reserve-a", "principal-a", 400);
        let first = authority.reserve(req.clone(), 10).await.unwrap();
        let first = match first {
            ReserveUploadOutcome::Reserved(record) => record,
            other => panic!("unexpected first outcome: {other:?}"),
        };
        assert!(matches!(
            authority.reserve(req.clone(), 20).await.unwrap(),
            ReserveUploadOutcome::Existing(record) if record == first
        ));
        authority.close().await;

        let reopened =
            SqliteUploadAdmissionAuthority::open(&path, 4, Duration::from_secs(2), config())
                .await
                .unwrap();
        assert!(matches!(
            reopened.reserve(req.clone(), 30).await.unwrap(),
            ReserveUploadOutcome::Existing(record) if record == first
        ));
        let mut changed = req;
        changed.expected_bytes = 401;
        assert_eq!(
            reopened.reserve(changed, 30).await.unwrap_err().code,
            "upload_admission_idempotency_conflict"
        );

        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn principal_and_tenant_concurrency_and_bytes_fail_closed() {
        let (path, authority) = setup("capacity").await;
        authority
            .reserve(request("a1", "principal-a", 500), 10)
            .await
            .unwrap();
        authority
            .reserve(request("a2", "principal-a", 500), 10)
            .await
            .unwrap();
        assert_eq!(
            authority
                .reserve(request("a3", "principal-a", 1), 10)
                .await
                .unwrap_err()
                .code,
            "upload_principal_capacity"
        );

        authority
            .reserve(request("b1", "principal-b", 700), 10)
            .await
            .unwrap();
        assert_eq!(
            authority
                .reserve(request("b2", "principal-b", 200), 10)
                .await
                .unwrap_err()
                .code,
            "upload_tenant_capacity"
        );

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn concurrent_reserve_serializes_capacity_check_and_insert() {
        let (path, authority) = setup("concurrent").await;
        authority.close().await;

        let strict = UploadAdmissionConfig {
            principal_concurrent_cap: 1,
            tenant_concurrent_cap: 1,
            principal_bytes_cap: 900,
            tenant_bytes_cap: 900,
            max_single_upload_bytes: 900,
            lease_duration: Duration::from_millis(100),
            retention: Duration::from_millis(200),
        };
        let left =
            SqliteUploadAdmissionAuthority::open(&path, 4, Duration::from_secs(2), strict.clone())
                .await
                .unwrap();
        let right =
            SqliteUploadAdmissionAuthority::open(&path, 4, Duration::from_secs(2), strict)
                .await
                .unwrap();

        let (left_result, right_result) = tokio::join!(
            left.reserve(request("race-left", "principal-a", 800), 10),
            right.reserve(request("race-right", "principal-a", 800), 10)
        );

        let accepted = usize::from(left_result.is_ok()) + usize::from(right_result.is_ok());
        assert_eq!(accepted, 1, "exactly one concurrent reservation may fit");

        let rejected = if let Err(error) = left_result {
            error
        } else {
            right_result.unwrap_err()
        };
        assert!(
            matches!(
                rejected.code,
                "upload_principal_capacity" | "upload_tenant_capacity"
            ),
            "unexpected rejection: {rejected:?}"
        );

        left.close().await;
        right.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn single_upload_cap_and_disabled_principal_reject_before_capacity() {
        let (path, authority) = setup("early-reject").await;
        assert_eq!(
            authority
                .reserve(request("too-big", "principal-a", 901), 10)
                .await
                .unwrap_err()
                .code,
            "upload_bytes_too_large"
        );

        sqlx::query("UPDATE principals SET disabled_at_ms=5 WHERE principal_id=?")
            .bind(b"principal-a".as_slice())
            .execute(&authority.pool)
            .await
            .unwrap();
        assert_eq!(
            authority
                .reserve(request("disabled", "principal-a", 100), 10)
                .await
                .unwrap_err()
                .code,
            "upload_admission_principal_disabled"
        );

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn expiry_stops_counting_and_old_rows_are_bounded_by_retention_cleanup() {
        let (path, authority) = setup("expiry").await;
        authority
            .reserve(request("old", "principal-a", 900), 10)
            .await
            .unwrap();
        let usage = authority
            .usage("tenant-a", "principal-a", 50)
            .await
            .unwrap();
        assert_eq!(usage, (1, 900, 1, 900));
        let usage = authority
            .usage("tenant-a", "principal-a", 111)
            .await
            .unwrap();
        assert_eq!(usage, (0, 0, 0, 0));

        authority
            .reserve(request("new", "principal-a", 100), 400)
            .await
            .unwrap();
        let old_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM upload_admission_reservations WHERE reservation_id=?",
        )
        .bind(b"old".as_slice())
        .fetch_one(&authority.pool)
        .await
        .unwrap();
        assert_eq!(old_count, 0);

        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn renew_is_generation_fenced_and_release_is_idempotent() {
        let (path, authority) = setup("lifecycle").await;
        let record = match authority
            .reserve(request("life", "principal-a", 400), 10)
            .await
            .unwrap()
        {
            ReserveUploadOutcome::Reserved(record) => record,
            other => panic!("unexpected reserve outcome: {other:?}"),
        };
        let renewed = authority
            .renew("tenant-a", "life", record.lease_generation, 20)
            .await
            .unwrap();
        assert_eq!(renewed.lease_generation, 1);
        assert_eq!(
            authority
                .renew("tenant-a", "life", 0, 30)
                .await
                .unwrap_err()
                .code,
            "upload_admission_renew_conflict"
        );
        assert_eq!(
            authority.release("tenant-a", "life", 1, 40).await.unwrap(),
            ReleaseUploadOutcome::Released
        );
        assert_eq!(
            authority.release("tenant-a", "life", 1, 41).await.unwrap(),
            ReleaseUploadOutcome::AlreadyReleased
        );
        assert_eq!(
            authority
                .usage("tenant-a", "principal-a", 42)
                .await
                .unwrap(),
            (0, 0, 0, 0)
        );

        authority.close().await;
        cleanup(&path);
    }
}
