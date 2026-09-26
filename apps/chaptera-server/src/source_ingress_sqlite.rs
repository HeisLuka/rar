use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::{
    Row, SqlitePool,
    sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
    },
};

use crate::source_ingress::{
    ConsumptionReceipt, IngressError, ProjectCreateResult, UploadPurpose, UploadRecord, UploadState,
};

#[derive(Clone)]
pub struct SqliteSourceIngressRepository {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteSourceIngressRepository {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, IngressError> {
        if max_connections == 0 || max_connections > 16 {
            return Err(IngressError::new(
                "source_ingress_pool_size_invalid",
                "source ingress SQLite pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(IngressError::new(
                "source_ingress_busy_timeout_invalid",
                "source ingress SQLite busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(IngressError::new(
                "source_ingress_database_missing",
                "run chaptera migrate up before opening source ingress persistence",
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

        let repository = Self { path, pool };
        repository.require_schema().await?;
        Ok(repository)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn issue_idempotent(
        &self,
        candidate: UploadRecord,
    ) -> Result<UploadRecord, IngressError> {
        validate_initial_candidate(&candidate)?;
        let mut tx = self.pool.begin().await.map_err(sqlite_error)?;

        sqlx::query(
            r#"
            INSERT OR IGNORE INTO uploads (
                upload_id, tenant_id, principal_id, purpose, expected_byte_len,
                declared_content_type, physical_upload_ref, state, upload_generation,
                object_version, object_etag, observed_byte_len, canonical_sha256,
                durable_binding_id, created_at_ms, expires_at_ms, completed_at_ms,
                terminal_code, idempotency_key, request_hash
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(candidate.upload_id.as_bytes())
        .bind(candidate.tenant_id.as_bytes())
        .bind(candidate.principal_id.as_bytes())
        .bind(encode_purpose(candidate.purpose))
        .bind(to_i64(candidate.expected_byte_len, "expected_byte_len")?)
        .bind(candidate.declared_content_type.clone())
        .bind(&candidate.physical_upload_ref)
        .bind(encode_state(candidate.state))
        .bind(to_i64(candidate.upload_generation, "upload_generation")?)
        .bind(candidate.object_version.clone())
        .bind(candidate.object_etag.clone())
        .bind(optional_u64_i64(
            candidate.observed_byte_len,
            "observed_byte_len",
        )?)
        .bind(optional_blob(&candidate.canonical_sha256))
        .bind(optional_blob(&candidate.durable_binding_id))
        .bind(to_i64(candidate.created_at_ms, "created_at_ms")?)
        .bind(to_i64(candidate.expires_at_ms, "expires_at_ms")?)
        .bind(optional_u64_i64(
            candidate.completed_at_ms,
            "completed_at_ms",
        )?)
        .bind(candidate.terminal_code.clone())
        .bind(candidate.idempotency_key.as_bytes())
        .bind(candidate.request_hash.as_bytes())
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        let current =
            fetch_upload_by_idempotency(&mut tx, &candidate.tenant_id, &candidate.idempotency_key)
                .await?
                .ok_or_else(|| {
                    IngressError::new(
                        "upload_id_collision",
                        "upload insert did not establish the requested idempotency identity",
                    )
                })?;

        if current.request_hash != candidate.request_hash {
            return Err(IngressError::new(
                "idempotency_conflict",
                "issue key reused with different request",
            ));
        }

        tx.commit().await.map_err(sqlite_error)?;
        Ok(current)
    }

    pub async fn get(&self, upload_id: &str) -> Result<Option<UploadRecord>, IngressError> {
        require_ident(upload_id, "upload_id")?;
        let rows = sqlx::query(UPLOAD_BY_ID)
            .bind(upload_id.as_bytes())
            .fetch_all(&self.pool)
            .await
            .map_err(sqlite_error)?;
        decode_optional_unique(rows, "upload_id_ambiguous")
    }

    pub async fn find_issue_by_idempotency(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<UploadRecord>, IngressError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(idempotency_key, "idempotency_key")?;
        let query = format!(
            "SELECT {UPLOAD_COLUMNS} FROM uploads WHERE tenant_id = ? AND idempotency_key = ? LIMIT 2"
        );
        let rows = sqlx::query(&query)
            .bind(tenant_id.as_bytes())
            .bind(idempotency_key.as_bytes())
            .fetch_all(&self.pool)
            .await
            .map_err(sqlite_error)?;
        decode_optional_unique(rows, "upload_idempotency_ambiguous")
    }

    pub async fn compare_and_swap(
        &self,
        upload_id: &str,
        expected_generation: u64,
        next: UploadRecord,
    ) -> Result<UploadRecord, IngressError> {
        require_ident(upload_id, "upload_id")?;
        let expected_next_generation = expected_generation.checked_add(1).ok_or_else(|| {
            IngressError::new(
                "upload_generation_overflow",
                "upload generation cannot advance",
            )
        })?;
        if next.upload_id != upload_id || next.upload_generation != expected_next_generation {
            return Err(IngressError::new(
                "invalid_upload_transition",
                "next upload identity/generation does not match one-step CAS",
            ));
        }

        let mut tx = self.pool.begin().await.map_err(sqlite_error)?;
        let current = fetch_upload_by_id(&mut tx, upload_id)
            .await?
            .ok_or_else(|| IngressError::new("upload_not_found", "upload does not exist"))?;
        if current.upload_generation != expected_generation {
            return Err(IngressError::new(
                "stale_upload_generation",
                "compare-and-swap generation mismatch",
            ));
        }
        require_immutable_identity(&current, &next)?;

        let result = sqlx::query(
            r#"
            UPDATE uploads
            SET state = ?,
                upload_generation = ?,
                object_version = ?,
                object_etag = ?,
                observed_byte_len = ?,
                canonical_sha256 = ?,
                durable_binding_id = ?,
                completed_at_ms = ?,
                terminal_code = ?
            WHERE upload_id = ? AND upload_generation = ?
            "#,
        )
        .bind(encode_state(next.state))
        .bind(to_i64(next.upload_generation, "upload_generation")?)
        .bind(next.object_version.clone())
        .bind(next.object_etag.clone())
        .bind(optional_u64_i64(
            next.observed_byte_len,
            "observed_byte_len",
        )?)
        .bind(optional_blob(&next.canonical_sha256))
        .bind(optional_blob(&next.durable_binding_id))
        .bind(optional_u64_i64(next.completed_at_ms, "completed_at_ms")?)
        .bind(next.terminal_code.clone())
        .bind(upload_id.as_bytes())
        .bind(to_i64(expected_generation, "upload_generation")?)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        if result.rows_affected() != 1 {
            return Err(IngressError::new(
                "stale_upload_generation",
                "compare-and-swap generation changed concurrently",
            ));
        }
        tx.commit().await.map_err(sqlite_error)?;
        Ok(next)
    }

    pub async fn find_consumption(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<ConsumptionReceipt>, IngressError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(idempotency_key, "idempotency_key")?;
        let rows = sqlx::query(CONSUMPTION_BY_KEY)
            .bind(tenant_id.as_bytes())
            .bind(idempotency_key.as_bytes())
            .fetch_all(&self.pool)
            .await
            .map_err(sqlite_error)?;
        decode_optional_consumption(rows)
    }

    pub async fn commit_consumption(
        &self,
        upload_id: &str,
        expected_generation: u64,
        receipt: ConsumptionReceipt,
    ) -> Result<ConsumptionReceipt, IngressError> {
        require_ident(upload_id, "upload_id")?;
        if receipt.upload_id != upload_id {
            return Err(IngressError::new(
                "consumption_upload_mismatch",
                "consumption receipt names a different upload",
            ));
        }

        let mut tx = self.pool.begin().await.map_err(sqlite_error)?;
        if let Some(prior) =
            fetch_consumption_by_key(&mut tx, &receipt.tenant_id, &receipt.idempotency_key).await?
        {
            require_same_consumption(&prior, &receipt)?;
            tx.commit().await.map_err(sqlite_error)?;
            return Ok(prior);
        }

        let upload = fetch_upload_by_id(&mut tx, upload_id)
            .await?
            .ok_or_else(|| IngressError::new("upload_not_found", "upload does not exist"))?;
        if upload.tenant_id != receipt.tenant_id {
            return Err(IngressError::new(
                "tenant_mismatch",
                "consumption tenant differs from upload tenant",
            ));
        }
        if upload.upload_generation != expected_generation
            || upload.state != UploadState::ValidatedDurable
        {
            return Err(IngressError::new(
                "stale_upload_generation",
                "consumption compare-and-swap failed",
            ));
        }

        let insert = sqlx::query(
            r#"
            INSERT OR IGNORE INTO upload_consumptions (
                upload_id, tenant_id, idempotency_key, request_hash,
                project_id, document_id, genesis_revision_id, committed_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(receipt.upload_id.as_bytes())
        .bind(receipt.tenant_id.as_bytes())
        .bind(receipt.idempotency_key.as_bytes())
        .bind(receipt.request_hash.as_bytes())
        .bind(receipt.project.project_id.as_bytes())
        .bind(receipt.project.document_id.as_bytes())
        .bind(receipt.project.genesis_revision_id.as_bytes())
        .bind(to_i64(receipt.committed_at_ms, "committed_at_ms")?)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        if insert.rows_affected() == 0 {
            let prior =
                fetch_consumption_by_key(&mut tx, &receipt.tenant_id, &receipt.idempotency_key)
                    .await?
                    .ok_or_else(|| {
                        IngressError::new(
                            "consumption_identity_collision",
                            "consumption insert collided without matching idempotency row",
                        )
                    })?;
            require_same_consumption(&prior, &receipt)?;
            tx.commit().await.map_err(sqlite_error)?;
            return Ok(prior);
        }

        let next_generation = expected_generation.checked_add(1).ok_or_else(|| {
            IngressError::new(
                "upload_generation_overflow",
                "upload generation cannot advance",
            )
        })?;
        let update = sqlx::query(
            r#"
            UPDATE uploads
            SET state = 'CONSUMED', upload_generation = ?
            WHERE upload_id = ? AND upload_generation = ? AND state = 'VALIDATED_DURABLE'
            "#,
        )
        .bind(to_i64(next_generation, "upload_generation")?)
        .bind(upload_id.as_bytes())
        .bind(to_i64(expected_generation, "upload_generation")?)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        if update.rows_affected() != 1 {
            return Err(IngressError::new(
                "stale_upload_generation",
                "consumption compare-and-swap changed concurrently",
            ));
        }

        tx.commit().await.map_err(sqlite_error)?;
        Ok(receipt)
    }

    async fn require_schema(&self) -> Result<(), IngressError> {
        for table in ["uploads", "upload_consumptions"] {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if count != 1 {
                return Err(IngressError::new(
                    "source_ingress_schema_missing",
                    format!("{table} table is absent; run chaptera migrate up"),
                ));
            }
        }
        Ok(())
    }
}

const UPLOAD_COLUMNS: &str = r#"
    upload_id, tenant_id, principal_id, purpose, expected_byte_len,
    declared_content_type, physical_upload_ref, state, upload_generation,
    object_version, object_etag, observed_byte_len, canonical_sha256,
    durable_binding_id, created_at_ms, expires_at_ms, completed_at_ms,
    terminal_code, idempotency_key, request_hash
"#;

const UPLOAD_BY_ID: &str = r#"
    SELECT
        upload_id, tenant_id, principal_id, purpose, expected_byte_len,
        declared_content_type, physical_upload_ref, state, upload_generation,
        object_version, object_etag, observed_byte_len, canonical_sha256,
        durable_binding_id, created_at_ms, expires_at_ms, completed_at_ms,
        terminal_code, idempotency_key, request_hash
    FROM uploads
    WHERE upload_id = ?
    LIMIT 2
"#;

const CONSUMPTION_BY_KEY: &str = r#"
    SELECT
        upload_id, tenant_id, idempotency_key, request_hash,
        project_id, document_id, genesis_revision_id, committed_at_ms
    FROM upload_consumptions
    WHERE tenant_id = ? AND idempotency_key = ?
    LIMIT 2
"#;

async fn fetch_upload_by_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    upload_id: &str,
) -> Result<Option<UploadRecord>, IngressError> {
    let rows = sqlx::query(UPLOAD_BY_ID)
        .bind(upload_id.as_bytes())
        .fetch_all(&mut **tx)
        .await
        .map_err(sqlite_error)?;
    decode_optional_unique(rows, "upload_id_ambiguous")
}

async fn fetch_upload_by_idempotency(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<UploadRecord>, IngressError> {
    let query = format!(
        "SELECT {UPLOAD_COLUMNS} FROM uploads WHERE tenant_id = ? AND idempotency_key = ? LIMIT 2"
    );
    let rows = sqlx::query(&query)
        .bind(tenant_id.as_bytes())
        .bind(idempotency_key.as_bytes())
        .fetch_all(&mut **tx)
        .await
        .map_err(sqlite_error)?;
    decode_optional_unique(rows, "upload_idempotency_ambiguous")
}

async fn fetch_consumption_by_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<ConsumptionReceipt>, IngressError> {
    let rows = sqlx::query(CONSUMPTION_BY_KEY)
        .bind(tenant_id.as_bytes())
        .bind(idempotency_key.as_bytes())
        .fetch_all(&mut **tx)
        .await
        .map_err(sqlite_error)?;
    decode_optional_consumption(rows)
}

fn decode_optional_unique(
    rows: Vec<SqliteRow>,
    ambiguity_code: &'static str,
) -> Result<Option<UploadRecord>, IngressError> {
    match rows.as_slice() {
        [] => Ok(None),
        [row] => decode_upload(row).map(Some),
        _ => Err(IngressError::new(
            ambiguity_code,
            "source ingress persistence returned duplicate identity rows",
        )),
    }
}

fn decode_optional_consumption(
    rows: Vec<SqliteRow>,
) -> Result<Option<ConsumptionReceipt>, IngressError> {
    match rows.as_slice() {
        [] => Ok(None),
        [row] => decode_consumption(row).map(Some),
        _ => Err(IngressError::new(
            "consumption_idempotency_ambiguous",
            "source ingress persistence returned duplicate consumption rows",
        )),
    }
}

fn decode_upload(row: &SqliteRow) -> Result<UploadRecord, IngressError> {
    let purpose: String = row.try_get("purpose").map_err(sqlite_error)?;
    let state: String = row.try_get("state").map_err(sqlite_error)?;
    Ok(UploadRecord {
        upload_id: blob_text(row, "upload_id")?,
        tenant_id: blob_text(row, "tenant_id")?,
        principal_id: blob_text(row, "principal_id")?,
        purpose: decode_purpose(&purpose)?,
        expected_byte_len: positive_u64(row, "expected_byte_len")?,
        declared_content_type: row.try_get("declared_content_type").map_err(sqlite_error)?,
        physical_upload_ref: row.try_get("physical_upload_ref").map_err(sqlite_error)?,
        state: decode_state(&state)?,
        upload_generation: nonnegative_u64(row, "upload_generation")?,
        object_version: row.try_get("object_version").map_err(sqlite_error)?,
        object_etag: row.try_get("object_etag").map_err(sqlite_error)?,
        observed_byte_len: optional_nonnegative_u64(row, "observed_byte_len")?,
        canonical_sha256: optional_blob_text(row, "canonical_sha256")?,
        durable_binding_id: optional_blob_text(row, "durable_binding_id")?,
        created_at_ms: nonnegative_u64(row, "created_at_ms")?,
        expires_at_ms: nonnegative_u64(row, "expires_at_ms")?,
        completed_at_ms: optional_nonnegative_u64(row, "completed_at_ms")?,
        terminal_code: row.try_get("terminal_code").map_err(sqlite_error)?,
        idempotency_key: blob_text(row, "idempotency_key")?,
        request_hash: blob_text(row, "request_hash")?,
    })
}

fn decode_consumption(row: &SqliteRow) -> Result<ConsumptionReceipt, IngressError> {
    Ok(ConsumptionReceipt {
        upload_id: blob_text(row, "upload_id")?,
        tenant_id: blob_text(row, "tenant_id")?,
        idempotency_key: blob_text(row, "idempotency_key")?,
        request_hash: blob_text(row, "request_hash")?,
        committed_at_ms: nonnegative_u64(row, "committed_at_ms")?,
        project: ProjectCreateResult {
            project_id: blob_text(row, "project_id")?,
            document_id: blob_text(row, "document_id")?,
            genesis_revision_id: blob_text(row, "genesis_revision_id")?,
        },
    })
}

fn validate_initial_candidate(candidate: &UploadRecord) -> Result<(), IngressError> {
    require_ident(&candidate.upload_id, "upload_id")?;
    require_ident(&candidate.tenant_id, "tenant_id")?;
    require_ident(&candidate.principal_id, "principal_id")?;
    require_ident(&candidate.idempotency_key, "idempotency_key")?;
    if candidate.state != UploadState::Issued
        || candidate.upload_generation != 0
        || candidate.object_version.is_some()
        || candidate.object_etag.is_some()
        || candidate.observed_byte_len.is_some()
        || candidate.canonical_sha256.is_some()
        || candidate.durable_binding_id.is_some()
        || candidate.completed_at_ms.is_some()
        || candidate.terminal_code.is_some()
    {
        return Err(IngressError::new(
            "invalid_initial_upload",
            "new upload persistence accepts only a generation-0 ISSUED record",
        ));
    }
    if candidate.expected_byte_len == 0 || candidate.expires_at_ms <= candidate.created_at_ms {
        return Err(IngressError::new(
            "invalid_initial_upload",
            "new upload length/expiry is invalid",
        ));
    }
    Ok(())
}

fn require_immutable_identity(
    current: &UploadRecord,
    next: &UploadRecord,
) -> Result<(), IngressError> {
    if current.upload_id != next.upload_id
        || current.tenant_id != next.tenant_id
        || current.principal_id != next.principal_id
        || current.purpose != next.purpose
        || current.expected_byte_len != next.expected_byte_len
        || current.declared_content_type != next.declared_content_type
        || current.physical_upload_ref != next.physical_upload_ref
        || current.created_at_ms != next.created_at_ms
        || current.expires_at_ms != next.expires_at_ms
        || current.idempotency_key != next.idempotency_key
        || current.request_hash != next.request_hash
    {
        return Err(IngressError::new(
            "upload_immutable_identity_changed",
            "upload transition attempted to rewrite immutable identity",
        ));
    }
    Ok(())
}

fn require_same_consumption(
    prior: &ConsumptionReceipt,
    requested: &ConsumptionReceipt,
) -> Result<(), IngressError> {
    if prior.upload_id != requested.upload_id || prior.request_hash != requested.request_hash {
        return Err(IngressError::new(
            "idempotency_conflict",
            "consumption key reused with different request",
        ));
    }
    Ok(())
}

fn encode_purpose(value: UploadPurpose) -> &'static str {
    match value {
        UploadPurpose::PubSource => "pub_source",
    }
}

fn decode_purpose(value: &str) -> Result<UploadPurpose, IngressError> {
    match value {
        "pub_source" => Ok(UploadPurpose::PubSource),
        _ => Err(IngressError::new(
            "source_ingress_row_corrupt",
            "unknown persisted upload purpose",
        )),
    }
}

fn encode_state(value: UploadState) -> &'static str {
    match value {
        UploadState::Issued => "ISSUED",
        UploadState::StoredUnverified => "STORED_UNVERIFIED",
        UploadState::Validating => "VALIDATING",
        UploadState::ValidatedDurable => "VALIDATED_DURABLE",
        UploadState::Consumed => "CONSUMED",
        UploadState::Rejected => "REJECTED",
        UploadState::Expired => "EXPIRED",
    }
}

fn decode_state(value: &str) -> Result<UploadState, IngressError> {
    match value {
        "ISSUED" => Ok(UploadState::Issued),
        "STORED_UNVERIFIED" => Ok(UploadState::StoredUnverified),
        "VALIDATING" => Ok(UploadState::Validating),
        "VALIDATED_DURABLE" => Ok(UploadState::ValidatedDurable),
        "CONSUMED" => Ok(UploadState::Consumed),
        "REJECTED" => Ok(UploadState::Rejected),
        "EXPIRED" => Ok(UploadState::Expired),
        _ => Err(IngressError::new(
            "source_ingress_row_corrupt",
            "unknown persisted upload state",
        )),
    }
}

fn to_i64(value: u64, label: &'static str) -> Result<i64, IngressError> {
    i64::try_from(value).map_err(|_| {
        IngressError::new(
            "source_ingress_integer_overflow",
            format!("{label} exceeds SQLite signed integer range"),
        )
    })
}

fn optional_u64_i64(value: Option<u64>, label: &'static str) -> Result<Option<i64>, IngressError> {
    value.map(|value| to_i64(value, label)).transpose()
}

fn positive_u64(row: &SqliteRow, column: &str) -> Result<u64, IngressError> {
    let value: i64 = row.try_get(column).map_err(sqlite_error)?;
    if value <= 0 {
        return Err(IngressError::new(
            "source_ingress_row_corrupt",
            format!("{column} must be positive"),
        ));
    }
    Ok(value as u64)
}

fn nonnegative_u64(row: &SqliteRow, column: &str) -> Result<u64, IngressError> {
    let value: i64 = row.try_get(column).map_err(sqlite_error)?;
    u64::try_from(value).map_err(|_| {
        IngressError::new(
            "source_ingress_row_corrupt",
            format!("{column} must be non-negative"),
        )
    })
}

fn optional_nonnegative_u64(row: &SqliteRow, column: &str) -> Result<Option<u64>, IngressError> {
    let value: Option<i64> = row.try_get(column).map_err(sqlite_error)?;
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                IngressError::new(
                    "source_ingress_row_corrupt",
                    format!("{column} must be non-negative"),
                )
            })
        })
        .transpose()
}

fn blob_text(row: &SqliteRow, column: &str) -> Result<String, IngressError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        IngressError::new(
            "source_ingress_row_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn optional_blob_text(row: &SqliteRow, column: &str) -> Result<Option<String>, IngressError> {
    let bytes: Option<Vec<u8>> = row.try_get(column).map_err(sqlite_error)?;
    bytes
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| {
                IngressError::new(
                    "source_ingress_row_corrupt",
                    format!("{column} is not UTF-8"),
                )
            })
        })
        .transpose()
}

