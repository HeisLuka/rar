use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{
    source_ingress::{ConsumeUploadRequest, IngressError, ProjectCreateResult, UploadState},
    sqlite_store::AUTHORING_REVISION_SCHEMA_V1,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedProjectIdentity {
    pub project_id: String,
    pub document_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBaselineIdentity {
    pub service_revision_id: String,
    pub canonical_schema_version: String,
    pub canonical_authoring_revision_id: String,
}

pub fn plan_project_identity(
    request: &ConsumeUploadRequest,
) -> Result<PlannedProjectIdentity, IngressError> {
    validate_request(request)?;
    Ok(PlannedProjectIdentity {
        project_id: stable_id("project", &request.tenant_id, &request.client_idempotency_id)?,
        document_id: stable_id("document", &request.tenant_id, &request.client_idempotency_id)?,
    })
}

#[derive(Clone)]
pub struct SqliteProjectPersistence {
    path: PathBuf,
    pool: SqlitePool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitFailpoint {
    None,
    BeforeCommit,
}

impl SqliteProjectPersistence {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, IngressError> {
        if max_connections == 0 || max_connections > 16 {
            return Err(IngressError::new(
                "project_persistence_pool_size_invalid",
                "project persistence SQLite pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(IngressError::new(
                "project_persistence_busy_timeout_invalid",
                "project persistence SQLite busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(IngressError::new(
                "project_persistence_database_missing",
                "run chaptera migrate up before opening project persistence",
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

        let adapter = Self { path, pool };
        adapter.require_schema().await?;
        Ok(adapter)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn create_project_from_upload(
        &self,
        request: ConsumeUploadRequest,
        baseline: ProjectBaselineIdentity,
    ) -> Result<ProjectCreateResult, IngressError> {
        self.create_project_from_upload_inner(request, baseline, CommitFailpoint::None)
            .await
    }

    async fn create_project_from_upload_inner(
        &self,
        request: ConsumeUploadRequest,
        baseline: ProjectBaselineIdentity,
        failpoint: CommitFailpoint,
    ) -> Result<ProjectCreateResult, IngressError> {
        validate_request(&request)?;
        validate_baseline_identity(&baseline)?;
        let planned = plan_project_identity(&request)?;
        let request_hash = consumption_request_hash(&request)?;

        let mut tx = self.pool.begin().await.map_err(sqlite_error)?;

        if let Some(prior) =
            fetch_consumption_by_key(&mut tx, &request.tenant_id, &request.client_idempotency_id)
                .await?
        {
            if prior.upload_id != request.upload_id || prior.request_hash != request_hash {
                return Err(IngressError::new(
                    "idempotency_conflict",
                    "project creation idempotency key was reused with different input",
                ));
            }
            verify_lifecycle_rows(
                &mut tx,
                &request.tenant_id,
                &prior.project,
                &baseline,
            )
            .await?;
            tx.commit().await.map_err(sqlite_error)?;
            return Ok(prior.project);
        }

        let upload = sqlx::query(
            r#"
            SELECT tenant_id, state, upload_generation, canonical_sha256, durable_binding_id
            FROM uploads
            WHERE upload_id = ?
            LIMIT 2
            "#,
        )
        .bind(request.upload_id.as_bytes())
        .fetch_all(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        if upload.is_empty() {
            return Err(IngressError::new(
                "upload_not_found",
                "upload does not exist",
            ));
        }
        if upload.len() != 1 {
            return Err(IngressError::new(
                "upload_id_ambiguous",
                "upload identity resolved to multiple rows",
            ));
        }
        let upload = &upload[0];
        let tenant_id = blob_text(upload, "tenant_id")?;
        if tenant_id != request.tenant_id {
            return Err(IngressError::new(
                "tenant_mismatch",
                "validated source is outside authenticated tenant",
            ));
        }

        let state: String = upload.try_get("state").map_err(sqlite_error)?;
        if state != "VALIDATED_DURABLE" {
            return Err(IngressError::new(
                "source_not_validated_durable",
                "project creation may consume only VALIDATED_DURABLE source",
            ));
        }

        let generation: i64 = upload.try_get("upload_generation").map_err(sqlite_error)?;
        let generation = u64::try_from(generation).map_err(|_| {
            IngressError::new(
                "project_persistence_row_corrupt",
                "upload generation is negative",
            )
        })?;
        if generation != request.expected_upload_generation {
            return Err(IngressError::new(
                "stale_upload_generation",
                "project creation upload generation is stale",
            ));
        }

        let source_sha256 = optional_blob_text(upload, "canonical_sha256")?.ok_or_else(|| {
            IngressError::new(
                "validated_source_missing",
                "VALIDATED_DURABLE upload is missing canonical source hash",
            )
        })?;
        require_sha256(&source_sha256)?;
        let durable_binding_id =
            optional_blob_text(upload, "durable_binding_id")?.ok_or_else(|| {
                IngressError::new(
                    "validated_source_missing",
                    "VALIDATED_DURABLE upload is missing durable binding id",
                )
            })?;
        require_ident(&durable_binding_id, "durable_binding_id")?;

        let project = ProjectCreateResult {
            project_id: planned.project_id,
            document_id: planned.document_id,
            genesis_revision_id: baseline.service_revision_id.clone(),
        };

        sqlx::query(
            r#"
            INSERT INTO projects (
                project_id, tenant_id, workspace_id, name,
                lifecycle_state, lifecycle_generation, metadata_version, deleted,
                created_at_ms
            ) VALUES (?, ?, ?, ?, 'active', 0, 0, 0, ?)
            "#,
        )
        .bind(project.project_id.as_bytes())
        .bind(request.tenant_id.as_bytes())
        .bind(request.workspace_id.as_bytes())
        .bind(&request.name)
        .bind(to_i64(request.now_ms, "now_ms")?)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        sqlx::query(
            r#"
            INSERT INTO documents (
                document_id, tenant_id, project_id, source_upload_id,
                durable_binding_id, source_sha256, genesis_revision_id, created_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(project.document_id.as_bytes())
        .bind(request.tenant_id.as_bytes())
        .bind(project.project_id.as_bytes())
        .bind(request.upload_id.as_bytes())
        .bind(durable_binding_id.as_bytes())
        .bind(source_sha256.as_bytes())
        .bind(project.genesis_revision_id.as_bytes())
        .bind(to_i64(request.now_ms, "now_ms")?)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        sqlx::query(
            r#"
            INSERT INTO revision_identity_bindings (
                document_id, service_revision_id, canonical_schema_version,
                canonical_revision_id, bound_at_ms
            ) VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(project.document_id.as_bytes())
        .bind(baseline.service_revision_id.as_bytes())
        .bind(&baseline.canonical_schema_version)
        .bind(&baseline.canonical_authoring_revision_id)
        .bind(to_i64(request.now_ms, "now_ms")?)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            if is_unique_violation(&error) {
                IngressError::new(
                    "revision_identity_conflict",
                    "baseline revision identity is already bound differently",
                )
            } else {
                sqlite_error(error)
            }
        })?;

        sqlx::query(
            r#"
            INSERT INTO upload_consumptions (
                upload_id, tenant_id, idempotency_key, request_hash,
                project_id, document_id, genesis_revision_id, committed_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(request.upload_id.as_bytes())
        .bind(request.tenant_id.as_bytes())
        .bind(request.client_idempotency_id.as_bytes())
        .bind(request_hash.as_bytes())
        .bind(project.project_id.as_bytes())
        .bind(project.document_id.as_bytes())
        .bind(project.genesis_revision_id.as_bytes())
        .bind(to_i64(request.now_ms, "now_ms")?)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            if is_unique_violation(&error) {
                IngressError::new(
                    "upload_already_consumed",
                    "validated upload was consumed by another project creation request",
                )
            } else {
                sqlite_error(error)
            }
        })?;

        let next_generation = request
            .expected_upload_generation
            .checked_add(1)
            .ok_or_else(|| {
                IngressError::new(
                    "upload_generation_overflow",
                    "upload generation cannot advance",
                )
            })?;

        let updated = sqlx::query(
            r#"
            UPDATE uploads
            SET state = 'CONSUMED', upload_generation = ?
            WHERE upload_id = ?
              AND tenant_id = ?
              AND upload_generation = ?
              AND state = 'VALIDATED_DURABLE'
              AND canonical_sha256 = ?
              AND durable_binding_id = ?
            "#,
        )
        .bind(to_i64(next_generation, "upload_generation")?)
        .bind(request.upload_id.as_bytes())
        .bind(request.tenant_id.as_bytes())
        .bind(to_i64(
            request.expected_upload_generation,
            "upload_generation",
        )?)
        .bind(source_sha256.as_bytes())
        .bind(durable_binding_id.as_bytes())
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        if updated.rows_affected() != 1 {
            return Err(IngressError::new(
                "stale_upload_generation",
                "validated upload changed before atomic project commit",
            ));
        }

        if failpoint == CommitFailpoint::BeforeCommit {
            tx.rollback().await.map_err(sqlite_error)?;
            return Err(IngressError::new(
                "injected_before_commit",
                "test failure injected before project transaction commit",
            ));
        }

        tx.commit().await.map_err(sqlite_error)?;
        Ok(project)
    }

    async fn require_schema(&self) -> Result<(), IngressError> {
        for table in [
            "uploads",
            "upload_consumptions",
            "projects",
            "documents",
            "revision_identity_bindings",
        ] {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if count != 1 {
                return Err(IngressError::new(
                    "project_persistence_schema_missing",
                    format!("{table} table is absent; run chaptera migrate up"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct PersistedConsumption {
    upload_id: String,
    request_hash: String,
    project: ProjectCreateResult,
}

async fn fetch_consumption_by_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<PersistedConsumption>, IngressError> {
    let rows = sqlx::query(
        r#"
        SELECT upload_id, request_hash, project_id, document_id, genesis_revision_id
        FROM upload_consumptions
        WHERE tenant_id = ? AND idempotency_key = ?
        LIMIT 2
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(idempotency_key.as_bytes())
    .fetch_all(&mut **tx)
    .await
    .map_err(sqlite_error)?;

    if rows.len() > 1 {
        return Err(IngressError::new(
            "project_persistence_row_ambiguous",
            "project idempotency identity resolved to multiple rows",
        ));
    }

    rows.first()
        .map(|row| {
            Ok(PersistedConsumption {
                upload_id: blob_text(row, "upload_id")?,
                request_hash: blob_text(row, "request_hash")?,
                project: ProjectCreateResult {
                    project_id: blob_text(row, "project_id")?,
                    document_id: blob_text(row, "document_id")?,
                    genesis_revision_id: blob_text(row, "genesis_revision_id")?,
                },
            })
        })
        .transpose()
}

async fn verify_lifecycle_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    tenant_id: &str,
    project: &ProjectCreateResult,
    baseline: &ProjectBaselineIdentity,
) -> Result<(), IngressError> {
    let project_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE project_id = ? AND tenant_id = ?")
            .bind(project.project_id.as_bytes())
            .bind(tenant_id.as_bytes())
            .fetch_one(&mut **tx)
            .await
            .map_err(sqlite_error)?;

    let document_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM documents
        WHERE document_id = ?
          AND tenant_id = ?
          AND project_id = ?
          AND genesis_revision_id = ?
        "#,
    )
    .bind(project.document_id.as_bytes())
    .bind(tenant_id.as_bytes())
    .bind(project.project_id.as_bytes())
    .bind(project.genesis_revision_id.as_bytes())
    .fetch_one(&mut **tx)
    .await
    .map_err(sqlite_error)?;

    let identity_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM revision_identity_bindings
        WHERE document_id = ?
          AND service_revision_id = ?
          AND canonical_schema_version = ?
          AND canonical_revision_id = ?
        "#,
    )
    .bind(project.document_id.as_bytes())
    .bind(project.genesis_revision_id.as_bytes())
    .bind(&baseline.canonical_schema_version)
    .bind(&baseline.canonical_authoring_revision_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(sqlite_error)?;

    if project_count != 1 || document_count != 1 {
        return Err(IngressError::new(
            "project_persistence_corrupt",
            "persisted consumption is missing exact project/document lifecycle rows",
        ));
    }
    if identity_count != 1 {
        return Err(IngressError::new(
            "idempotency_conflict",
            "project creation key was replayed with a different baseline revision identity",
        ));
    }
    Ok(())
}

fn validate_baseline_identity(baseline: &ProjectBaselineIdentity) -> Result<(), IngressError> {
    require_ident(&baseline.service_revision_id, "service_revision_id")?;
    if baseline.canonical_schema_version != AUTHORING_REVISION_SCHEMA_V1 {
        return Err(IngressError::new(
            "canonical_revision_schema_mismatch",
            "baseline canonical revision schema is not REVISION-MODEL-01 V1",
        ));
    }
    require_sha256(&baseline.canonical_authoring_revision_id).map_err(|_| {
        IngressError::new(
            "canonical_revision_id_invalid",
            "canonical AuthoringRevisionId must be 64 lowercase SHA-256 hex characters",
        )
    })?;
    Ok(())
}

fn validate_request(request: &ConsumeUploadRequest) -> Result<(), IngressError> {
    require_ident(&request.tenant_id, "tenant_id")?;
    require_ident(&request.upload_id, "upload_id")?;
    require_ident(&request.client_idempotency_id, "client_idempotency_id")?;
    require_ident(&request.workspace_id, "workspace_id")?;
    if request.name.is_empty()
        || request.name.len() > 512
        || request.name.chars().any(char::is_control)
    {
        return Err(IngressError::new(
            "invalid_project_name",
            "project name must be bounded, non-empty and free of control characters",
        ));
    }
    Ok(())
}

fn consumption_request_hash(request: &ConsumeUploadRequest) -> Result<String, IngressError> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        protocol: &'static str,
        tenant_id: &'a str,
        upload_id: &'a str,
        expected_upload_generation: u64,
        workspace_id: &'a str,
        name: &'a str,
    }

    let bytes = serde_json::to_vec(&Fingerprint {
        protocol: "chaptera.project-from-upload.v1",
        tenant_id: &request.tenant_id,
        upload_id: &request.upload_id,
        expected_upload_generation: request.expected_upload_generation,
        workspace_id: &request.workspace_id,
        name: &request.name,
    })
    .map_err(|error| IngressError::new("request_hash_failed", error.to_string()))?;

    let digest = Sha256::digest(bytes);
    Ok(hex_lower(&digest))
}

fn stable_id(
    prefix: &str,
    tenant_id: &str,
    request_id: &str,
) -> Result<String, IngressError> {
    let bytes = serde_json::to_vec(&(tenant_id, request_id))
        .map_err(|error| IngressError::new("identity_generation_failed", error.to_string()))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{prefix}:{}", &hex_lower(&digest)[..24]))
}

fn hex_lower(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn require_ident(value: &str, label: &'static str) -> Result<(), IngressError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(IngressError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_sha256(value: &str) -> Result<(), IngressError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(IngressError::new(
            "invalid_hash",
            "canonical source hash must be 64 lowercase SHA-256 hex characters",
        ));
    }
    Ok(())
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, IngressError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        IngressError::new(
            "project_persistence_row_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn optional_blob_text(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<String>, IngressError> {
    let bytes: Option<Vec<u8>> = row.try_get(column).map_err(sqlite_error)?;
    bytes
        .map(|value| {
            String::from_utf8(value).map_err(|_| {
                IngressError::new(
                    "project_persistence_row_corrupt",
                    format!("{column} is not UTF-8"),
                )
            })
        })
        .transpose()
}

fn to_i64(value: u64, label: &'static str) -> Result<i64, IngressError> {
    i64::try_from(value).map_err(|_| {
        IngressError::new(
            "project_persistence_integer_overflow",
            format!("{label} exceeds SQLite signed integer range"),
        )
    })
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code == "2067" || code == "1555")
}

fn sqlite_error(error: impl fmt::Display) -> IngressError {
    IngressError::new(
        "sqlite_project_persistence_error",
        error.to_string().chars().take(512).collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use sqlx::SqlitePool;

    use crate::{
        schema_migration::SqliteMigrationRuntime, source_authority::SqliteDocumentSourceAuthority,
    };

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-project-persistence-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn setup(label: &str) -> (PathBuf, SqliteProjectPersistence, SqlitePool) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let adapter = SqliteProjectPersistence::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        (path, adapter, pool)
    }

    async fn seed_upload(
        pool: &SqlitePool,
        upload_id: &str,
        tenant_id: &str,
        state: UploadState,
        generation: u64,
        source_hash: &str,
        binding_id: &str,
    ) {
        let state = match state {
            UploadState::ValidatedDurable => "VALIDATED_DURABLE",
            UploadState::StoredUnverified => "STORED_UNVERIFIED",
            UploadState::Consumed => "CONSUMED",
            other => panic!("unsupported test state: {other:?}"),
        };
        sqlx::query(
            r#"
            INSERT INTO uploads (
                upload_id, tenant_id, principal_id, purpose, expected_byte_len,
                physical_upload_ref, state, upload_generation,
                object_version, object_etag, observed_byte_len,
                canonical_sha256, durable_binding_id,
                created_at_ms, expires_at_ms, completed_at_ms,
                idempotency_key, request_hash
            ) VALUES (?, ?, 'principal-a', 'pub_source', 4, ?, ?, ?,
                      'v1', 'etag', 4, ?, ?, 1, 10000, 2, ?, ?)
            "#,
        )
        .bind(upload_id.as_bytes())
        .bind(tenant_id.as_bytes())
        .bind(format!("quarantine/{tenant_id}/{upload_id}"))
        .bind(state)
        .bind(i64::try_from(generation).unwrap())
        .bind(source_hash.as_bytes())
        .bind(binding_id.as_bytes())
        .bind(format!("issue-{upload_id}").as_bytes())
        .bind("a".repeat(64).as_bytes())
        .execute(pool)
        .await
        .unwrap();
    }

    fn request(upload_id: &str, key: &str, generation: u64) -> ConsumeUploadRequest {
        ConsumeUploadRequest {
            tenant_id: "tenant-a".into(),
            upload_id: upload_id.into(),
            expected_upload_generation: generation,
            workspace_id: "workspace-a".into(),
            name: "Imported PUB".into(),
            client_idempotency_id: key.into(),
            now_ms: 500,
        }
    }

    fn baseline(upload_id: &str) -> ProjectBaselineIdentity {
        ProjectBaselineIdentity {
            service_revision_id: format!("service-baseline-{upload_id}"),
            canonical_schema_version: AUTHORING_REVISION_SCHEMA_V1.into(),
            canonical_authoring_revision_id: hex_lower(Sha256::digest(upload_id.as_bytes())),
        }
    }

    #[test]
    fn planned_project_identity_matches_closed_lifecycle_contract() {
        let planned = plan_project_identity(&request("upload-1", "create-1", 3)).unwrap();
        assert_eq!(planned.project_id, "project:c54c2429d0bf699b890aab84");
        assert_eq!(planned.document_id, "document:c54c2429d0bf699b890aab84");
    }

    #[test]
    fn consumption_hash_matches_source_ingress_contract() {
        assert_eq!(
            consumption_request_hash(&request("upload-1", "create-1", 3)).unwrap(),
            "2ad057fed8e6fc5bb8ec842a91ae88dd99fe04dff7619814d1d86ec9f56de6e9"
        );
    }

    #[tokio::test]
    async fn fresh_validated_upload_atomically_creates_project_document_genesis_and_consumption() {
        let (path, adapter, pool) = setup("fresh").await;
        seed_upload(
            &pool,
            "upload-1",
            "tenant-a",
            UploadState::ValidatedDurable,
            3,
            &"b".repeat(64),
            "binding-1",
        )
        .await;

        let created = adapter
            .create_project_from_upload(request("upload-1", "create-1", 3), baseline("upload-1"))
            .await
            .unwrap();

        assert_ne!(created.project_id, created.document_id);
        assert_ne!(created.document_id, created.genesis_revision_id);

        let project_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE project_id = ?")
                .bind(created.project_id.as_bytes())
                .fetch_one(&pool)
                .await
                .unwrap();
        let document_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE document_id = ?")
                .bind(created.document_id.as_bytes())
                .fetch_one(&pool)
                .await
                .unwrap();
        let consumption_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upload_consumptions WHERE upload_id = ?")
                .bind(b"upload-1".as_slice())
                .fetch_one(&pool)
                .await
                .unwrap();
        let state: String = sqlx::query_scalar("SELECT state FROM uploads WHERE upload_id = ?")
            .bind(b"upload-1".as_slice())
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(project_count, 1);
        assert_eq!(document_count, 1);

        let lifecycle: (String, i64, i64, i64) = sqlx::query_as(
            "SELECT lifecycle_state, lifecycle_generation, metadata_version, deleted FROM projects WHERE project_id = ?",
        )
        .bind(created.project_id.as_bytes())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(lifecycle, ("active".into(), 0, 0, 0));
        assert_eq!(consumption_count, 1);
        assert_eq!(state, "CONSUMED");

        let identity: (String, String) = sqlx::query_as(
            "SELECT canonical_schema_version, canonical_revision_id FROM revision_identity_bindings WHERE document_id = ? AND service_revision_id = ?",
        )
        .bind(created.document_id.as_bytes())
        .bind(created.genesis_revision_id.as_bytes())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(identity.0, AUTHORING_REVISION_SCHEMA_V1);
        assert_eq!(
            identity.1,
            baseline("upload-1").canonical_authoring_revision_id
        );

        adapter.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn exact_retry_survives_reopen_and_returns_original_identities() {
        let (path, adapter, pool) = setup("retry").await;
        seed_upload(
            &pool,
            "upload-1",
            "tenant-a",
            UploadState::ValidatedDurable,
            3,
            &"c".repeat(64),
            "binding-1",
        )
        .await;
        let req = request("upload-1", "create-1", 3);
        let identity = baseline("upload-1");
        let first = adapter
            .create_project_from_upload(req.clone(), identity.clone())
            .await
            .unwrap();
        adapter.close().await;

        let reopened = SqliteProjectPersistence::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let second = reopened
            .create_project_from_upload(req, identity)
            .await
            .unwrap();
        assert_eq!(second, first);

        reopened.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn changed_request_under_same_idempotency_key_fails_closed() {
        let (path, adapter, pool) = setup("idem-conflict").await;
        seed_upload(
            &pool,
            "upload-1",
            "tenant-a",
            UploadState::ValidatedDurable,
            3,
            &"d".repeat(64),
            "binding-1",
        )
        .await;
        adapter
            .create_project_from_upload(request("upload-1", "create-1", 3), baseline("upload-1"))
            .await
            .unwrap();

        let mut changed = request("upload-1", "create-1", 3);
        changed.name = "Different name".into();
        let error = adapter
            .create_project_from_upload(changed, baseline("upload-1"))
            .await
            .unwrap_err();
        assert_eq!(error.code, "idempotency_conflict");

        let mut changed_baseline = baseline("upload-1");
        changed_baseline.canonical_authoring_revision_id = "f".repeat(64);
        let error = adapter
            .create_project_from_upload(
                request("upload-1", "create-1", 3),
                changed_baseline,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "idempotency_conflict");

        adapter.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn same_source_bytes_imported_twice_receive_distinct_document_and_genesis_identity() {
        let (path, adapter, pool) = setup("same-source").await;
        for upload_id in ["upload-1", "upload-2"] {
            seed_upload(
                &pool,
                upload_id,
                "tenant-a",
                UploadState::ValidatedDurable,
                3,
                &"e".repeat(64),
                "shared-binding",
            )
            .await;
        }

        let first = adapter
            .create_project_from_upload(request("upload-1", "create-1", 3), baseline("upload-1"))
            .await
            .unwrap();
        let second = adapter
            .create_project_from_upload(request("upload-2", "create-2", 3), baseline("upload-2"))
            .await
            .unwrap();

        assert_ne!(first.project_id, second.project_id);
        assert_ne!(first.document_id, second.document_id);
        assert_ne!(first.genesis_revision_id, second.genesis_revision_id);

        adapter.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn non_durable_wrong_tenant_and_stale_generation_create_nothing() {
        let (path, adapter, pool) = setup("reject").await;
        seed_upload(
            &pool,
            "upload-1",
            "tenant-a",
            UploadState::StoredUnverified,
            2,
            &"f".repeat(64),
            "binding-1",
        )
        .await;

        let error = adapter
            .create_project_from_upload(request("upload-1", "create-1", 2), baseline("upload-1"))
            .await
            .unwrap_err();
        assert_eq!(error.code, "source_not_validated_durable");

        sqlx::query("UPDATE uploads SET state='VALIDATED_DURABLE' WHERE upload_id=?")
            .bind(b"upload-1".as_slice())
            .execute(&pool)
            .await
            .unwrap();

        let mut wrong_tenant = request("upload-1", "create-2", 2);
        wrong_tenant.tenant_id = "tenant-b".into();
        assert_eq!(
            adapter
                .create_project_from_upload(wrong_tenant, baseline("upload-1"))
                .await
                .unwrap_err()
                .code,
            "tenant_mismatch"
        );

        assert_eq!(
            adapter
                .create_project_from_upload(request("upload-1", "create-3", 1), baseline("upload-1"))
                .await
                .unwrap_err()
                .code,
            "stale_upload_generation"
        );

        let projects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(projects, 0);

        adapter.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn injected_failure_before_commit_leaves_zero_visible_lifecycle_state() {
        let (path, adapter, pool) = setup("rollback").await;
        seed_upload(
            &pool,
            "upload-1",
            "tenant-a",
            UploadState::ValidatedDurable,
            3,
            &"1".repeat(64),
            "binding-1",
        )
        .await;

        let error = adapter
            .create_project_from_upload_inner(
                request("upload-1", "create-1", 3),
                baseline("upload-1"),
                CommitFailpoint::BeforeCommit,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "injected_before_commit");

        for table in [
            "projects",
            "documents",
            "upload_consumptions",
            "revision_identity_bindings",
        ] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, 0, "{table} must stay empty after rollback");
        }
        let state: String = sqlx::query_scalar("SELECT state FROM uploads WHERE upload_id=?")
            .bind(b"upload-1".as_slice())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(state, "VALIDATED_DURABLE");

        adapter.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn committed_unknown_outcome_reopens_to_exact_original_receipt() {
        let (path, adapter, pool) = setup("unknown-outcome").await;
        seed_upload(
            &pool,
            "upload-1",
            "tenant-a",
            UploadState::ValidatedDurable,
            3,
            &"2".repeat(64),
            "binding-1",
        )
        .await;
        let req = request("upload-1", "create-1", 3);
        let identity = baseline("upload-1");
        let committed = adapter
            .create_project_from_upload(req.clone(), identity.clone())
            .await
            .unwrap();
        adapter.close().await;

        let reopened = SqliteProjectPersistence::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .create_project_from_upload(req, identity)
                .await
                .unwrap(),
            committed
        );

        reopened.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn source_authority_resolves_exact_consumed_binding_hash_and_genesis() {
        let (path, adapter, pool) = setup("authority").await;
        let hash = "3".repeat(64);
        seed_upload(
            &pool,
            "upload-1",
            "tenant-a",
            UploadState::ValidatedDurable,
            3,
            &hash,
            "binding-1",
        )
        .await;
        let created = adapter
            .create_project_from_upload(request("upload-1", "create-1", 3), baseline("upload-1"))
            .await
            .unwrap();

        let authority = SqliteDocumentSourceAuthority::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        let source = authority
            .resolve("tenant-a", &created.document_id)
            .await
            .unwrap();

        assert_eq!(source.binding_id, "binding-1");
        assert_eq!(source.source_sha256, hash);
        assert_eq!(source.baseline_revision_id, created.genesis_revision_id);
        assert_eq!(source.baseline_cursor, 0);

        authority.close().await;
        adapter.close().await;
        pool.close().await;
        cleanup(&path);
    }
}
