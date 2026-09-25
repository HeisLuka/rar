use std::{
    collections::BTreeMap,
    fmt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use sha2::{Digest, Sha256};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

const MAX_IDENTITY_BYTES: usize = 2048;
const MAX_EMAIL_BYTES: usize = 512;
const MAX_PRINCIPAL_ID_BYTES: usize = 160;
const MIN_OPAQUE_TOKEN_BYTES: usize = 32;
const MAX_OPAQUE_TOKEN_BYTES: usize = 512;
const MAX_FLOW_TOKEN_BYTES: usize = 2048;
const MAX_RETURN_PATH_BYTES: usize = 2048;
const MAX_LOGIN_FLOWS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthnError {
    pub code: &'static str,
    pub message: String,
}

impl AuthnError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AuthnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AuthnError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalRecord {
    pub principal_id: String,
    pub created_at_ms: i64,
    pub disabled_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalIdentity {
    pub issuer: String,
    pub subject: String,
    pub principal_id: String,
    pub email_snapshot: Option<String>,
    pub linked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvePrincipalRequest {
    pub issuer: String,
    pub subject: String,
    pub proposed_principal_id: String,
    pub email_snapshot: Option<String>,
    pub now_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub principal_id: String,
    pub session_generation: i64,
    pub created_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub idle_expires_at_ms: i64,
    pub absolute_expires_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionRequest<'a> {
    pub principal_id: String,
    pub raw_session_token: &'a [u8],
    pub raw_csrf_token: &'a [u8],
    pub session_generation: i64,
    pub created_at_ms: i64,
    pub idle_expires_at_ms: i64,
    pub absolute_expires_at_ms: i64,
}

#[derive(Clone)]
pub struct SqliteAuthnStore {
    pool: SqlitePool,
}

impl SqliteAuthnStore {
    pub async fn open(
        path: impl AsRef<Path>,
        max_connections: u32,
        busy_timeout: Duration,
    ) -> Result<Self, AuthnError> {
        if path.as_ref().as_os_str().is_empty() {
            return Err(AuthnError::new(
                "authn_database_path_invalid",
                "authn SQLite path must be non-empty",
            ));
        }
        if max_connections == 0 || max_connections > 16 {
            return Err(AuthnError::new(
                "authn_pool_size_invalid",
                "authn SQLite pool size must be between 1 and 16",
            ));
        }
        if busy_timeout.is_zero() || busy_timeout > Duration::from_secs(30) {
            return Err(AuthnError::new(
                "authn_busy_timeout_invalid",
                "authn SQLite busy timeout must be >0 and <=30 seconds",
            ));
        }
        if !path.as_ref().exists() {
            return Err(AuthnError::new(
                "authn_database_missing",
                "authn SQLite database must be created by the operator migration step",
            ));
        }

        let options = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(false)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .foreign_keys(true)
            .busy_timeout(busy_timeout);

        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .map_err(sqlite_open_error)?;

        let store = Self { pool };
        store.require_schema().await?;
        Ok(store)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    async fn require_schema(&self) -> Result<(), AuthnError> {
        for table in ["principals", "principal_identities", "sessions"] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_one(&self.pool)
            .await
            .map_err(sqlite_read_error)?;

            if exists != 1 {
                return Err(AuthnError::new(
                    "authn_schema_missing",
                    format!(
                        "required authn table {table} is absent; run chaptera migrate up before serving auth"
                    ),
                ));
            }
        }
        Ok(())
    }

    pub async fn resolve_principal(
        &self,
        request: ResolvePrincipalRequest,
    ) -> Result<(PrincipalRecord, PrincipalIdentity), AuthnError> {
        validate_identity(&request.issuer, "issuer")?;
        validate_identity(&request.subject, "subject")?;
        validate_principal_id(&request.proposed_principal_id)?;
        validate_email(request.email_snapshot.as_deref())?;
        require_nonnegative(request.now_ms, "now_ms")?;

        let mut tx = self.pool.begin().await.map_err(sqlite_write_error)?;

        if let Some(row) = sqlx::query(
            r#"
            SELECT issuer, subject, principal_id, email_snapshot, linked_at_ms
            FROM principal_identities
            WHERE issuer=? AND subject=?
            "#,
        )
        .bind(&request.issuer)
        .bind(&request.subject)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sqlite_read_error)?
        {
            let principal_id = blob_text(&row, "principal_id")?;

            if request.email_snapshot.is_some() {
                sqlx::query(
                    r#"
                    UPDATE principal_identities
                    SET email_snapshot=?
                    WHERE issuer=? AND subject=?
                    "#,
                )
                .bind(request.email_snapshot.as_deref())
                .bind(&request.issuer)
                .bind(&request.subject)
                .execute(&mut *tx)
                .await
                .map_err(sqlite_write_error)?;
            }

            let principal = load_principal_in(&mut tx, &principal_id).await?;
            let identity = PrincipalIdentity {
                issuer: row.try_get("issuer").map_err(sqlite_decode_error)?,
                subject: row.try_get("subject").map_err(sqlite_decode_error)?,
                principal_id,
                email_snapshot: request.email_snapshot.or_else(|| {
                    row.try_get::<Option<String>, _>("email_snapshot")
                        .ok()
                        .flatten()
                }),
                linked_at_ms: row.try_get("linked_at_ms").map_err(sqlite_decode_error)?,
            };
            tx.commit().await.map_err(sqlite_write_error)?;
            return Ok((principal, identity));
        }

        sqlx::query(
            r#"
            INSERT INTO principals(principal_id, created_at_ms, disabled_at_ms)
            VALUES (?, ?, NULL)
            "#,
        )
        .bind(request.proposed_principal_id.as_bytes())
        .bind(request.now_ms)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            AuthnError::new("principal_insert_failed", bounded_sqlx_message(&error))
        })?;

        sqlx::query(
            r#"
            INSERT INTO principal_identities(
                issuer, subject, principal_id, email_snapshot, linked_at_ms
            ) VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&request.issuer)
        .bind(&request.subject)
        .bind(request.proposed_principal_id.as_bytes())
        .bind(request.email_snapshot.as_deref())
        .bind(request.now_ms)
        .execute(&mut *tx)
        .await
        .map_err(|error| AuthnError::new("identity_insert_failed", bounded_sqlx_message(&error)))?;

        let principal = PrincipalRecord {
            principal_id: request.proposed_principal_id.clone(),
            created_at_ms: request.now_ms,
            disabled_at_ms: None,
        };
        let identity = PrincipalIdentity {
            issuer: request.issuer,
            subject: request.subject,
            principal_id: request.proposed_principal_id,
            email_snapshot: request.email_snapshot,
            linked_at_ms: request.now_ms,
        };

        tx.commit().await.map_err(sqlite_write_error)?;
        Ok((principal, identity))
    }

    pub async fn identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<PrincipalIdentity>, AuthnError> {
        validate_identity(issuer, "issuer")?;
        validate_identity(subject, "subject")?;

        let row = sqlx::query(
            r#"
            SELECT issuer, subject, principal_id, email_snapshot, linked_at_ms
            FROM principal_identities
            WHERE issuer=? AND subject=?
            "#,
        )
        .bind(issuer)
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        row.map(decode_identity_row).transpose()
    }

    pub async fn create_session(
        &self,
        request: CreateSessionRequest<'_>,
    ) -> Result<SessionRecord, AuthnError> {
        validate_principal_id(&request.principal_id)?;
        validate_opaque_token(request.raw_session_token, "session token")?;
        validate_opaque_token(request.raw_csrf_token, "csrf token")?;
        validate_session_times(
            request.session_generation,
            request.created_at_ms,
            request.idle_expires_at_ms,
            request.absolute_expires_at_ms,
        )?;

        let principal = self
            .principal(&request.principal_id)
            .await?
            .ok_or_else(|| AuthnError::new("principal_not_found", "principal does not exist"))?;
        if principal.disabled_at_ms.is_some() {
            return Err(AuthnError::new(
                "principal_disabled",
                "disabled principal cannot create a session",
            ));
        }

        let session_hash = digest(request.raw_session_token);
        let csrf_hash = digest(request.raw_csrf_token);

        sqlx::query(
            r#"
            INSERT INTO sessions(
                session_id_hash, principal_id, csrf_token_hash, session_generation,
                created_at_ms, last_seen_at_ms, idle_expires_at_ms,
                absolute_expires_at_ms, revoked_at_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)
            "#,
        )
        .bind(session_hash.as_slice())
        .bind(request.principal_id.as_bytes())
        .bind(csrf_hash.as_slice())
        .bind(request.session_generation)
        .bind(request.created_at_ms)
        .bind(request.created_at_ms)
        .bind(request.idle_expires_at_ms)
        .bind(request.absolute_expires_at_ms)
        .execute(&self.pool)
        .await
        .map_err(|error| AuthnError::new("session_insert_failed", bounded_sqlx_message(&error)))?;

        Ok(SessionRecord {
            principal_id: request.principal_id,
            session_generation: request.session_generation,
            created_at_ms: request.created_at_ms,
            last_seen_at_ms: request.created_at_ms,
            idle_expires_at_ms: request.idle_expires_at_ms,
            absolute_expires_at_ms: request.absolute_expires_at_ms,
            revoked_at_ms: None,
        })
    }

    pub async fn authenticate_session(
        &self,
        raw_session_token: &[u8],
        now_ms: i64,
        refresh_idle_expires_at_ms: i64,
    ) -> Result<SessionRecord, AuthnError> {
        validate_opaque_token(raw_session_token, "session token")?;
        require_nonnegative(now_ms, "now_ms")?;
        if refresh_idle_expires_at_ms <= now_ms {
            return Err(AuthnError::new(
                "session_idle_refresh_invalid",
                "refreshed idle expiry must be after now",
            ));
        }

        let session_hash = digest(raw_session_token);
        let result = sqlx::query(
            r#"
            UPDATE sessions
            SET last_seen_at_ms=?,
                idle_expires_at_ms=?
            WHERE session_id_hash=?
              AND revoked_at_ms IS NULL
              AND idle_expires_at_ms>?
              AND absolute_expires_at_ms>?
              AND ?>?
              AND ?<=absolute_expires_at_ms
              AND EXISTS (
                  SELECT 1 FROM principals
                  WHERE principals.principal_id=sessions.principal_id
                    AND principals.disabled_at_ms IS NULL
              )
            "#,
        )
        .bind(now_ms)
        .bind(refresh_idle_expires_at_ms)
        .bind(session_hash.as_slice())
        .bind(now_ms)
        .bind(now_ms)
        .bind(refresh_idle_expires_at_ms)
        .bind(now_ms)
        .bind(refresh_idle_expires_at_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_write_error)?;

        if result.rows_affected() != 1 {
            return Err(AuthnError::new(
                "session_invalid",
                "session is absent, expired, revoked or belongs to a disabled principal",
            ));
        }

        self.session_by_hash(session_hash.as_slice())
            .await?
            .ok_or_else(|| AuthnError::new("session_invalid", "session disappeared after refresh"))
    }

    pub async fn rotate_csrf(
        &self,
        raw_session_token: &[u8],
        raw_csrf_token: &[u8],
        now_ms: i64,
    ) -> Result<(), AuthnError> {
        validate_opaque_token(raw_session_token, "session token")?;
        validate_opaque_token(raw_csrf_token, "csrf token")?;
        require_nonnegative(now_ms, "now_ms")?;

        let session_hash = digest(raw_session_token);
        let csrf_hash = digest(raw_csrf_token);
        let result = sqlx::query(
            r#"
            UPDATE sessions
            SET csrf_token_hash=?
            WHERE session_id_hash=?
              AND revoked_at_ms IS NULL
              AND idle_expires_at_ms>?
              AND absolute_expires_at_ms>?
              AND EXISTS (
                  SELECT 1 FROM principals
                  WHERE principals.principal_id=sessions.principal_id
                    AND principals.disabled_at_ms IS NULL
              )
            "#,
        )
        .bind(csrf_hash.as_slice())
        .bind(session_hash.as_slice())
        .bind(now_ms)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_write_error)?;

        if result.rows_affected() != 1 {
            return Err(AuthnError::new(
                "session_invalid",
                "cannot rotate CSRF token for an invalid session",
            ));
        }
        Ok(())
    }

    pub async fn verify_csrf(
        &self,
        raw_session_token: &[u8],
        raw_csrf_token: &[u8],
        now_ms: i64,
    ) -> Result<(), AuthnError> {
        validate_opaque_token(raw_session_token, "session token")?;
        validate_opaque_token(raw_csrf_token, "csrf token")?;
        require_nonnegative(now_ms, "now_ms")?;

        let session_hash = digest(raw_session_token);
        let expected: Option<Vec<u8>> = sqlx::query_scalar(
            r#"
            SELECT sessions.csrf_token_hash
            FROM sessions
            JOIN principals ON principals.principal_id=sessions.principal_id
            WHERE sessions.session_id_hash=?
              AND sessions.revoked_at_ms IS NULL
              AND sessions.idle_expires_at_ms>?
              AND sessions.absolute_expires_at_ms>?
              AND principals.disabled_at_ms IS NULL
            "#,
        )
        .bind(session_hash.as_slice())
        .bind(now_ms)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        let expected = expected.ok_or_else(|| {
            AuthnError::new(
                "session_invalid",
                "session is absent, expired, revoked or disabled",
            )
        })?;
        let actual = digest(raw_csrf_token);
        if !constant_time_eq(&expected, actual.as_slice()) {
            return Err(AuthnError::new(
                "csrf_invalid",
                "CSRF token does not match the authenticated session",
            ));
        }
        Ok(())
    }

    pub async fn revoke_session(
        &self,
        raw_session_token: &[u8],
        revoked_at_ms: i64,
    ) -> Result<bool, AuthnError> {
        validate_opaque_token(raw_session_token, "session token")?;
        require_nonnegative(revoked_at_ms, "revoked_at_ms")?;
        let session_hash = digest(raw_session_token);

        let result = sqlx::query(
            r#"
            UPDATE sessions
            SET revoked_at_ms=?
            WHERE session_id_hash=?
              AND revoked_at_ms IS NULL
              AND created_at_ms<=?
            "#,
        )
        .bind(revoked_at_ms)
        .bind(session_hash.as_slice())
        .bind(revoked_at_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_write_error)?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn disable_principal(
        &self,
        principal_id: &str,
        disabled_at_ms: i64,
    ) -> Result<bool, AuthnError> {
        validate_principal_id(principal_id)?;
        require_nonnegative(disabled_at_ms, "disabled_at_ms")?;

        let result = sqlx::query(
            r#"
            UPDATE principals
            SET disabled_at_ms=?
            WHERE principal_id=?
              AND disabled_at_ms IS NULL
              AND created_at_ms<=?
            "#,
        )
        .bind(disabled_at_ms)
        .bind(principal_id.as_bytes())
        .bind(disabled_at_ms)
        .execute(&self.pool)
        .await
        .map_err(sqlite_write_error)?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn principal(
        &self,
        principal_id: &str,
    ) -> Result<Option<PrincipalRecord>, AuthnError> {
        validate_principal_id(principal_id)?;

        let row = sqlx::query(
            "SELECT principal_id, created_at_ms, disabled_at_ms FROM principals WHERE principal_id=?",
        )
        .bind(principal_id.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        row.map(decode_principal_row).transpose()
    }

    async fn session_by_hash(
        &self,
        session_hash: &[u8],
    ) -> Result<Option<SessionRecord>, AuthnError> {
        let row = sqlx::query(
            r#"
            SELECT principal_id, session_generation, created_at_ms,
                   last_seen_at_ms, idle_expires_at_ms,
                   absolute_expires_at_ms, revoked_at_ms
            FROM sessions
            WHERE session_id_hash=?
            "#,
        )
        .bind(session_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlite_read_error)?;

        row.map(decode_session_row).transpose()
    }
}

async fn load_principal_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    principal_id: &str,
) -> Result<PrincipalRecord, AuthnError> {
    let row = sqlx::query(
        "SELECT principal_id, created_at_ms, disabled_at_ms FROM principals WHERE principal_id=?",
    )
    .bind(principal_id.as_bytes())
    .fetch_optional(&mut **tx)
    .await
    .map_err(sqlite_read_error)?
    .ok_or_else(|| {
        AuthnError::new(
            "principal_identity_corrupt",
            "identity points to a missing principal",
        )
    })?;
    decode_principal_row(row)
}

fn decode_principal_row(row: sqlx::sqlite::SqliteRow) -> Result<PrincipalRecord, AuthnError> {
    Ok(PrincipalRecord {
        principal_id: blob_text(&row, "principal_id")?,
        created_at_ms: row.try_get("created_at_ms").map_err(sqlite_decode_error)?,
        disabled_at_ms: row.try_get("disabled_at_ms").map_err(sqlite_decode_error)?,
    })
}

fn decode_identity_row(row: sqlx::sqlite::SqliteRow) -> Result<PrincipalIdentity, AuthnError> {
    Ok(PrincipalIdentity {
        issuer: row.try_get("issuer").map_err(sqlite_decode_error)?,
        subject: row.try_get("subject").map_err(sqlite_decode_error)?,
        principal_id: blob_text(&row, "principal_id")?,
        email_snapshot: row.try_get("email_snapshot").map_err(sqlite_decode_error)?,
        linked_at_ms: row.try_get("linked_at_ms").map_err(sqlite_decode_error)?,
    })
}

fn decode_session_row(row: sqlx::sqlite::SqliteRow) -> Result<SessionRecord, AuthnError> {
    Ok(SessionRecord {
        principal_id: blob_text(&row, "principal_id")?,
        session_generation: row
            .try_get("session_generation")
            .map_err(sqlite_decode_error)?,
        created_at_ms: row.try_get("created_at_ms").map_err(sqlite_decode_error)?,
        last_seen_at_ms: row
            .try_get("last_seen_at_ms")
            .map_err(sqlite_decode_error)?,
        idle_expires_at_ms: row
            .try_get("idle_expires_at_ms")
            .map_err(sqlite_decode_error)?,
        absolute_expires_at_ms: row
            .try_get("absolute_expires_at_ms")
            .map_err(sqlite_decode_error)?,
        revoked_at_ms: row.try_get("revoked_at_ms").map_err(sqlite_decode_error)?,
    })
}

fn blob_text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, AuthnError> {
    let bytes: Vec<u8> = row.try_get(column).map_err(sqlite_decode_error)?;
    String::from_utf8(bytes)
        .map_err(|_| AuthnError::new("authn_row_corrupt", format!("{column} is not UTF-8")))
}

fn validate_identity(value: &str, label: &'static str) -> Result<(), AuthnError> {
    if value.is_empty() || value.len() > MAX_IDENTITY_BYTES || value.chars().any(char::is_control) {
        return Err(AuthnError::new(
            "identity_invalid",
            format!("{label} must be a bounded non-control string"),
        ));
    }
    Ok(())
}

fn validate_principal_id(value: &str) -> Result<(), AuthnError> {
    if value.is_empty()
        || value.len() > MAX_PRINCIPAL_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(AuthnError::new(
            "principal_id_invalid",
            "principal id must be a bounded opaque identifier",
        ));
    }
    Ok(())
}

