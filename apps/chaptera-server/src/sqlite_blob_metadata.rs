use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::{
    Connection, Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::blob_store::{
    BindingLifecycle, BlobBindingRepository, BlobNamespace, BlobStoreError, PhysicalBlobRecord,
    ResourceBinding, ResourceKind,
};

#[derive(Clone)]
pub struct SqliteBlobBindingRepository {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteBlobBindingRepository {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, BlobStoreError> {
        if max_connections == 0 || max_connections > 16 {
            return Err(BlobStoreError::new(
                "invalid_pool_size",
                "blob metadata SQLite pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(BlobStoreError::new(
                "invalid_busy_timeout",
                "blob metadata busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(BlobStoreError::new(
                "sqlite_database_missing",
                "run chaptera migrate up before opening blob metadata",
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

        let repo = Self { path, pool };
        repo.require_schema().await?;
        repo.verify_profile().await?;
        Ok(repo)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    async fn require_schema(&self) -> Result<(), BlobStoreError> {
        for table in [
            "chaptera_schema_migrations",
            "physical_blobs",
            "resource_bindings",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if exists != 1 {
                return Err(BlobStoreError::new(
                    "sqlite_schema_missing",
                    format!(
                        "required blob metadata table {table} is absent; run chaptera migrate up"
                    ),
                ));
            }
        }

        let blob_migration: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM chaptera_schema_migrations WHERE version = 2 AND name = 'blob_store'")
                .fetch_one(&self.pool)
                .await
                .map_err(sqlite_error)?;
        if blob_migration != 1 {
            return Err(BlobStoreError::new(
                "sqlite_schema_missing",
                "blob metadata migration v2 is not recorded",
            ));
        }

        let gc_fence_migration: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chaptera_schema_migrations WHERE version = 12 AND name = 'blob_gc_delete_fence'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;
        if gc_fence_migration != 1 {
            return Err(BlobStoreError::new(
                "sqlite_schema_missing",
                "blob GC delete-fence migration v12 is not recorded",
            ));
        }
        Ok(())
    }

    async fn verify_profile(&self) -> Result<(), BlobStoreError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(BlobStoreError::new(
                "sqlite_profile_mismatch",
                format!("expected WAL journal mode, got {journal_mode}"),
            ));
        }

        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if synchronous != 2 {
            return Err(BlobStoreError::new(
                "sqlite_profile_mismatch",
                format!("expected synchronous=FULL(2), got {synchronous}"),
            ));
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if foreign_keys != 1 {
            return Err(BlobStoreError::new(
                "sqlite_profile_mismatch",
                "foreign_keys pragma is not enabled",
            ));
        }
        Ok(())
    }

    async fn read_physical_by_id(
        &self,
        physical_blob_id: &str,
    ) -> Result<Option<PhysicalBlobRecord>, BlobStoreError> {
        let row = sqlx::query(PHYSICAL_SELECT_BY_ID)
            .bind(physical_blob_id.as_bytes())
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlite_error)?;
        row.map(decode_physical).transpose()
    }

    async fn read_binding_by_id(
        &self,
        binding_id: &str,
    ) -> Result<Option<ResourceBinding>, BlobStoreError> {
        let row = sqlx::query(BINDING_SELECT_BY_ID)
            .bind(binding_id.as_bytes())
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlite_error)?;
        row.map(decode_binding).transpose()
    }
}

const PHYSICAL_COLUMNS: &str = r#"
    physical_blob_id,
    tenant_id,
    content_sha256,
    byte_len,
    canonical_mime,
    object_namespace,
    object_locator,
    storage_generation,
    created_at_ms,
    delete_eligible_at_ms,
    deleted
"#;

const PHYSICAL_SELECT_BY_ID: &str = r#"
SELECT
    physical_blob_id,
    tenant_id,
    content_sha256,
    byte_len,
    canonical_mime,
    object_namespace,
    object_locator,
    storage_generation,
    created_at_ms,
    delete_eligible_at_ms,
    deleted
FROM physical_blobs
WHERE physical_blob_id = ?
"#;

const BINDING_SELECT_BY_ID: &str = r#"
SELECT
    binding_id,
    tenant_id,
    project_id,
    document_id,
    physical_blob_id,
    content_sha256,
    byte_len,
    resource_kind,
    validation_profile,
    lifecycle_state,
    created_at_ms,
    retired_at_ms
FROM resource_bindings
WHERE binding_id = ?
"#;

#[async_trait::async_trait]
impl BlobBindingRepository for SqliteBlobBindingRepository {
    async fn find_physical_by_content(
        &self,
        tenant_id: &str,
        content_sha256: &str,
        byte_len: u64,
    ) -> Result<Option<PhysicalBlobRecord>, BlobStoreError> {
        let byte_len = to_i64(byte_len, "byte_len")?;
        let sql = format!(
            "SELECT {PHYSICAL_COLUMNS} FROM physical_blobs \
             WHERE tenant_id = ? AND content_sha256 = ? AND byte_len = ?                AND deleted = 0 AND gc_delete_fence IS NULL"
        );
        let row = sqlx::query(&sql)
            .bind(tenant_id.as_bytes())
            .bind(content_sha256.as_bytes())
            .bind(byte_len)
            .fetch_optional(&self.pool)
            .await
            .map_err(sqlite_error)?;
        row.map(decode_physical).transpose()
    }

    async fn get_physical(
        &self,
        physical_blob_id: &str,
    ) -> Result<Option<PhysicalBlobRecord>, BlobStoreError> {
        self.read_physical_by_id(physical_blob_id).await
    }

    async fn get_binding(
        &self,
        binding_id: &str,
    ) -> Result<Option<ResourceBinding>, BlobStoreError> {
        self.read_binding_by_id(binding_id).await
    }

    async fn commit_physical_and_binding(
        &self,
        physical: PhysicalBlobRecord,
        binding: ResourceBinding,
    ) -> Result<ResourceBinding, BlobStoreError> {
        require_binding_matches_physical(&binding, &physical)?;
        let mut connection = self.pool.acquire().await.map_err(sqlite_error)?;
        let mut tx = (*connection)
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlite_error)?;

        let existing_physical = fetch_physical_tx(&mut tx, &physical.physical_blob_id).await?;
        match existing_physical {
            Some(existing) if existing == physical => {}
            Some(_) => {
                return Err(BlobStoreError::new(
                    "physical_blob_collision",
                    "physical blob id already maps to different metadata",
                ));
            }
            None => {
                insert_physical_tx(&mut tx, &physical).await?;
            }
        }
        require_gc_fence_clear_tx(&mut tx, &physical.physical_blob_id).await?;

        let existing_binding = fetch_binding_tx(&mut tx, &binding.binding_id).await?;
        let outcome = match existing_binding {
            Some(existing) if existing == binding => existing,
            Some(_) => {
                return Err(BlobStoreError::new(
                    "binding_collision",
                    "binding id already maps to different metadata",
                ));
            }
            None => {
                insert_binding_tx(&mut tx, &binding).await?;
                binding.clone()
            }
        };

        tx.commit().await.map_err(sqlite_error)?;
        Ok(outcome)
    }

    async fn commit_binding(
        &self,
        binding: ResourceBinding,
    ) -> Result<ResourceBinding, BlobStoreError> {
        let mut connection = self.pool.acquire().await.map_err(sqlite_error)?;
        let mut tx = (*connection)
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlite_error)?;

        let physical = fetch_physical_tx(&mut tx, &binding.physical_blob_id)
            .await?
            .ok_or_else(|| {
                BlobStoreError::new(
                    "physical_blob_missing",
                    "binding references a missing physical blob",
                )
            })?;
        require_binding_matches_physical(&binding, &physical)?;
        require_gc_fence_clear_tx(&mut tx, &binding.physical_blob_id).await?;

        let outcome = match fetch_binding_tx(&mut tx, &binding.binding_id).await? {
            Some(existing) if existing == binding => existing,
            Some(_) => {
                return Err(BlobStoreError::new(
                    "binding_collision",
                    "binding id already maps to different metadata",
                ));
            }
            None => {
                insert_binding_tx(&mut tx, &binding).await?;
                binding.clone()
            }
        };

        tx.commit().await.map_err(sqlite_error)?;
        Ok(outcome)
    }

    async fn mark_physical_deleted(
        &self,
        physical_blob_id: &str,
        expected_generation: &str,
    ) -> Result<PhysicalBlobRecord, BlobStoreError> {
        let current = self
            .read_physical_by_id(physical_blob_id)
            .await?
            .ok_or_else(|| BlobStoreError::new("physical_blob_missing", "physical blob missing"))?;
        if current.storage_generation != expected_generation {
            return Err(BlobStoreError::new(
                "storage_generation_mismatch",
                "delete generation differs from durable metadata",
            ));
        }
        if current.deleted {
            return Ok(current);
        }

        let result = sqlx::query(
            r#"
            UPDATE physical_blobs
            SET deleted = 1
            WHERE physical_blob_id = ?
              AND storage_generation = ?
              AND deleted = 0
            "#,
        )
        .bind(physical_blob_id.as_bytes())
        .bind(expected_generation)
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if result.rows_affected() != 1 {
            return Err(BlobStoreError::new(
                "physical_delete_race",
                "physical metadata changed while applying generation-fenced delete",
            ));
        }

        self.read_physical_by_id(physical_blob_id)
            .await?
            .ok_or_else(|| {
                BlobStoreError::new(
                    "physical_blob_missing",
                    "physical blob disappeared after delete mark",
                )
            })
    }
}

async fn insert_physical_tx(
    tx: &mut Transaction<'_, Sqlite>,
    record: &PhysicalBlobRecord,
) -> Result<(), BlobStoreError> {
    let result = sqlx::query(
        r#"
        INSERT INTO physical_blobs (
            physical_blob_id,
            tenant_id,
            content_sha256,
            byte_len,
            canonical_mime,
            object_namespace,
            object_locator,
            storage_generation,
            created_at_ms,
            delete_eligible_at_ms,
            deleted
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(record.physical_blob_id.as_bytes())
    .bind(record.tenant_id.as_bytes())
    .bind(record.content_sha256.as_bytes())
    .bind(to_i64(record.byte_len, "byte_len")?)
    .bind(record.canonical_mime.as_deref())
    .bind(namespace_text(record.object_namespace))
    .bind(&record.object_locator)
    .bind(&record.storage_generation)
    .bind(to_i64(record.created_at_ms, "created_at_ms")?)
    .bind(
        record
            .delete_eligible_at_ms
            .map(|value| to_i64(value, "delete_eligible_at_ms"))
            .transpose()?,
    )
    .bind(if record.deleted { 1_i64 } else { 0_i64 })
    .execute(&mut **tx)
    .await;

    match result {
        Ok(done) if done.rows_affected() == 1 => Ok(()),
        Ok(_) => Err(BlobStoreError::new(
            "blob_metadata_insert_no_effect",
            "physical blob insert completed without one row",
        )),
        Err(error) if is_unique(&error) => Err(BlobStoreError::new(
            "physical_blob_content_conflict",
            "physical blob identity/content uniqueness conflict",
        )),
        Err(error) => Err(sqlite_error(error)),
    }
}

async fn insert_binding_tx(
    tx: &mut Transaction<'_, Sqlite>,
    record: &ResourceBinding,
) -> Result<(), BlobStoreError> {
    let result = sqlx::query(
        r#"
        INSERT INTO resource_bindings (
            binding_id,
            tenant_id,
            project_id,
            document_id,
            physical_blob_id,
            content_sha256,
            byte_len,
            resource_kind,
            validation_profile,
            lifecycle_state,
            created_at_ms,
            retired_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(record.binding_id.as_bytes())
    .bind(record.tenant_id.as_bytes())
    .bind(record.project_id.as_ref().map(|value| value.as_bytes()))
    .bind(record.document_id.as_ref().map(|value| value.as_bytes()))
    .bind(record.physical_blob_id.as_bytes())
    .bind(record.content_sha256.as_bytes())
    .bind(to_i64(record.byte_len, "byte_len")?)
    .bind(resource_kind_text(record.resource_kind))
    .bind(&record.validation_profile)
    .bind(lifecycle_text(record.lifecycle_state))
    .bind(to_i64(record.created_at_ms, "created_at_ms")?)
    .bind(
        record
            .retired_at_ms
            .map(|value| to_i64(value, "retired_at_ms"))
            .transpose()?,
    )
    .execute(&mut **tx)
    .await;

    match result {
        Ok(done) if done.rows_affected() == 1 => Ok(()),
        Ok(_) => Err(BlobStoreError::new(
            "blob_metadata_insert_no_effect",
            "resource binding insert completed without one row",
        )),
        Err(error) if is_unique(&error) => Err(BlobStoreError::new(
            "binding_collision",
            "resource binding identity uniqueness conflict",
        )),
        Err(error) => Err(sqlite_error(error)),
    }
}

async fn fetch_physical_tx(
    tx: &mut Transaction<'_, Sqlite>,
    physical_blob_id: &str,
) -> Result<Option<PhysicalBlobRecord>, BlobStoreError> {
    let row = sqlx::query(PHYSICAL_SELECT_BY_ID)
        .bind(physical_blob_id.as_bytes())
        .fetch_optional(&mut **tx)
        .await
        .map_err(sqlite_error)?;
    row.map(decode_physical).transpose()
}

async fn require_gc_fence_clear_tx(
    tx: &mut Transaction<'_, Sqlite>,
    physical_blob_id: &str,
) -> Result<(), BlobStoreError> {
    let row = sqlx::query(
        "SELECT gc_delete_fence FROM physical_blobs WHERE physical_blob_id = ?",
    )
    .bind(physical_blob_id.as_bytes())
    .fetch_optional(&mut **tx)
    .await
    .map_err(sqlite_error)?
    .ok_or_else(|| BlobStoreError::new("physical_blob_missing", "physical blob missing"))?;

    let fence: Option<Vec<u8>> = row.try_get("gc_delete_fence").map_err(sqlite_error)?;
    if fence.is_some() {
        return Err(BlobStoreError::new(
            "physical_blob_gc_fenced",
            "physical blob is fenced for GC and cannot accept new bindings",
        ));
    }
    Ok(())
}

async fn fetch_binding_tx(
    tx: &mut Transaction<'_, Sqlite>,
    binding_id: &str,
) -> Result<Option<ResourceBinding>, BlobStoreError> {
    let row = sqlx::query(BINDING_SELECT_BY_ID)
        .bind(binding_id.as_bytes())
        .fetch_optional(&mut **tx)
        .await
        .map_err(sqlite_error)?;
    row.map(decode_binding).transpose()
}

fn require_binding_matches_physical(
    binding: &ResourceBinding,
    physical: &PhysicalBlobRecord,
) -> Result<(), BlobStoreError> {
    if physical.deleted
        || binding.tenant_id != physical.tenant_id
        || binding.physical_blob_id != physical.physical_blob_id
        || binding.content_sha256 != physical.content_sha256
        || binding.byte_len != physical.byte_len
    {
        return Err(BlobStoreError::new(
            "binding_physical_mismatch",
            "resource binding does not match exact active physical metadata",
        ));
    }
    Ok(())
}

fn decode_physical(row: sqlx::sqlite::SqliteRow) -> Result<PhysicalBlobRecord, BlobStoreError> {
    Ok(PhysicalBlobRecord {
        physical_blob_id: blob_text(&row, "physical_blob_id")?,
        tenant_id: blob_text(&row, "tenant_id")?,
        content_sha256: blob_text(&row, "content_sha256")?,
        byte_len: from_i64(row.try_get("byte_len").map_err(sqlite_error)?, "byte_len")?,
        canonical_mime: row.try_get("canonical_mime").map_err(sqlite_error)?,
        object_namespace: parse_namespace(
            &row.try_get::<String, _>("object_namespace")
                .map_err(sqlite_error)?,
        )?,
        object_locator: row.try_get("object_locator").map_err(sqlite_error)?,
        storage_generation: row.try_get("storage_generation").map_err(sqlite_error)?,
        created_at_ms: from_i64(
            row.try_get("created_at_ms").map_err(sqlite_error)?,
            "created_at_ms",
        )?,
        delete_eligible_at_ms: row
            .try_get::<Option<i64>, _>("delete_eligible_at_ms")
            .map_err(sqlite_error)?
            .map(|value| from_i64(value, "delete_eligible_at_ms"))
            .transpose()?,
        deleted: row.try_get::<i64, _>("deleted").map_err(sqlite_error)? != 0,
    })
}

fn decode_binding(row: sqlx::sqlite::SqliteRow) -> Result<ResourceBinding, BlobStoreError> {
    Ok(ResourceBinding {
        binding_id: blob_text(&row, "binding_id")?,
        tenant_id: blob_text(&row, "tenant_id")?,
        project_id: optional_blob_text(&row, "project_id")?,
        document_id: optional_blob_text(&row, "document_id")?,
        physical_blob_id: blob_text(&row, "physical_blob_id")?,
        content_sha256: blob_text(&row, "content_sha256")?,
        byte_len: from_i64(row.try_get("byte_len").map_err(sqlite_error)?, "byte_len")?,
        resource_kind: parse_resource_kind(
            &row.try_get::<String, _>("resource_kind")
                .map_err(sqlite_error)?,
        )?,
        validation_profile: row.try_get("validation_profile").map_err(sqlite_error)?,
        lifecycle_state: parse_lifecycle(
            &row.try_get::<String, _>("lifecycle_state")
                .map_err(sqlite_error)?,
        )?,
        created_at_ms: from_i64(
            row.try_get("created_at_ms").map_err(sqlite_error)?,
            "created_at_ms",
        )?,
        retired_at_ms: row
            .try_get::<Option<i64>, _>("retired_at_ms")
            .map_err(sqlite_error)?
            .map(|value| from_i64(value, "retired_at_ms"))
            .transpose()?,
    })
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, BlobStoreError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        BlobStoreError::new(
            "sqlite_blob_metadata_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn optional_blob_text(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<String>, BlobStoreError> {
    let value: Option<Vec<u8>> = row.try_get(column).map_err(sqlite_error)?;
    value
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| {
                BlobStoreError::new(
                    "sqlite_blob_metadata_corrupt",
                    format!("{column} is not UTF-8"),
                )
            })
        })
        .transpose()
}

fn namespace_text(value: BlobNamespace) -> &'static str {
    match value {
        BlobNamespace::Quarantine => "quarantine",
        BlobNamespace::Canonical => "canonical",
        BlobNamespace::Checkpoint => "checkpoint",
        BlobNamespace::Derived => "derived",
        BlobNamespace::Export => "export",
    }
}

fn parse_namespace(value: &str) -> Result<BlobNamespace, BlobStoreError> {
    match value {
        "quarantine" => Ok(BlobNamespace::Quarantine),
        "canonical" => Ok(BlobNamespace::Canonical),
        "checkpoint" => Ok(BlobNamespace::Checkpoint),
        "derived" => Ok(BlobNamespace::Derived),
        "export" => Ok(BlobNamespace::Export),
        _ => Err(BlobStoreError::new(
            "sqlite_blob_metadata_corrupt",
            format!("unknown object_namespace {value:?}"),
        )),
    }
}

fn resource_kind_text(value: ResourceKind) -> &'static str {
    match value {
        ResourceKind::PubSource => "pub_source",
        ResourceKind::Asset => "asset",
        ResourceKind::Checkpoint => "checkpoint",
        ResourceKind::DerivedArtifact => "derived_artifact",
        ResourceKind::ExportArtifact => "export_artifact",
    }
}

fn parse_resource_kind(value: &str) -> Result<ResourceKind, BlobStoreError> {
    match value {
        "pub_source" => Ok(ResourceKind::PubSource),
        "asset" => Ok(ResourceKind::Asset),
        "checkpoint" => Ok(ResourceKind::Checkpoint),
        "derived_artifact" => Ok(ResourceKind::DerivedArtifact),
        "export_artifact" => Ok(ResourceKind::ExportArtifact),
        _ => Err(BlobStoreError::new(
            "sqlite_blob_metadata_corrupt",
            format!("unknown resource_kind {value:?}"),
        )),
    }
}

fn lifecycle_text(value: BindingLifecycle) -> &'static str {
    match value {
        BindingLifecycle::Active => "active",
        BindingLifecycle::Retired => "retired",
        BindingLifecycle::PurgeEligible => "purge_eligible",
    }
}

