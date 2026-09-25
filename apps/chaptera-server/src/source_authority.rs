use std::{fmt, path::{Path, PathBuf}, time::Duration};

use async_trait::async_trait;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::revision_materializer::{
    AuthorizedDocumentSource, DocumentSourceAuthority, RevisionMaterializerError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthorityError {
    pub code: &'static str,
    pub message: String,
}

impl SourceAuthorityError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for SourceAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SourceAuthorityError {}

#[derive(Clone)]
pub struct SqliteDocumentSourceAuthority {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteDocumentSourceAuthority {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, SourceAuthorityError> {
        if max_connections == 0 || max_connections > 16 {
            return Err(SourceAuthorityError::new(
                "invalid_pool_size",
                "document source authority must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(SourceAuthorityError::new(
                "invalid_busy_timeout",
                "busy timeout must be >0 and <=30 seconds",
            ));
        }
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(SourceAuthorityError::new(
                "sqlite_database_missing",
                "run chaptera migrate up before opening document source authority",
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

        let authority = Self { path, pool };
        authority.require_schema().await?;
        Ok(authority)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn resolve(
        &self,
        tenant_id: &str,
        document_id: &str,
    ) -> Result<AuthorizedDocumentSource, SourceAuthorityError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(document_id, "document_id")?;

        let row = sqlx::query(
            r#"
            SELECT
                c.tenant_id,
                c.document_id,
                c.genesis_revision_id,
                u.upload_id,
                u.state,
                u.durable_binding_id,
                u.canonical_sha256,
                u.observed_byte_len
            FROM upload_consumptions c
            INNER JOIN uploads u ON u.upload_id = c.upload_id
            WHERE c.tenant_id = ? AND c.document_id = ?
            "#,
        )
        .bind(tenant_id.as_bytes())
        .bind(document_id.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?
        .ok_or_else(|| SourceAuthorityError::new(
            "document_source_not_found",
            "document has no persisted consumed source authority",
        ))?;

        let stored_tenant = blob_text(&row, "tenant_id")?;
        let stored_document = blob_text(&row, "document_id")?;
        let genesis_revision_id = blob_text(&row, "genesis_revision_id")?;
        let state: String = row.try_get("state").map_err(sqlite_error)?;
        if stored_tenant != tenant_id || stored_document != document_id {
            return Err(SourceAuthorityError::new(
                "document_source_identity_mismatch",
                "source authority row belongs to a different tenant or document",
            ));
        }
        if state != "CONSUMED" {
            return Err(SourceAuthorityError::new(
                "document_source_not_consumed",
                "document source authority is not backed by a consumed upload",
            ));
        }

        let binding_id = optional_blob_text(&row, "durable_binding_id")?.ok_or_else(|| {
            SourceAuthorityError::new(
                "document_source_corrupt",
                "consumed upload is missing durable binding id",
            )
        })?;
        let source_sha256 = optional_blob_text(&row, "canonical_sha256")?.ok_or_else(|| {
            SourceAuthorityError::new(
                "document_source_corrupt",
                "consumed upload is missing canonical source hash",
            )
        })?;
        let observed_len: Option<i64> = row.try_get("observed_byte_len").map_err(sqlite_error)?;
        let observed_len = observed_len.ok_or_else(|| {
            SourceAuthorityError::new(
                "document_source_corrupt",
                "consumed upload is missing observed byte length",
            )
        })?;
        let byte_len = u64::try_from(observed_len).map_err(|_| {
            SourceAuthorityError::new(
                "document_source_corrupt",
                "consumed upload byte length is negative",
            )
        })?;

        require_ident(&binding_id, "durable_binding_id")?;
        require_ident(&genesis_revision_id, "genesis_revision_id")?;
        require_sha256(&source_sha256, "canonical_sha256")?;
        if byte_len == 0 {
            return Err(SourceAuthorityError::new(
                "document_source_corrupt",
                "consumed upload byte length must be positive",
            ));
        }

        Ok(AuthorizedDocumentSource {
            tenant_id: stored_tenant,
            document_id: stored_document,
            binding_id,
            source_sha256,
            byte_len,
            baseline_revision_id: genesis_revision_id,
            baseline_cursor: 0,
        })
    }

    async fn require_schema(&self) -> Result<(), SourceAuthorityError> {
        for table in ["uploads", "upload_consumptions"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if exists != 1 {
                return Err(SourceAuthorityError::new(
                    "sqlite_schema_missing",
                    format!("{table} table is absent; run chaptera migrate up"),
                ));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl DocumentSourceAuthority for SqliteDocumentSourceAuthority {
    async fn resolve_document_source(
        &self,
        tenant_id: &str,
        document_id: &str,
    ) -> Result<AuthorizedDocumentSource, RevisionMaterializerError> {
        self.resolve(tenant_id, document_id)
            .await
            .map_err(|error| RevisionMaterializerError::new(error.code, error.message))
    }
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, SourceAuthorityError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        SourceAuthorityError::new(
            "document_source_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn optional_blob_text(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<String>, SourceAuthorityError> {
    let bytes: Option<Vec<u8>> = row.try_get(column).map_err(sqlite_error)?;
    bytes.map(|value| {
        String::from_utf8(value).map_err(|_| {
            SourceAuthorityError::new(
                "document_source_corrupt",
                format!("{column} is not UTF-8"),
            )
        })
    }).transpose()
}

fn require_ident(value: &str, label: &'static str) -> Result<(), SourceAuthorityError> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(SourceAuthorityError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_sha256(value: &str, label: &'static str) -> Result<(), SourceAuthorityError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SourceAuthorityError::new(
            "invalid_hash",
            format!("{label} must be 64 lowercase SHA-256 hex characters"),
        ));
    }
    Ok(())
}

fn sqlite_error(error: impl fmt::Display) -> SourceAuthorityError {
    SourceAuthorityError::new(
        "sqlite_document_source_error",
        error.to_string().chars().take(512).collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::atomic::{AtomicU64, Ordering}};

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-source-authority-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn consumed_upload_resolves_exact_document_source_and_genesis() {
        let path = temp_db("resolve");
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();

        let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        let hash = "a".repeat(64);
        sqlx::query(
            r#"
            INSERT INTO uploads (
                upload_id, tenant_id, principal_id, purpose, expected_byte_len,
                physical_upload_ref, state, upload_generation,
                object_version, object_etag, observed_byte_len,
                canonical_sha256, durable_binding_id,
                created_at_ms, expires_at_ms, completed_at_ms,
                idempotency_key, request_hash
            ) VALUES (?, ?, ?, 'PUB_SOURCE', 4, 'q/ref', 'CONSUMED', 2,
                      'v1', 'etag', 4, ?, ?, 1, 100, 2, ?, ?)
            "#,
        )
        .bind(b"upload-1".as_slice())
        .bind(b"tenant-a".as_slice())
        .bind(b"principal-a".as_slice())
        .bind(hash.as_bytes())
        .bind(b"binding-source-1".as_slice())
        .bind(b"upload-idem-1".as_slice())
        .bind(b"b".repeat(64))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO upload_consumptions (
                upload_id, tenant_id, idempotency_key, request_hash,
                project_id, document_id, genesis_revision_id, committed_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, 3)
            "#,
        )
        .bind(b"upload-1".as_slice())
        .bind(b"tenant-a".as_slice())
        .bind(b"consume-idem-1".as_slice())
        .bind(b"c".repeat(64))
        .bind(b"project-1".as_slice())
        .bind(b"document-1".as_slice())
        .bind(b"revision-genesis-1".as_slice())
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;

        let authority = SqliteDocumentSourceAuthority::open(
            &path,
            2,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let source = authority.resolve("tenant-a", "document-1").await.unwrap();
        assert_eq!(source.binding_id, "binding-source-1");
        assert_eq!(source.source_sha256, hash);
        assert_eq!(source.byte_len, 4);
        assert_eq!(source.baseline_revision_id, "revision-genesis-1");
        assert_eq!(source.baseline_cursor, 0);

        let denied = authority.resolve("tenant-b", "document-1").await.unwrap_err();
        assert_eq!(denied.code, "document_source_not_found");

        authority.close().await;
        cleanup(&path);
    }
}