fn validate_email(email: Option<&str>) -> Result<(), AuthnError> {
    if let Some(email) = email
        && (email.is_empty()
            || email.len() > MAX_EMAIL_BYTES
            || email.chars().any(char::is_control))
    {
        return Err(AuthnError::new(
            "email_snapshot_invalid",
            "email snapshot must be bounded and contain no control characters",
        ));
    }
    Ok(())
}

fn validate_opaque_token(token: &[u8], label: &'static str) -> Result<(), AuthnError> {
    if !(MIN_OPAQUE_TOKEN_BYTES..=MAX_OPAQUE_TOKEN_BYTES).contains(&token.len()) {
        return Err(AuthnError::new(
            "opaque_token_invalid",
            format!("{label} must carry at least 256 bits of opaque material"),
        ));
    }
    Ok(())
}

fn validate_session_times(
    generation: i64,
    created_at_ms: i64,
    idle_expires_at_ms: i64,
    absolute_expires_at_ms: i64,
) -> Result<(), AuthnError> {
    if generation <= 0 {
        return Err(AuthnError::new(
            "session_generation_invalid",
            "session generation must be positive",
        ));
    }
    require_nonnegative(created_at_ms, "created_at_ms")?;
    if idle_expires_at_ms <= created_at_ms {
        return Err(AuthnError::new(
            "session_idle_expiry_invalid",
            "session idle expiry must be after creation",
        ));
    }
    if absolute_expires_at_ms < idle_expires_at_ms {
        return Err(AuthnError::new(
            "session_absolute_expiry_invalid",
            "session absolute expiry must not precede idle expiry",
        ));
    }
    Ok(())
}

