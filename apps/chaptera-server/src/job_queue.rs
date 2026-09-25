use std::{collections::BTreeMap, fmt, path::Path, time::Duration};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_IDEMPOTENCY_BYTES: usize = 256;
const MAX_OWNER_BYTES: usize = 128;
const MAX_EFFECT_KEY_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobQueueError {
    pub code: &'static str,
    pub message: String,
}

impl JobQueueError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for JobQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for JobQueueError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Parse,
    Export,
    Snapshot,
    Projection,
    BlobGc,
}

impl JobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Export => "export",
            Self::Snapshot => "snapshot",
            Self::Projection => "projection",
            Self::BlobGc => "blob_gc",
        }
    }

    fn parse(value: &str) -> Result<Self, JobQueueError> {
        match value {
            "parse" => Ok(Self::Parse),
            "export" => Ok(Self::Export),
            "snapshot" => Ok(Self::Snapshot),
            "projection" => Ok(Self::Projection),
            "blob_gc" => Ok(Self::BlobGc),
            _ => Err(JobQueueError::new(
                "invalid_job_kind",
                "persisted job kind is invalid",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Result<Self, JobQueueError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(JobQueueError::new(
                "invalid_job_status",
                "persisted job status is invalid",
            )),
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    pub tenant_id: String,
    pub job_kind: JobKind,
    pub payload_schema_version: i64,
    pub payload: Vec<u8>,
    pub request_hash: String,
    pub status: JobStatus,
    pub available_at_ms: i64,
    pub attempt: i64,
    pub max_attempts: i64,
    pub lease_owner: Option<String>,
    pub lease_generation: i64,
    pub lease_expires_at_ms: Option<i64>,
    pub cancel_requested_at_ms: Option<i64>,
    pub idempotency_key: String,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub terminal_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueRequest {
    pub job_id: String,
    pub tenant_id: String,
    pub job_kind: JobKind,
    pub payload_schema_version: i64,
    pub payload: Vec<u8>,
    pub idempotency_key: String,
    pub max_attempts: i64,
    pub now_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Enqueued(JobRecord),
    Existing(JobRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub job: JobRecord,
    pub lease_owner: String,
    pub lease_generation: i64,
    pub lease_expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureOutcome {
    Requeued(JobRecord),
    Failed(JobRecord),
    Cancelled(JobRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionDeferOutcome {
    Requeued(JobRecord),
    Cancelled(JobRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOutcome {
    Published(JobRecord),
    AlreadyPublished(JobRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMetrics {
    pub counts: BTreeMap<String, i64>,
    pub due_queued: i64,
    pub expired_running: i64,
}

#[derive(Clone)]
pub struct SqliteJobQueue {
    pool: SqlitePool,
}

impl SqliteJobQueue {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, JobQueueError> {
        if !(1..=16).contains(&max_connections) {
            return Err(JobQueueError::new(
                "invalid_pool_size",
                "job queue pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(JobQueueError::new(
                "invalid_busy_timeout",
                "busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(JobQueueError::new(
                "invalid_database_path",
                "database path must be non-empty",
            ));
        }

        if !path.exists() {
            return Err(JobQueueError::new(
                "job_queue_database_missing",
                "job queue database must be created by chaptera migrate up before worker startup",
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

        let queue = Self { pool };
        queue.require_schema().await?;
        queue.verify_profile().await?;
        Ok(queue)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    async fn require_schema(&self) -> Result<(), JobQueueError> {
        for object in ["jobs", "job_effects"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(object)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;

            if exists != 1 {
                return Err(JobQueueError::new(
                    "job_queue_schema_missing",
                    format!(
                        "required job queue table {object} is absent; run chaptera migrate up before worker startup"
                    ),
                ));
            }
        }

        for index in ["jobs_ready", "jobs_expired_lease"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?",
            )
            .bind(index)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;

            if exists != 1 {
                return Err(JobQueueError::new(
                    "job_queue_schema_missing",
                    format!(
                        "required job queue index {index} is absent; run chaptera migrate up before worker startup"
                    ),
                ));
            }
        }
        Ok(())
    }

    async fn verify_profile(&self) -> Result<(), JobQueueError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(JobQueueError::new(
                "job_queue_profile_mismatch",
                format!("expected WAL journal mode, got {journal_mode}"),
            ));
        }

        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if synchronous != 2 {
            return Err(JobQueueError::new(
                "job_queue_profile_mismatch",
                format!("expected synchronous=FULL(2), got {synchronous}"),
            ));
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if foreign_keys != 1 {
            return Err(JobQueueError::new(
                "job_queue_profile_mismatch",
                "foreign_keys pragma is not enabled",
            ));
        }
        Ok(())
    }

    pub async fn enqueue(&self, request: EnqueueRequest) -> Result<EnqueueOutcome, JobQueueError> {
        validate_enqueue(&request)?;
        let request_hash = enqueue_hash(&request);

        let inserted = sqlx::query(
            r#"
            INSERT INTO jobs (
              job_id, tenant_id, job_kind, payload_schema_version, payload,
              request_hash, status, available_at_ms, attempt, max_attempts,
              lease_owner, lease_generation, lease_expires_at_ms,
              cancel_requested_at_ms, idempotency_key, created_at_ms,
              started_at_ms, finished_at_ms, terminal_code
            ) VALUES (
              ?, ?, ?, ?, ?, ?, 'queued', ?, 0, ?,
              NULL, 0, NULL, NULL, ?, ?, NULL, NULL, NULL
            )
            "#,
        )
        .bind(request.job_id.as_bytes())
        .bind(request.tenant_id.as_bytes())
        .bind(request.job_kind.as_str())
        .bind(request.payload_schema_version)
        .bind(&request.payload)
        .bind(request_hash.as_bytes())
        .bind(request.now_ms)
        .bind(request.max_attempts)
        .bind(request.idempotency_key.as_bytes())
        .bind(request.now_ms)
        .execute(&self.pool)
        .await;

        if let Ok(done) = inserted {
            if done.rows_affected() != 1 {
                return Err(JobQueueError::new(
                    "enqueue_no_effect",
                    "enqueue did not create a row",
                ));
            }
            let job = self
                .get(&request.job_id)
                .await?
                .ok_or_else(|| JobQueueError::new("job_not_found", "inserted job disappeared"))?;
            return Ok(EnqueueOutcome::Enqueued(job));
        }

        let existing = self
            .find_idempotent(
                &request.tenant_id,
                request.job_kind,
                &request.idempotency_key,
            )
            .await?
            .ok_or_else(|| {
                JobQueueError::new("enqueue_failed", "enqueue failed without an idempotent row")
            })?;

        if existing.request_hash != request_hash {
            return Err(JobQueueError::new(
                "idempotency_conflict",
                "idempotency key reused with different request",
            ));
        }

        Ok(EnqueueOutcome::Existing(existing))
    }

    pub async fn get(&self, job_id: &str) -> Result<Option<JobRecord>, JobQueueError> {
        require_ident(job_id, "job_id")?;
        sqlx::query("SELECT * FROM jobs WHERE job_id = ?")
            .bind(job_id.as_bytes())
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlite_error)?
            .map(decode_job)
            .transpose()
    }

    async fn find_idempotent(
        &self,
        tenant_id: &str,
        kind: JobKind,
        key: &str,
    ) -> Result<Option<JobRecord>, JobQueueError> {
        sqlx::query(
            "SELECT * FROM jobs              WHERE tenant_id = ? AND job_kind = ? AND idempotency_key = ?",
        )
        .bind(tenant_id.as_bytes())
        .bind(kind.as_str())
        .bind(key.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?
        .map(decode_job)
        .transpose()
    }

    pub async fn claim_one(
        &self,
        owner: &str,
        now_ms: i64,
        lease_ms: i64,
        allowed_kinds: &[JobKind],
    ) -> Result<Option<Lease>, JobQueueError> {
        require_owner(owner)?;
        validate_lease_ms(lease_ms)?;

        if allowed_kinds.is_empty() {
            return Ok(None);
        }

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = self
            .claim_one_in_transaction(&mut conn, owner, now_ms, lease_ms, allowed_kinds)
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

    async fn claim_one_in_transaction(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
        owner: &str,
        now_ms: i64,
        lease_ms: i64,
        allowed_kinds: &[JobKind],
    ) -> Result<Option<Lease>, JobQueueError> {
        let kinds: Vec<&str> = allowed_kinds.iter().map(|kind| kind.as_str()).collect();
        let placeholders = std::iter::repeat_n("?", kinds.len())
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(
            "SELECT * FROM jobs              WHERE job_kind IN ({placeholders})                AND attempt < max_attempts                AND (                     (status = 'queued' AND available_at_ms <= ?)                     OR                     (status = 'running' AND lease_expires_at_ms IS NOT NULL                      AND lease_expires_at_ms <= ?)                )              ORDER BY available_at_ms ASC, created_at_ms ASC, job_id ASC              LIMIT 1"
        );

        let mut query = sqlx::query(&sql);
        for kind in kinds {
            query = query.bind(kind);
        }

        let row = query
            .bind(now_ms)
            .bind(now_ms)
            .fetch_optional(&mut **conn)
            .await
            .map_err(sqlite_error)?;

        let Some(row) = row else {
            return Ok(None);
        };
        let current = decode_job(row)?;

        if current.cancel_requested_at_ms.is_some() {
            sqlx::query(
                "UPDATE jobs                  SET status='cancelled', finished_at_ms=?,                      terminal_code='cancel_requested',                      lease_owner=NULL, lease_expires_at_ms=NULL                  WHERE job_id=? AND lease_generation=?",
            )
            .bind(now_ms)
            .bind(current.job_id.as_bytes())
            .bind(current.lease_generation)
            .execute(&mut **conn)
            .await
            .map_err(sqlite_error)?;
            return Ok(None);
        }

        let next_generation = current.lease_generation + 1;
        let next_attempt = current.attempt + 1;
        let lease_expires_at_ms = now_ms
            .checked_add(lease_ms)
            .ok_or_else(|| JobQueueError::new("lease_overflow", "lease expiry overflow"))?;

        let done = sqlx::query(
            r#"
            UPDATE jobs
            SET status='running',
                attempt=?,
                lease_owner=?,
                lease_generation=?,
                lease_expires_at_ms=?,
                started_at_ms=COALESCE(started_at_ms, ?),
                terminal_code=NULL
            WHERE job_id=? AND lease_generation=? AND status=?
            "#,
        )
        .bind(next_attempt)
        .bind(owner)
        .bind(next_generation)
        .bind(lease_expires_at_ms)
        .bind(now_ms)
        .bind(current.job_id.as_bytes())
        .bind(current.lease_generation)
        .bind(current.status.as_str())
        .execute(&mut **conn)
        .await
        .map_err(sqlite_error)?;

        if done.rows_affected() != 1 {
            return Err(JobQueueError::new(
                "claim_race",
                "claim candidate changed before ownership update",
            ));
        }

        let job = sqlx::query("SELECT * FROM jobs WHERE job_id = ?")
            .bind(current.job_id.as_bytes())
            .fetch_one(&mut **conn)
            .await
            .map_err(sqlite_error)
            .and_then(decode_job)?;

        Ok(Some(Lease {
            job,
            lease_owner: owner.to_owned(),
            lease_generation: next_generation,
            lease_expires_at_ms,
        }))
    }

    pub async fn heartbeat(
        &self,
        job_id: &str,
        owner: &str,
        generation: i64,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<JobRecord, JobQueueError> {
        require_owner(owner)?;
        validate_lease_ms(lease_ms)?;

        let expires_at_ms = now_ms
            .checked_add(lease_ms)
            .ok_or_else(|| JobQueueError::new("lease_overflow", "lease expiry overflow"))?;

        let done = sqlx::query(
            "UPDATE jobs SET lease_expires_at_ms=?              WHERE job_id=? AND status='running'                AND lease_owner=? AND lease_generation=?                AND lease_expires_at_ms>?",
        )
        .bind(expires_at_ms)
        .bind(job_id.as_bytes())
        .bind(owner)
        .bind(generation)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if done.rows_affected() != 1 {
            return Err(JobQueueError::new(
                "stale_lease",
                "heartbeat rejected for stale or expired lease",
            ));
        }

        self.get(job_id)
            .await?
            .ok_or_else(|| JobQueueError::new("job_not_found", "job disappeared"))
    }

    pub async fn request_cancel(
        &self,
        job_id: &str,
        now_ms: i64,
    ) -> Result<JobRecord, JobQueueError> {
        require_ident(job_id, "job_id")?;

        sqlx::query(
            "UPDATE jobs              SET cancel_requested_at_ms=COALESCE(cancel_requested_at_ms, ?)              WHERE job_id=? AND status IN ('queued','running')",
        )
        .bind(now_ms)
        .bind(job_id.as_bytes())
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        self.get(job_id)
            .await?
            .ok_or_else(|| JobQueueError::new("job_not_found", "job does not exist"))
    }

    pub async fn defer_admission(
        &self,
        lease: &Lease,
        now_ms: i64,
        reason_code: &str,
    ) -> Result<AdmissionDeferOutcome, JobQueueError> {
        require_code(reason_code)?;
        let current = self.require_live_lease(lease, now_ms).await?;

        if current.cancel_requested_at_ms.is_some() {
            let job = self
                .finish(lease, now_ms, JobStatus::Cancelled, "cancel_requested")
                .await?;
            return Ok(AdmissionDeferOutcome::Cancelled(job));
        }

        if current.attempt <= 0 {
            return Err(JobQueueError::new(
                "invalid_attempt_state",
                "admission deferral requires a claimed execution attempt",
            ));
        }

        let restored_attempt = current.attempt - 1;
        let available_at_ms =
            now_ms.saturating_add(retry_delay_ms(&current.job_id, current.lease_generation));

        let done = sqlx::query(
            r#"
            UPDATE jobs
            SET status='queued',
                available_at_ms=?,
                attempt=?,
                lease_owner=NULL,
                lease_expires_at_ms=NULL,
                started_at_ms=CASE WHEN attempt=1 THEN NULL ELSE started_at_ms END,
                terminal_code=?
            WHERE job_id=? AND status='running'
              AND lease_owner=? AND lease_generation=?
            "#,
        )
        .bind(available_at_ms)
        .bind(restored_attempt)
        .bind(reason_code)
        .bind(current.job_id.as_bytes())
        .bind(&lease.lease_owner)
        .bind(lease.lease_generation)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if done.rows_affected() != 1 {
            return Err(JobQueueError::new(
                "stale_lease",
                "admission deferral lost lease ownership",
            ));
        }

        let job = self
            .get(&current.job_id)
            .await?
            .ok_or_else(|| JobQueueError::new("job_not_found", "job disappeared"))?;
        Ok(AdmissionDeferOutcome::Requeued(job))
    }

    pub async fn fail(
        &self,
        lease: &Lease,
        now_ms: i64,
        retryable: bool,
        terminal_code: &str,
    ) -> Result<FailureOutcome, JobQueueError> {
        require_code(terminal_code)?;
        let current = self.require_live_lease(lease, now_ms).await?;

        if current.cancel_requested_at_ms.is_some() {
            let job = self
                .finish(lease, now_ms, JobStatus::Cancelled, "cancel_requested")
                .await?;
            return Ok(FailureOutcome::Cancelled(job));
        }

        if retryable && current.attempt < current.max_attempts {
            let available_at_ms =
                now_ms.saturating_add(retry_delay_ms(&current.job_id, current.attempt));

            let done = sqlx::query(
                "UPDATE jobs                  SET status='queued', available_at_ms=?,                      lease_owner=NULL, lease_expires_at_ms=NULL,                      terminal_code=?                  WHERE job_id=? AND status='running'                    AND lease_owner=? AND lease_generation=?",
            )
            .bind(available_at_ms)
            .bind(terminal_code)
            .bind(current.job_id.as_bytes())
            .bind(&lease.lease_owner)
            .bind(lease.lease_generation)
            .execute(&self.pool)
            .await
            .map_err(sqlite_error)?;

            if done.rows_affected() != 1 {
                return Err(JobQueueError::new(
                    "stale_lease",
                    "retry update lost lease ownership",
                ));
            }

            let job = self
                .get(&current.job_id)
                .await?
                .ok_or_else(|| JobQueueError::new("job_not_found", "job disappeared"))?;
            return Ok(FailureOutcome::Requeued(job));
        }

        let job = self
            .finish(lease, now_ms, JobStatus::Failed, terminal_code)
            .await?;
        Ok(FailureOutcome::Failed(job))
    }

    pub async fn publish_success(
        &self,
        lease: &Lease,
        now_ms: i64,
        effect_key: &str,
    ) -> Result<PublishOutcome, JobQueueError> {
        require_ident(effect_key, "effect_key")?;
        if effect_key.len() > MAX_EFFECT_KEY_BYTES {
            return Err(JobQueueError::new(
                "invalid_effect_key",
                "effect key too long",
            ));
        }

        let mut conn = self.pool.acquire().await.map_err(sqlite_error)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

        let result = self
            .publish_success_in_transaction(&mut conn, lease, now_ms, effect_key)
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

    async fn publish_success_in_transaction(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
        lease: &Lease,
        now_ms: i64,
        effect_key: &str,
    ) -> Result<PublishOutcome, JobQueueError> {
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=?")
            .bind(lease.job.job_id.as_bytes())
            .fetch_optional(&mut **conn)
            .await
            .map_err(sqlite_error)?
            .ok_or_else(|| JobQueueError::new("job_not_found", "job does not exist"))?;
        let current = decode_job(row)?;

        if current.status == JobStatus::Succeeded {
            let exists = sqlx::query_scalar::<_, i64>(
                "SELECT 1 FROM job_effects WHERE job_id=? AND effect_key=?",
            )
            .bind(current.job_id.as_bytes())
            .bind(effect_key.as_bytes())
            .fetch_optional(&mut **conn)
            .await
            .map_err(sqlite_error)?;

            if exists.is_some() {
                return Ok(PublishOutcome::AlreadyPublished(current));
            }
            return Err(JobQueueError::new(
                "effect_identity_conflict",
                "job succeeded with different effect identity",
            ));
        }

        assert_live_lease(&current, lease, now_ms)?;
        if current.cancel_requested_at_ms.is_some() {
            return Err(JobQueueError::new(
                "cancel_requested",
                "job is cancelled before publish barrier",
            ));
        }

        sqlx::query(
            "INSERT INTO job_effects(              job_id,effect_key,lease_generation,published_at_ms              ) VALUES(?,?,?,?)",
        )
        .bind(current.job_id.as_bytes())
        .bind(effect_key.as_bytes())
        .bind(lease.lease_generation)
        .bind(now_ms)
        .execute(&mut **conn)
        .await
        .map_err(|error| {
            JobQueueError::new(
                "effect_publish_failed",
                bounded_sqlx(&error),
            )
        })?;

        let done = sqlx::query(
            "UPDATE jobs              SET status='succeeded', finished_at_ms=?,                  lease_owner=NULL, lease_expires_at_ms=NULL,                  terminal_code=NULL              WHERE job_id=? AND status='running'                AND lease_owner=? AND lease_generation=?",
        )
        .bind(now_ms)
        .bind(current.job_id.as_bytes())
        .bind(&lease.lease_owner)
        .bind(lease.lease_generation)
        .execute(&mut **conn)
        .await
        .map_err(sqlite_error)?;

        if done.rows_affected() != 1 {
            return Err(JobQueueError::new(
                "stale_lease",
                "success publication lost lease ownership",
            ));
        }

        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=?")
            .bind(current.job_id.as_bytes())
            .fetch_one(&mut **conn)
            .await
            .map_err(sqlite_error)?;

        Ok(PublishOutcome::Published(decode_job(row)?))
    }

    async fn finish(
        &self,
        lease: &Lease,
        now_ms: i64,
        status: JobStatus,
        terminal_code: &str,
    ) -> Result<JobRecord, JobQueueError> {
        if !matches!(status, JobStatus::Failed | JobStatus::Cancelled) {
            return Err(JobQueueError::new(
                "invalid_terminal_status",
                "finish supports failed/cancelled only",
            ));
        }

        let done = sqlx::query(
            "UPDATE jobs              SET status=?, finished_at_ms=?, terminal_code=?,                  lease_owner=NULL, lease_expires_at_ms=NULL              WHERE job_id=? AND status='running'                AND lease_owner=? AND lease_generation=?",
        )
        .bind(status.as_str())
        .bind(now_ms)
        .bind(terminal_code)
        .bind(lease.job.job_id.as_bytes())
        .bind(&lease.lease_owner)
        .bind(lease.lease_generation)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if done.rows_affected() != 1 {
            return Err(JobQueueError::new(
                "stale_lease",
                "terminal update lost lease ownership",
            ));
        }

        self.get(&lease.job.job_id)
            .await?
            .ok_or_else(|| JobQueueError::new("job_not_found", "job disappeared"))
    }

    async fn require_live_lease(
        &self,
        lease: &Lease,
        now_ms: i64,
    ) -> Result<JobRecord, JobQueueError> {
        let current = self
            .get(&lease.job.job_id)
            .await?
            .ok_or_else(|| JobQueueError::new("job_not_found", "job does not exist"))?;
        assert_live_lease(&current, lease, now_ms)?;
        Ok(current)
    }

    pub async fn metrics(&self, now_ms: i64) -> Result<QueueMetrics, JobQueueError> {
        let rows = sqlx::query("SELECT status, COUNT(*) AS n FROM jobs GROUP BY status")
            .fetch_all(&self.pool)
            .await
            .map_err(sqlite_error)?;

        let mut counts = BTreeMap::new();
        for row in rows {
            let status = row.try_get::<String, _>("status").map_err(sqlite_error)?;
            let count = row.try_get::<i64, _>("n").map_err(sqlite_error)?;
            counts.insert(status, count);
        }

        let due_queued = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM jobs              WHERE status='queued' AND available_at_ms<=?",
        )
        .bind(now_ms)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;

        let expired_running = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM jobs              WHERE status='running'                AND lease_expires_at_ms IS NOT NULL                AND lease_expires_at_ms<=?",
        )
        .bind(now_ms)
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;

        Ok(QueueMetrics {
            counts,
            due_queued,
            expired_running,
        })
    }
}

fn assert_live_lease(current: &JobRecord, lease: &Lease, now_ms: i64) -> Result<(), JobQueueError> {
    let live_expiry = current
        .lease_expires_at_ms
        .map(|expiry| expiry > now_ms)
        .unwrap_or(false);

    if current.status != JobStatus::Running
        || current.lease_owner.as_deref() != Some(lease.lease_owner.as_str())
        || current.lease_generation != lease.lease_generation
        || !live_expiry
    {
        return Err(JobQueueError::new(
            "stale_lease",
            "worker no longer owns live lease",
        ));
    }

    Ok(())
}

fn validate_enqueue(request: &EnqueueRequest) -> Result<(), JobQueueError> {
    require_ident(&request.job_id, "job_id")?;
    require_ident(&request.tenant_id, "tenant_id")?;
    require_ident(&request.idempotency_key, "idempotency_key")?;

    if request.idempotency_key.len() > MAX_IDEMPOTENCY_BYTES {
        return Err(JobQueueError::new(
            "invalid_idempotency_key",
            "idempotency key too long",
        ));
    }
    if request.payload_schema_version <= 0 {
        return Err(JobQueueError::new(
            "invalid_payload_schema",
            "payload schema version must be positive",
        ));
    }
    if request.payload.is_empty() || request.payload.len() > MAX_PAYLOAD_BYTES {
        return Err(JobQueueError::new(
            "invalid_payload_size",
            "job payload must be bounded and non-empty",
        ));
    }
    if !(1..=32).contains(&request.max_attempts) {
        return Err(JobQueueError::new(
            "invalid_max_attempts",
            "max attempts must be 1..=32",
        ));
    }

    Ok(())
}

fn validate_lease_ms(lease_ms: i64) -> Result<(), JobQueueError> {
    if !(1..=300_000).contains(&lease_ms) {
        return Err(JobQueueError::new(
            "invalid_lease",
            "lease must be >0 and <=300000 ms",
        ));
    }
    Ok(())
}

fn require_ident(value: &str, label: &str) -> Result<(), JobQueueError> {
    let allowed = value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_.:@/-".contains(&byte));

    if value.is_empty() || value.len() > 256 || !allowed {
        return Err(JobQueueError::new(
            "invalid_identity",
            format!("invalid {label}"),
        ));
    }

    Ok(())
}

fn require_owner(value: &str) -> Result<(), JobQueueError> {
    require_ident(value, "lease_owner")?;
    if value.len() > MAX_OWNER_BYTES {
        return Err(JobQueueError::new(
            "invalid_lease_owner",
            "lease owner too long",
        ));
    }
    Ok(())
}

fn require_code(value: &str) -> Result<(), JobQueueError> {
    require_ident(value, "terminal_code")
}

fn enqueue_hash(request: &EnqueueRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.tenant_id.as_bytes());
    hasher.update([0]);
    hasher.update(request.job_kind.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(request.payload_schema_version.to_be_bytes());
    hasher.update([0]);
    hasher.update(&request.payload);
    format!("{:x}", hasher.finalize())
}

fn retry_delay_ms(job_id: &str, attempt: i64) -> i64 {
    let exponent = attempt.saturating_sub(1).clamp(0, 10) as u32;
    let base = 1_000_i64.saturating_mul(1_i64 << exponent).min(60_000);
    let digest = Sha256::digest(job_id.as_bytes());
    let jitter = i64::from(digest[0]) * 500 / 255;
    base.saturating_add(jitter)
}

fn decode_job(row: sqlx::sqlite::SqliteRow) -> Result<JobRecord, JobQueueError> {
    let job_id = text_blob(&row, "job_id")?;
    let tenant_id = text_blob(&row, "tenant_id")?;
    let request_hash = text_blob(&row, "request_hash")?;
    let idempotency_key = text_blob(&row, "idempotency_key")?;
    let job_kind = JobKind::parse(&row.try_get::<String, _>("job_kind").map_err(sqlite_error)?)?;
    let status = JobStatus::parse(&row.try_get::<String, _>("status").map_err(sqlite_error)?)?;

    Ok(JobRecord {
        job_id,
        tenant_id,
        job_kind,
        payload_schema_version: row
            .try_get("payload_schema_version")
            .map_err(sqlite_error)?,
        payload: row.try_get("payload").map_err(sqlite_error)?,
        request_hash,
        status,
        available_at_ms: row.try_get("available_at_ms").map_err(sqlite_error)?,
        attempt: row.try_get("attempt").map_err(sqlite_error)?,
        max_attempts: row.try_get("max_attempts").map_err(sqlite_error)?,
        lease_owner: row.try_get("lease_owner").map_err(sqlite_error)?,
        lease_generation: row.try_get("lease_generation").map_err(sqlite_error)?,
        lease_expires_at_ms: row.try_get("lease_expires_at_ms").map_err(sqlite_error)?,
        cancel_requested_at_ms: row
            .try_get("cancel_requested_at_ms")
            .map_err(sqlite_error)?,
        idempotency_key,
        created_at_ms: row.try_get("created_at_ms").map_err(sqlite_error)?,
        started_at_ms: row.try_get("started_at_ms").map_err(sqlite_error)?,
        finished_at_ms: row.try_get("finished_at_ms").map_err(sqlite_error)?,
        terminal_code: row.try_get("terminal_code").map_err(sqlite_error)?,
    })
}

fn text_blob(row: &sqlx::sqlite::SqliteRow, name: &str) -> Result<String, JobQueueError> {
    let value = row.try_get::<Vec<u8>, _>(name).map_err(sqlite_error)?;
    String::from_utf8(value)
        .map_err(|_| JobQueueError::new("corrupt_job_row", format!("{name} is not UTF-8")))
}

fn sqlite_error(error: impl fmt::Display) -> JobQueueError {
    JobQueueError::new("sqlite_job_queue_error", error.to_string())
}

fn bounded_sqlx(error: &sqlx::Error) -> String {
    error.to_string().chars().take(256).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    async fn queue() -> (SqliteJobQueue, PathBuf) {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("chaptera-job-{}-{n}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let queue = SqliteJobQueue::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (queue, path)
    }

    #[tokio::test]
    async fn open_requires_operator_migration() {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "chaptera-job-unmigrated-{}-{n}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let error = match SqliteJobQueue::open(&path, 4, Duration::from_secs(2)).await {
            Ok(_) => panic!("job queue opened without operator migration"),
            Err(error) => error,
        };
        assert_eq!(error.code, "job_queue_database_missing");
        assert!(!path.exists());
    }

    fn req(id: &str, idem: &str) -> EnqueueRequest {
        EnqueueRequest {
            job_id: id.into(),
            tenant_id: "tenant-a".into(),
            job_kind: JobKind::Export,
            payload_schema_version: 1,
            payload: br#"{"revision":"rev-1"}"#.to_vec(),
            idempotency_key: idem.into(),
            max_attempts: 3,
            now_ms: 100,
        }
    }

    #[tokio::test]
    async fn exact_enqueue_retry_reuses_logical_job_and_changed_payload_conflicts() {
        let (queue, path) = queue().await;

        let first = queue.enqueue(req("job-1", "idem-1")).await.unwrap();
        assert!(matches!(first, EnqueueOutcome::Enqueued(_)));

        let second = queue.enqueue(req("job-2", "idem-1")).await.unwrap();
        assert!(matches!(second, EnqueueOutcome::Existing(_)));

        let mut changed = req("job-3", "idem-1");
        changed.payload = b"different".to_vec();
        let error = queue.enqueue(changed).await.unwrap_err();
        assert_eq!(error.code, "idempotency_conflict");

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn claim_is_atomic_and_stale_generation_cannot_publish() {
        let (queue, path) = queue().await;
        queue.enqueue(req("job-1", "idem-1")).await.unwrap();

        let first = queue
            .claim_one("worker-a", 200, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();

        assert!(
            queue
                .claim_one("worker-b", 210, 100, &[JobKind::Export])
                .await
                .unwrap()
                .is_none()
        );

        let second = queue
            .claim_one("worker-b", 301, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();
        assert!(second.lease_generation > first.lease_generation);

        let stale = queue
            .publish_success(&first, 302, "export:1")
            .await
            .unwrap_err();
        assert_eq!(stale.code, "stale_lease");

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn heartbeat_extends_only_live_owned_lease() {
        let (queue, path) = queue().await;
        queue.enqueue(req("job-1", "idem-1")).await.unwrap();

        let lease = queue
            .claim_one("worker-a", 200, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();

        let live = queue
            .heartbeat("job-1", "worker-a", lease.lease_generation, 250, 100)
            .await
            .unwrap();
        assert_eq!(live.lease_expires_at_ms, Some(350));

        let error = queue
            .heartbeat("job-1", "worker-b", lease.lease_generation, 260, 100)
            .await
            .unwrap_err();
        assert_eq!(error.code, "stale_lease");

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn retry_backoff_is_bounded_and_max_attempt_becomes_terminal() {
        let (queue, path) = queue().await;
        let mut request = req("job-1", "idem-1");
        request.max_attempts = 2;
        queue.enqueue(request).await.unwrap();

        let first = queue
            .claim_one("worker-a", 200, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();

        let requeued = queue.fail(&first, 210, true, "transient").await.unwrap();
        let FailureOutcome::Requeued(job) = requeued else {
            panic!("expected requeue")
        };
        assert!(job.available_at_ms > 210);

        let second = queue
            .claim_one("worker-a", job.available_at_ms, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();

        let failed = queue
            .fail(&second, job.available_at_ms + 1, true, "transient")
            .await
            .unwrap();
        let FailureOutcome::Failed(job) = failed else {
            panic!("expected terminal failure")
        };
        assert_eq!(job.status, JobStatus::Failed);

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn cancellation_wins_before_publish_barrier() {
        let (queue, path) = queue().await;
        queue.enqueue(req("job-1", "idem-1")).await.unwrap();

        let lease = queue
            .claim_one("worker-a", 200, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();
        queue.request_cancel("job-1", 220).await.unwrap();

        let error = queue
            .publish_success(&lease, 230, "export:1")
            .await
            .unwrap_err();
        assert_eq!(error.code, "cancel_requested");

        let outcome = queue.fail(&lease, 230, false, "ignored").await.unwrap();
        let FailureOutcome::Cancelled(job) = outcome else {
            panic!("expected cancellation")
        };
        assert_eq!(job.status, JobStatus::Cancelled);

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn publication_is_idempotent_and_duplicate_delivery_cannot_double_publish() {
        let (queue, path) = queue().await;
        queue.enqueue(req("job-1", "idem-1")).await.unwrap();

        let lease = queue
            .claim_one("worker-a", 200, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();

        let first = queue
            .publish_success(&lease, 220, "export:stable-effect")
            .await
            .unwrap();
        assert!(matches!(first, PublishOutcome::Published(_)));

        let second = queue
            .publish_success(&lease, 230, "export:stable-effect")
            .await
            .unwrap();
        assert!(matches!(second, PublishOutcome::AlreadyPublished(_)));

        let conflict = queue
            .publish_success(&lease, 230, "export:different-effect")
            .await
            .unwrap_err();
        assert_eq!(conflict.code, "effect_identity_conflict");

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn restart_reclaims_expired_lease_from_same_database() {
        let (queue, path) = queue().await;
        queue.enqueue(req("job-1", "idem-1")).await.unwrap();

        let first = queue
            .claim_one("worker-a", 200, 50, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();
        queue.close().await;

        let reopened = SqliteJobQueue::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let second = reopened
            .claim_one("worker-b", 251, 100, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();

        assert!(second.lease_generation > first.lease_generation);

        reopened.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn metrics_are_structural_and_payload_free() {
        let (queue, path) = queue().await;
        queue.enqueue(req("job-1", "idem-1")).await.unwrap();

        let metrics = queue.metrics(100).await.unwrap();
        assert_eq!(metrics.due_queued, 1);
        assert_eq!(metrics.counts.get("queued"), Some(&1));

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }
}
