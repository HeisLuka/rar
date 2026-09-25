use std::{
    fmt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sqlx::{
    Row, Sqlite, SqlitePool,
    pool::PoolConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{
    export_executor::{
        ExportExecutorError, ExportJobPayloadV1, ExportPublicationCommitFuture,
        ExportPublicationCommitter, ExportPublishAuthFuture, ExportPublishAuthorizer,
    },
    export_publication::{
        ExportPublicationInputV1, ExportPublicationPrepareOutcomeV1, SqliteExportPublicationStore,
    },
    job_queue::{JobKind, JobRecord},
};

pub const CAP_VIEW: &str = "document.view";
pub const CAP_COMMENT_READ: &str = "comment.read";
pub const CAP_COMMENT_WRITE: &str = "comment.write";
pub const CAP_EDIT: &str = "document.edit";
pub const CAP_EDIT_TEXT: &str = "document.edit_text";
pub const CAP_EDIT_GEOMETRY: &str = "document.edit_geometry";
pub const CAP_ASSET_UPLOAD: &str = "asset.upload";
pub const CAP_EXPORT: &str = "document.export";
pub const CAP_SHARE_MANAGE: &str = "share.manage";
pub const CAP_MEMBER_MANAGE: &str = "member.manage";
pub const CAP_DELETE: &str = "document.delete";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentRole {
    Viewer,
    Commenter,
    Editor,
    Owner,
}

impl DocumentRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Commenter => "commenter",
            Self::Editor => "editor",
            Self::Owner => "owner",
        }
    }

    fn parse(value: &str) -> Result<Self, AuthzError> {
        match value {
            "viewer" => Ok(Self::Viewer),
            "commenter" => Ok(Self::Commenter),
            "editor" => Ok(Self::Editor),
            "owner" => Ok(Self::Owner),
            _ => Err(AuthzError::new(
                "authz_row_corrupt",
                "persisted role is outside the canonical role set",
            )),
        }
    }

    fn allows(self, capability: &str) -> bool {
        match self {
            Self::Viewer => matches!(capability, CAP_VIEW),
            Self::Commenter => {
                matches!(capability, CAP_VIEW | CAP_COMMENT_READ | CAP_COMMENT_WRITE)
            }
            Self::Editor => matches!(
                capability,
                CAP_VIEW
                    | CAP_COMMENT_READ
                    | CAP_COMMENT_WRITE
                    | CAP_EDIT
                    | CAP_EDIT_TEXT
                    | CAP_EDIT_GEOMETRY
                    | CAP_ASSET_UPLOAD
                    | CAP_EXPORT
            ),
            Self::Owner => known_capability(capability),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzDecision {
    pub tenant_id: String,
    pub document_id: String,
    pub principal_id: String,
    pub capability: String,
    pub role: DocumentRole,
    pub authz_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzMutationReceipt {
    pub tenant_id: String,
    pub document_id: String,
    pub authz_version: i64,
    pub active_session_barrier_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzError {
    pub code: &'static str,
    pub message: String,
}

impl AuthzError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AuthzError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AuthzError {}

#[derive(Debug)]
struct AuthorizationDenied {
    code: &'static str,
    message: &'static str,
    authz_version: i64,
}

#[derive(Debug)]
enum AuthorizationCheckError {
    Denied(AuthorizationDenied),
    Internal(AuthzError),
}

#[derive(Clone)]
pub struct SqliteAuthzAuthority {
    path: PathBuf,
    pool: SqlitePool,
}

impl SqliteAuthzAuthority {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, AuthzError> {
        validate_pool(max_connections, busy_timeout)?;
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(AuthzError::new(
                "sqlite_database_missing",
                "run chaptera migrate up before opening AuthZ authority",
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

    pub async fn set_role(
        &self,
        tenant_id: &str,
        document_id: &str,
        principal_id: &str,
        role: DocumentRole,
        expires_at_ms: Option<i64>,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<AuthzMutationReceipt, AuthzError> {
        validate_identity_set(tenant_id, document_id, principal_id, operation_id)?;
        validate_now(now_ms)?;
        if expires_at_ms.is_some_and(|value| value < 0) {
            return Err(AuthzError::new(
                "invalid_expiry",
                "grant expiry must be non-negative when present",
            ));
        }

        let mut conn = begin_immediate(&self.pool).await?;
        let result = async {
            ensure_document(&mut conn, tenant_id, document_id).await?;
            sqlx::query(
                r#"
                INSERT INTO authz_principal_grants (
                    tenant_id, document_id, principal_id, role, expires_at_ms, updated_at_ms
                ) VALUES (?, ?, ?, ?, ?, ?)
                ON CONFLICT(tenant_id, document_id, principal_id)
                DO UPDATE SET
                    role=excluded.role,
                    expires_at_ms=excluded.expires_at_ms,
                    updated_at_ms=excluded.updated_at_ms
                "#,
            )
            .bind(tenant_id.as_bytes())
            .bind(document_id.as_bytes())
            .bind(principal_id.as_bytes())
            .bind(role.as_str())
            .bind(expires_at_ms)
            .bind(now_ms)
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            let version = bump_version(&mut conn, tenant_id, document_id).await?;
            insert_audit(
                &mut conn,
                tenant_id,
                document_id,
                principal_id,
                operation_id,
                "grant.set",
                "allowed",
                CAP_MEMBER_MANAGE,
                version,
                None,
                now_ms,
            )
            .await?;

            Ok(AuthzMutationReceipt {
                tenant_id: tenant_id.to_owned(),
                document_id: document_id.to_owned(),
                authz_version: version,
                active_session_barrier_complete: true,
            })
        }
        .await;
        finish_transaction(&mut conn, result).await
    }

    pub async fn revoke(
        &self,
        tenant_id: &str,
        document_id: &str,
        principal_id: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<AuthzMutationReceipt, AuthzError> {
        validate_identity_set(tenant_id, document_id, principal_id, operation_id)?;
        validate_now(now_ms)?;

        let mut conn = begin_immediate(&self.pool).await?;
        let result = async {
            ensure_document(&mut conn, tenant_id, document_id).await?;
            sqlx::query(
                r#"
                DELETE FROM authz_principal_grants
                WHERE tenant_id=? AND document_id=? AND principal_id=?
                "#,
            )
            .bind(tenant_id.as_bytes())
            .bind(document_id.as_bytes())
            .bind(principal_id.as_bytes())
            .execute(&mut *conn)
            .await
            .map_err(sqlite_error)?;

            let version = bump_version(&mut conn, tenant_id, document_id).await?;
            insert_audit(
                &mut conn,
                tenant_id,
                document_id,
                principal_id,
                operation_id,
                "grant.revoke",
                "allowed",
                CAP_MEMBER_MANAGE,
                version,
                None,
                now_ms,
            )
            .await?;

            Ok(AuthzMutationReceipt {
                tenant_id: tenant_id.to_owned(),
                document_id: document_id.to_owned(),
                authz_version: version,
                active_session_barrier_complete: true,
            })
        }
        .await;
        finish_transaction(&mut conn, result).await
    }

    pub async fn authorize(
        &self,
        tenant_id: &str,
        document_id: &str,
        principal_id: &str,
        capability: &str,
        operation_id: &str,
        now_ms: i64,
    ) -> Result<AuthzDecision, AuthzError> {
        validate_identity_set(tenant_id, document_id, principal_id, operation_id)?;
        validate_capability(capability)?;
        validate_now(now_ms)?;

        let mut conn = begin_immediate(&self.pool).await?;
        match check_authorization(
            &mut conn,
            tenant_id,
            document_id,
            principal_id,
            capability,
            now_ms,
        )
        .await
        {
            Ok(decision) => {
                let result = async {
                    insert_audit(
                        &mut conn,
                        tenant_id,
                        document_id,
                        principal_id,
                        operation_id,
                        "authorize",
                        "allowed",
                        capability,
                        decision.authz_version,
                        None,
                        now_ms,
                    )
                    .await?;
                    Ok(decision)
                }
                .await;
                finish_transaction(&mut conn, result).await
            }
            Err(AuthorizationCheckError::Denied(denied)) => {
                insert_audit(
                    &mut conn,
                    tenant_id,
                    document_id,
                    principal_id,
                    operation_id,
                    "authorize",
                    "denied",
                    capability,
                    denied.authz_version,
                    Some(denied.code),
                    now_ms,
                )
                .await?;
                commit(&mut conn).await?;
                Err(AuthzError::new(denied.code, denied.message))
            }
            Err(AuthorizationCheckError::Internal(error)) => {
                rollback(&mut conn).await?;
                Err(error)
            }
        }
    }

    async fn require_schema(&self) -> Result<(), AuthzError> {
        for table in [
            "authz_documents",
            "authz_principal_grants",
            "authz_audit_events",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_error)?;
            if exists != 1 {
                return Err(AuthzError::new(
                    "sqlite_schema_missing",
                    format!("{table} table is absent; run chaptera migrate up"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct SqliteExportPublishPreflightAuthorizer {
    authority: SqliteAuthzAuthority,
}

impl SqliteExportPublishPreflightAuthorizer {
    pub fn new(authority: SqliteAuthzAuthority) -> Self {
        Self { authority }
    }
}

impl ExportPublishAuthorizer for SqliteExportPublishPreflightAuthorizer {
    fn authorize<'a>(
        &'a self,
        job: &'a JobRecord,
        payload: &'a ExportJobPayloadV1,
    ) -> ExportPublishAuthFuture<'a> {
        Box::pin(async move {
            let now_ms = unix_now_ms().map_err(export_error)?;
            self.authority
                .authorize(
                    &payload.tenant_id,
                    &payload.document_id,
                    &payload.requesting_principal_id,
                    CAP_EXPORT,
                    &job.job_id,
                    now_ms,
                )
                .await
                .map(|_| ())
                .map_err(|error| {
                    ExportExecutorError::new("export_publish_unauthorized", error.message)
                })
        })
    }
}

#[derive(Clone)]
pub struct SqliteAuthorizedExportPublicationCommitter {
    authority: SqliteAuthzAuthority,
    publications: SqliteExportPublicationStore,
}

impl SqliteAuthorizedExportPublicationCommitter {
    pub fn new(
        authority: SqliteAuthzAuthority,
        publications: SqliteExportPublicationStore,
    ) -> Result<Self, AuthzError> {
        if authority.path() != publications.path() {
            return Err(AuthzError::new(
                "authz_publication_database_mismatch",
                "AuthZ authority and export publication store must share one SQLite database",
            ));
        }
        Ok(Self {
            authority,
            publications,
        })
    }
}

impl ExportPublicationCommitter for SqliteAuthorizedExportPublicationCommitter {
    fn commit_authorized<'a>(
        &'a self,
        job: &'a JobRecord,
        payload: &'a ExportJobPayloadV1,
        input: ExportPublicationInputV1,
        created_at_ms: i64,
    ) -> ExportPublicationCommitFuture<'a> {
        Box::pin(async move {
            validate_export_identity(job, payload, &input)?;
            validate_now(created_at_ms).map_err(export_error)?;

            let mut conn = begin_immediate(&self.authority.pool)
                .await
                .map_err(export_error)?;

            let decision = match check_authorization(
                &mut conn,
                &payload.tenant_id,
                &payload.document_id,
                &payload.requesting_principal_id,
                CAP_EXPORT,
                created_at_ms,
            )
            .await
            {
                Ok(decision) => decision,
                Err(AuthorizationCheckError::Denied(denied)) => {
                    insert_audit(
                        &mut conn,
                        &payload.tenant_id,
                        &payload.document_id,
                        &payload.requesting_principal_id,
                        &job.job_id,
                        "export.publish",
                        "denied",
                        CAP_EXPORT,
                        denied.authz_version,
                        Some(denied.code),
                        created_at_ms,
                    )
                    .await
                    .map_err(export_error)?;
                    commit(&mut conn).await.map_err(export_error)?;
                    return Err(ExportExecutorError::new(
                        "export_publish_unauthorized",
                        denied.message,
                    ));
                }
                Err(AuthorizationCheckError::Internal(error)) => {
                    rollback(&mut conn).await.map_err(export_error)?;
                    return Err(export_error(error));
                }
            };

            let result = async {
                insert_audit(
                    &mut conn,
                    &payload.tenant_id,
                    &payload.document_id,
                    &payload.requesting_principal_id,
                    &job.job_id,
                    "export.publish",
                    "allowed",
                    CAP_EXPORT,
                    decision.authz_version,
                    None,
                    created_at_ms,
                )
                .await
                .map_err(export_error)?;

                let prepared = self
                    .publications
                    .prepare_in_transaction(&mut conn, input, created_at_ms)
                    .await
                    .map_err(|error| ExportExecutorError::new(error.code, error.message))?;
                Ok(match prepared {
                    ExportPublicationPrepareOutcomeV1::Prepared(record)
                    | ExportPublicationPrepareOutcomeV1::AlreadyPrepared(record) => {
                        record.effect_key
                    }
                })
            }
            .await;

            match result {
                Ok(effect_key) => {
                    commit(&mut conn).await.map_err(export_error)?;
                    Ok(effect_key)
                }
                Err(error) => {
                    rollback(&mut conn).await.map_err(export_error)?;
                    Err(error)
                }
            }
        })
    }
}

async fn check_authorization(
    conn: &mut PoolConnection<Sqlite>,
    tenant_id: &str,
    document_id: &str,
    principal_id: &str,
    capability: &str,
    now_ms: i64,
) -> Result<AuthzDecision, AuthorizationCheckError> {
    let version: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT authz_version
        FROM authz_documents
        WHERE tenant_id=? AND document_id=?
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(document_id.as_bytes())
    .fetch_optional(&mut **conn)
    .await
    .map_err(|error| AuthorizationCheckError::Internal(sqlite_error(error)))?;
    let authz_version = version.unwrap_or(0);

    let row = sqlx::query(
        r#"
        SELECT role, expires_at_ms
        FROM authz_principal_grants
        WHERE tenant_id=? AND document_id=? AND principal_id=?
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(document_id.as_bytes())
    .bind(principal_id.as_bytes())
    .fetch_optional(&mut **conn)
    .await
    .map_err(|error| AuthorizationCheckError::Internal(sqlite_error(error)))?;

    let Some(row) = row else {
        return Err(AuthorizationCheckError::Denied(AuthorizationDenied {
            code: "grant_missing",
            message: "principal has no grant for the document",
            authz_version,
        }));
    };
    let role_raw: String = row
        .try_get("role")
        .map_err(|error| AuthorizationCheckError::Internal(sqlite_error(error)))?;
    let expires_at_ms: Option<i64> = row
        .try_get("expires_at_ms")
        .map_err(|error| AuthorizationCheckError::Internal(sqlite_error(error)))?;
    if expires_at_ms.is_some_and(|expiry| now_ms >= expiry) {
        return Err(AuthorizationCheckError::Denied(AuthorizationDenied {
            code: "grant_expired",
            message: "principal grant has expired",
            authz_version,
        }));
    }

    let role = DocumentRole::parse(&role_raw).map_err(AuthorizationCheckError::Internal)?;
    if !role.allows(capability) {
        return Err(AuthorizationCheckError::Denied(AuthorizationDenied {
            code: "capability_denied",
            message: "principal role does not grant the requested capability",
            authz_version,
        }));
    }

    Ok(AuthzDecision {
        tenant_id: tenant_id.to_owned(),
        document_id: document_id.to_owned(),
        principal_id: principal_id.to_owned(),
        capability: capability.to_owned(),
        role,
        authz_version,
    })
}

async fn ensure_document(
    conn: &mut PoolConnection<Sqlite>,
    tenant_id: &str,
    document_id: &str,
) -> Result<(), AuthzError> {
    sqlx::query(
        r#"
        INSERT INTO authz_documents (tenant_id, document_id, authz_version)
        VALUES (?, ?, 0)
        ON CONFLICT(tenant_id, document_id) DO NOTHING
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(document_id.as_bytes())
    .execute(&mut **conn)
    .await
    .map_err(sqlite_error)?;
    Ok(())
}

async fn bump_version(
    conn: &mut PoolConnection<Sqlite>,
    tenant_id: &str,
    document_id: &str,
) -> Result<i64, AuthzError> {
    let done = sqlx::query(
        r#"
        UPDATE authz_documents
        SET authz_version=authz_version+1
        WHERE tenant_id=? AND document_id=?
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(document_id.as_bytes())
    .execute(&mut **conn)
    .await
    .map_err(sqlite_error)?;
    if done.rows_affected() != 1 {
        return Err(AuthzError::new(
            "authz_version_update_failed",
            "document access generation did not advance exactly once",
        ));
    }

    sqlx::query_scalar(
        r#"
        SELECT authz_version
        FROM authz_documents
        WHERE tenant_id=? AND document_id=?
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(document_id.as_bytes())
    .fetch_one(&mut **conn)
    .await
    .map_err(sqlite_error)
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit(
    conn: &mut PoolConnection<Sqlite>,
    tenant_id: &str,
    document_id: &str,
    principal_id: &str,
    operation_id: &str,
    action: &str,
    result: &str,
    capability: &str,
    authz_version: i64,
    error_code: Option<&str>,
    created_at_ms: i64,
) -> Result<(), AuthzError> {
    sqlx::query(
        r#"
        INSERT INTO authz_audit_events (
            tenant_id, document_id, principal_id, operation_id,
            action, result, capability, authz_version, error_code, created_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(tenant_id.as_bytes())
    .bind(document_id.as_bytes())
    .bind(principal_id.as_bytes())
    .bind(operation_id.as_bytes())
    .bind(action)
    .bind(result)
    .bind(capability)
    .bind(authz_version)
    .bind(error_code)
    .bind(created_at_ms)
    .execute(&mut **conn)
    .await
    .map_err(sqlite_error)?;
    Ok(())
}

async fn begin_immediate(pool: &SqlitePool) -> Result<PoolConnection<Sqlite>, AuthzError> {
    let mut conn = pool.acquire().await.map_err(sqlite_error)?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .map_err(sqlite_error)?;
    Ok(conn)
}

async fn finish_transaction<T>(
    conn: &mut PoolConnection<Sqlite>,
    result: Result<T, AuthzError>,
) -> Result<T, AuthzError> {
    match result {
        Ok(value) => {
            commit(conn).await?;
            Ok(value)
        }
        Err(error) => {
            rollback(conn).await?;
            Err(error)
        }
    }
}

async fn commit(conn: &mut PoolConnection<Sqlite>) -> Result<(), AuthzError> {
    sqlx::query("COMMIT")
        .execute(&mut **conn)
        .await
        .map_err(sqlite_error)?;
    Ok(())
}

async fn rollback(conn: &mut PoolConnection<Sqlite>) -> Result<(), AuthzError> {
    sqlx::query("ROLLBACK")
        .execute(&mut **conn)
        .await
        .map_err(sqlite_error)?;
    Ok(())
}

fn validate_export_identity(
    job: &JobRecord,
    payload: &ExportJobPayloadV1,
    input: &ExportPublicationInputV1,
) -> Result<(), ExportExecutorError> {
    if job.job_kind != JobKind::Export
        || job.tenant_id != payload.tenant_id
        || input.tenant_id != payload.tenant_id
        || input.job_id != job.job_id
        || input.document_id != payload.document_id
        || input.exact_revision_id != payload.exact_revision_id
        || input.canonical_revision_id != payload.canonical_authoring_revision_id
        || input.target_profile != payload.target_profile
        || input.layout_environment_id != payload.layout_environment_id
    {
        return Err(ExportExecutorError::new(
            "export_publish_identity_mismatch",
            "final publication identity differs from the authorized export job",
        ));
    }
    Ok(())
}

fn validate_pool(max_connections: u32, busy_timeout: Duration) -> Result<(), AuthzError> {
    if max_connections == 0 || max_connections > 16 {
        return Err(AuthzError::new(
            "invalid_pool_size",
            "AuthZ authority must use 1..=16 connections",
        ));
    }
    if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
        return Err(AuthzError::new(
            "invalid_busy_timeout",
            "busy timeout must be >0 and <=30 seconds",
        ));
    }
    Ok(())
}

fn validate_identity_set(
    tenant_id: &str,
    document_id: &str,
    principal_id: &str,
    operation_id: &str,
) -> Result<(), AuthzError> {
    require_ident(tenant_id, "tenant_id")?;
    require_ident(document_id, "document_id")?;
    require_ident(principal_id, "principal_id")?;
    require_ident(operation_id, "operation_id")?;
    Ok(())
}

fn require_ident(value: &str, label: &'static str) -> Result<(), AuthzError> {
    if value.is_empty()
        || value.len() > 192
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b':' | b'@' | b'/' | b'-')
        })
    {
        return Err(AuthzError::new(
            "invalid_identity",
            format!("invalid {label}"),
        ));
    }
    Ok(())
}

fn validate_capability(capability: &str) -> Result<(), AuthzError> {
    if !known_capability(capability) {
        return Err(AuthzError::new(
            "unknown_capability",
            "capability is outside the canonical CLOUD-AUTHZ-01 set",
        ));
    }
    Ok(())
}

fn known_capability(capability: &str) -> bool {
    matches!(
        capability,
        CAP_VIEW
            | CAP_COMMENT_READ
            | CAP_COMMENT_WRITE
            | CAP_EDIT
            | CAP_EDIT_TEXT
            | CAP_EDIT_GEOMETRY
            | CAP_ASSET_UPLOAD
            | CAP_EXPORT
            | CAP_SHARE_MANAGE
            | CAP_MEMBER_MANAGE
            | CAP_DELETE
    )
}

fn validate_now(now_ms: i64) -> Result<(), AuthzError> {
    if now_ms < 0 {
        return Err(AuthzError::new(
            "invalid_now",
            "timestamp must be non-negative",
        ));
    }
    Ok(())
}

fn unix_now_ms() -> Result<i64, AuthzError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthzError::new("clock_before_epoch", "system clock is before UNIX epoch"))?;
    i64::try_from(elapsed.as_millis())
        .map_err(|_| AuthzError::new("clock_overflow", "system clock does not fit i64 milliseconds"))
}

fn sqlite_error(error: impl fmt::Display) -> AuthzError {
    AuthzError::new(
        "sqlite_authz_error",
        error.to_string().chars().take(512).collect::<String>(),
    )
}

fn export_error(error: AuthzError) -> ExportExecutorError {
    ExportExecutorError::new(error.code, error.message)
}


#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
        export_executor::{
            EXPORT_JOB_PAYLOAD_SCHEMA_V1, ExportPublicationCommitter,
            IDML_BOUNDED_EDITABLE_PROFILE,
        },
        job_queue::JobStatus,
        schema_migration::SqliteMigrationRuntime,
    };

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-authz-{label}-{}-{serial}.sqlite",
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
        PathBuf,
        SqliteAuthzAuthority,
        SqliteExportPublicationStore,
    ) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let authority = SqliteAuthzAuthority::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let publications = SqliteExportPublicationStore::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (path, authority, publications)
    }

    fn job(job_id: &str, tenant_id: &str) -> JobRecord {
        JobRecord {
            job_id: job_id.to_owned(),
            tenant_id: tenant_id.to_owned(),
            job_kind: JobKind::Export,
            payload_schema_version: 1,
            payload: Vec::new(),
            request_hash: "a".repeat(64),
            status: JobStatus::Running,
            available_at_ms: 0,
            attempt: 1,
            max_attempts: 3,
            lease_owner: Some("worker-authz-test".to_owned()),
            lease_generation: 1,
            lease_expires_at_ms: Some(10_000),
            cancel_requested_at_ms: None,
            idempotency_key: format!("idem-{job_id}"),
            created_at_ms: 0,
            started_at_ms: Some(0),
            finished_at_ms: None,
            terminal_code: None,
        }
    }

    fn payload(job_suffix: &str) -> ExportJobPayloadV1 {
        ExportJobPayloadV1 {
            schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.to_owned(),
            tenant_id: "tenant:authz".to_owned(),
            document_id: "document:authz".to_owned(),
            requesting_principal_id: "principal:editor".to_owned(),
            exact_revision_id: format!("service-rev-{job_suffix}"),
            canonical_authoring_revision_id: "c".repeat(64),
            target_profile: IDML_BOUNDED_EDITABLE_PROFILE.to_owned(),
            layout_environment_id: format!("sha256:{}", "d".repeat(64)),
        }
    }

    fn publication(job_id: &str, payload: &ExportJobPayloadV1) -> ExportPublicationInputV1 {
        ExportPublicationInputV1 {
            tenant_id: payload.tenant_id.clone(),
            job_id: job_id.to_owned(),
            document_id: payload.document_id.clone(),
            exact_revision_id: payload.exact_revision_id.clone(),
            canonical_revision_id: payload.canonical_authoring_revision_id.clone(),
            target_profile: payload.target_profile.clone(),
            layout_environment_id: payload.layout_environment_id.clone(),
            fence_id: format!("sha256:{}", "e".repeat(64)),
            artifact_binding_id: format!("binding-artifact-{job_id}"),
            artifact_content_hash: format!("sha256:{}", "f".repeat(64)),
            loss_binding_id: format!("binding-loss-{job_id}"),
            loss_report_hash: format!("sha256:{}", "b".repeat(64)),
        }
    }

    #[tokio::test]
    async fn role_change_and_revoke_advance_access_generation_and_audit() {
        let (path, authority, publications) = stores("grant-revoke").await;

        let granted = authority
            .set_role(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                DocumentRole::Editor,
                None,
                "grant-op-1",
                10,
            )
            .await
            .unwrap();
        assert_eq!(granted.authz_version, 1);
        assert!(granted.active_session_barrier_complete);

        let allowed = authority
            .authorize(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                CAP_EXPORT,
                "job-authz-1",
                20,
            )
            .await
            .unwrap();
        assert_eq!(allowed.role, DocumentRole::Editor);
        assert_eq!(allowed.authz_version, 1);

        let revoked = authority
            .revoke(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                "revoke-op-1",
                30,
            )
            .await
            .unwrap();
        assert_eq!(revoked.authz_version, 2);

        let denied = authority
            .authorize(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                CAP_EXPORT,
                "job-authz-2",
                40,
            )
            .await
            .unwrap_err();
        assert_eq!(denied.code, "grant_missing");

        let audit: Vec<(String, String, String, i64, Option<String>)> = sqlx::query_as(
            r#"
            SELECT action, result, capability, authz_version, error_code
            FROM authz_audit_events
            ORDER BY event_id
            "#,
        )
        .fetch_all(&authority.pool)
        .await
        .unwrap();
        assert_eq!(audit.len(), 4);
        assert_eq!(audit[0].0, "grant.set");
        assert_eq!(audit[1].1, "allowed");
        assert_eq!(audit[2].0, "grant.revoke");
        assert_eq!(audit[3].1, "denied");
        assert_eq!(audit[3].4.as_deref(), Some("grant_missing"));

        publications.close().await;
        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn viewer_cannot_export_but_editor_can() {
        let (path, authority, publications) = stores("capability").await;

        authority
            .set_role(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                DocumentRole::Viewer,
                None,
                "grant-viewer",
                10,
            )
            .await
            .unwrap();
        let denied = authority
            .authorize(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                CAP_EXPORT,
                "export-as-viewer",
                20,
            )
            .await
            .unwrap_err();
        assert_eq!(denied.code, "capability_denied");

        authority
            .set_role(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                DocumentRole::Editor,
                None,
                "grant-editor",
                30,
            )
            .await
            .unwrap();
        authority
            .authorize(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                CAP_EXPORT,
                "export-as-editor",
                40,
            )
            .await
            .unwrap();

        publications.close().await;
        authority.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn revoke_and_publication_share_one_sqlite_barrier() {
        let (path, authority, publications) = stores("barrier").await;
        authority
            .set_role(
                "tenant:authz",
                "document:authz",
                "principal:editor",
                DocumentRole::Editor,
                None,
                "grant-editor",
                10,
            )
            .await
            .unwrap();

        let first_job = job("job-export-before-revoke", "tenant:authz");
        let first_payload = payload("before-revoke");
        let first_input = publication(&first_job.job_id, &first_payload);

        let mut conn = begin_immediate(&authority.pool).await.unwrap();
        let decision = check_authorization(
            &mut conn,
            &first_payload.tenant_id,
            &first_payload.document_id,
            &first_payload.requesting_principal_id,
            CAP_EXPORT,
            20,
        )
        .await
        .unwrap();
        assert_eq!(decision.authz_version, 1);

        insert_audit(
            &mut conn,
            &first_payload.tenant_id,
            &first_payload.document_id,
            &first_payload.requesting_principal_id,
            &first_job.job_id,
            "export.publish",
            "allowed",
            CAP_EXPORT,
            decision.authz_version,
            None,
            20,
        )
        .await
        .unwrap();

        let revoke_authority = authority.clone();
        let revoke_task = tokio::spawn(async move {
            revoke_authority
                .revoke(
                    "tenant:authz",
                    "document:authz",
                    "principal:editor",
                    "concurrent-revoke",
                    30,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !revoke_task.is_finished(),
            "revoke must wait while authorized publication owns BEGIN IMMEDIATE"
        );

        publications
            .prepare_in_transaction(&mut conn, first_input, 20)
            .await
            .unwrap();
        commit(&mut conn).await.unwrap();

        let revoked = revoke_task.await.unwrap().unwrap();
        assert_eq!(revoked.authz_version, 2);
        assert!(
            publications
                .get_by_job("tenant:authz", &first_job.job_id)
                .await
                .unwrap()
                .is_some()
        );

        let committer = SqliteAuthorizedExportPublicationCommitter::new(
            authority.clone(),
            publications.clone(),
        )
        .unwrap();
        let second_job = job("job-export-after-revoke", "tenant:authz");
        let second_payload = payload("after-revoke");
        let error = committer
            .commit_authorized(
                &second_job,
                &second_payload,
                publication(&second_job.job_id, &second_payload),
                40,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "export_publish_unauthorized");
        assert!(
            publications
                .get_by_job("tenant:authz", &second_job.job_id)
                .await
                .unwrap()
                .is_none(),
            "revoked principal must not prepare a later logical publication"
        );

        publications.close().await;
        authority.close().await;
        cleanup(&path);
    }
}
