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
            Err(denied) => {
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
                Err(denied) => {
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
) -> Result<AuthzDecision, AuthorizationDenied> {
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
    .map_err(|_| AuthorizationDenied {
        code: "sqlite_authz_error",
        message: "could not read document access generation",
        authz_version: 0,
    })?;
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
    .map_err(|_| AuthorizationDenied {
        code: "sqlite_authz_error",
        message: "could not read principal grant",
        authz_version,
    })?;

    let Some(row) = row else {
        return Err(AuthorizationDenied {
            code: "grant_missing",
            message: "principal has no grant for the document",
            authz_version,
        });
    };
    let role_raw: String = row.try_get("role").map_err(|_| AuthorizationDenied {
        code: "authz_row_corrupt",
        message: "persisted grant role is unreadable",
        authz_version,
    })?;
    let expires_at_ms: Option<i64> =
        row.try_get("expires_at_ms").map_err(|_| AuthorizationDenied {
            code: "authz_row_corrupt",
            message: "persisted grant expiry is unreadable",
            authz_version,
        })?;
    if expires_at_ms.is_some_and(|expiry| now_ms >= expiry) {
        return Err(AuthorizationDenied {
            code: "grant_expired",
            message: "principal grant has expired",
            authz_version,
        });
    }

    let role = DocumentRole::parse(&role_raw).map_err(|_| AuthorizationDenied {
        code: "authz_row_corrupt",
        message: "persisted grant role is outside the canonical role set",
        authz_version,
    })?;
    if !role.allows(capability) {
        return Err(AuthorizationDenied {
            code: "capability_denied",
            message: "principal role does not grant the requested capability",
            authz_version,
        });
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
