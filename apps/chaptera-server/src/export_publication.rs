use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

pub const EXPORT_PUBLICATION_SCHEMA_V1: &str = "chaptera.export-publication.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPublicationError {
    pub code: &'static str,
    pub message: String,
}

impl ExportPublicationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExportPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ExportPublicationError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportPublicationInputV1 {
    pub tenant_id: String,
    pub job_id: String,
    pub document_id: String,
    pub exact_revision_id: String,
    pub canonical_revision_id: String,
    pub target_profile: String,
    pub layout_environment_id: String,
    pub fence_id: String,
    pub artifact_binding_id: String,
    pub artifact_content_hash: String,
    pub loss_binding_id: String,
    pub loss_report_hash: String,
}

impl ExportPublicationInputV1 {
    pub fn validate(&self) -> Result<(), ExportPublicationError> {
        for (label, value) in [
            ("tenant_id", self.tenant_id.as_str()),
            ("job_id", self.job_id.as_str()),
            ("document_id", self.document_id.as_str()),
            ("exact_revision_id", self.exact_revision_id.as_str()),
            ("target_profile", self.target_profile.as_str()),
            ("artifact_binding_id", self.artifact_binding_id.as_str()),
            ("loss_binding_id", self.loss_binding_id.as_str()),
        ] {
            require_ident(value, label)?;
        }
        require_hex_sha256(&self.canonical_revision_id, "canonical_revision_id")?;
        for (label, value) in [
            ("layout_environment_id", self.layout_environment_id.as_str()),
            ("fence_id", self.fence_id.as_str()),
            ("artifact_content_hash", self.artifact_content_hash.as_str()),
            ("loss_report_hash", self.loss_report_hash.as_str()),
        ] {
            require_prefixed_sha256(value, label)?;
        }
        Ok(())
    }

    pub fn publication_id(&self) -> Result<String, ExportPublicationError> {
        self.validate()?;
        hash_envelope(
            "chaptera-export-publication-v1\0",
            [
                ("tenant_id", self.tenant_id.as_str()),
                ("job_id", self.job_id.as_str()),
                ("document_id", self.document_id.as_str()),
                ("exact_revision_id", self.exact_revision_id.as_str()),
                ("canonical_revision_id", self.canonical_revision_id.as_str()),
                ("target_profile", self.target_profile.as_str()),
                ("layout_environment_id", self.layout_environment_id.as_str()),
                ("fence_id", self.fence_id.as_str()),
                ("artifact_content_hash", self.artifact_content_hash.as_str()),
                ("loss_report_hash", self.loss_report_hash.as_str()),
            ],
        )
    }

