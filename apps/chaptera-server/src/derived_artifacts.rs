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

pub const DERIVED_ARTIFACT_FENCE_SCHEMA_V1: &str = "chaptera.derived-artifact-fence.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedArtifactError {
    pub code: &'static str,
    pub message: String,
}

impl DerivedArtifactError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for DerivedArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DerivedArtifactError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedArtifactFenceV1 {
    pub document_id: String,
    pub service_revision_id: String,
    pub canonical_revision_id: String,
    pub stage: String,
    pub stage_version: String,
    pub environment_fingerprint: String,
    pub input_fingerprint: String,
}

impl DerivedArtifactFenceV1 {
    pub fn validate(&self) -> Result<(), DerivedArtifactError> {
        require_ident(&self.document_id, "document_id")?;
        require_ident(&self.service_revision_id, "service_revision_id")?;
        require_hex_sha256(&self.canonical_revision_id, "canonical_revision_id")?;
        if !matches!(self.stage.as_str(), "layout" | "scene" | "preview" | "export") {
            return Err(DerivedArtifactError::new(
                "invalid_stage",
                "stage must be layout, scene, preview, or export",
            ));
        }
        require_ident(&self.stage_version, "stage_version")?;
        require_prefixed_sha256(&self.environment_fingerprint, "environment_fingerprint")?;
        require_prefixed_sha256(&self.input_fingerprint, "input_fingerprint")?;
        Ok(())
    }