fn require_nonnegative(value: i64, label: &'static str) -> Result<(), AuthnError> {
    if value < 0 {
        Err(AuthnError::new(
            "authn_time_invalid",
            format!("{label} must be non-negative"),
        ))
    } else {
        Ok(())
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

fn bounded_sqlx_message(error: &sqlx::Error) -> String {
    error.to_string().chars().take(256).collect()
}

fn sqlite_open_error(error: sqlx::Error) -> AuthnError {
    AuthnError::new("authn_sqlite_open_failed", bounded_sqlx_message(&error))
}

fn sqlite_read_error(error: sqlx::Error) -> AuthnError {
    AuthnError::new("authn_sqlite_read_failed", bounded_sqlx_message(&error))
}

fn sqlite_write_error(error: sqlx::Error) -> AuthnError {
    AuthnError::new("authn_sqlite_write_failed", bounded_sqlx_message(&error))
}

fn sqlite_decode_error(error: sqlx::Error) -> AuthnError {
    AuthnError::new("authn_row_corrupt", bounded_sqlx_message(&error))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginFlow {
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_path: String,
    pub expires_at_ms: u64,
}

#[derive(Clone)]
pub struct LoginFlowStore {
    max_entries: usize,
    flows: Arc<Mutex<BTreeMap<String, LoginFlow>>>,
}

impl LoginFlowStore {
    pub fn new(max_entries: usize) -> Result<Self, AuthnError> {
        if max_entries == 0 || max_entries > MAX_LOGIN_FLOWS {
            return Err(AuthnError::new(
                "login_flow_capacity_invalid",
                format!("login flow capacity must be between 1 and {MAX_LOGIN_FLOWS}"),
            ));
        }
        Ok(Self {
            max_entries,
            flows: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }

    pub fn insert(&self, flow: LoginFlow, now_ms: u64) -> Result<(), AuthnError> {
        validate_flow(&flow, now_ms)?;
        let mut flows = self.flows.lock().map_err(|_| {
            AuthnError::new("login_flow_lock_poisoned", "login flow lock is poisoned")
        })?;
        flows.retain(|_, existing| existing.expires_at_ms > now_ms);

        if flows.contains_key(&flow.state) {
            return Err(AuthnError::new(
                "login_flow_state_collision",
                "login flow state already exists",
            ));
        }
        if flows.len() >= self.max_entries {
            return Err(AuthnError::new(
                "login_flow_capacity_exceeded",
                "bounded login flow store is full",
            ));
        }

        flows.insert(flow.state.clone(), flow);
        Ok(())
    }

    pub fn consume(&self, state: &str, now_ms: u64) -> Result<LoginFlow, AuthnError> {
        validate_flow_token(state, "state")?;
        let mut flows = self.flows.lock().map_err(|_| {
            AuthnError::new("login_flow_lock_poisoned", "login flow lock is poisoned")
        })?;
        let flow = flows.remove(state).ok_or_else(|| {
            AuthnError::new(
                "login_flow_not_found",
                "login flow state is absent or already consumed",
            )
        })?;

        if flow.expires_at_ms <= now_ms {
            return Err(AuthnError::new(
                "login_flow_expired",
                "login flow expired before callback",
            ));
        }
        Ok(flow)
    }

    pub fn len(&self, now_ms: u64) -> Result<usize, AuthnError> {
        let mut flows = self.flows.lock().map_err(|_| {
            AuthnError::new("login_flow_lock_poisoned", "login flow lock is poisoned")
        })?;
        flows.retain(|_, existing| existing.expires_at_ms > now_ms);
        Ok(flows.len())
    }
}

fn validate_flow(flow: &LoginFlow, now_ms: u64) -> Result<(), AuthnError> {
    validate_flow_token(&flow.state, "state")?;
    validate_flow_token(&flow.nonce, "nonce")?;
    validate_flow_token(&flow.pkce_verifier, "pkce_verifier")?;
    validate_return_path(&flow.return_path)?;
    if flow.expires_at_ms <= now_ms {
        return Err(AuthnError::new(
            "login_flow_expiry_invalid",
            "login flow expiry must be after insertion time",
        ));
    }
    Ok(())
}

fn validate_flow_token(value: &str, label: &'static str) -> Result<(), AuthnError> {
    if value.is_empty() || value.len() > MAX_FLOW_TOKEN_BYTES || value.chars().any(char::is_control)
    {
        return Err(AuthnError::new(
            "login_flow_token_invalid",
            format!("{label} must be bounded and contain no control characters"),
        ));
    }
    Ok(())
}

fn validate_return_path(value: &str) -> Result<(), AuthnError> {
    if value.is_empty()
        || value.len() > MAX_RETURN_PATH_BYTES
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(AuthnError::new(
            "return_path_invalid",
            "return path must be one bounded local absolute path",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::schema_migration::SqliteMigrationRuntime;

    use super::*;

    static NEXT_DB: AtomicU64 = AtomicU64::new(1);

    fn temp_db(label: &str) -> PathBuf {
        let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "chaptera-authn-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    async fn store(label: &str) -> (PathBuf, SqliteAuthnStore) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let store = SqliteAuthnStore::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (path, store)
    }

    async fn principal(
        store: &SqliteAuthnStore,
        id: &str,
        issuer: &str,
        subject: &str,
        email: &str,
    ) {
        store
            .resolve_principal(ResolvePrincipalRequest {
                issuer: issuer.into(),
                subject: subject.into(),
                proposed_principal_id: id.into(),
                email_snapshot: Some(email.into()),
                now_ms: 100,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn store_requires_operator_migration() {
        let path = temp_db("missing-schema");
        fs::write(&path, []).unwrap();
        let error = match SqliteAuthnStore::open(&path, 1, Duration::from_secs(1)).await {
            Ok(_) => panic!("authn store opened without operator migration"),
            Err(error) => error,
        };
        assert_eq!(error.code, "authn_schema_missing");
        cleanup(&path);
    }

    #[tokio::test]
    async fn issuer_subject_is_authority_and_email_is_snapshot_only() {
        let (path, store) = store("identity").await;

        let (first, _) = store
            .resolve_principal(ResolvePrincipalRequest {
                issuer: "https://id.example".into(),
                subject: "subject-a".into(),
                proposed_principal_id: "principal-a".into(),
                email_snapshot: Some("old@example.test".into()),
                now_ms: 100,
            })
            .await
            .unwrap();

        let (same, identity) = store
            .resolve_principal(ResolvePrincipalRequest {
                issuer: "https://id.example".into(),
                subject: "subject-a".into(),
                proposed_principal_id: "ignored-proposed-id".into(),
                email_snapshot: Some("new@example.test".into()),
                now_ms: 200,
            })
            .await
            .unwrap();

        assert_eq!(same.principal_id, first.principal_id);
        assert_eq!(identity.email_snapshot.as_deref(), Some("new@example.test"));

        let (other, _) = store
            .resolve_principal(ResolvePrincipalRequest {
                issuer: "https://other-id.example".into(),
                subject: "subject-b".into(),
                proposed_principal_id: "principal-b".into(),
                email_snapshot: Some("new@example.test".into()),
                now_ms: 300,
            })
            .await
            .unwrap();
        assert_ne!(other.principal_id, first.principal_id);

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn raw_session_and_csrf_tokens_are_not_persisted() {
        let (path, store) = store("hashes").await;
        principal(
            &store,
            "principal-a",
            "https://id.example",
            "subject-a",
            "a@example.test",
        )
        .await;

        let session = b"0123456789abcdef0123456789abcdef";
        let csrf = b"abcdef0123456789abcdef0123456789";
        store
            .create_session(CreateSessionRequest {
                principal_id: "principal-a".into(),
                raw_session_token: session,
                raw_csrf_token: csrf,
                session_generation: 1,
                created_at_ms: 100,
                idle_expires_at_ms: 500,
                absolute_expires_at_ms: 1_000,
            })
            .await
            .unwrap();

        let raw_session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE session_id_hash=?")
                .bind(session.as_slice())
                .fetch_one(&store.pool)
                .await
                .unwrap();
        let raw_csrf_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE csrf_token_hash=?")
                .bind(csrf.as_slice())
                .fetch_one(&store.pool)
                .await
                .unwrap();

        assert_eq!(raw_session_count, 0);
        assert_eq!(raw_csrf_count, 0);

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn session_revocation_expiry_and_principal_disable_fail_closed() {
        let (path, store) = store("session-state").await;
        principal(
            &store,
            "principal-a",
            "https://id.example",
            "subject-a",
            "a@example.test",
        )
        .await;

        let active = b"active-session-token-0123456789abcdef";
        let csrf = b"active-csrf-token----0123456789abcdef";
        store
            .create_session(CreateSessionRequest {
                principal_id: "principal-a".into(),
                raw_session_token: active,
                raw_csrf_token: csrf,
                session_generation: 1,
                created_at_ms: 100,
                idle_expires_at_ms: 500,
                absolute_expires_at_ms: 1_000,
            })
            .await
            .unwrap();

        let authenticated = store.authenticate_session(active, 200, 600).await.unwrap();
        assert_eq!(authenticated.principal_id, "principal-a");

        assert!(store.revoke_session(active, 250).await.unwrap());
        assert_eq!(
            store
                .authenticate_session(active, 300, 650)
                .await
                .unwrap_err()
                .code,
            "session_invalid"
        );

        let expired = b"expired-session-token-0123456789abcdef";
        store
            .create_session(CreateSessionRequest {
                principal_id: "principal-a".into(),
                raw_session_token: expired,
                raw_csrf_token: csrf,
                session_generation: 2,
                created_at_ms: 100,
                idle_expires_at_ms: 150,
                absolute_expires_at_ms: 1_000,
            })
            .await
            .unwrap();
        assert_eq!(
            store
                .authenticate_session(expired, 200, 600)
                .await
                .unwrap_err()
                .code,
            "session_invalid"
        );

        let disabled = b"disabled-session-token0123456789abcdef";
        store
            .create_session(CreateSessionRequest {
                principal_id: "principal-a".into(),
                raw_session_token: disabled,
                raw_csrf_token: csrf,
                session_generation: 3,
                created_at_ms: 100,
                idle_expires_at_ms: 500,
                absolute_expires_at_ms: 1_000,
            })
            .await
            .unwrap();
        assert!(store.disable_principal("principal-a", 250).await.unwrap());
        assert_eq!(
            store
                .authenticate_session(disabled, 300, 600)
                .await
                .unwrap_err()
                .code,
            "session_invalid"
        );

        store.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn csrf_rotation_invalidates_the_previous_secret() {
        let (path, store) = store("csrf").await;
        principal(
            &store,
            "principal-a",
            "https://id.example",
            "subject-a",
            "a@example.test",
        )
        .await;

        let session = b"csrf-session-token---0123456789abcdef";
        let old_csrf = b"old-csrf-token-------0123456789abcdef";
        let new_csrf = b"new-csrf-token-------0123456789abcdef";
        store
            .create_session(CreateSessionRequest {
                principal_id: "principal-a".into(),
                raw_session_token: session,
                raw_csrf_token: old_csrf,
                session_generation: 1,
                created_at_ms: 100,
                idle_expires_at_ms: 500,
                absolute_expires_at_ms: 1_000,
            })
            .await
            .unwrap();

        store.verify_csrf(session, old_csrf, 200).await.unwrap();
        store.rotate_csrf(session, new_csrf, 200).await.unwrap();
        assert_eq!(
            store
                .verify_csrf(session, old_csrf, 200)
                .await
                .unwrap_err()
                .code,
            "csrf_invalid"
        );
        store.verify_csrf(session, new_csrf, 200).await.unwrap();

        store.close().await;
        cleanup(&path);
    }

    #[test]
    fn login_flow_store_is_bounded_single_use_and_expiring() {
        let store = LoginFlowStore::new(2).unwrap();
        store
            .insert(
                LoginFlow {
                    state: "state-a".into(),
                    nonce: "nonce-a".into(),
                    pkce_verifier: "pkce-a".into(),
                    return_path: "/projects".into(),
                    expires_at_ms: 200,
                },
                100,
            )
            .unwrap();

        let consumed = store.consume("state-a", 150).unwrap();
        assert_eq!(consumed.nonce, "nonce-a");
        assert_eq!(
            store.consume("state-a", 150).unwrap_err().code,
            "login_flow_not_found"
        );

        store
            .insert(
                LoginFlow {
                    state: "state-expired".into(),
                    nonce: "nonce-expired".into(),
                    pkce_verifier: "pkce-expired".into(),
                    return_path: "/".into(),
                    expires_at_ms: 200,
                },
                100,
            )
            .unwrap();
        assert_eq!(
            store.consume("state-expired", 200).unwrap_err().code,
            "login_flow_expired"
        );

        for state in ["state-1", "state-2"] {
            store
                .insert(
                    LoginFlow {
                        state: state.into(),
                        nonce: format!("nonce-{state}"),
                        pkce_verifier: format!("pkce-{state}"),
                        return_path: "/".into(),
                        expires_at_ms: 1_000,
                    },
                    300,
                )
                .unwrap();
        }
        assert_eq!(
            store
                .insert(
                    LoginFlow {
                        state: "state-3".into(),
                        nonce: "nonce-3".into(),
                        pkce_verifier: "pkce-3".into(),
                        return_path: "/".into(),
                        expires_at_ms: 1_000,
                    },
                    300,
                )
                .unwrap_err()
                .code,
            "login_flow_capacity_exceeded"
        );
    }

    #[test]
    fn login_flow_rejects_external_return_path() {
        let store = LoginFlowStore::new(1).unwrap();
        let error = store
            .insert(
                LoginFlow {
                    state: "state".into(),
                    nonce: "nonce".into(),
                    pkce_verifier: "pkce".into(),
                    return_path: "//evil.example/path".into(),
                    expires_at_ms: 200,
                },
                100,
            )
            .unwrap_err();
        assert_eq!(error.code, "return_path_invalid");
    }
}