    pub fn effect_key(&self) -> Result<String, ExportPublicationError> {
        self.validate()?;
        // Preserve the existing export-job logical identity: tenant + job +
        // exact revision + artifact content hash. Physical BlobStore binding IDs
        // are intentionally excluded because dedupe retry may allocate new
        // tenant-local bindings for identical bytes.
        hash_envelope(
            "chaptera-export-effect-v1\0",
            [
                ("tenant_id", self.tenant_id.as_str()),
                ("job_id", self.job_id.as_str()),
                ("exact_revision_id", self.exact_revision_id.as_str()),
                ("artifact_content_hash", self.artifact_content_hash.as_str()),
            ],
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPublicationRecordV1 {
    pub publication_id: String,
    pub effect_key: String,
    pub input: ExportPublicationInputV1,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportPublicationPrepareOutcomeV1 {
    Prepared(ExportPublicationRecordV1),
    AlreadyPrepared(ExportPublicationRecordV1),
}

#[derive(Clone)]
pub struct SqliteExportPublicationStore {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteExportPublicationStore {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, ExportPublicationError> {
        if max_connections == 0 || max_connections > 16 {
            return Err(ExportPublicationError::new(
                "invalid_pool_size",
                "export publication store must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(ExportPublicationError::new(
                "invalid_busy_timeout",
                "busy timeout must be >0 and <=30 seconds",
            ));
        }
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(ExportPublicationError::new(
                "sqlite_database_missing",
                "run chaptera migrate up before opening export publications",
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

        let store = Self { path, pool };
        store.require_schema().await?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn prepare(
        &self,
        input: ExportPublicationInputV1,
        created_at_ms: i64,
    ) -> Result<ExportPublicationPrepareOutcomeV1, ExportPublicationError> {
        input.validate()?;
        if created_at_ms < 0 {
            return Err(ExportPublicationError::new(
                "invalid_created_at",
                "created_at_ms must be non-negative",
            ));
        }
        let publication_id = input.publication_id()?;
        let effect_key = input.effect_key()?;

        let result = sqlx::query(
            r#"
            INSERT INTO export_publications (
                publication_id,
                effect_key,
                tenant_id,
                job_id,
                document_id,
                exact_revision_id,
                canonical_revision_id,
                target_profile,
                layout_environment_id,
                fence_id,
                artifact_binding_id,
                artifact_content_hash,
                loss_binding_id,
                loss_report_hash,
                created_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&publication_id)
        .bind(&effect_key)
        .bind(input.tenant_id.as_bytes())
        .bind(input.job_id.as_bytes())
        .bind(input.document_id.as_bytes())
        .bind(input.exact_revision_id.as_bytes())
        .bind(&input.canonical_revision_id)
        .bind(&input.target_profile)
        .bind(&input.layout_environment_id)
        .bind(&input.fence_id)
        .bind(input.artifact_binding_id.as_bytes())
        .bind(&input.artifact_content_hash)
        .bind(input.loss_binding_id.as_bytes())
        .bind(&input.loss_report_hash)
        .bind(created_at_ms)
        .execute(&self.pool)
        .await;

        match result {
            Ok(done) if done.rows_affected() == 1 => Ok(ExportPublicationPrepareOutcomeV1::Prepared(
                ExportPublicationRecordV1 {
                    publication_id,
                    effect_key,
                    input,
                    created_at_ms,
                },
            )),
            Ok(_) => Err(ExportPublicationError::new(
                "export_publication_no_effect",
                "export publication insert did not create one row",
            )),
            Err(error) if is_constraint(&error) => {
                let existing = self
                    .get_by_job(&input.tenant_id, &input.job_id)
                    .await?
                    .ok_or_else(|| {
                        ExportPublicationError::new(
                            "export_publication_conflict",
                            "logical publication conflicted without a readable row",
                        )
                    })?;

                if equivalent_retry(&existing.input, &input)
                    && existing.effect_key == effect_key
                    && existing.publication_id == publication_id
                {
                    Ok(ExportPublicationPrepareOutcomeV1::AlreadyPrepared(existing))
                } else {
                    Err(ExportPublicationError::new(
                        "export_publication_conflict",
                        "same tenant/job publication identity already maps to different revision, fence, artifact, or loss evidence",
                    ))
                }
            }
            Err(error) => Err(sqlite_error(error)),
        }
    }

    pub async fn get_visible_by_job(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Option<ExportPublicationRecordV1>, ExportPublicationError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(job_id, "job_id")?;
        let row = sqlx::query(
            r#"
            SELECT
                p.publication_id,
                p.effect_key,
                p.tenant_id,
                p.job_id,
                p.document_id,
                p.exact_revision_id,
                p.canonical_revision_id,
                p.target_profile,
                p.layout_environment_id,
                p.fence_id,
                p.artifact_binding_id,
                p.artifact_content_hash,
                p.loss_binding_id,
                p.loss_report_hash,
                p.created_at_ms
            FROM export_publications p
            INNER JOIN job_effects e
              ON e.job_id = p.job_id
             AND e.effect_key = p.effect_key
            WHERE p.tenant_id=? AND p.job_id=?
            "#,
        )
        .bind(tenant_id.as_bytes())
        .bind(job_id.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?;
        row.map(decode_record).transpose()
    }

    pub async fn get_by_job(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Option<ExportPublicationRecordV1>, ExportPublicationError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(job_id, "job_id")?;
        let row = sqlx::query(
            r#"
            SELECT
                publication_id,
                effect_key,
                tenant_id,
                job_id,
                document_id,
                exact_revision_id,
                canonical_revision_id,
                target_profile,
                layout_environment_id,
                fence_id,
                artifact_binding_id,
                artifact_content_hash,
                loss_binding_id,
                loss_report_hash,
                created_at_ms
            FROM export_publications
            WHERE tenant_id=? AND job_id=?
            "#,
        )
        .bind(tenant_id.as_bytes())
        .bind(job_id.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?;
        row.map(decode_record).transpose()
    }

    async fn require_schema(&self) -> Result<(), ExportPublicationError> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='export_publications'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;
        if exists != 1 {
            return Err(ExportPublicationError::new(
                "sqlite_schema_missing",
                "export_publications table is absent; run chaptera migrate up",
            ));
        }
        Ok(())
    }
}

fn decode_record(
    row: sqlx::sqlite::SqliteRow,
) -> Result<ExportPublicationRecordV1, ExportPublicationError> {
    let input = ExportPublicationInputV1 {
        tenant_id: blob_text(&row, "tenant_id")?,
        job_id: blob_text(&row, "job_id")?,
        document_id: blob_text(&row, "document_id")?,
        exact_revision_id: blob_text(&row, "exact_revision_id")?,
        canonical_revision_id: row.try_get("canonical_revision_id").map_err(sqlite_error)?,
        target_profile: row.try_get("target_profile").map_err(sqlite_error)?,
        layout_environment_id: row
            .try_get("layout_environment_id")
            .map_err(sqlite_error)?,
        fence_id: row.try_get("fence_id").map_err(sqlite_error)?,
        artifact_binding_id: blob_text(&row, "artifact_binding_id")?,
        artifact_content_hash: row
            .try_get("artifact_content_hash")
            .map_err(sqlite_error)?,
        loss_binding_id: blob_text(&row, "loss_binding_id")?,
        loss_report_hash: row.try_get("loss_report_hash").map_err(sqlite_error)?,
    };
    input.validate()?;

    let publication_id: String = row.try_get("publication_id").map_err(sqlite_error)?;
    let effect_key: String = row.try_get("effect_key").map_err(sqlite_error)?;
    let created_at_ms: i64 = row.try_get("created_at_ms").map_err(sqlite_error)?;
    require_prefixed_sha256(&publication_id, "publication_id")?;
    require_prefixed_sha256(&effect_key, "effect_key")?;
    if created_at_ms < 0
        || publication_id != input.publication_id()?
        || effect_key != input.effect_key()?
    {
        return Err(ExportPublicationError::new(
            "export_publication_row_corrupt",
            "stored export publication identity does not match canonical input",
        ));
    }

    Ok(ExportPublicationRecordV1 {
        publication_id,
        effect_key,
        input,
        created_at_ms,
    })
}

fn equivalent_retry(left: &ExportPublicationInputV1, right: &ExportPublicationInputV1) -> bool {
    left.tenant_id == right.tenant_id
        && left.job_id == right.job_id
        && left.document_id == right.document_id
        && left.exact_revision_id == right.exact_revision_id
        && left.canonical_revision_id == right.canonical_revision_id
        && left.target_profile == right.target_profile
        && left.layout_environment_id == right.layout_environment_id
        && left.fence_id == right.fence_id
        && left.artifact_content_hash == right.artifact_content_hash
        && left.loss_report_hash == right.loss_report_hash
    // Blob binding IDs are physical/logical storage handles. Exact retry may
    // allocate fresh binding IDs over the same tenant-local deduped bytes
    // before the durable publication record is observed.
}

fn hash_envelope<const N: usize>(
    domain: &str,
    fields: [(&str, &str); N],
) -> Result<String, ExportPublicationError> {
    let mut envelope = BTreeMap::new();
    envelope.insert("domain", domain);
    for (key, value) in fields {
        envelope.insert(key, value);
    }
    let bytes = serde_json::to_vec(&envelope).map_err(|error| {
        ExportPublicationError::new(
            "export_publication_hash_failed",
            bounded_message(&error.to_string()),
        )
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, ExportPublicationError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        ExportPublicationError::new(
            "export_publication_row_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn require_ident(value: &str, label: &'static str) -> Result<(), ExportPublicationError> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ExportPublicationError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_hex_sha256(value: &str, label: &'static str) -> Result<(), ExportPublicationError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ExportPublicationError::new(
            "invalid_hash",
            format!("{label} must be 64 lowercase hex characters"),
        ));
    }
    Ok(())
}

fn require_prefixed_sha256(value: &str, label: &'static str) -> Result<(), ExportPublicationError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(ExportPublicationError::new(
            "invalid_hash",
            format!("{label} must use sha256:<64 lowercase hex>"),
        ));
    };
    require_hex_sha256(hex, label)
}

fn is_constraint(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.is_unique_violation())
}

fn sqlite_error(error: impl fmt::Display) -> ExportPublicationError {
    ExportPublicationError::new(
        "sqlite_export_publication_error",
        bounded_message(&error.to_string()),
    )
}

fn bounded_message(message: &str) -> String {
    message.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
        job_queue::{EnqueueRequest, JobKind, SqliteJobQueue},
        schema_migration::SqliteMigrationRuntime,
    };

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn path(label: &str) -> PathBuf {
        let serial = NEXT.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-export-publication-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn hash(ch: char) -> String {
        format!("sha256:{}", std::iter::repeat_n(ch, 64).collect::<String>())
    }

    fn input(artifact: char, loss: char) -> ExportPublicationInputV1 {
        ExportPublicationInputV1 {
            tenant_id: "tenant-a".into(),
            job_id: "job-export-1".into(),
            document_id: "doc-a".into(),
            exact_revision_id: "service-rev-1".into(),
            canonical_revision_id: std::iter::repeat_n('c', 64).collect(),
            target_profile: "idml:bounded-editable".into(),
            layout_environment_id: hash('e'),
            fence_id: hash('f'),
            artifact_binding_id: "binding-artifact-1".into(),
            artifact_content_hash: hash(artifact),
            loss_binding_id: "binding-loss-1".into(),
            loss_report_hash: hash(loss),
        }
    }

    async fn store(path: &Path) -> SqliteExportPublicationStore {
        SqliteMigrationRuntime::new(path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        SqliteExportPublicationStore::open(path, 2, Duration::from_secs(2))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn exact_retry_returns_same_stable_effect_even_with_new_blob_binding_ids() {
        let db = path("retry");
        let store = store(&db).await;
        let first_input = input('a', 'b');
        let first = store.prepare(first_input.clone(), 100).await.unwrap();
        let ExportPublicationPrepareOutcomeV1::Prepared(first) = first else {
            panic!("first publish must create publication")
        };

        let mut retry_input = first_input;
        retry_input.artifact_binding_id = "binding-artifact-retry".into();
        retry_input.loss_binding_id = "binding-loss-retry".into();
        let retry = store.prepare(retry_input, 200).await.unwrap();
        let ExportPublicationPrepareOutcomeV1::AlreadyPrepared(retry) = retry else {
            panic!("exact retry must resolve prior publication")
        };
        assert_eq!(retry.effect_key, first.effect_key);
        assert_eq!(retry.publication_id, first.publication_id);
        assert_eq!(retry.created_at_ms, 100);

        store.close().await;
        cleanup(&db);
    }

    #[tokio::test]
    async fn prepared_publication_is_invisible_until_matching_job_effect_commits() {
        let db = path("visibility");
        let store = store(&db).await;
        let queue = SqliteJobQueue::open(&db, 2, Duration::from_secs(2))
            .await
            .unwrap();
        queue
            .enqueue(EnqueueRequest {
                job_id: "job-export-1".into(),
                tenant_id: "tenant-a".into(),
                job_kind: JobKind::Export,
                payload_schema_version: 1,
                payload: br#"{"bounded":true}"#.to_vec(),
                idempotency_key: "idem-export-1".into(),
                max_attempts: 3,
                now_ms: 10,
            })
            .await
            .unwrap();

        let prepared = store.prepare(input('a', 'b'), 20).await.unwrap();
        let effect_key = match prepared {
            ExportPublicationPrepareOutcomeV1::Prepared(record)
            | ExportPublicationPrepareOutcomeV1::AlreadyPrepared(record) => record.effect_key,
        };
        assert!(
            store
                .get_visible_by_job("tenant-a", "job-export-1")
                .await
                .unwrap()
                .is_none()
        );

        let lease = queue
            .claim_one("worker-a", 30, 1_000, &[JobKind::Export])
            .await
            .unwrap()
            .unwrap();
        queue
            .publish_success(&lease, 40, &effect_key)
            .await
            .unwrap();

        let visible = store
            .get_visible_by_job("tenant-a", "job-export-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(visible.effect_key, effect_key);

        queue.close().await;
        store.close().await;
        cleanup(&db);
    }

    #[tokio::test]
    async fn changed_artifact_or_loss_under_same_job_fails_closed() {
        let db = path("conflict");
        let store = store(&db).await;
        store.prepare(input('a', 'b'), 100).await.unwrap();

        for changed in [input('d', 'b'), input('a', 'e')] {
            let error = store.prepare(changed, 200).await.unwrap_err();
            assert_eq!(error.code, "export_publication_conflict");
        }

        store.close().await;
        cleanup(&db);
    }
}
