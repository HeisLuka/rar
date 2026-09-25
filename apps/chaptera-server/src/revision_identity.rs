use std::{
    fmt,
    path::{Path, PathBuf},
    str,
    time::Duration,
};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const MAX_CONNECTIONS: u32 = 16;
const MAX_BUSY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionIdentityError {
    pub code: &'static str,
    pub message: String,
}

impl RevisionIdentityError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RevisionIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RevisionIdentityError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionIdentityBinding {
    pub document_id: String,
    pub service_revision_id: String,
    pub canonical_revision_id: String,
    pub service_parent_revision_id: Option<String>,
    pub canonical_parent_revision_id: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindRevisionIdentityOutcome {
    Bound(RevisionIdentityBinding),
    AlreadyBound(RevisionIdentityBinding),
}

#[derive(Clone)]
pub struct SqliteRevisionIdentityStore {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteRevisionIdentityStore {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, RevisionIdentityError> {
        if !(1..=MAX_CONNECTIONS).contains(&max_connections) {
            return Err(RevisionIdentityError::new(
                "invalid_pool_size",
                "revision identity store must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > MAX_BUSY_TIMEOUT {
            return Err(RevisionIdentityError::new(
                "invalid_busy_timeout",
                "revision identity busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref().to_path_buf();
        if path.as_os_str().is_empty() {
            return Err(RevisionIdentityError::new(
                "invalid_database_path",
                "revision identity database path must be non-empty",
            ));
        }
        if !path.exists() {
            return Err(RevisionIdentityError::new(
                "revision_identity_database_missing",
                "run chaptera migrate up before opening revision identity bindings",
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
        store.verify_profile().await?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn bind_baseline(
        &self,
        document_id: &str,
        service_revision_id: &str,
        canonical_revision_id: &str,
        created_at_ms: i64,
    ) -> Result<BindRevisionIdentityOutcome, RevisionIdentityError> {
        require_ident(document_id, "document_id")?;
        require_ident(service_revision_id, "service_revision_id")?;
        require_canonical_revision(canonical_revision_id, "canonical_revision_id")?;
        require_time(created_at_ms)?;

        let incoming: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM revision_edges
            WHERE document_id = ? AND child_revision = ?
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(service_revision_id.as_bytes())
        .fetch_one(&self.pool)
        .await
        .map_err(sqlite_error)?;
        if incoming != 0 {
            return Err(RevisionIdentityError::new(
                "baseline_has_service_parent",
                "baseline service revision already has an incoming durable RevisionStream edge",
            ));
        }

        let binding = RevisionIdentityBinding {
            document_id: document_id.to_owned(),
            service_revision_id: service_revision_id.to_owned(),
            canonical_revision_id: canonical_revision_id.to_owned(),
            service_parent_revision_id: None,
            canonical_parent_revision_id: None,
            created_at_ms,
        };
        self.insert_binding(binding).await
    }

    pub async fn bind_child(
        &self,
        document_id: &str,
        service_parent_revision_id: &str,
        service_revision_id: &str,
        canonical_parent_revision_id: &str,
        canonical_revision_id: &str,
        created_at_ms: i64,
    ) -> Result<BindRevisionIdentityOutcome, RevisionIdentityError> {
        require_ident(document_id, "document_id")?;
        require_ident(service_parent_revision_id, "service_parent_revision_id")?;
        require_ident(service_revision_id, "service_revision_id")?;
        require_canonical_revision(canonical_parent_revision_id, "canonical_parent_revision_id")?;
        require_canonical_revision(canonical_revision_id, "canonical_revision_id")?;
        require_time(created_at_ms)?;

        if service_revision_id == service_parent_revision_id {
            return Err(RevisionIdentityError::new(
                "service_revision_cycle",
                "service child revision must differ from its parent",
            ));
        }
        if canonical_revision_id == canonical_parent_revision_id {
            return Err(RevisionIdentityError::new(
                "canonical_revision_cycle",
                "canonical child revision must differ from its parent",
            ));
        }

        let durable_parent = self
            .durable_service_parent(document_id, service_revision_id)
            .await?
            .ok_or_else(|| {
                RevisionIdentityError::new(
                    "service_revision_not_found",
                    "service child revision is absent from the durable RevisionStream",
                )
            })?;
        if durable_parent != service_parent_revision_id {
            return Err(RevisionIdentityError::new(
                "service_parent_mismatch",
                "service child revision is parented by a different durable service revision",
            ));
        }

        let parent_binding = self
            .resolve(document_id, service_parent_revision_id)
            .await?
            .ok_or_else(|| {
                RevisionIdentityError::new(
                    "canonical_parent_unbound",
                    "service parent revision has no canonical AuthoringRevisionId binding",
                )
            })?;
        if parent_binding.canonical_revision_id != canonical_parent_revision_id {
            return Err(RevisionIdentityError::new(
                "canonical_parent_mismatch",
                "canonical parent does not match the durable binding of the service parent",
            ));
        }

        let binding = RevisionIdentityBinding {
            document_id: document_id.to_owned(),
            service_revision_id: service_revision_id.to_owned(),
            canonical_revision_id: canonical_revision_id.to_owned(),
            service_parent_revision_id: Some(service_parent_revision_id.to_owned()),
            canonical_parent_revision_id: Some(canonical_parent_revision_id.to_owned()),
            created_at_ms,
        };
        self.insert_binding(binding).await
    }

    pub async fn resolve(
        &self,
        document_id: &str,
        service_revision_id: &str,
    ) -> Result<Option<RevisionIdentityBinding>, RevisionIdentityError> {
        require_ident(document_id, "document_id")?;
        require_ident(service_revision_id, "service_revision_id")?;

        let row = sqlx::query(
            r#"
            SELECT
                document_id,
                service_revision_id,
                canonical_revision_id,
                service_parent_revision_id,
                canonical_parent_revision_id,
                created_at_ms
            FROM revision_identity_bindings
            WHERE document_id = ? AND service_revision_id = ?
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(service_revision_id.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?;

        row.map(decode_binding).transpose()
    }

    pub async fn require_binding(
        &self,
        document_id: &str,
        service_revision_id: &str,
    ) -> Result<RevisionIdentityBinding, RevisionIdentityError> {
        self.resolve(document_id, service_revision_id)
            .await?
            .ok_or_else(|| {
                RevisionIdentityError::new(
                    "canonical_revision_unbound",
                    "service revision has no canonical AuthoringRevisionId binding for this document",
                )
            })
    }

    pub async fn resolve_service_revision(
        &self,
        document_id: &str,
        canonical_revision_id: &str,
    ) -> Result<Option<RevisionIdentityBinding>, RevisionIdentityError> {
        require_ident(document_id, "document_id")?;
        require_canonical_revision(canonical_revision_id, "canonical_revision_id")?;

        let row = sqlx::query(
            r#"
            SELECT
                document_id,
                service_revision_id,
                canonical_revision_id,
                service_parent_revision_id,
                canonical_parent_revision_id,
                created_at_ms
            FROM revision_identity_bindings
            WHERE document_id = ? AND canonical_revision_id = ?
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(canonical_revision_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?;

        row.map(decode_binding).transpose()
    }

    async fn insert_binding(
        &self,
        binding: RevisionIdentityBinding,
    ) -> Result<BindRevisionIdentityOutcome, RevisionIdentityError> {
        validate_binding(&binding)?;

        let result = sqlx::query(
            r#"
            INSERT INTO revision_identity_bindings (
                document_id,
                service_revision_id,
                canonical_revision_id,
                service_parent_revision_id,
                canonical_parent_revision_id,
                created_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(binding.document_id.as_bytes())
        .bind(binding.service_revision_id.as_bytes())
        .bind(&binding.canonical_revision_id)
        .bind(
            binding
                .service_parent_revision_id
                .as_ref()
                .map(String::as_bytes),
        )
        .bind(&binding.canonical_parent_revision_id)
        .bind(binding.created_at_ms)
        .execute(&self.pool)
        .await;

        match result {
            Ok(done) if done.rows_affected() == 1 => {
                Ok(BindRevisionIdentityOutcome::Bound(binding))
            }
            Ok(_) => Err(RevisionIdentityError::new(
                "revision_identity_bind_no_effect",
                "revision identity INSERT completed without one row",
            )),
            Err(error) if is_constraint(&error) => {
                if let Some(existing) = self
                    .resolve(&binding.document_id, &binding.service_revision_id)
                    .await?
                {
                    if same_binding_identity(&existing, &binding) {
                        return Ok(BindRevisionIdentityOutcome::AlreadyBound(existing));
                    }
                    return Err(RevisionIdentityError::new(
                        "service_revision_binding_conflict",
                        "service revision is already bound to a different canonical identity",
                    ));
                }

                if let Some(existing) = self
                    .resolve_service_revision(&binding.document_id, &binding.canonical_revision_id)
                    .await?
                {
                    return Err(RevisionIdentityError::new(
                        "canonical_revision_binding_conflict",
                        format!(
                            "canonical revision is already bound to service revision {}",
                            existing.service_revision_id
                        ),
                    ));
                }

                Err(RevisionIdentityError::new(
                    "revision_identity_constraint",
                    "revision identity binding violated a durable uniqueness/check constraint",
                ))
            }
            Err(error) => Err(sqlite_error(error)),
        }
    }

    async fn durable_service_parent(
        &self,
        document_id: &str,
        service_revision_id: &str,
    ) -> Result<Option<String>, RevisionIdentityError> {
        let row = sqlx::query(
            r#"
            SELECT parent_revision
            FROM revision_edges
            WHERE document_id = ? AND child_revision = ?
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(service_revision_id.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_error)?;

        row.map(|row| blob_text(&row, "parent_revision"))
            .transpose()
    }

    async fn require_schema(&self) -> Result<(), RevisionIdentityError> {
        for table in ["revision_edges", "revision_identity_bindings"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;

            if exists != 1 {
                return Err(RevisionIdentityError::new(
                    "revision_identity_schema_missing",
                    format!("{table} is absent; run chaptera migrate up"),
                ));
            }
        }
        Ok(())
    }

    async fn verify_profile(&self) -> Result<(), RevisionIdentityError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(RevisionIdentityError::new(
                "revision_identity_profile_mismatch",
                format!("expected WAL journal mode, got {journal_mode}"),
            ));
        }

        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if synchronous != 2 {
            return Err(RevisionIdentityError::new(
                "revision_identity_profile_mismatch",
                format!("expected synchronous=FULL(2), got {synchronous}"),
            ));
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
        if foreign_keys != 1 {
            return Err(RevisionIdentityError::new(
                "revision_identity_profile_mismatch",
                "foreign_keys pragma is not enabled",
            ));
        }

        Ok(())
    }
}

fn same_binding_identity(
    existing: &RevisionIdentityBinding,
    requested: &RevisionIdentityBinding,
) -> bool {
    existing.document_id == requested.document_id
        && existing.service_revision_id == requested.service_revision_id
        && existing.canonical_revision_id == requested.canonical_revision_id
        && existing.service_parent_revision_id == requested.service_parent_revision_id
        && existing.canonical_parent_revision_id == requested.canonical_parent_revision_id
}

fn validate_binding(binding: &RevisionIdentityBinding) -> Result<(), RevisionIdentityError> {
    require_ident(&binding.document_id, "document_id")?;
    require_ident(&binding.service_revision_id, "service_revision_id")?;
    require_canonical_revision(&binding.canonical_revision_id, "canonical_revision_id")?;
    require_time(binding.created_at_ms)?;

    match (
        &binding.service_parent_revision_id,
        &binding.canonical_parent_revision_id,
    ) {
        (None, None) => {}
        (Some(service_parent), Some(canonical_parent)) => {
            require_ident(service_parent, "service_parent_revision_id")?;
            require_canonical_revision(canonical_parent, "canonical_parent_revision_id")?;
        }
        _ => {
            return Err(RevisionIdentityError::new(
                "revision_parent_binding_incomplete",
                "service and canonical parent bindings must be both present or both absent",
            ));
        }
    }
    Ok(())
}

fn decode_binding(
    row: sqlx::sqlite::SqliteRow,
) -> Result<RevisionIdentityBinding, RevisionIdentityError> {
    let binding = RevisionIdentityBinding {
        document_id: blob_text(&row, "document_id")?,
        service_revision_id: blob_text(&row, "service_revision_id")?,
        canonical_revision_id: row.try_get("canonical_revision_id").map_err(sqlite_error)?,
        service_parent_revision_id: optional_blob_text(&row, "service_parent_revision_id")?,
        canonical_parent_revision_id: row
            .try_get("canonical_parent_revision_id")
            .map_err(sqlite_error)?,
        created_at_ms: row.try_get("created_at_ms").map_err(sqlite_error)?,
    };
    validate_binding(&binding)?;
    Ok(binding)
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, RevisionIdentityError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    str::from_utf8(&bytes).map(str::to_owned).map_err(|_| {
        RevisionIdentityError::new(
            "revision_identity_row_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn optional_blob_text(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<String>, RevisionIdentityError> {
    let bytes: Option<Vec<u8>> = row.try_get(column).map_err(sqlite_error)?;
    bytes
        .map(|value| {
            str::from_utf8(&value).map(str::to_owned).map_err(|_| {
                RevisionIdentityError::new(
                    "revision_identity_row_corrupt",
                    format!("{column} is not UTF-8"),
                )
            })
        })
        .transpose()
}

fn require_ident(value: &str, label: &'static str) -> Result<(), RevisionIdentityError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(RevisionIdentityError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_canonical_revision(
    value: &str,
    label: &'static str,
) -> Result<(), RevisionIdentityError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(RevisionIdentityError::new(
            "invalid_canonical_revision_id",
            format!("{label} must be 64 lowercase hex characters"),
        ));
    }
    Ok(())
}

fn require_time(created_at_ms: i64) -> Result<(), RevisionIdentityError> {
    if created_at_ms < 0 {
        return Err(RevisionIdentityError::new(
            "invalid_created_at",
            "created_at_ms must be non-negative",
        ));
    }
    Ok(())
}

fn is_constraint(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database) if database.is_unique_violation()
    )
}

fn sqlite_error(error: impl fmt::Display) -> RevisionIdentityError {
    RevisionIdentityError::new(
        "sqlite_revision_identity_error",
        error.to_string().chars().take(512).collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
        schema_migration::SqliteMigrationRuntime,
        sqlite_store::{RevisionEdge, SqliteRevisionStore, encode_canonical_event},
    };

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let n = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-revision-identity-{label}-{}-{n}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for candidate in [
            path.to_path_buf(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            let _ = fs::remove_file(candidate);
        }
    }

    async fn migrated_store(
        label: &str,
    ) -> (SqliteRevisionIdentityStore, SqliteRevisionStore, PathBuf) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let identity = SqliteRevisionIdentityStore::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let revisions = SqliteRevisionStore::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (identity, revisions, path)
    }

    fn edge(parent: &str, child: &str) -> RevisionEdge {
        RevisionEdge {
            document_id: "doc-a".into(),
            parent_revision: parent.into(),
            parent_cursor: 0,
            operation_id: format!("op-{child}"),
            request_hash: "a".repeat(64),
            canonical_event: encode_canonical_event(br#"{"kind":"move"}"#).unwrap(),
            child_revision: child.into(),
            child_cursor: 1,
            resulting_state_hash: "b".repeat(64),
            authoring_root_hash: Some("c".repeat(64)),
            semantic_schema_version: 1,
            committed_at_ms: 1,
        }
    }

    #[tokio::test]
    async fn baseline_and_child_bindings_survive_restart() {
        let (identity, revisions, path) = migrated_store("restart").await;
        let canonical_r0 = "1".repeat(64);
        let canonical_r1 = "2".repeat(64);

        assert!(matches!(
            identity
                .bind_baseline("doc-a", "service-r0", &canonical_r0, 1)
                .await
                .unwrap(),
            BindRevisionIdentityOutcome::Bound(_)
        ));

        revisions
            .append_edge(edge("service-r0", "service-r1"))
            .await
            .unwrap();

        let bound = identity
            .bind_child(
                "doc-a",
                "service-r0",
                "service-r1",
                &canonical_r0,
                &canonical_r1,
                2,
            )
            .await
            .unwrap();
        assert!(matches!(bound, BindRevisionIdentityOutcome::Bound(_)));

        let resolved = identity
            .resolve("doc-a", "service-r1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.canonical_revision_id, canonical_r1);
        assert_eq!(
            resolved.canonical_parent_revision_id.as_deref(),
            Some(canonical_r0.as_str())
        );

        identity.close().await;
        revisions.close().await;

        let reopened = SqliteRevisionIdentityStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .resolve("doc-a", "service-r1")
                .await
                .unwrap()
                .unwrap()
                .canonical_revision_id,
            canonical_r1
        );
        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn child_binding_requires_durable_edge_and_exact_bound_parent() {
        let (identity, revisions, path) = migrated_store("parent").await;
        let canonical_r0 = "1".repeat(64);
        let canonical_r1 = "2".repeat(64);

        identity
            .bind_baseline("doc-a", "service-r0", &canonical_r0, 1)
            .await
            .unwrap();

        let missing = identity
            .bind_child(
                "doc-a",
                "service-r0",
                "service-r1",
                &canonical_r0,
                &canonical_r1,
                2,
            )
            .await
            .unwrap_err();
        assert_eq!(missing.code, "service_revision_not_found");

        revisions
            .append_edge(edge("service-r0", "service-r1"))
            .await
            .unwrap();

        let wrong_parent = identity
            .bind_child(
                "doc-a",
                "service-other",
                "service-r1",
                &canonical_r0,
                &canonical_r1,
                2,
            )
            .await
            .unwrap_err();
        assert_eq!(wrong_parent.code, "service_parent_mismatch");

        let wrong_canonical_parent = identity
            .bind_child(
                "doc-a",
                "service-r0",
                "service-r1",
                &"3".repeat(64),
                &canonical_r1,
                2,
            )
            .await
            .unwrap_err();
        assert_eq!(wrong_canonical_parent.code, "canonical_parent_mismatch");

        identity.close().await;
        revisions.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn exact_retry_is_idempotent_and_changed_binding_conflicts() {
        let (identity, revisions, path) = migrated_store("retry").await;
        let canonical_r0 = "1".repeat(64);
        let canonical_r1 = "2".repeat(64);

        identity
            .bind_baseline("doc-a", "service-r0", &canonical_r0, 1)
            .await
            .unwrap();
        revisions
            .append_edge(edge("service-r0", "service-r1"))
            .await
            .unwrap();

        identity
            .bind_child(
                "doc-a",
                "service-r0",
                "service-r1",
                &canonical_r0,
                &canonical_r1,
                2,
            )
            .await
            .unwrap();

        assert!(matches!(
            identity
                .bind_child(
                    "doc-a",
                    "service-r0",
                    "service-r1",
                    &canonical_r0,
                    &canonical_r1,
                    99,
                )
                .await
                .unwrap(),
            BindRevisionIdentityOutcome::AlreadyBound(_)
        ));

        let conflict = identity
            .bind_child(
                "doc-a",
                "service-r0",
                "service-r1",
                &canonical_r0,
                &"4".repeat(64),
                2,
            )
            .await
            .unwrap_err();
        assert_eq!(conflict.code, "service_revision_binding_conflict");

        identity.close().await;
        revisions.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn historical_mapping_remains_exact_and_wrong_document_is_unbound() {
        let (identity, revisions, path) = migrated_store("historical").await;
        let canonical_r0 = "1".repeat(64);
        let canonical_r1 = "2".repeat(64);
        let canonical_r2 = "3".repeat(64);

        identity
            .bind_baseline("doc-a", "service-r0", &canonical_r0, 1)
            .await
            .unwrap();
        revisions
            .append_edge(edge("service-r0", "service-r1"))
            .await
            .unwrap();
        identity
            .bind_child(
                "doc-a",
                "service-r0",
                "service-r1",
                &canonical_r0,
                &canonical_r1,
                2,
            )
            .await
            .unwrap();

        let mut second = edge("service-r1", "service-r2");
        second.parent_cursor = 1;
        second.child_cursor = 2;
        revisions.append_edge(second).await.unwrap();
        identity
            .bind_child(
                "doc-a",
                "service-r1",
                "service-r2",
                &canonical_r1,
                &canonical_r2,
                3,
            )
            .await
            .unwrap();

        assert_eq!(
            identity
                .require_binding("doc-a", "service-r1")
                .await
                .unwrap()
                .canonical_revision_id,
            canonical_r1
        );
        let wrong_document = identity
            .require_binding("doc-other", "service-r1")
            .await
            .unwrap_err();
        assert_eq!(wrong_document.code, "canonical_revision_unbound");

        identity.close().await;
        revisions.close().await;

        let reopened = SqliteRevisionIdentityStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .require_binding("doc-a", "service-r1")
                .await
                .unwrap()
                .canonical_revision_id,
            canonical_r1
        );
        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn canonical_revision_cannot_bind_to_two_service_revisions() {
        let (identity, revisions, path) = migrated_store("canonical-unique").await;
        let canonical_r0 = "1".repeat(64);

        identity
            .bind_baseline("doc-a", "service-r0", &canonical_r0, 1)
            .await
            .unwrap();

        let conflict = identity
            .bind_baseline("doc-a", "service-other", &canonical_r0, 1)
            .await
            .unwrap_err();
        assert_eq!(conflict.code, "canonical_revision_binding_conflict");

        identity.close().await;
        revisions.close().await;
        cleanup(&path);
    }
}