fn optional_blob(value: &Option<String>) -> Option<Vec<u8>> {
    value.as_ref().map(|value| value.as_bytes().to_vec())
}

fn require_ident(value: &str, label: &'static str) -> Result<(), IngressError> {
    if value.is_empty()
        || value.len() > 200
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

fn sqlite_error(error: impl fmt::Display) -> IngressError {
    IngressError::new(
        "sqlite_source_ingress_error",
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
            "chaptera-source-ingress-sqlite-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn repository(label: &str) -> (PathBuf, SqliteSourceIngressRepository) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let repository = SqliteSourceIngressRepository::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (path, repository)
    }

    fn candidate(upload_id: &str, request_hash_byte: char) -> UploadRecord {
        UploadRecord {
            upload_id: upload_id.into(),
            tenant_id: "tenant-a".into(),
            principal_id: "principal-a".into(),
            purpose: UploadPurpose::PubSource,
            expected_byte_len: 4,
            declared_content_type: Some("application/x-mspublisher".into()),
            physical_upload_ref: format!("quarantine/tenant-a/{upload_id}"),
            state: UploadState::Issued,
            upload_generation: 0,
            object_version: None,
            object_etag: None,
            observed_byte_len: None,
            canonical_sha256: None,
            durable_binding_id: None,
            created_at_ms: 100,
            expires_at_ms: 10_000,
            completed_at_ms: None,
            terminal_code: None,
            idempotency_key: "issue-1".into(),
            request_hash: request_hash_byte.to_string().repeat(64),
        }
    }

    #[tokio::test]
    async fn issue_is_durable_idempotent_and_restart_safe() {
        let (path, repository) = repository("issue").await;
        let first = repository
            .issue_idempotent(candidate("upload-1", 'a'))
            .await
            .unwrap();
        let replay = repository
            .issue_idempotent(candidate("upload-2", 'a'))
            .await
            .unwrap();
        assert_eq!(replay.upload_id, first.upload_id);

        let error = repository
            .issue_idempotent(candidate("upload-3", 'b'))
            .await
            .unwrap_err();
        assert_eq!(error.code, "idempotency_conflict");

        repository.close().await;
        let reopened = SqliteSourceIngressRepository::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(reopened.get("upload-1").await.unwrap().unwrap(), first);
        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn compare_and_swap_preserves_immutable_identity() {
        let (path, repository) = repository("cas").await;
        let issued = repository
            .issue_idempotent(candidate("upload-1", 'a'))
            .await
            .unwrap();

        let mut next = issued.clone();
        next.state = UploadState::StoredUnverified;
        next.upload_generation = 1;
        next.object_version = Some("generation-1".into());
        next.object_etag = Some("etag-1".into());
        next.observed_byte_len = Some(4);
        next.completed_at_ms = Some(200);
        repository
            .compare_and_swap("upload-1", 0, next.clone())
            .await
            .unwrap();

        let error = repository
            .compare_and_swap("upload-1", 0, next.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code, "stale_upload_generation");

        let mut rewritten = next.clone();
        rewritten.upload_generation = 2;
        rewritten.tenant_id = "tenant-b".into();
        let error = repository
            .compare_and_swap("upload-1", 1, rewritten)
            .await
            .unwrap_err();
        assert_eq!(error.code, "upload_immutable_identity_changed");

        repository.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn consumption_is_atomic_durable_and_retry_converges() {
        let (path, repository) = repository("consume").await;
        let issued = repository
            .issue_idempotent(candidate("upload-1", 'a'))
            .await
            .unwrap();

        let mut validated = issued;
        validated.state = UploadState::ValidatedDurable;
        validated.upload_generation = 1;
        validated.object_version = Some("generation-1".into());
        validated.object_etag = Some("etag-1".into());
        validated.observed_byte_len = Some(4);
        validated.canonical_sha256 = Some("c".repeat(64));
        validated.durable_binding_id = Some("binding-1".into());
        validated.completed_at_ms = Some(300);
        repository
            .compare_and_swap("upload-1", 0, validated.clone())
            .await
            .unwrap();

        let receipt = ConsumptionReceipt {
            upload_id: "upload-1".into(),
            tenant_id: "tenant-a".into(),
            idempotency_key: "create-project-1".into(),
            request_hash: "d".repeat(64),
            committed_at_ms: 500,
            project: ProjectCreateResult {
                project_id: "project-1".into(),
                document_id: "document-1".into(),
                genesis_revision_id: "revision-1".into(),
            },
        };
        let committed = repository
            .commit_consumption("upload-1", 1, receipt.clone())
            .await
            .unwrap();
        assert_eq!(committed, receipt);
        assert_eq!(
            repository.get("upload-1").await.unwrap().unwrap().state,
            UploadState::Consumed
        );

        let mut retry = receipt.clone();
        retry.committed_at_ms = 900;
        assert_eq!(
            repository
                .commit_consumption("upload-1", 1, retry)
                .await
                .unwrap()
                .committed_at_ms,
            500
        );
        assert_eq!(
            repository
                .find_consumption("tenant-a", "create-project-1")
                .await
                .unwrap()
                .unwrap()
                .committed_at_ms,
            500
        );

        repository.close().await;
        let reopened = SqliteSourceIngressRepository::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .find_consumption("tenant-a", "create-project-1")
                .await
                .unwrap()
                .unwrap()
                .project
                .document_id,
            "document-1"
        );
        reopened.close().await;
        cleanup(&path);
    }
}
