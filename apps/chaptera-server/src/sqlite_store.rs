use std::{
    fmt,
    path::{Path, PathBuf},
    str,
    time::Duration,
};

use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const MIGRATION_VERSION: i64 = 1;
pub const AUTHORING_REVISION_SCHEMA_V1: &str = "chaptera.cdm.authoring-revision.v1";
const EVENT_MAGIC: &[u8; 8] = b"CHREV2\0\0";
const EVENT_HASH_BYTES: usize = 32;
const EVENT_HEADER_BYTES: usize = EVENT_MAGIC.len() + 4 + EVENT_HASH_BYTES;
const MAX_CANONICAL_EVENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteStoreError {
    pub code: &'static str,
    pub message: String,
}

impl SqliteStoreError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SqliteStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SqliteStoreError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEdge {
    pub document_id: String,
    pub parent_revision: String,
    pub parent_cursor: i64,
    pub operation_id: String,
    pub request_hash: String,
    pub canonical_event: Vec<u8>,
    pub child_revision: String,
    pub child_cursor: i64,
    pub resulting_state_hash: String,
    pub authoring_root_hash: Option<String>,
    pub semantic_schema_version: i64,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendOutcome {
    Committed(RevisionEdge),
    AlreadyCommitted(RevisionEdge),
    Conflict(RevisionEdge),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionIdentityBinding {
    pub document_id: String,
    pub service_revision_id: String,
    pub canonical_schema_version: String,
    pub canonical_revision_id: String,
    pub bound_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionIdentityBindOutcome {
    Bound(RevisionIdentityBinding),
    AlreadyBound(RevisionIdentityBinding),
}

#[derive(Clone)]
pub struct SqliteRevisionStore {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteRevisionStore {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, SqliteStoreError> {
        if max_connections == 0 || max_connections > 16 {
            return Err(SqliteStoreError::new(
                "invalid_pool_size",
                "SQLite pool must use 1..=16 bounded connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(SqliteStoreError::new(
                "invalid_busy_timeout",
                "SQLite busy timeout must be >0 and <=30 seconds",
            ));
        }

        let path = path.as_ref().to_path_buf();
        if path.as_os_str().is_empty() {
            return Err(SqliteStoreError::new(
                "invalid_database_path",
                "SQLite database path must be non-empty",
            ));
        }

        if !path.exists() {
            return Err(SqliteStoreError::new(
                "sqlite_database_missing",
                "SQLite database must be created by chaptera migrate up before opening RevisionStream",
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
            .map_err(sqlite_open_error)?;

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

    pub async fn schema_version(&self) -> Result<i64, SqliteStoreError> {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_read_error)
    }

    pub async fn append_edge(&self, edge: RevisionEdge) -> Result<AppendOutcome, SqliteStoreError> {
        validate_edge(&edge)?;

        let result = sqlx::query(
            r#"
            INSERT INTO revision_edges (
                document_id,
                parent_revision,
                parent_cursor,
                operation_id,
                request_hash,
                canonical_event,
                child_revision,
                child_cursor,
                resulting_state_hash,
                authoring_root_hash,
                semantic_schema_version,
                committed_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(edge.document_id.as_bytes())
        .bind(edge.parent_revision.as_bytes())
        .bind(edge.parent_cursor)
        .bind(edge.operation_id.as_bytes())
        .bind(edge.request_hash.as_bytes())
        .bind(&edge.canonical_event)
        .bind(edge.child_revision.as_bytes())
        .bind(edge.child_cursor)
        .bind(edge.resulting_state_hash.as_bytes())
        .bind(edge.authoring_root_hash.as_ref().map(String::as_bytes))
        .bind(edge.semantic_schema_version)
        .bind(edge.committed_at_ms)
        .execute(&self.pool)
        .await;

        match result {
            Ok(done) if done.rows_affected() == 1 => Ok(AppendOutcome::Committed(edge)),
            Ok(_) => Err(SqliteStoreError::new(
                "sqlite_append_no_effect",
                "revision edge INSERT succeeded without creating one row",
            )),
            Err(write_error) => {
                if let Some(existing) = self
                    .read_edge(&edge.document_id, &edge.parent_revision)
                    .await?
                {
                    if same_retry_identity(&existing, &edge) {
                        return Ok(AppendOutcome::AlreadyCommitted(existing));
                    }
                    return Ok(AppendOutcome::Conflict(existing));
                }
                Err(SqliteStoreError::new(
                    "sqlite_append_failed",
                    bounded_sqlx_message(&write_error),
                ))
            }
        }
    }

    pub async fn read_edge(
        &self,
        document_id: &str,
        parent_revision: &str,
    ) -> Result<Option<RevisionEdge>, SqliteStoreError> {
        require_ident(document_id, "document_id")?;
        require_ident(parent_revision, "parent_revision")?;

        let row = sqlx::query(
            r#"
            SELECT
                document_id,
                parent_revision,
                parent_cursor,
                operation_id,
                request_hash,
                canonical_event,
                child_revision,
                child_cursor,
                resulting_state_hash,
                authoring_root_hash,
                semantic_schema_version,
                committed_at_ms
            FROM revision_edges
            WHERE document_id = ? AND parent_revision = ?
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(parent_revision.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        row.map(decode_edge_row).transpose()
    }

    pub async fn load_document_edges(
        &self,
        document_id: &str,
    ) -> Result<Vec<RevisionEdge>, SqliteStoreError> {
        require_ident(document_id, "document_id")?;

        let rows = sqlx::query(
            r#"
            SELECT
                document_id,
                parent_revision,
                parent_cursor,
                operation_id,
                request_hash,
                canonical_event,
                child_revision,
                child_cursor,
                resulting_state_hash,
                authoring_root_hash,
                semantic_schema_version,
                committed_at_ms
            FROM revision_edges
            WHERE document_id = ?
            ORDER BY child_cursor ASC, child_revision ASC
            "#,
        )
        .bind(document_id.as_bytes())
        .fetch_all(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        let mut edges = Vec::with_capacity(rows.len());
        for row in rows {
            edges.push(decode_edge_row(row)?);
        }
        verify_replay_chain(&edges)?;
        Ok(edges)
    }

    /// Load exactly the contiguous RevisionStream prefix from one authorized
    /// baseline to the requested revision.
    ///
    /// This deliberately does not decode or validate rows after the requested
    /// revision. Historical materialization must not silently become
    /// materialization of the current/latest head, and a later unrelated tail
    /// cannot change the bytes needed for an already named historical state.
    pub async fn load_chain_to_revision(
        &self,
        document_id: &str,
        baseline_revision: &str,
        baseline_cursor: i64,
        requested_revision: &str,
    ) -> Result<Vec<RevisionEdge>, SqliteStoreError> {
        require_ident(document_id, "document_id")?;
        require_ident(baseline_revision, "baseline_revision")?;
        require_ident(requested_revision, "requested_revision")?;
        if baseline_cursor < 0 {
            return Err(SqliteStoreError::new(
                "invalid_baseline_cursor",
                "baseline cursor must be non-negative",
            ));
        }
        if requested_revision == baseline_revision {
            return Ok(Vec::new());
        }

        let target_cursor = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT child_cursor
            FROM revision_edges
            WHERE document_id = ? AND child_revision = ?
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(requested_revision.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_read_error)?
        .ok_or_else(|| {
            SqliteStoreError::new(
                "requested_revision_not_found",
                "requested revision is not present in this document RevisionStream",
            )
        })?;

        if target_cursor <= baseline_cursor {
            return Err(SqliteStoreError::new(
                "requested_revision_before_baseline",
                "requested revision does not descend from the authorized baseline cursor",
            ));
        }

        let rows = sqlx::query(
            r#"
            SELECT
                document_id,
                parent_revision,
                parent_cursor,
                operation_id,
                request_hash,
                canonical_event,
                child_revision,
                child_cursor,
                resulting_state_hash,
                authoring_root_hash,
                semantic_schema_version,
                committed_at_ms
            FROM revision_edges
            WHERE document_id = ?
              AND child_cursor > ?
              AND child_cursor <= ?
            ORDER BY child_cursor ASC, child_revision ASC
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(baseline_cursor)
        .bind(target_cursor)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        let expected_len = usize::try_from(target_cursor - baseline_cursor).map_err(|_| {
            SqliteStoreError::new(
                "revision_chain_corrupt",
                "requested revision cursor distance does not fit memory bounds",
            )
        })?;
        if rows.len() != expected_len {
            return Err(SqliteStoreError::new(
                "revision_chain_corrupt",
                "RevisionStream prefix contains a cursor gap before the requested revision",
            ));
        }

        let mut edges = Vec::with_capacity(rows.len());
        for row in rows {
            edges.push(decode_edge_row(row)?);
        }

        let mut expected_parent = baseline_revision.to_owned();
        let mut expected_cursor = baseline_cursor;
        for edge in &edges {
            if edge.parent_revision != expected_parent
                || edge.parent_cursor != expected_cursor
                || edge.child_cursor != expected_cursor.saturating_add(1)
            {
                return Err(SqliteStoreError::new(
                    "revision_chain_corrupt",
                    "RevisionStream prefix is not one exact contiguous chain from the authorized baseline",
                ));
            }
            expected_parent.clone_from(&edge.child_revision);
            expected_cursor = edge.child_cursor;
        }

        if expected_parent != requested_revision || expected_cursor != target_cursor {
            return Err(SqliteStoreError::new(
                "revision_chain_corrupt",
                "RevisionStream prefix did not terminate at the exact requested revision",
            ));
        }

        Ok(edges)
    }

    pub async fn bind_revision_identity(
        &self,
        binding: RevisionIdentityBinding,
    ) -> Result<RevisionIdentityBindOutcome, SqliteStoreError> {
        validate_revision_identity_binding(&binding)?;

        let result = sqlx::query(
            r#"
            INSERT INTO revision_identity_bindings (
                document_id,
                service_revision_id,
                canonical_schema_version,
                canonical_revision_id,
                bound_at_ms
            ) VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(binding.document_id.as_bytes())
        .bind(binding.service_revision_id.as_bytes())
        .bind(&binding.canonical_schema_version)
        .bind(&binding.canonical_revision_id)
        .bind(binding.bound_at_ms)
        .execute(&self.pool)
        .await;

        match result {
            Ok(done) if done.rows_affected() == 1 => {
                Ok(RevisionIdentityBindOutcome::Bound(binding))
            }
            Ok(_) => Err(SqliteStoreError::new(
                "revision_identity_bind_no_effect",
                "revision identity INSERT succeeded without creating one row",
            )),
            Err(write_error) => {
                if let Some(existing) = self
                    .read_revision_identity(
                        &binding.document_id,
                        &binding.service_revision_id,
                    )
                    .await?
                {
                    if same_revision_identity(&existing, &binding) {
                        return Ok(RevisionIdentityBindOutcome::AlreadyBound(existing));
                    }
                    return Err(SqliteStoreError::new(
                        "revision_identity_conflict",
                        "service/history revision is already bound to a different canonical authoring revision",
                    ));
                }
                Err(SqliteStoreError::new(
                    "revision_identity_bind_failed",
                    bounded_sqlx_message(&write_error),
                ))
            }
        }
    }

    pub async fn read_revision_identity(
        &self,
        document_id: &str,
        service_revision_id: &str,
    ) -> Result<Option<RevisionIdentityBinding>, SqliteStoreError> {
        require_ident(document_id, "document_id")?;
        require_ident(service_revision_id, "service_revision_id")?;

        let row = sqlx::query(
            r#"
            SELECT
                document_id,
                service_revision_id,
                canonical_schema_version,
                canonical_revision_id,
                bound_at_ms
            FROM revision_identity_bindings
            WHERE document_id = ? AND service_revision_id = ?
            "#,
        )
        .bind(document_id.as_bytes())
        .bind(service_revision_id.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        row.map(decode_revision_identity_row).transpose()
    }

    pub async fn require_revision_identity(
        &self,
        document_id: &str,
        service_revision_id: &str,
    ) -> Result<RevisionIdentityBinding, SqliteStoreError> {
        self.read_revision_identity(document_id, service_revision_id)
            .await?
            .ok_or_else(|| {
                SqliteStoreError::new(
                    "canonical_revision_unbound",
                    "service/history revision has no explicit canonical AuthoringRevisionId binding",
                )
            })
    }

    async fn require_schema(&self) -> Result<(), SqliteStoreError> {
        for table in [
            "schema_migrations",
            "revision_edges",
            "revision_identity_bindings",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_read_error)?;

            if exists != 1 {
                return Err(SqliteStoreError::new(
                    "sqlite_schema_missing",
                    format!(
                        "required RevisionStream table {table} is absent; run chaptera migrate up before serving"
                    ),
                ));
            }
        }

        let current = self.schema_version().await?;
        if current != MIGRATION_VERSION {
            return Err(SqliteStoreError::new(
                "schema_version_mismatch",
                format!(
                    "RevisionStream schema version {current} does not match supported {MIGRATION_VERSION}; run chaptera migrate up with a compatible binary"
                ),
            ));
        }
        Ok(())
    }

    async fn verify_profile(&self) -> Result<(), SqliteStoreError> {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_read_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(SqliteStoreError::new(
                "sqlite_profile_mismatch",
                format!("expected WAL journal mode, got {journal_mode}"),
            ));
        }

        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_read_error)?;
        if synchronous != 2 {
            return Err(SqliteStoreError::new(
                "sqlite_profile_mismatch",
                format!("expected synchronous=FULL(2), got {synchronous}"),
            ));
        }

        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_read_error)?;
        if foreign_keys != 1 {
            return Err(SqliteStoreError::new(
                "sqlite_profile_mismatch",
                "foreign_keys pragma is not enabled",
            ));
        }
        Ok(())
    }
}

pub fn encode_canonical_event(payload: &[u8]) -> Result<Vec<u8>, SqliteStoreError> {
    if payload.is_empty() || payload.len() > MAX_CANONICAL_EVENT_BYTES {
        return Err(SqliteStoreError::new(
            "canonical_event_size_invalid",
            "canonical event payload must be non-empty and bounded",
        ));
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| {
        SqliteStoreError::new(
            "canonical_event_size_invalid",
            "canonical event payload does not fit V2 length field",
        )
    })?;
    let digest = Sha256::digest(payload);

    let mut encoded = Vec::with_capacity(EVENT_HEADER_BYTES + payload.len());
    encoded.extend_from_slice(EVENT_MAGIC);
    encoded.extend_from_slice(&payload_len.to_be_bytes());
    encoded.extend_from_slice(&digest);
    encoded.extend_from_slice(payload);
    Ok(encoded)
}

pub fn decode_canonical_event(encoded: &[u8]) -> Result<Vec<u8>, SqliteStoreError> {
    if encoded.len() < EVENT_HEADER_BYTES {
        return Err(SqliteStoreError::new(
            "canonical_event_corrupt",
            "canonical event is truncated before V2 header",
        ));
    }
    if &encoded[..EVENT_MAGIC.len()] != EVENT_MAGIC {
        return Err(SqliteStoreError::new(
            "canonical_event_corrupt",
            "canonical event V2 magic mismatch",
        ));
    }

    let length_start = EVENT_MAGIC.len();
    let length_end = length_start + 4;
    let payload_len =
        u32::from_be_bytes(encoded[length_start..length_end].try_into().map_err(|_| {
            SqliteStoreError::new(
                "canonical_event_corrupt",
                "canonical event length field is malformed",
            )
        })?) as usize;
    if payload_len == 0 || payload_len > MAX_CANONICAL_EVENT_BYTES {
        return Err(SqliteStoreError::new(
            "canonical_event_corrupt",
            "canonical event payload length is outside V2 bounds",
        ));
    }

    let hash_start = length_end;
    let hash_end = hash_start + EVENT_HASH_BYTES;
    let expected_total = EVENT_HEADER_BYTES.checked_add(payload_len).ok_or_else(|| {
        SqliteStoreError::new(
            "canonical_event_corrupt",
            "canonical event length overflows V2 envelope",
        )
    })?;
    if encoded.len() != expected_total {
        return Err(SqliteStoreError::new(
            "canonical_event_corrupt",
            "canonical event byte length differs from V2 envelope",
        ));
    }

    let payload = &encoded[EVENT_HEADER_BYTES..];
    let actual = Sha256::digest(payload);
    if actual[..] != encoded[hash_start..hash_end] {
        return Err(SqliteStoreError::new(
            "canonical_event_corrupt",
            "canonical event payload hash mismatch",
        ));
    }
    Ok(payload.to_vec())
}

fn validate_edge(edge: &RevisionEdge) -> Result<(), SqliteStoreError> {
    require_ident(&edge.document_id, "document_id")?;
    require_ident(&edge.parent_revision, "parent_revision")?;
    require_ident(&edge.operation_id, "operation_id")?;
    require_ident(&edge.child_revision, "child_revision")?;
    require_sha256(&edge.request_hash, "request_hash")?;
    require_sha256(&edge.resulting_state_hash, "resulting_state_hash")?;
    if let Some(root) = &edge.authoring_root_hash {
        require_sha256(root, "authoring_root_hash")?;
    }
    if edge.parent_cursor < 0
        || edge.child_cursor != edge.parent_cursor.saturating_add(1)
        || edge.semantic_schema_version <= 0
        || edge.committed_at_ms < 0
    {
        return Err(SqliteStoreError::new(
            "revision_edge_invalid",
            "revision cursors/schema/time violate V2 bounds",
        ));
    }
    decode_canonical_event(&edge.canonical_event)?;
    Ok(())
}

fn validate_revision_identity_binding(
    binding: &RevisionIdentityBinding,
) -> Result<(), SqliteStoreError> {
    require_ident(&binding.document_id, "document_id")?;
    require_ident(&binding.service_revision_id, "service_revision_id")?;
    if binding.canonical_schema_version != AUTHORING_REVISION_SCHEMA_V1 {
        return Err(SqliteStoreError::new(
            "unsupported_canonical_revision_schema",
            format!(
                "canonical revision schema {:?} is unsupported",
                binding.canonical_schema_version
            ),
        ));
    }
    require_sha256(
        &binding.canonical_revision_id,
        "canonical_revision_id",
    )?;
    if binding.bound_at_ms < 0 {
        return Err(SqliteStoreError::new(
            "revision_identity_invalid",
            "revision identity bound_at_ms must be non-negative",
        ));
    }
    Ok(())
}

fn same_revision_identity(
    existing: &RevisionIdentityBinding,
    requested: &RevisionIdentityBinding,
) -> bool {
    existing.document_id == requested.document_id
        && existing.service_revision_id == requested.service_revision_id
        && existing.canonical_schema_version == requested.canonical_schema_version
        && existing.canonical_revision_id == requested.canonical_revision_id
}

fn decode_revision_identity_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<RevisionIdentityBinding, SqliteStoreError> {
    let binding = RevisionIdentityBinding {
        document_id: blob_text(&row, "document_id")?,
        service_revision_id: blob_text(&row, "service_revision_id")?,
        canonical_schema_version: row
            .try_get("canonical_schema_version")
            .map_err(sqlite_decode_error)?,
        canonical_revision_id: row
            .try_get("canonical_revision_id")
            .map_err(sqlite_decode_error)?,
        bound_at_ms: row.try_get("bound_at_ms").map_err(sqlite_decode_error)?,
    };
    validate_revision_identity_binding(&binding)?;
    Ok(binding)
}

fn same_retry_identity(existing: &RevisionEdge, requested: &RevisionEdge) -> bool {
    existing.operation_id == requested.operation_id
        && existing.request_hash == requested.request_hash
        && existing.canonical_event == requested.canonical_event
        && existing.child_revision == requested.child_revision
        && existing.child_cursor == requested.child_cursor
        && existing.resulting_state_hash == requested.resulting_state_hash
        && existing.authoring_root_hash == requested.authoring_root_hash
        && existing.semantic_schema_version == requested.semantic_schema_version
}

fn verify_replay_chain(edges: &[RevisionEdge]) -> Result<(), SqliteStoreError> {
    for edge in edges {
        validate_edge(edge)?;
    }
    for pair in edges.windows(2) {
        if pair[1].parent_cursor != pair[0].child_cursor
            || pair[1].parent_revision != pair[0].child_revision
        {
            return Err(SqliteStoreError::new(
                "revision_chain_corrupt",
                "stored revision edges are not one exact contiguous chain",
            ));
        }
    }
    Ok(())
}

fn decode_edge_row(row: sqlx::sqlite::SqliteRow) -> Result<RevisionEdge, SqliteStoreError> {
    let edge = RevisionEdge {
        document_id: blob_text(&row, "document_id")?,
        parent_revision: blob_text(&row, "parent_revision")?,
        parent_cursor: row.try_get("parent_cursor").map_err(sqlite_decode_error)?,
        operation_id: blob_text(&row, "operation_id")?,
        request_hash: blob_text(&row, "request_hash")?,
        canonical_event: row
            .try_get("canonical_event")
            .map_err(sqlite_decode_error)?,
        child_revision: blob_text(&row, "child_revision")?,
        child_cursor: row.try_get("child_cursor").map_err(sqlite_decode_error)?,
        resulting_state_hash: blob_text(&row, "resulting_state_hash")?,
        authoring_root_hash: optional_blob_text(&row, "authoring_root_hash")?,
        semantic_schema_version: row
            .try_get("semantic_schema_version")
            .map_err(sqlite_decode_error)?,
        committed_at_ms: row
            .try_get("committed_at_ms")
            .map_err(sqlite_decode_error)?,
    };
    validate_edge(&edge)?;
    Ok(edge)
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, SqliteStoreError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_decode_error)?;
    str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| SqliteStoreError::new("sqlite_row_corrupt", format!("{column} is not UTF-8")))
}

fn optional_blob_text(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<String>, SqliteStoreError> {
    let bytes: Option<Vec<u8>> = row.try_get(column).map_err(sqlite_decode_error)?;
    bytes
        .map(|value| {
            str::from_utf8(&value).map(str::to_owned).map_err(|_| {
                SqliteStoreError::new("sqlite_row_corrupt", format!("{column} is not UTF-8"))
            })
        })
        .transpose()
}

fn require_ident(value: &str, label: &'static str) -> Result<(), SqliteStoreError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(SqliteStoreError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_sha256(value: &str, label: &'static str) -> Result<(), SqliteStoreError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SqliteStoreError::new(
            "invalid_hash",
            format!("{label} must be lowercase SHA-256"),
        ));
    }
    Ok(())
}

fn bounded_sqlx_message(error: &sqlx::Error) -> String {
    let text = error.to_string();
    text.chars().take(256).collect()
}

fn sqlite_open_error(error: sqlx::Error) -> SqliteStoreError {
    SqliteStoreError::new("sqlite_open_failed", bounded_sqlx_message(&error))
}

fn sqlite_read_error(error: sqlx::Error) -> SqliteStoreError {
    SqliteStoreError::new("sqlite_read_failed", bounded_sqlx_message(&error))
}

fn sqlite_decode_error(error: sqlx::Error) -> SqliteStoreError {
    SqliteStoreError::new("sqlite_row_corrupt", bounded_sqlx_message(&error))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use tokio::sync::Barrier;

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = TEST_ID.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-sqlite-{label}-{}-{serial}.db",
            std::process::id()
        ))
    }

    fn hash(ch: u8) -> String {
        std::iter::repeat_n(char::from(ch), 64).collect()
    }

    fn edge(
        document: &str,
        parent: &str,
        child: &str,
        operation: &str,
        request_hash: &str,
        payload: &[u8],
        parent_cursor: i64,
    ) -> RevisionEdge {
        RevisionEdge {
            document_id: document.into(),
            parent_revision: parent.into(),
            parent_cursor,
            operation_id: operation.into(),
            request_hash: request_hash.into(),
            canonical_event: encode_canonical_event(payload).unwrap(),
            child_revision: child.into(),
            child_cursor: parent_cursor + 1,
            resulting_state_hash: hash(b'c'),
            authoring_root_hash: Some(hash(b'd')),
            semantic_schema_version: 1,
            committed_at_ms: 1_000 + parent_cursor,
        }
    }

    fn identity(
        document_id: &str,
        service_revision_id: &str,
        canonical: u8,
        bound_at_ms: i64,
    ) -> RevisionIdentityBinding {
        RevisionIdentityBinding {
            document_id: document_id.into(),
            service_revision_id: service_revision_id.into(),
            canonical_schema_version: AUTHORING_REVISION_SCHEMA_V1.into(),
            canonical_revision_id: hash(canonical),
            bound_at_ms,
        }
    }

    async fn store(path: &Path) -> SqliteRevisionStore {
        SqliteMigrationRuntime::new(path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        SqliteRevisionStore::open(path, 4, Duration::from_secs(2))
            .await
            .unwrap()
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn open_requires_operator_migration_and_preserves_profile() {
        let path = temp_db("profile");
        let error = match SqliteRevisionStore::open(&path, 4, Duration::from_secs(2)).await {
            Ok(_) => panic!("RevisionStream opened without operator migration"),
            Err(error) => error,
        };
        assert_eq!(error.code, "sqlite_database_missing");
        assert!(!path.exists());

        let store = store(&path).await;
        assert_eq!(store.schema_version().await.unwrap(), MIGRATION_VERSION);
        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn exact_retry_returns_same_accepted_edge_and_changed_retry_conflicts() {
        let path = temp_db("retry");
        let store = store(&path).await;
        let first = edge(
            "doc-a",
            "rev-0",
            "rev-1",
            "op-1",
            &hash(b'a'),
            br#"{"kind":"move"}"#,
            0,
        );

        assert!(matches!(
            store.append_edge(first.clone()).await.unwrap(),
            AppendOutcome::Committed(_)
        ));
        assert_eq!(
            store.append_edge(first.clone()).await.unwrap(),
            AppendOutcome::AlreadyCommitted(first.clone())
        );

        let mut changed = first.clone();
        changed.operation_id = "op-2".into();
        changed.request_hash = hash(b'b');
        changed.child_revision = "rev-other".into();
        assert_eq!(
            store.append_edge(changed).await.unwrap(),
            AppendOutcome::Conflict(first)
        );

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn same_parent_writer_race_yields_exactly_one_successor() {
        let path = temp_db("race");
        let store = store(&path).await;
        let barrier = Arc::new(Barrier::new(3));

        let left_store = store.clone();
        let left_barrier = barrier.clone();
        let left = tokio::spawn(async move {
            left_barrier.wait().await;
            left_store
                .append_edge(edge(
                    "doc-race",
                    "rev-0",
                    "rev-left",
                    "op-left",
                    &hash(b'a'),
                    b"left",
                    0,
                ))
                .await
                .unwrap()
        });

        let right_store = store.clone();
        let right_barrier = barrier.clone();
        let right = tokio::spawn(async move {
            right_barrier.wait().await;
            right_store
                .append_edge(edge(
                    "doc-race",
                    "rev-0",
                    "rev-right",
                    "op-right",
                    &hash(b'b'),
                    b"right",
                    0,
                ))
                .await
                .unwrap()
        });

        barrier.wait().await;
        let left = left.await.unwrap();
        let right = right.await.unwrap();

        let committed = [&left, &right]
            .iter()
            .filter(|outcome| matches!(outcome, AppendOutcome::Committed(_)))
            .count();
        let conflicts = [&left, &right]
            .iter()
            .filter(|outcome| matches!(outcome, AppendOutcome::Conflict(_)))
            .count();
        assert_eq!(committed, 1);
        assert_eq!(conflicts, 1);
        assert_eq!(
            store.load_document_edges("doc-race").await.unwrap().len(),
            1
        );

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn durable_commit_reconciles_after_reopen_before_ack() {
        let path = temp_db("reopen");
        let accepted = edge(
            "doc-reopen",
            "rev-0",
            "rev-1",
            "op-1",
            &hash(b'a'),
            b"durable-before-ack",
            0,
        );

        {
            let store = store(&path).await;
            assert!(matches!(
                store.append_edge(accepted.clone()).await.unwrap(),
                AppendOutcome::Committed(_)
            ));
            store.close().await;
        }

        let reopened = store(&path).await;
        assert_eq!(
            reopened.append_edge(accepted.clone()).await.unwrap(),
            AppendOutcome::AlreadyCommitted(accepted.clone())
        );
        assert_eq!(
            reopened.load_document_edges("doc-reopen").await.unwrap(),
            vec![accepted]
        );
        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn corrupt_or_truncated_canonical_event_fails_closed_on_replay() {
        let path = temp_db("corrupt");
        let store = store(&path).await;

        sqlx::query(
            r#"
            INSERT INTO revision_edges (
                document_id, parent_revision, parent_cursor, operation_id,
                request_hash, canonical_event, child_revision, child_cursor,
                resulting_state_hash, authoring_root_hash,
                semantic_schema_version, committed_at_ms
            ) VALUES (?, ?, 0, ?, ?, ?, ?, 1, ?, NULL, 1, 1000)
            "#,
        )
        .bind(b"doc-corrupt".as_slice())
        .bind(b"rev-0".as_slice())
        .bind(b"op-1".as_slice())
        .bind(hash(b'a').as_bytes())
        .bind(vec![1_u8, 2, 3])
        .bind(b"rev-1".as_slice())
        .bind(hash(b'c').as_bytes())
        .execute(&store.pool)
        .await
        .unwrap();

        let error = store.load_document_edges("doc-corrupt").await.unwrap_err();
        assert_eq!(error.code, "canonical_event_corrupt");

        store.close().await;
        cleanup(&path);
    }

    #[test]
    fn canonical_event_codec_detects_truncation_and_corruption() {
        let encoded = encode_canonical_event(b"canonical-event").unwrap();
        assert_eq!(
            decode_canonical_event(&encoded).unwrap(),
            b"canonical-event"
        );

        let mut corrupt = encoded.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x01;
        assert_eq!(
            decode_canonical_event(&corrupt).unwrap_err().code,
            "canonical_event_corrupt"
        );
        assert_eq!(
            decode_canonical_event(&encoded[..encoded.len() - 1])
                .unwrap_err()
                .code,
            "canonical_event_corrupt"
        );
    }

    #[tokio::test]
    async fn revision_identity_binding_is_idempotent_conflict_safe_and_restart_durable() {
        let path = temp_db("revision-identity");
        let store = store(&path).await;
        let first = identity("doc-id", "service-r1", b'a', 100);

        assert_eq!(
            store.bind_revision_identity(first.clone()).await.unwrap(),
            RevisionIdentityBindOutcome::Bound(first.clone())
        );

        let mut retry = first.clone();
        retry.bound_at_ms = 999;
        assert_eq!(
            store.bind_revision_identity(retry).await.unwrap(),
            RevisionIdentityBindOutcome::AlreadyBound(first.clone())
        );

        let conflict = identity("doc-id", "service-r1", b'b', 101);
        let error = store.bind_revision_identity(conflict).await.unwrap_err();
        assert_eq!(error.code, "revision_identity_conflict");

        store.close().await;
        let reopened = SqliteRevisionStore::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .require_revision_identity("doc-id", "service-r1")
                .await
                .unwrap(),
            first
        );
        reopened.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn revision_identity_binding_is_document_scoped_and_historical() {
        let path = temp_db("revision-identity-history");
        let store = store(&path).await;

        let doc_a_r1 = identity("doc-a", "service-r1", b'a', 1);
        let doc_a_r2 = identity("doc-a", "service-r2", b'b', 2);
        let doc_b_r1 = identity("doc-b", "service-r1", b'c', 3);

        for binding in [&doc_a_r1, &doc_a_r2, &doc_b_r1] {
            assert!(matches!(
                store
                    .bind_revision_identity(binding.clone())
                    .await
                    .unwrap(),
                RevisionIdentityBindOutcome::Bound(_)
            ));
        }

        assert_eq!(
            store
                .require_revision_identity("doc-a", "service-r1")
                .await
                .unwrap(),
            doc_a_r1
        );
        assert_eq!(
            store
                .require_revision_identity("doc-a", "service-r2")
                .await
                .unwrap(),
            doc_a_r2
        );
        assert_eq!(
            store
                .require_revision_identity("doc-b", "service-r1")
                .await
                .unwrap(),
            doc_b_r1
        );
        assert_eq!(
            store
                .require_revision_identity("doc-a", "missing")
                .await
                .unwrap_err()
                .code,
            "canonical_revision_unbound"
        );

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn restart_replay_preserves_exact_order_and_bytes() {
        let path = temp_db("chain");
        let first = edge(
            "doc-chain",
            "rev-0",
            "rev-1",
            "op-1",
            &hash(b'a'),
            b"first",
            0,
        );
        let second = edge(
            "doc-chain",
            "rev-1",
            "rev-2",
            "op-2",
            &hash(b'b'),
            b"second",
            1,
        );

        {
            let store = store(&path).await;
            store.append_edge(first.clone()).await.unwrap();
            store.append_edge(second.clone()).await.unwrap();
            store.close().await;
        }

        let reopened = store(&path).await;
        let loaded = reopened.load_document_edges("doc-chain").await.unwrap();
        assert_eq!(loaded, vec![first, second]);
        reopened.close().await;
        cleanup(&path);
    }
}
