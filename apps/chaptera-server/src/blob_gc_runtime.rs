use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use sha2::{Digest, Sha256};
use sqlx::{
    Connection, Row, Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{
    blob_gc::{
        BlobGcAuthority, BlobGcError, BlobGcObjectStore, GcCandidate, GcDeleteOutcome,
        GcObjectKind, GcPreDelete,
    },
    blob_store::{BlobProvider, ProviderError, ProviderErrorKind},
};

const GC_FENCE_RETRY_DELAY_MS: i64 = 1_000;

#[derive(Clone)]
pub struct SqlitePhysicalBlobGcAuthority {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqlitePhysicalBlobGcAuthority {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, BlobGcError> {
        if !(1..=16).contains(&max_connections) {
            return Err(BlobGcError::new(
                "invalid_gc_authority_pool_size",
                "GC authority pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(BlobGcError::new(
                "invalid_gc_authority_busy_timeout",
                "GC authority busy timeout must be >0 and <=30 seconds",
            ));
        }
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(BlobGcError::new(
                "gc_authority_database_missing",
                "run chaptera migrate up before opening GC authority",
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

    async fn require_schema(&self) -> Result<(), BlobGcError> {
        let migration: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chaptera_schema_migrations WHERE version=12 AND name='blob_gc_delete_fence'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;
        if migration != 1 {
            return Err(BlobGcError::new(
                "gc_authority_schema_missing",
                "blob GC delete-fence migration v12 is not recorded",
            ));
        }

        let columns = sqlx::query("PRAGMA table_info(physical_blobs)")
            .fetch_all(&self.pool)
            .await
            .map_err(sqlite_error)?;
        let has_fence = columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .map(|name| name == "gc_delete_fence")
                .unwrap_or(false)
        });
        let has_fenced_at = columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .map(|name| name == "gc_fenced_at_ms")
                .unwrap_or(false)
        });
        if !has_fence || !has_fenced_at {
            return Err(BlobGcError::new(
                "gc_authority_schema_missing",
                "physical blob GC fence columns are absent",
            ));
        }
        Ok(())
    }

    async fn physical_recheck_and_fence(
        &self,
        candidate: &GcCandidate,
        now_ms: i64,
    ) -> Result<GcPreDelete, BlobGcError> {
        let mut connection = self.pool.acquire().await.map_err(sqlite_error)?;
        let mut tx = (*connection)
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlite_error)?;

        let result = self
            .physical_recheck_and_fence_tx(&mut tx, candidate, now_ms)
            .await;
        match result {
            Ok(outcome) => {
                tx.commit().await.map_err(sqlite_error)?;
                Ok(outcome)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn physical_recheck_and_fence_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        candidate: &GcCandidate,
        now_ms: i64,
    ) -> Result<GcPreDelete, BlobGcError> {
        let row = sqlx::query(
            r#"
            SELECT
                tenant_id,
                object_locator,
                storage_generation,
                delete_eligible_at_ms,
                deleted,
                gc_delete_fence
            FROM physical_blobs
            WHERE physical_blob_id=?
            "#,
        )
        .bind(candidate.object_id.as_bytes())
        .fetch_optional(&mut **tx)
        .await
        .map_err(sqlite_error)?;

        let Some(row) = row else {
            return Ok(GcPreDelete::Cancel {
                code: "physical_blob_missing",
            });
        };

        let tenant_id = blob_text(&row, "tenant_id")?;
        if tenant_id != candidate.tenant_id {
            return Err(BlobGcError::new(
                "gc_tenant_scope_mismatch",
                "GC candidate tenant differs from physical blob tenant",
            ));
        }

        let generation: String = row.try_get("storage_generation").map_err(sqlite_error)?;
        if let Some(observed) = &candidate.observed_generation
            && observed != &generation
        {
            return Ok(GcPreDelete::Cancel {
                code: "generation_changed",
            });
        }

        let deleted: i64 = row.try_get("deleted").map_err(sqlite_error)?;
        if deleted != 0 {
            return Ok(GcPreDelete::Cancel {
                code: "already_deleted",
            });
        }

        let eligible_at: Option<i64> =
            row.try_get("delete_eligible_at_ms").map_err(sqlite_error)?;
        let Some(eligible_at) = eligible_at else {
            return Ok(GcPreDelete::Cancel {
                code: "not_delete_eligible",
            });
        };
        if eligible_at > now_ms {
            return Ok(GcPreDelete::Defer {
                not_before_ms: eligible_at,
                code: "retention_not_elapsed",
            });
        }

        let live_bindings: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM resource_bindings
            WHERE tenant_id=?
              AND physical_blob_id=?
              AND lifecycle_state IN ('active','retired')
            "#,
        )
        .bind(candidate.tenant_id.as_bytes())
        .bind(candidate.object_id.as_bytes())
        .fetch_one(&mut **tx)
        .await
        .map_err(sqlite_error)?;
        if live_bindings != 0 {
            return Ok(GcPreDelete::Cancel {
                code: "became_reachable",
            });
        }

        let fence = physical_fence(candidate, &generation);
        let current_fence = optional_blob_text(&row, "gc_delete_fence")?;
        if let Some(current) = current_fence {
            if current != fence {
                return Ok(GcPreDelete::Defer {
                    not_before_ms: now_ms.saturating_add(GC_FENCE_RETRY_DELAY_MS),
                    code: "gc_fence_owned",
                });
            }
        } else {
            let changed = sqlx::query(
                r#"
                UPDATE physical_blobs
                SET gc_delete_fence=?, gc_fenced_at_ms=?
                WHERE physical_blob_id=?
                  AND tenant_id=?
                  AND storage_generation=?
                  AND deleted=0
                  AND gc_delete_fence IS NULL
                "#,
            )
            .bind(fence.as_bytes())
            .bind(now_ms)
            .bind(candidate.object_id.as_bytes())
            .bind(candidate.tenant_id.as_bytes())
            .bind(&generation)
            .execute(&mut **tx)
            .await
            .map_err(sqlite_error)?;
            if changed.rows_affected() != 1 {
                return Err(BlobGcError::new(
                    "gc_fence_race",
                    "physical metadata changed while installing GC delete fence",
                ));
            }
        }

        let object_locator: String = row.try_get("object_locator").map_err(sqlite_error)?;
        Ok(GcPreDelete::Ready {
            object_locator,
            storage_generation: generation,
            delete_fence: fence,
        })
    }

    async fn commit_physical_deleted(
        &self,
        candidate: &GcCandidate,
        storage_generation: &str,
        delete_fence: &str,
    ) -> Result<(), BlobGcError> {
        let mut connection = self.pool.acquire().await.map_err(sqlite_error)?;
        let mut tx = (*connection)
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(sqlite_error)?;

        let result = async {
            let row = sqlx::query(
                r#"
                SELECT tenant_id, storage_generation, deleted, gc_delete_fence
                FROM physical_blobs
                WHERE physical_blob_id=?
                "#,
            )
            .bind(candidate.object_id.as_bytes())
            .fetch_optional(&mut *tx)
            .await
            .map_err(sqlite_error)?
            .ok_or_else(|| {
                BlobGcError::new(
                    "physical_blob_missing",
                    "physical metadata missing during GC delete commit",
                )
            })?;

            let tenant_id = blob_text(&row, "tenant_id")?;
            if tenant_id != candidate.tenant_id {
                return Err(BlobGcError::new(
                    "gc_tenant_scope_mismatch",
                    "GC delete commit tenant differs from physical blob tenant",
                ));
            }
            let generation: String = row.try_get("storage_generation").map_err(sqlite_error)?;
            if generation != storage_generation {
                return Err(BlobGcError::new(
                    "storage_generation_mismatch",
                    "GC delete commit generation differs from physical metadata",
                ));
            }

            let deleted: i64 = row.try_get("deleted").map_err(sqlite_error)?;
            if deleted != 0 {
                return Ok(());
            }

            let current_fence = optional_blob_text(&row, "gc_delete_fence")?;
            if current_fence.as_deref() != Some(delete_fence) {
                return Err(BlobGcError::new(
                    "gc_delete_fence_mismatch",
                    "GC delete commit does not own the durable physical fence",
                ));
            }

            let changed = sqlx::query(
                r#"
                UPDATE physical_blobs
                SET deleted=1, gc_delete_fence=NULL, gc_fenced_at_ms=NULL
                WHERE physical_blob_id=?
                  AND tenant_id=?
                  AND storage_generation=?
                  AND deleted=0
                  AND gc_delete_fence=?
                "#,
            )
            .bind(candidate.object_id.as_bytes())
            .bind(candidate.tenant_id.as_bytes())
            .bind(storage_generation)
            .bind(delete_fence.as_bytes())
            .execute(&mut *tx)
            .await
            .map_err(sqlite_error)?;
            if changed.rows_affected() != 1 {
                return Err(BlobGcError::new(
                    "gc_delete_commit_race",
                    "physical metadata changed while committing GC deletion",
                ));
            }
            Ok(())
        }
        .await;

        match result {
            Ok(()) => tx.commit().await.map_err(sqlite_error),
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }
}

#[async_trait::async_trait]
impl BlobGcAuthority for SqlitePhysicalBlobGcAuthority {
    async fn recheck_and_fence(
        &self,
        candidate: &GcCandidate,
        now_ms: i64,
    ) -> Result<GcPreDelete, BlobGcError> {
        if candidate.object_kind != GcObjectKind::PhysicalBlob {
            return Ok(GcPreDelete::Cancel {
                code: "gc_kind_unsupported",
            });
        }
        if now_ms < 0 {
            return Err(BlobGcError::new(
                "invalid_gc_time",
                "GC timestamp must be non-negative",
            ));
        }
        self.physical_recheck_and_fence(candidate, now_ms).await
    }

    async fn commit_deleted(
        &self,
        candidate: &GcCandidate,
        storage_generation: &str,
        delete_fence: &str,
        _now_ms: i64,
    ) -> Result<(), BlobGcError> {
        if candidate.object_kind != GcObjectKind::PhysicalBlob {
            return Err(BlobGcError::new(
                "gc_kind_unsupported",
                "physical GC authority cannot commit another object kind",
            ));
        }
        self.commit_physical_deleted(candidate, storage_generation, delete_fence)
            .await
    }
}

#[derive(Clone)]
pub struct ProviderBlobGcObjectStore {
    provider: Arc<dyn BlobProvider>,
}

impl ProviderBlobGcObjectStore {
    pub fn new(provider: Arc<dyn BlobProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl BlobGcObjectStore for ProviderBlobGcObjectStore {
    async fn delete_exact(
        &self,
        object_locator: &str,
        storage_generation: &str,
    ) -> Result<GcDeleteOutcome, BlobGcError> {
        match self
            .provider
            .delete_exact(object_locator, storage_generation)
            .await
        {
            Ok(()) => Ok(GcDeleteOutcome::Deleted),
            Err(error) => match error.kind {
                ProviderErrorKind::NotFound => Ok(GcDeleteOutcome::NotFound),
                ProviderErrorKind::UnknownOutcome => Ok(GcDeleteOutcome::UnknownOutcome),
                _ => Err(provider_error(error)),
            },
        }
    }

    async fn exists_exact(
        &self,
        object_locator: &str,
        storage_generation: &str,
    ) -> Result<bool, BlobGcError> {
        let Some(metadata) = self
            .provider
            .head_exact(object_locator)
            .await
            .map_err(provider_error)?
        else {
            return Ok(false);
        };
        if metadata.generation != storage_generation {
            return Err(BlobGcError::new(
                "provider_generation_changed",
                "provider object exists with a different generation during GC reconciliation",
            ));
        }
        Ok(true)
    }
}

fn physical_fence(candidate: &GcCandidate, storage_generation: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"chaptera-blob-gc-physical-fence-v1\0");
    hasher.update(candidate.tenant_id.as_bytes());
    hasher.update([0]);
    hasher.update(candidate.candidate_id.as_bytes());
    hasher.update([0]);
    hasher.update(candidate.object_id.as_bytes());
    hasher.update([0]);
    hasher.update(storage_generation.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, BlobGcError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        BlobGcError::new(
            "gc_metadata_corrupt",
            format!("{column} is not valid UTF-8"),
        )
    })
}

fn optional_blob_text(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<String>, BlobGcError> {
    let value: Option<Vec<u8>> = row.try_get(column).map_err(sqlite_error)?;
    value
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| {
                BlobGcError::new(
                    "gc_metadata_corrupt",
                    format!("{column} is not valid UTF-8"),
                )
            })
        })
        .transpose()
}