fn parse_lifecycle(value: &str) -> Result<BindingLifecycle, BlobStoreError> {
    match value {
        "active" => Ok(BindingLifecycle::Active),
        "retired" => Ok(BindingLifecycle::Retired),
        "purge_eligible" => Ok(BindingLifecycle::PurgeEligible),
        _ => Err(BlobStoreError::new(
            "sqlite_blob_metadata_corrupt",
            format!("unknown lifecycle_state {value:?}"),
        )),
    }
}

fn to_i64(value: u64, label: &'static str) -> Result<i64, BlobStoreError> {
    i64::try_from(value).map_err(|_| {
        BlobStoreError::new(
            "blob_metadata_range_error",
            format!("{label} does not fit SQLite signed integer"),
        )
    })
}

fn from_i64(value: i64, label: &'static str) -> Result<u64, BlobStoreError> {
    u64::try_from(value).map_err(|_| {
        BlobStoreError::new(
            "sqlite_blob_metadata_corrupt",
            format!("{label} is negative"),
        )
    })
}

fn is_unique(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database) if database.is_unique_violation()
    )
}

fn sqlite_error(error: impl fmt::Display) -> BlobStoreError {
    BlobStoreError::new(
        "sqlite_blob_metadata_error",
        error.to_string().chars().take(512).collect::<String>(),
    )
}
