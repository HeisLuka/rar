use std::{fmt, path::Path, sync::Arc, time::Duration};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const MAX_OWNER_BYTES: usize = 128;
const MAX_CODE_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobGcError {
    pub code: &'static str,
    pub message: String,
}

impl BlobGcError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for BlobGcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BlobGcError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcObjectKind {
    PhysicalBlob,
    QuarantineUpload,
    TempObject,
    DerivedArtifact,
    ExportArtifact,
}

impl GcObjectKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::PhysicalBlob => "physical_blob",
            Self::QuarantineUpload => "quarantine_upload",
            Self::TempObject => "temp_object",
            Self::DerivedArtifact => "derived_artifact",
            Self::ExportArtifact => "export_artifact",
        }
    }

    fn parse(value: &str) -> Result<Self, BlobGcError> {
        match value {
            "physical_blob" => Ok(Self::PhysicalBlob),
            "quarantine_upload" => Ok(Self::QuarantineUpload),
            "temp_object" => Ok(Self::TempObject),
            "derived_artifact" => Ok(Self::DerivedArtifact),
            "export_artifact" => Ok(Self::ExportArtifact),
            _ => Err(BlobGcError::new(
                "corrupt_gc_candidate",
                "persisted GC object kind is invalid",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcCandidateState {
    Pending,
    Running,
    Completed,
    Cancelled,
}

impl GcCandidateState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Result<Self, BlobGcError> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(BlobGcError::new(
                "corrupt_gc_candidate",
                "persisted GC candidate state is invalid",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcCandidate {
    pub candidate_id: String,
    pub tenant_id: String,
    pub object_kind: GcObjectKind,
    pub object_id: String,
    pub reason_code: String,
    pub not_before_ms: i64,
    pub observed_generation: Option<String>,
    pub state: GcCandidateState,
    pub attempt: i64,
    pub lease_owner: Option<String>,
    pub lease_generation: i64,
    pub lease_expires_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleGcCandidate {
    pub candidate_id: String,
    pub tenant_id: String,
    pub object_kind: GcObjectKind,
    pub object_id: String,
    pub reason_code: String,
    pub not_before_ms: i64,
    pub observed_generation: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcLease {
    pub candidate: GcCandidate,
    pub lease_owner: String,
    pub lease_generation: i64,
    pub lease_expires_at_ms: i64,
}

#[derive(Clone)]
pub struct SqliteBlobGcLedger {
    pool: SqlitePool,
}

impl SqliteBlobGcLedger {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, BlobGcError> {
        if !(1..=16).contains(&max_connections) {
            return Err(BlobGcError::new(
                "invalid_gc_pool_size",
                "GC ledger pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(BlobGcError::new(
                "invalid_gc_busy_timeout",
                "GC ledger busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(BlobGcError::new(
                "invalid_gc_database_path",
                "GC ledger database path must be non-empty",
            ));
        }
        if !path.exists() {
            return Err(BlobGcError::new(
                "gc_database_missing",
                "GC ledger database must be created by chaptera migrate up before worker startup",
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

        let ledger = Self { pool };
        ledger.require_schema().await?;
        ledger.verify_profile().await?;
        Ok(ledger)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    async fn require_schema(&self) -> Result<(), BlobGcError> {
        for table in ["chaptera_schema_migrations", "gc_candidates"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if exists != 1 {
                return Err(BlobGcError::new(
                    "gc_schema_missing",
                    format!("required GC table {table} is absent; run chaptera migrate up"),
                ));
            }
        }

        let index_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='gc_due'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;
        if index_exists != 1 {
            return Err(BlobGcError::new(
                "gc_schema_missing",
                "required GC index gc_due is absent; run chaptera migrate up",
            ));
        }

        let migration: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chaptera_schema_migrations WHERE version = 4 AND name = 'blob_gc'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;
        if migration != 1 {
            return Err(BlobGcError::new(
                "gc_schema_missing",
                "blob GC migration v4 is not recorded",
            ));
        }

        Ok(())
    }

    async fn verify_profile(&self) -> Result<(), BlobGcError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(BlobGcError::new(
                "gc_profile_mismatch",
                format!("expected WAL journal mode, got {journal_mode}"),
            ));
        }

        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if synchronous != 2 {
            return Err(BlobGcError::new(
                "gc_profile_mismatch",
                format!("expected synchronous=FULL(2), got {synchronous}"),
            ));
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if foreign_keys != 1 {
            return Err(BlobGcError::new(
                "gc_profile_mismatch",
                "foreign_keys pragma is not enabled",
            ));
        }

        Ok(())
    }

    pub async fn schedule(&self, request: ScheduleGcCandidate) -> Result<GcCandidate, BlobGcError> {
        validate_schedule(&request)?;
        let inserted = sqlx::query(
            r#"
            INSERT INTO gc_candidates (
              candidate_id, tenant_id, object_kind, object_id, reason_code,
              not_before_ms, observed_generation, state, attempt,
              lease_owner, lease_generation, lease_expires_at_ms,
              last_error_code, created_at_ms, completed_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', 0, NULL, 0, NULL, NULL, ?, NULL)
            ON CONFLICT(tenant_id, object_kind, object_id, reason_code) DO NOTHING
            "#,
        )
        .bind(request.candidate_id.as_bytes())
        .bind(request.tenant_id.as_bytes())
        .bind(request.object_kind.as_str())
        .bind(request.object_id.as_bytes())
        .bind(&request.reason_code)
        .bind(request.not_before_ms)
        .bind(&request.observed_generation)
        .bind(request.created_at_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if inserted.rows_affected() == 1 {
            return self.get(&request.candidate_id).await?.ok_or_else(|| {
                BlobGcError::new("gc_candidate_missing", "inserted candidate vanished")
            });
        }

        self.find_equivalent(
            &request.tenant_id,
            request.object_kind,
            &request.object_id,
            &request.reason_code,
        )
        .await?
        .ok_or_else(|| {
            BlobGcError::new(
                "gc_schedule_conflict",
                "candidate schedule conflicted without equivalent row",
            )
        })
    }

    pub async fn get(&self, candidate_id: &str) -> Result<Option<GcCandidate>, BlobGcError> {
        require_ident(candidate_id, "candidate_id")?;
        sqlx::query("SELECT * FROM gc_candidates WHERE candidate_id=?")
            .bind(candidate_id.as_bytes())
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlite_error)?
            .map(decode_candidate)
            .transpose()
    }

    async fn find_equivalent(
        &self,
        tenant_id: &str,
        object_kind: GcObjectKind,
        object_id: &str,
        reason_code: &str,
    ) -> Result<Option<GcCandidate>, BlobGcError> {
        sqlx::query(
            "SELECT * FROM gc_candidates WHERE tenant_id=? AND object_kind=? AND object_id=? AND reason_code=?",
        )
        .bind(tenant_id.as_bytes())
        .bind(object_kind.as_str())
        .bind(object_id.as_bytes())
        .bind(reason_code)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?
        .map(decode_candidate)
        .transpose()
    }

    pub async fn claim_due(
        &self,
        owner: &str,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<Option<GcLease>, BlobGcError> {
        require_owner(owner)?;
        if !(1..=300_000).contains(&lease_ms) {
            return Err(BlobGcError::new(
                "invalid_gc_lease",
                "GC lease must be >0 and <=300000 ms",
            ));
        }

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = async {
            let row = sqlx::query(
                r#"
                SELECT * FROM gc_candidates
                WHERE not_before_ms <= ?
                  AND (
                    state='pending'
                    OR (
                      state='running'
                      AND lease_expires_at_ms IS NOT NULL
                      AND lease_expires_at_ms <= ?
                    )
                  )
                ORDER BY not_before_ms ASC, created_at_ms ASC, candidate_id ASC
                LIMIT 1
                "#,
            )
            .bind(now_ms)
            .bind(now_ms)
            .fetch_optional(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            let Some(row) = row else {
                return Ok(None);
            };
            let current = decode_candidate(row)?;
            let next_generation = current.lease_generation + 1;
            let expires = now_ms
                .checked_add(lease_ms)
                .ok_or_else(|| BlobGcError::new("gc_lease_overflow", "GC lease expiry overflow"))?;

            let changed = sqlx::query(
                r#"
                UPDATE gc_candidates
                SET state='running',
                    attempt=attempt+1,
                    lease_owner=?,
                    lease_generation=?,
                    lease_expires_at_ms=?,
                    last_error_code=NULL
                WHERE candidate_id=? AND lease_generation=? AND state=?
                "#,
            )
            .bind(owner)
            .bind(next_generation)
            .bind(expires)
            .bind(current.candidate_id.as_bytes())
            .bind(current.lease_generation)
            .bind(current.state.as_str())
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            if changed.rows_affected() != 1 {
                return Err(BlobGcError::new(
                    "gc_claim_race",
                    "GC candidate changed before lease claim",
                ));
            }

            let row = sqlx::query("SELECT * FROM gc_candidates WHERE candidate_id=?")
                .bind(current.candidate_id.as_bytes())
                .fetch_one(&mut *conn)
                .await
                .map_err(sqlite_error)?;
            let candidate = decode_candidate(row)?;
            Ok(Some(GcLease {
                candidate,
                lease_owner: owner.to_owned(),
                lease_generation: next_generation,
                lease_expires_at_ms: expires,
            }))
        }
        .await;

        match result {
            Ok(value) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(sqlite_error)?;
                Ok(value)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    async fn terminal(
        &self,
        lease: &GcLease,
        now_ms: i64,
        state: GcCandidateState,
        code: Option<&str>,
    ) -> Result<GcCandidate, BlobGcError> {
        if !matches!(
            state,
            GcCandidateState::Completed | GcCandidateState::Cancelled
        ) {
            return Err(BlobGcError::new(
                "invalid_gc_terminal_state",
                "GC terminal update must complete or cancel",
            ));
        }
        if let Some(code) = code {
            require_code(code)?;
        }

        let changed = sqlx::query(
            r#"
            UPDATE gc_candidates
            SET state=?, completed_at_ms=?, last_error_code=?,
                lease_owner=NULL, lease_expires_at_ms=NULL
            WHERE candidate_id=? AND state='running'
              AND lease_owner=? AND lease_generation=?
              AND lease_expires_at_ms>?
            "#,
        )
        .bind(state.as_str())
        .bind(now_ms)
        .bind(code)
        .bind(lease.candidate.candidate_id.as_bytes())
        .bind(&lease.lease_owner)
        .bind(lease.lease_generation)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if changed.rows_affected() != 1 {
            return Err(BlobGcError::new(
                "stale_gc_lease",
                "GC terminal update lost lease ownership",
            ));
        }
        self.get(&lease.candidate.candidate_id)
            .await?
            .ok_or_else(|| BlobGcError::new("gc_candidate_missing", "candidate vanished"))
    }

    async fn retry(
        &self,
        lease: &GcLease,
        not_before_ms: i64,
        error_code: &str,
    ) -> Result<GcCandidate, BlobGcError> {
        require_code(error_code)?;
        let changed = sqlx::query(
            r#"
            UPDATE gc_candidates
            SET state='pending', not_before_ms=?, last_error_code=?,
                lease_owner=NULL, lease_expires_at_ms=NULL
            WHERE candidate_id=? AND state='running'
              AND lease_owner=? AND lease_generation=?
            "#,
        )
        .bind(not_before_ms)
        .bind(error_code)
        .bind(lease.candidate.candidate_id.as_bytes())
        .bind(&lease.lease_owner)
        .bind(lease.lease_generation)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if changed.rows_affected() != 1 {
            return Err(BlobGcError::new(
                "stale_gc_lease",
                "GC retry update lost lease ownership",
            ));
        }
        self.get(&lease.candidate.candidate_id)
            .await?
            .ok_or_else(|| BlobGcError::new("gc_candidate_missing", "candidate vanished"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcPreDelete {
    Cancel {
        code: &'static str,
    },
    Defer {
        not_before_ms: i64,
        code: &'static str,
    },
    Ready {
        object_locator: String,
        storage_generation: String,
        delete_fence: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcDeleteOutcome {
    Deleted,
    NotFound,
    UnknownOutcome,
}

pub trait BlobGcAuthority: Send + Sync {
    /// Must re-read current durable lifecycle/reachability and atomically install
    /// a fence preventing a new binding/reference from making this object live
    /// before commit_deleted clears durable metadata.
    fn recheck_and_fence(
        &self,
        candidate: &GcCandidate,
        now_ms: i64,
    ) -> Result<GcPreDelete, BlobGcError>;

    /// Commits durable deletion under the exact fence returned above. Must be
    /// idempotent after provider success/NotFound and fail closed on fence drift.
    fn commit_deleted(
        &self,
        candidate: &GcCandidate,
        storage_generation: &str,
        delete_fence: &str,
        now_ms: i64,
    ) -> Result<(), BlobGcError>;
}

pub trait BlobGcObjectStore: Send + Sync {
    fn delete_exact(
        &self,
        object_locator: &str,
        storage_generation: &str,
    ) -> Result<GcDeleteOutcome, BlobGcError>;

    fn exists_exact(
        &self,
        object_locator: &str,
        storage_generation: &str,
    ) -> Result<bool, BlobGcError>;
}

pub struct BlobGcProcessor {
    ledger: SqliteBlobGcLedger,
    authority: Arc<dyn BlobGcAuthority>,
    objects: Arc<dyn BlobGcObjectStore>,
    retry_delay_ms: i64,
}

impl BlobGcProcessor {
    pub fn new(
        ledger: SqliteBlobGcLedger,
        authority: Arc<dyn BlobGcAuthority>,
        objects: Arc<dyn BlobGcObjectStore>,
        retry_delay_ms: i64,
    ) -> Result<Self, BlobGcError> {
        if !(1..=3_600_000).contains(&retry_delay_ms) {
            return Err(BlobGcError::new(
                "invalid_gc_retry_delay",
                "GC retry delay must be >0 and <=1 hour",
            ));
        }
        Ok(Self {
            ledger,
            authority,
            objects,
            retry_delay_ms,
        })
    }

    pub async fn process_one(
        &self,
        owner: &str,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<Option<GcCandidate>, BlobGcError> {
        let Some(lease) = self.ledger.claim_due(owner, now_ms, lease_ms).await? else {
            return Ok(None);
        };

        match self.authority.recheck_and_fence(&lease.candidate, now_ms)? {
            GcPreDelete::Cancel { code } => self
                .ledger
                .terminal(&lease, now_ms, GcCandidateState::Cancelled, Some(code))
                .await
                .map(Some),
            GcPreDelete::Defer {
                not_before_ms,
                code,
            } => self
                .ledger
                .retry(&lease, not_before_ms, code)
                .await
                .map(Some),
            GcPreDelete::Ready {
                object_locator,
                storage_generation,
                delete_fence,
            } => {
                if let Some(observed) = &lease.candidate.observed_generation
                    && observed != &storage_generation
                {
                    return self
                        .ledger
                        .terminal(
                            &lease,
                            now_ms,
                            GcCandidateState::Cancelled,
                            Some("generation_changed"),
                        )
                        .await
                        .map(Some);
                }

                let deleted = match self
                    .objects
                    .delete_exact(&object_locator, &storage_generation)?
                {
                    GcDeleteOutcome::Deleted | GcDeleteOutcome::NotFound => true,
                    GcDeleteOutcome::UnknownOutcome => !self
                        .objects
                        .exists_exact(&object_locator, &storage_generation)?,
                };

                if !deleted {
                    let next = now_ms.saturating_add(self.retry_delay_ms);
                    return self
                        .ledger
                        .retry(&lease, next, "provider_delete_unknown")
                        .await
                        .map(Some);
                }

                self.authority.commit_deleted(
                    &lease.candidate,
                    &storage_generation,
                    &delete_fence,
                    now_ms,
                )?;
                self.ledger
                    .terminal(&lease, now_ms, GcCandidateState::Completed, None)
                    .await
                    .map(Some)
            }
        }
    }
}

fn validate_schedule(request: &ScheduleGcCandidate) -> Result<(), BlobGcError> {
    require_ident(&request.candidate_id, "candidate_id")?;
    require_ident(&request.tenant_id, "tenant_id")?;
    require_ident(&request.object_id, "object_id")?;
    require_code(&request.reason_code)?;
    if let Some(generation) = &request.observed_generation {
        require_ident(generation, "observed_generation")?;
    }
    Ok(())
}

fn require_owner(value: &str) -> Result<(), BlobGcError> {
    require_ident(value, "lease_owner")?;
    if value.len() > MAX_OWNER_BYTES {
        return Err(BlobGcError::new("invalid_gc_owner", "GC owner is too long"));
    }
    Ok(())
}

fn require_code(value: &str) -> Result<(), BlobGcError> {
    require_ident(value, "code")?;
    if value.len() > MAX_CODE_BYTES {
        return Err(BlobGcError::new("invalid_gc_code", "GC code is too long"));
    }
    Ok(())
}

fn require_ident(value: &str, label: &str) -> Result<(), BlobGcError> {
    let allowed = value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_.:@/-".contains(&byte));
    if value.is_empty() || value.len() > 256 || !allowed {
        return Err(BlobGcError::new(
            "invalid_gc_identity",
            format!("invalid {label}"),
        ));
    }
    Ok(())
}

fn sqlite_error(error: impl fmt::Display) -> BlobGcError {
    BlobGcError::new("sqlite_blob_gc_error", error.to_string())
}

fn text_blob(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<String, BlobGcError> {
    let value = row.try_get::<Vec<u8>, _>(name).map_err(sqlite_error)?;
    String::from_utf8(value)
        .map_err(|_| BlobGcError::new("corrupt_gc_candidate", format!("{name} is not UTF-8")))
}

fn decode_candidate(row: sqlx::sqlite::SqliteRow) -> Result<GcCandidate, BlobGcError> {
    Ok(GcCandidate {
        candidate_id: text_blob(&row, "candidate_id")?,
        tenant_id: text_blob(&row, "tenant_id")?,
        object_kind: GcObjectKind::parse(
            &row.try_get::<String, _>("object_kind")
                .map_err(sqlite_error)?,
        )?,
        object_id: text_blob(&row, "object_id")?,
        reason_code: row.try_get("reason_code").map_err(sqlite_error)?,
        not_before_ms: row.try_get("not_before_ms").map_err(sqlite_error)?,
        observed_generation: row.try_get("observed_generation").map_err(sqlite_error)?,
        state: GcCandidateState::parse(&row.try_get::<String, _>("state").map_err(sqlite_error)?)?,
        attempt: row.try_get("attempt").map_err(sqlite_error)?,
        lease_owner: row.try_get("lease_owner").map_err(sqlite_error)?,
        lease_generation: row.try_get("lease_generation").map_err(sqlite_error)?,
        lease_expires_at_ms: row.try_get("lease_expires_at_ms").map_err(sqlite_error)?,
        last_error_code: row.try_get("last_error_code").map_err(sqlite_error)?,
        created_at_ms: row.try_get("created_at_ms").map_err(sqlite_error)?,
        completed_at_ms: row.try_get("completed_at_ms").map_err(sqlite_error)?,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::PathBuf,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
    };

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    async fn ledger() -> (SqliteBlobGcLedger, PathBuf) {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("chaptera-gc-{}-{n}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let ledger = SqliteBlobGcLedger::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (ledger, path)
    }

    #[tokio::test]
    async fn unmigrated_open_fails_closed_without_creating_database() {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "chaptera-gc-unmigrated-{}-{n}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let error = match SqliteBlobGcLedger::open(&path, 1, Duration::from_secs(2)).await {
            Ok(ledger) => {
                ledger.close().await;
                panic!("unmigrated GC ledger unexpectedly opened")
            }
            Err(error) => error,
        };
        assert_eq!(error.code, "gc_database_missing");
        assert!(!path.exists());
    }

    async fn schedule(
        ledger: &SqliteBlobGcLedger,
        id: &str,
        object_id: &str,
        generation: Option<&str>,
    ) {
        ledger
            .schedule(ScheduleGcCandidate {
                candidate_id: id.into(),
                tenant_id: "tenant-a".into(),
                object_kind: GcObjectKind::PhysicalBlob,
                object_id: object_id.into(),
                reason_code: "orphan_prewrite".into(),
                not_before_ms: 100,
                observed_generation: generation.map(str::to_owned),
                created_at_ms: 10,
            })
            .await
            .unwrap();
    }

    struct MemoryAuthority {
        reachable: Mutex<BTreeSet<String>>,
        generations: Mutex<BTreeMap<String, String>>,
        committed: Mutex<BTreeSet<String>>,
        fence_counter: AtomicU64,
    }

    impl MemoryAuthority {
        fn new() -> Self {
            Self {
                reachable: Mutex::new(BTreeSet::new()),
                generations: Mutex::new(BTreeMap::new()),
                committed: Mutex::new(BTreeSet::new()),
                fence_counter: AtomicU64::new(1),
            }
        }

        fn add(&self, object_id: &str, generation: &str) {
            self.generations
                .lock()
                .unwrap()
                .insert(object_id.into(), generation.into());
        }
    }

    impl BlobGcAuthority for MemoryAuthority {
        fn recheck_and_fence(
            &self,
            candidate: &GcCandidate,
            _now_ms: i64,
        ) -> Result<GcPreDelete, BlobGcError> {
            if self
                .reachable
                .lock()
                .unwrap()
                .contains(&candidate.object_id)
            {
                return Ok(GcPreDelete::Cancel {
                    code: "became_reachable",
                });
            }
            let generation = self
                .generations
                .lock()
                .unwrap()
                .get(&candidate.object_id)
                .cloned()
                .ok_or_else(|| BlobGcError::new("object_missing", "object metadata missing"))?;
            let n = self.fence_counter.fetch_add(1, Ordering::SeqCst);
            Ok(GcPreDelete::Ready {
                object_locator: format!("canonical/tenant-a/{}", candidate.object_id),
                storage_generation: generation,
                delete_fence: format!("fence-{n}"),
            })
        }

        fn commit_deleted(
            &self,
            candidate: &GcCandidate,
            _storage_generation: &str,
            _delete_fence: &str,
            _now_ms: i64,
        ) -> Result<(), BlobGcError> {
            self.committed
                .lock()
                .unwrap()
                .insert(candidate.object_id.clone());
            Ok(())
        }
    }

    struct MemoryObjects {
        existing: Mutex<BTreeSet<(String, String)>>,
        unknown_once: AtomicBool,
    }

    impl MemoryObjects {
        fn new() -> Self {
            Self {
                existing: Mutex::new(BTreeSet::new()),
                unknown_once: AtomicBool::new(false),
            }
        }

        fn insert(&self, locator: &str, generation: &str) {
            self.existing
                .lock()
                .unwrap()
                .insert((locator.into(), generation.into()));
        }
    }

    impl BlobGcObjectStore for MemoryObjects {
        fn delete_exact(
            &self,
            object_locator: &str,
            storage_generation: &str,
        ) -> Result<GcDeleteOutcome, BlobGcError> {
            if self.unknown_once.swap(false, Ordering::SeqCst) {
                return Ok(GcDeleteOutcome::UnknownOutcome);
            }
            let removed = self
                .existing
                .lock()
                .unwrap()
                .remove(&(object_locator.into(), storage_generation.into()));
            Ok(if removed {
                GcDeleteOutcome::Deleted
            } else {
                GcDeleteOutcome::NotFound
            })
        }

        fn exists_exact(
            &self,
            object_locator: &str,
            storage_generation: &str,
        ) -> Result<bool, BlobGcError> {
            Ok(self
                .existing
                .lock()
                .unwrap()
                .contains(&(object_locator.into(), storage_generation.into())))
        }
    }

    #[tokio::test]
    async fn new_reachability_cancels_candidate_before_delete() {
        let (ledger, path) = ledger().await;
        schedule(&ledger, "gc-1", "blob-1", Some("g1")).await;

        let authority = Arc::new(MemoryAuthority::new());
        authority.add("blob-1", "g1");
        authority.reachable.lock().unwrap().insert("blob-1".into());
        let objects = Arc::new(MemoryObjects::new());
        objects.insert("canonical/tenant-a/blob-1", "g1");

        let processor =
            BlobGcProcessor::new(ledger.clone(), authority.clone(), objects.clone(), 100).unwrap();
        let result = processor
            .process_one("gc-worker", 100, 1_000)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.state, GcCandidateState::Cancelled);
        assert_eq!(result.last_error_code.as_deref(), Some("became_reachable"));
        assert!(
            objects
                .exists_exact("canonical/tenant-a/blob-1", "g1")
                .unwrap()
        );

        ledger.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn generation_drift_cancels_without_touching_new_object() {
        let (ledger, path) = ledger().await;
        schedule(&ledger, "gc-2", "blob-2", Some("g1")).await;

        let authority = Arc::new(MemoryAuthority::new());
        authority.add("blob-2", "g2");
        let objects = Arc::new(MemoryObjects::new());
        objects.insert("canonical/tenant-a/blob-2", "g2");

        let processor =
            BlobGcProcessor::new(ledger.clone(), authority, objects.clone(), 100).unwrap();
        let result = processor
            .process_one("gc-worker", 100, 1_000)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.state, GcCandidateState::Cancelled);
        assert_eq!(
            result.last_error_code.as_deref(),
            Some("generation_changed")
        );
        assert!(
            objects
                .exists_exact("canonical/tenant-a/blob-2", "g2")
                .unwrap()
        );

        ledger.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn unknown_delete_is_reconciled_before_completion() {
        let (ledger, path) = ledger().await;
        schedule(&ledger, "gc-3", "blob-3", Some("g1")).await;

        let authority = Arc::new(MemoryAuthority::new());
        authority.add("blob-3", "g1");
        let objects = Arc::new(MemoryObjects::new());
        objects.insert("canonical/tenant-a/blob-3", "g1");
        objects.unknown_once.store(true, Ordering::SeqCst);

        let processor =
            BlobGcProcessor::new(ledger.clone(), authority.clone(), objects.clone(), 100).unwrap();
        let first = processor
            .process_one("gc-worker", 100, 1_000)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(first.state, GcCandidateState::Pending);
        assert_eq!(
            first.last_error_code.as_deref(),
            Some("provider_delete_unknown")
        );
        assert!(!authority.committed.lock().unwrap().contains("blob-3"));

        objects
            .existing
            .lock()
            .unwrap()
            .remove(&("canonical/tenant-a/blob-3".into(), "g1".into()));
        let second = processor
            .process_one("gc-worker", 200, 1_000)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(second.state, GcCandidateState::Completed);
        assert!(authority.committed.lock().unwrap().contains("blob-3"));

        ledger.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn expired_running_candidate_is_reclaimed_after_restart() {
        let (ledger, path) = ledger().await;
        schedule(&ledger, "gc-4", "blob-4", Some("g1")).await;
        let first = ledger
            .claim_due("gc-worker-a", 100, 50)
            .await
            .unwrap()
            .unwrap();

        let reopened = SqliteBlobGcLedger::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let reclaimed = reopened
            .claim_due("gc-worker-b", 151, 50)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            reclaimed.candidate.candidate_id,
            first.candidate.candidate_id
        );
        assert!(reclaimed.lease_generation > first.lease_generation);

        ledger.close().await;
        reopened.close().await;
        let _ = std::fs::remove_file(path);
    }
}