    pub fn fence_id(&self) -> Result<String, DerivedArtifactError> {
        self.validate()?;

        // Match the canonical JSON + sorted-key hash used by the Rar service
        // contract landed in rar#432. BTreeMap makes key order explicit.
        let mut envelope = BTreeMap::new();
        envelope.insert(
            "canonical_revision_id",
            self.canonical_revision_id.as_str(),
        );
        envelope.insert("document_id", self.document_id.as_str());
        envelope.insert(
            "environment_fingerprint",
            self.environment_fingerprint.as_str(),
        );
        envelope.insert("input_fingerprint", self.input_fingerprint.as_str());
        envelope.insert(
            "schema_version",
            DERIVED_ARTIFACT_FENCE_SCHEMA_V1,
        );
        envelope.insert(
            "service_revision_id",
            self.service_revision_id.as_str(),
        );
        envelope.insert("stage", self.stage.as_str());
        envelope.insert("stage_version", self.stage_version.as_str());

        let bytes = serde_json::to_vec(&envelope).map_err(|error| {
            DerivedArtifactError::new(
                "fence_serialization_failed",
                bounded_message(&error.to_string()),
            )
        })?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedArtifactRecordV1 {
    pub fence_id: String,
    pub fence: DerivedArtifactFenceV1,
    pub content_hash: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOutcomeV1 {
    Published(DerivedArtifactRecordV1),
    AlreadyPublished(DerivedArtifactRecordV1),
}

#[derive(Clone)]
pub struct SqliteDerivedArtifactStore {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteDerivedArtifactStore {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, DerivedArtifactError> {
        if max_connections == 0 || max_connections > 16 {
            return Err(DerivedArtifactError::new(
                "invalid_pool_size",
                "derived artifact store must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(DerivedArtifactError::new(
                "invalid_busy_timeout",
                "busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(DerivedArtifactError::new(
                "sqlite_database_missing",
                "run chaptera migrate up before opening derived artifacts",
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

    pub async fn publish(
        &self,
        fence: DerivedArtifactFenceV1,
        content_hash: String,
        created_at_ms: i64,
    ) -> Result<PublishOutcomeV1, DerivedArtifactError> {
        fence.validate()?;
        require_prefixed_sha256(&content_hash, "content_hash")?;
        if created_at_ms < 0 {
            return Err(DerivedArtifactError::new(
                "invalid_created_at",
                "created_at_ms must be non-negative",
            ));
        }

        let fence_id = fence.fence_id()?;
        let result = sqlx::query(
            r#"
            INSERT INTO derived_artifacts (
                fence_id,
                document_id,
                service_revision_id,
                canonical_revision_id,
                stage,
                stage_version,
                environment_fingerprint,
                input_fingerprint,
                content_hash,
                created_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&fence_id)
        .bind(fence.document_id.as_bytes())
        .bind(fence.service_revision_id.as_bytes())
        .bind(&fence.canonical_revision_id)
        .bind(&fence.stage)
        .bind(&fence.stage_version)
        .bind(&fence.environment_fingerprint)
        .bind(&fence.input_fingerprint)
        .bind(&content_hash)
        .bind(created_at_ms)
        .execute(&self.pool)
        .await;

        match result {
            Ok(done) if done.rows_affected() == 1 => Ok(PublishOutcomeV1::Published(
                DerivedArtifactRecordV1 {
                    fence_id,
                    fence,
                    content_hash,
                    created_at_ms,
                },
            )),
            Ok(_) => Err(DerivedArtifactError::new(
                "artifact_publish_no_effect",
                "derived artifact insert completed without one row",
            )),
            Err(error) if is_constraint(&error) => {
                let existing = self
                    .resolve_by_fence_id(&fence_id)
                    .await?
                    .ok_or_else(|| {
                        DerivedArtifactError::new(
                            "artifact_fence_conflict",
                            "exact fence conflicted without a readable historical row",
                        )
                    })?;

                if existing.fence == fence && existing.content_hash == content_hash {
                    Ok(PublishOutcomeV1::AlreadyPublished(existing))
                } else {
                    Err(DerivedArtifactError::new(
                        "artifact_fence_nondeterministic",
                        "same exact derived-artifact fence already maps to different content",
                    ))
                }
            }
            Err(error) => Err(sqlite_error(error)),
        }
    }

    pub async fn resolve(
        &self,
        fence: &DerivedArtifactFenceV1,
    ) -> Result<Option<DerivedArtifactRecordV1>, DerivedArtifactError> {
        let fence_id = fence.fence_id()?;
        self.resolve_by_fence_id(&fence_id).await
    }

    pub async fn rebuild_required(
        &self,
        fence: &DerivedArtifactFenceV1,
    ) -> Result<bool, DerivedArtifactError> {
        Ok(self.resolve(fence).await?.is_none())
    }

    async fn resolve_by_fence_id(
        &self,
        fence_id: &str,
    ) -> Result<Option<DerivedArtifactRecordV1>, DerivedArtifactError> {
        require_prefixed_sha256(fence_id, "fence_id")?;
        let row = sqlx::query(
            r#"
            SELECT
                fence_id,
                document_id,
                service_revision_id,
                canonical_revision_id,
                stage,
                stage_version,
                environment_fingerprint,
                input_fingerprint,
                content_hash,
                created_at_ms
            FROM derived_artifacts
            WHERE fence_id = ?
            "#,
        )
        .bind(fence_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?;

        row.map(decode_row).transpose()
    }

    async fn require_schema(&self) -> Result<(), DerivedArtifactError> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='derived_artifacts'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if exists != 1 {
            return Err(DerivedArtifactError::new(
                "sqlite_schema_missing",
                "derived_artifacts table is absent; run chaptera migrate up",
            ));
        }
        Ok(())
    }
}

fn decode_row(row: sqlx::sqlite::SqliteRow) -> Result<DerivedArtifactRecordV1, DerivedArtifactError> {
    let fence = DerivedArtifactFenceV1 {
        document_id: blob_text(&row, "document_id")?,
        service_revision_id: blob_text(&row, "service_revision_id")?,
        canonical_revision_id: row
            .try_get("canonical_revision_id")
            .map_err(sqlite_error)?,
        stage: row.try_get("stage").map_err(sqlite_error)?,
        stage_version: row.try_get("stage_version").map_err(sqlite_error)?,
        environment_fingerprint: row
            .try_get("environment_fingerprint")
            .map_err(sqlite_error)?,
        input_fingerprint: row
            .try_get("input_fingerprint")
            .map_err(sqlite_error)?,
    };
    fence.validate()?;

    let record = DerivedArtifactRecordV1 {
        fence_id: row.try_get("fence_id").map_err(sqlite_error)?,
        fence,
        content_hash: row.try_get("content_hash").map_err(sqlite_error)?,
        created_at_ms: row.try_get("created_at_ms").map_err(sqlite_error)?,
    };
    require_prefixed_sha256(&record.fence_id, "fence_id")?;
    require_prefixed_sha256(&record.content_hash, "content_hash")?;
    if record.created_at_ms < 0 {
        return Err(DerivedArtifactError::new(
            "sqlite_row_corrupt",
            "derived artifact created_at_ms is negative",
        ));
    }
    if record.fence.fence_id()? != record.fence_id {
        return Err(DerivedArtifactError::new(
            "sqlite_row_corrupt",
            "stored derived artifact fence_id does not match its exact fence",
        ));
    }
    Ok(record)
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, DerivedArtifactError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        DerivedArtifactError::new(
            "sqlite_row_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn require_ident(value: &str, label: &'static str) -> Result<(), DerivedArtifactError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(DerivedArtifactError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_hex_sha256(value: &str, label: &'static str) -> Result<(), DerivedArtifactError> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f')) {
        return Err(DerivedArtifactError::new(
            "invalid_hash",
            format!("{label} must be 64 lowercase hex characters"),
        ));
    }
    Ok(())
}

fn require_prefixed_sha256(
    value: &str,
    label: &'static str,
) -> Result<(), DerivedArtifactError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(DerivedArtifactError::new(
            "invalid_hash",
            format!("{label} must use sha256:<64 lowercase hex>"),
        ));
    };
    require_hex_sha256(hex, label)
}

fn is_constraint(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database) if database.is_unique_violation()
    )
}

fn sqlite_error(error: impl fmt::Display) -> DerivedArtifactError {
    DerivedArtifactError::new("sqlite_artifact_error", bounded_message(&error.to_string()))
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

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-derived-artifact-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn fingerprint(ch: char) -> String {
        format!("sha256:{}", std::iter::repeat_n(ch, 64).collect::<String>())
    }

    fn fence(canonical: char, stage: &str, environment: char, input: char) -> DerivedArtifactFenceV1 {
        DerivedArtifactFenceV1 {
            document_id: "doc-1".into(),
            service_revision_id: fingerprint('a'),
            canonical_revision_id: std::iter::repeat_n(canonical, 64).collect(),
            stage: stage.into(),
            stage_version: format!("{stage}-v1"),
            environment_fingerprint: fingerprint(environment),
            input_fingerprint: fingerprint(input),
        }
    }

    async fn store(path: &Path) -> SqliteDerivedArtifactStore {
        SqliteMigrationRuntime::new(path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        SqliteDerivedArtifactStore::open(path, 2, Duration::from_secs(2))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn exact_fence_is_durable_and_retry_is_idempotent() {
        let path = temp_db("retry");
        let store = store(&path).await;
        let exact = fence('1', "scene", '2', '3');

        let first = store
            .publish(exact.clone(), fingerprint('4'), 100)
            .await
            .unwrap();
        assert!(matches!(first, PublishOutcomeV1::Published(_)));

        let retry = store
            .publish(exact.clone(), fingerprint('4'), 200)
            .await
            .unwrap();
        let PublishOutcomeV1::AlreadyPublished(retry) = retry else {
            panic!("exact retry must return historical row");
        };
        assert_eq!(retry.created_at_ms, 100);

        store.close().await;
        let reopened = SqliteDerivedArtifactStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            reopened.resolve(&exact).await.unwrap().unwrap().content_hash,
            fingerprint('4')
        );

        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn same_exact_fence_with_different_content_fails_closed() {
        let path = temp_db("nondeterministic");
        let store = store(&path).await;
        let exact = fence('1', "preview", '2', '3');

        store
            .publish(exact.clone(), fingerprint('4'), 100)
            .await
            .unwrap();
        let error = store
            .publish(exact, fingerprint('5'), 101)
            .await
            .unwrap_err();
        assert_eq!(error.code, "artifact_fence_nondeterministic");

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn revision_environment_stage_and_input_changes_are_independent_history() {
        let path = temp_db("history");
        let store = store(&path).await;
        let base = fence('1', "scene", '2', '3');
        store
            .publish(base.clone(), fingerprint('4'), 100)
            .await
            .unwrap();

        for changed in [
            fence('5', "scene", '2', '3'),
            fence('1', "scene", '6', '3'),
            fence('1', "preview", '2', '3'),
            fence('1', "scene", '2', '7'),
        ] {
            assert!(store.rebuild_required(&changed).await.unwrap());
            store
                .publish(changed.clone(), fingerprint('8'), 200)
                .await
                .unwrap();
            assert!(!store.rebuild_required(&changed).await.unwrap());
        }

        assert_eq!(
            store.resolve(&base).await.unwrap().unwrap().content_hash,
            fingerprint('4')
        );
        store.close().await;
        cleanup(&path);
    }

    #[test]
    fn rust_fence_id_matches_rar_service_contract_algorithm() {
        let exact = DerivedArtifactFenceV1 {
            document_id: "10000000-0000-4000-8000-000000000001".into(),
            service_revision_id: fingerprint('6'),
            canonical_revision_id: std::iter::repeat_n('2', 64).collect(),
            stage: "scene".into(),
            stage_version: "scene-v1".into(),
            environment_fingerprint: fingerprint('3'),
            input_fingerprint: fingerprint('4'),
        };

        // This is intentionally asserted as a stable value so the physical
        // store cannot silently drift from the Python bridge's canonical JSON.
        assert_eq!(
            exact.fence_id().unwrap(),
            "sha256:8bc52ef0bc7b4cb222585937872210627801d9821d9c90de15b9969d2065c9be"
        );
    }
}
