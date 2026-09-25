use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceContextError {
    pub code: &'static str,
    pub message: String,
}

impl WorkspaceContextError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for WorkspaceContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WorkspaceContextError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceContext {
    pub workspace_id: String,
    pub tenant_id: String,
    pub principal_id: String,
}

#[derive(Clone)]
pub struct SqliteWorkspaceContextAuthority {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteWorkspaceContextAuthority {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, WorkspaceContextError> {
        if path.as_ref().as_os_str().is_empty() {
            return Err(WorkspaceContextError::new(
                "workspace_context_database_path_invalid",
                "workspace context SQLite path must be non-empty",
            ));
        }
        if max_connections == 0 || max_connections > 16 {
            return Err(WorkspaceContextError::new(
                "workspace_context_pool_size_invalid",
                "workspace context SQLite pool must use 1..=16 connections",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(WorkspaceContextError::new(
                "workspace_context_busy_timeout_invalid",
                "workspace context SQLite busy timeout must be >0 and <=30 seconds",
            ));
        }
        if !path.as_ref().exists() {
            return Err(WorkspaceContextError::new(
                "workspace_context_database_missing",
                "run chaptera migrate up before opening workspace context authority",
            ));
        }

        let path = path.as_ref().to_path_buf();
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

    pub async fn ensure_workspace(
        &self,
        workspace_id: &str,
        tenant_id: &str,
        now_ms: u64,
    ) -> Result<(), WorkspaceContextError> {
        require_ident(workspace_id, "workspace_id")?;
        require_ident(tenant_id, "tenant_id")?;
        let now_ms = to_i64(now_ms, "now_ms")?;

        let mut tx = self.pool.begin().await.map_err(sqlite_error)?;
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO workspaces(
                workspace_id, tenant_id, lifecycle_state, created_at_ms, updated_at_ms
            ) VALUES (?, ?, 'active', ?, ?)
            "#,
        )
        .bind(workspace_id.as_bytes())
        .bind(tenant_id.as_bytes())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        let row = sqlx::query(
            r#"
            SELECT tenant_id, lifecycle_state
            FROM workspaces
            WHERE workspace_id = ?
            LIMIT 2
            "#,
        )
        .bind(workspace_id.as_bytes())
        .fetch_all(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        if row.len() != 1 {
            return Err(WorkspaceContextError::new(
                "workspace_identity_ambiguous",
                "workspace identity did not resolve to exactly one durable row",
            ));
        }
        let current_tenant = blob_text(&row[0], "tenant_id")?;
        if current_tenant != tenant_id {
            return Err(WorkspaceContextError::new(
                "workspace_tenant_conflict",
                "workspace id is already bound to a different tenant",
            ));
        }
        let lifecycle: String = row[0].try_get("lifecycle_state").map_err(sqlite_error)?;
        if lifecycle != "active" {
            return Err(WorkspaceContextError::new(
                "workspace_deleted",
                "deleted workspace cannot be reused or remapped",
            ));
        }

        tx.commit().await.map_err(sqlite_error)?;
        Ok(())
    }

    pub async fn grant_membership(
        &self,
        workspace_id: &str,
        principal_id: &str,
        now_ms: u64,
    ) -> Result<(), WorkspaceContextError> {
        require_ident(workspace_id, "workspace_id")?;
        require_ident(principal_id, "principal_id")?;
        let now_ms = to_i64(now_ms, "now_ms")?;

        let mut tx = self.pool.begin().await.map_err(sqlite_error)?;
        require_active_workspace(&mut tx, workspace_id).await?;
        require_active_principal(&mut tx, principal_id).await?;

        sqlx::query(
            r#"
            INSERT INTO workspace_memberships(
                workspace_id, principal_id, state, created_at_ms, updated_at_ms, revoked_at_ms
            ) VALUES (?, ?, 'active', ?, ?, NULL)
            ON CONFLICT(workspace_id, principal_id)
            DO UPDATE SET
                state='active',
                updated_at_ms=excluded.updated_at_ms,
                revoked_at_ms=NULL
            "#,
        )
        .bind(workspace_id.as_bytes())
        .bind(principal_id.as_bytes())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(sqlite_error)?;

        tx.commit().await.map_err(sqlite_error)?;
        Ok(())
    }

    pub async fn revoke_membership(
        &self,
        workspace_id: &str,
        principal_id: &str,
        now_ms: u64,
    ) -> Result<(), WorkspaceContextError> {
        require_ident(workspace_id, "workspace_id")?;
        require_ident(principal_id, "principal_id")?;
        let now_ms = to_i64(now_ms, "now_ms")?;

        let result = sqlx::query(
            r#"
            UPDATE workspace_memberships
            SET state='revoked', updated_at_ms=?, revoked_at_ms=?
            WHERE workspace_id=? AND principal_id=? AND state='active'
            "#,
        )
        .bind(now_ms)
        .bind(now_ms)
        .bind(workspace_id.as_bytes())
        .bind(principal_id.as_bytes())
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if result.rows_affected() != 1 {
            return Err(WorkspaceContextError::new(
                "workspace_membership_not_active",
                "workspace membership is absent or already revoked",
            ));
        }
        Ok(())
    }

    pub async fn delete_workspace(
        &self,
        workspace_id: &str,
        now_ms: u64,
    ) -> Result<(), WorkspaceContextError> {
        require_ident(workspace_id, "workspace_id")?;
        let now_ms = to_i64(now_ms, "now_ms")?;

        let result = sqlx::query(
            r#"
            UPDATE workspaces
            SET lifecycle_state='deleted', updated_at_ms=?
            WHERE workspace_id=? AND lifecycle_state='active'
            "#,
        )
        .bind(now_ms)
        .bind(workspace_id.as_bytes())
        .execute(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if result.rows_affected() != 1 {
            return Err(WorkspaceContextError::new(
                "workspace_not_active",
                "workspace is absent or already deleted",
            ));
        }
        Ok(())
    }

    pub async fn resolve(
        &self,
        principal_id: &str,
        workspace_id: &str,
    ) -> Result<WorkspaceContext, WorkspaceContextError> {
        require_ident(principal_id, "principal_id")?;
        require_ident(workspace_id, "workspace_id")?;

        let rows = sqlx::query(
            r#"
            SELECT
                w.tenant_id,
                w.lifecycle_state AS workspace_state,
                m.state AS membership_state,
                p.disabled_at_ms
            FROM workspaces w
            JOIN workspace_memberships m
              ON m.workspace_id = w.workspace_id
            JOIN principals p
              ON p.principal_id = m.principal_id
            WHERE w.workspace_id = ?
              AND m.principal_id = ?
            LIMIT 2
            "#,
        )
        .bind(workspace_id.as_bytes())
        .bind(principal_id.as_bytes())
        .fetch_all(&self.pool)
        .await
        .map_err(sqlite_error)?;

        if rows.is_empty() {
            return Err(WorkspaceContextError::new(
                "workspace_membership_missing",
                "authenticated principal has no membership in the selected workspace",
            ));
        }
        if rows.len() != 1 {
            return Err(WorkspaceContextError::new(
                "workspace_membership_ambiguous",
                "workspace membership resolved to multiple durable rows",
            ));
        }

        let row = &rows[0];
        let workspace_state: String = row.try_get("workspace_state").map_err(sqlite_error)?;
        if workspace_state != "active" {
            return Err(WorkspaceContextError::new(
                "workspace_deleted",
                "selected workspace is not active",
            ));
        }
        let membership_state: String = row.try_get("membership_state").map_err(sqlite_error)?;
        if membership_state != "active" {
            return Err(WorkspaceContextError::new(
                "workspace_membership_revoked",
                "authenticated principal membership is not active",
            ));
        }
        let disabled_at_ms: Option<i64> = row.try_get("disabled_at_ms").map_err(sqlite_error)?;
        if disabled_at_ms.is_some() {
            return Err(WorkspaceContextError::new(
                "principal_disabled",
                "disabled principal cannot resolve workspace context",
            ));
        }

        Ok(WorkspaceContext {
            workspace_id: workspace_id.to_owned(),
            tenant_id: blob_text(row, "tenant_id")?,
            principal_id: principal_id.to_owned(),
        })
    }

    async fn require_schema(&self) -> Result<(), WorkspaceContextError> {
        for table in ["workspaces", "workspace_memberships", "principals"] {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if count != 1 {
                return Err(WorkspaceContextError::new(
                    "workspace_context_schema_missing",
                    format!("{table} table is absent; run chaptera migrate up"),
                ));
            }
        }
        Ok(())
    }
}

async fn require_active_workspace(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    workspace_id: &str,
) -> Result<(), WorkspaceContextError> {
    let row = sqlx::query(
        "SELECT lifecycle_state FROM workspaces WHERE workspace_id=? LIMIT 2",
    )
    .bind(workspace_id.as_bytes())
    .fetch_all(&mut **tx)
    .await
    .map_err(sqlite_error)?;

    if row.is_empty() {
        return Err(WorkspaceContextError::new(
            "workspace_not_found",
            "workspace does not exist",
        ));
    }
    if row.len() != 1 {
        return Err(WorkspaceContextError::new(
            "workspace_identity_ambiguous",
            "workspace identity resolved to multiple durable rows",
        ));
    }
    let state: String = row[0].try_get("lifecycle_state").map_err(sqlite_error)?;
    if state != "active" {
        return Err(WorkspaceContextError::new(
            "workspace_deleted",
            "deleted workspace cannot accept memberships",
        ));
    }
    Ok(())
}

async fn require_active_principal(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    principal_id: &str,
) -> Result<(), WorkspaceContextError> {
    let row = sqlx::query(
        "SELECT disabled_at_ms FROM principals WHERE principal_id=? LIMIT 2",
    )
    .bind(principal_id.as_bytes())
    .fetch_all(&mut **tx)
    .await
    .map_err(sqlite_error)?;

    if row.is_empty() {
        return Err(WorkspaceContextError::new(
            "principal_not_found",
            "principal does not exist",
        ));
    }
    if row.len() != 1 {
        return Err(WorkspaceContextError::new(
            "principal_identity_ambiguous",
            "principal identity resolved to multiple durable rows",
        ));
    }
    let disabled_at_ms: Option<i64> = row[0].try_get("disabled_at_ms").map_err(sqlite_error)?;
    if disabled_at_ms.is_some() {
        return Err(WorkspaceContextError::new(
            "principal_disabled",
            "disabled principal cannot receive workspace membership",
        ));
    }
    Ok(())
}

fn require_ident(value: &str, label: &'static str) -> Result<(), WorkspaceContextError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(WorkspaceContextError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn to_i64(value: u64, label: &'static str) -> Result<i64, WorkspaceContextError> {
    i64::try_from(value).map_err(|_| {
        WorkspaceContextError::new(
            "workspace_context_integer_overflow",
            format!("{label} exceeds SQLite signed integer range"),
        )
    })
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, WorkspaceContextError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_error)?;
    String::from_utf8(bytes).map_err(|_| {
        WorkspaceContextError::new(
            "workspace_context_row_corrupt",
            format!("{column} is not UTF-8"),
        )
    })
}

fn sqlite_error(error: impl fmt::Display) -> WorkspaceContextError {
    WorkspaceContextError::new(
        "sqlite_workspace_context_error",
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

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-workspace-context-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn setup(
        label: &str,
    ) -> (PathBuf, SqliteWorkspaceContextAuthority, SqlitePool) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let authority =
            SqliteWorkspaceContextAuthority::open(&path, 4, Duration::from_secs(2))
                .await
                .unwrap();
        let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        (path, authority, pool)
    }

    async fn seed_principal(pool: &SqlitePool, principal_id: &str) {
        sqlx::query(
            "INSERT INTO principals(principal_id, created_at_ms, disabled_at_ms) VALUES (?, 1, NULL)",
        )
        .bind(principal_id.as_bytes())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn active_membership_resolves_authoritative_tenant_after_reopen() {
        let (path, authority, pool) = setup("resolve").await;
        seed_principal(&pool, "principal:1").await;
        authority
            .ensure_workspace("workspace:1", "tenant:1", 10)
            .await
            .unwrap();
        authority
            .grant_membership("workspace:1", "principal:1", 11)
            .await
            .unwrap();

        let resolved = authority
            .resolve("principal:1", "workspace:1")
            .await
            .unwrap();
        assert_eq!(
            resolved,
            WorkspaceContext {
                workspace_id: "workspace:1".into(),
                tenant_id: "tenant:1".into(),
                principal_id: "principal:1".into(),
            }
        );

        authority.close().await;
        let reopened =
            SqliteWorkspaceContextAuthority::open(&path, 4, Duration::from_secs(2))
                .await
                .unwrap();
        assert_eq!(
            reopened
                .resolve("principal:1", "workspace:1")
                .await
                .unwrap(),
            resolved
        );

        reopened.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn workspace_id_cannot_be_remapped_to_another_tenant() {
        let (path, authority, pool) = setup("tenant-conflict").await;
        authority
            .ensure_workspace("workspace:1", "tenant:1", 10)
            .await
            .unwrap();
        let error = authority
            .ensure_workspace("workspace:1", "tenant:2", 11)
            .await
            .unwrap_err();
        assert_eq!(error.code, "workspace_tenant_conflict");

        authority.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn revoked_disabled_and_deleted_contexts_fail_closed() {
        let (path, authority, pool) = setup("fail-closed").await;
        seed_principal(&pool, "principal:1").await;
        authority
            .ensure_workspace("workspace:1", "tenant:1", 10)
            .await
            .unwrap();
        authority
            .grant_membership("workspace:1", "principal:1", 11)
            .await
            .unwrap();

        authority
            .revoke_membership("workspace:1", "principal:1", 12)
            .await
            .unwrap();
        assert_eq!(
            authority
                .resolve("principal:1", "workspace:1")
                .await
                .unwrap_err()
                .code,
            "workspace_membership_revoked"
        );

        authority
            .grant_membership("workspace:1", "principal:1", 13)
            .await
            .unwrap();
        sqlx::query("UPDATE principals SET disabled_at_ms=14 WHERE principal_id=?")
            .bind(b"principal:1".as_slice())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            authority
                .resolve("principal:1", "workspace:1")
                .await
                .unwrap_err()
                .code,
            "principal_disabled"
        );

        sqlx::query("UPDATE principals SET disabled_at_ms=NULL WHERE principal_id=?")
            .bind(b"principal:1".as_slice())
            .execute(&pool)
            .await
            .unwrap();
        authority.delete_workspace("workspace:1", 15).await.unwrap();
        assert_eq!(
            authority
                .resolve("principal:1", "workspace:1")
                .await
                .unwrap_err()
                .code,
            "workspace_deleted"
        );

        authority.close().await;
        pool.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn missing_membership_and_disabled_grant_are_rejected() {
        let (path, authority, pool) = setup("missing").await;
        seed_principal(&pool, "principal:1").await;
        authority
            .ensure_workspace("workspace:1", "tenant:1", 10)
            .await
            .unwrap();

        assert_eq!(
            authority
                .resolve("principal:1", "workspace:1")
                .await
                .unwrap_err()
                .code,
            "workspace_membership_missing"
        );

        sqlx::query("UPDATE principals SET disabled_at_ms=11 WHERE principal_id=?")
            .bind(b"principal:1".as_slice())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            authority
                .grant_membership("workspace:1", "principal:1", 12)
                .await
                .unwrap_err()
                .code,
            "principal_disabled"
        );

        authority.close().await;
        pool.close().await;
        cleanup(&path);
    }
}