fn provider_error(error: ProviderError) -> BlobGcError {
    let code = match error.kind {
        ProviderErrorKind::AlreadyExists => "gc_provider_already_exists",
        ProviderErrorKind::UnknownOutcome => "gc_provider_unknown",
        ProviderErrorKind::NotFound => "gc_provider_not_found",
        ProviderErrorKind::AccessDenied => "gc_provider_access_denied",
        ProviderErrorKind::Other => "gc_provider_failure",
    };
    BlobGcError::new(code, bounded(&error.code))
}

fn sqlite_error(error: impl fmt::Display) -> BlobGcError {
    BlobGcError::new(
        "sqlite_blob_gc_authority_error",
        bounded(&error.to_string()),
    )
}

fn bounded(value: &str) -> String {
    value.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
        blob_store::{
            BindingLifecycle, BlobBindingRepository, BlobNamespace, PhysicalBlobRecord,
            ResourceBinding, ResourceKind,
        },
        schema_migration::SqliteMigrationRuntime,
        sqlite_blob_metadata::SqliteBlobBindingRepository,
    };

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn path(label: &str) -> PathBuf {
        let serial = NEXT.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-blob-gc-authority-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn stores(
        label: &str,
    ) -> (
        SqlitePhysicalBlobGcAuthority,
        SqliteBlobBindingRepository,
        PathBuf,
    ) {
        let db = path(label);
        SqliteMigrationRuntime::new(&db, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let authority = SqlitePhysicalBlobGcAuthority::open(&db, 2, Duration::from_secs(2))
            .await
            .unwrap();
        let bindings = SqliteBlobBindingRepository::open(&db, 2, Duration::from_secs(2))
            .await
            .unwrap();
        (authority, bindings, db)
    }

    fn physical(id: &str) -> PhysicalBlobRecord {
        PhysicalBlobRecord {
            physical_blob_id: id.into(),
            tenant_id: "tenant-a".into(),
            content_sha256: "a".repeat(64),
            byte_len: 123,
            canonical_mime: Some("application/octet-stream".into()),
            object_namespace: BlobNamespace::Canonical,
            object_locator: format!("canonical/tenant-a/{id}"),
            storage_generation: "generation-1".into(),
            created_at_ms: 10,
            delete_eligible_at_ms: Some(100),
            deleted: false,
        }
    }

    fn binding(
        id: &str,
        physical: &PhysicalBlobRecord,
        lifecycle: BindingLifecycle,
    ) -> ResourceBinding {
        ResourceBinding {
            binding_id: id.into(),
            tenant_id: physical.tenant_id.clone(),
            project_id: Some("project-a".into()),
            document_id: Some("doc-a".into()),
            physical_blob_id: physical.physical_blob_id.clone(),
            content_sha256: physical.content_sha256.clone(),
            byte_len: physical.byte_len,
            resource_kind: ResourceKind::ExportArtifact,
            validation_profile: "export:idml:bounded-editable".into(),
            lifecycle_state: lifecycle,
            created_at_ms: 11,
            retired_at_ms: None,
        }
    }

    fn candidate(id: &str, physical: &PhysicalBlobRecord) -> GcCandidate {
        GcCandidate {
            candidate_id: id.into(),
            tenant_id: physical.tenant_id.clone(),
            object_kind: GcObjectKind::PhysicalBlob,
            object_id: physical.physical_blob_id.clone(),
            reason_code: "purge_eligible".into(),
            not_before_ms: 100,
            observed_generation: Some(physical.storage_generation.clone()),
            state: crate::blob_gc::GcCandidateState::Running,
            attempt: 1,
            lease_owner: Some("worker-a".into()),
            lease_generation: 1,
            lease_expires_at_ms: Some(1_000),
            last_error_code: None,
            created_at_ms: 1,
            completed_at_ms: None,
        }
    }

    #[tokio::test]
    async fn live_binding_cancels_before_fence() {
        let (authority, bindings, db) = stores("live").await;
        let physical = physical("blob-live");
        bindings
            .commit_physical_and_binding(
                physical.clone(),
                binding("binding-live", &physical, BindingLifecycle::Active),
            )
            .await
            .unwrap();

        let result = authority
            .recheck_and_fence(&candidate("gc-live", &physical), 200)
            .await
            .unwrap();
        assert_eq!(
            result,
            GcPreDelete::Cancel {
                code: "became_reachable"
            }
        );

        authority.close().await;
        bindings.close().await;
        cleanup(&db);
    }

    #[tokio::test]
    async fn purge_eligible_binding_allows_idempotent_fence_and_blocks_new_binding() {
        let (authority, bindings, db) = stores("fence").await;
        let physical = physical("blob-fence");
        bindings
            .commit_physical_and_binding(
                physical.clone(),
                binding("binding-purge", &physical, BindingLifecycle::PurgeEligible),
            )
            .await
            .unwrap();
        let candidate = candidate("gc-fence", &physical);

        let first = authority.recheck_and_fence(&candidate, 200).await.unwrap();
        let second = authority.recheck_and_fence(&candidate, 201).await.unwrap();
        assert_eq!(first, second);

        let error = bindings
            .commit_binding(binding("binding-new", &physical, BindingLifecycle::Active))
            .await
            .unwrap_err();
        assert_eq!(error.code, "physical_blob_gc_fenced");

        authority.close().await;
        bindings.close().await;
        cleanup(&db);
    }

    #[tokio::test]
    async fn delete_commit_requires_exact_fence_and_is_idempotent() {
        let (authority, bindings, db) = stores("commit").await;
        let physical = physical("blob-commit");
        bindings
            .commit_physical_and_binding(
                physical.clone(),
                binding(
                    "binding-purge",
                    &physical,
                    BindingLifecycle::PurgeEligible,
                ),
            )
            .await
            .unwrap();
        let candidate = candidate("gc-commit", &physical);
        let GcPreDelete::Ready {
            storage_generation,
            delete_fence,
            ..
        } = authority.recheck_and_fence(&candidate, 200).await.unwrap()
        else {
            panic!("purge-eligible physical blob must be fenced")
        };

        let error = authority
            .commit_deleted(&candidate, &storage_generation, "sha256:wrong", 201)
            .await
            .unwrap_err();
        assert_eq!(error.code, "gc_delete_fence_mismatch");

        authority
            .commit_deleted(&candidate, &storage_generation, &delete_fence, 202)
            .await
            .unwrap();
        authority
            .commit_deleted(&candidate, &storage_generation, &delete_fence, 203)
            .await
            .unwrap();

        let deleted = bindings
            .get_physical(&physical.physical_blob_id)
            .await
            .unwrap()
            .unwrap();
        assert!(deleted.deleted);

        authority.close().await;
        bindings.close().await;
        cleanup(&db);
    }
}
