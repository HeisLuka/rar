use std::{fmt, time::Duration};

use rand::{RngCore, rngs::OsRng};

use crate::{
    authn::{AuthnError, CreateSessionRequest, ResolvePrincipalRequest, SqliteAuthnStore},
    oidc_authn::OidcVerifiedIdentity,
};

const OPAQUE_RANDOM_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    idle_ttl: Duration,
    absolute_ttl: Duration,
}

impl SessionPolicy {
    pub fn new(idle_ttl: Duration, absolute_ttl: Duration) -> Result<Self, AuthnError> {
        if idle_ttl.is_zero() {
            return Err(AuthnError::new(
                "session_idle_ttl_invalid",
                "session idle TTL must be positive",
            ));
        }
        if absolute_ttl < idle_ttl {
            return Err(AuthnError::new(
                "session_absolute_ttl_invalid",
                "session absolute TTL must not be shorter than idle TTL",
            ));
        }
        duration_millis(idle_ttl)?;
        duration_millis(absolute_ttl)?;
        Ok(Self {
            idle_ttl,
            absolute_ttl,
        })
    }
}

pub struct IssuedSession {
    pub principal_id: String,
    pub session_token: String,
    pub csrf_token: String,
    pub idle_expires_at_ms: i64,
    pub absolute_expires_at_ms: i64,
    pub return_path: String,
}

impl fmt::Debug for IssuedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedSession")
            .field("principal_id", &self.principal_id)
            .field("session_token", &"<redacted>")
            .field("csrf_token", &"<redacted>")
            .field("idle_expires_at_ms", &self.idle_expires_at_ms)
            .field("absolute_expires_at_ms", &self.absolute_expires_at_ms)
            .field("return_path", &self.return_path)
            .finish()
    }
}

pub async fn issue_verified_login_session(
    store: &SqliteAuthnStore,
    identity: OidcVerifiedIdentity,
    now_ms: i64,
    policy: SessionPolicy,
) -> Result<IssuedSession, AuthnError> {
    if now_ms < 0 {
        return Err(AuthnError::new(
            "authn_time_invalid",
            "now_ms must be non-negative",
        ));
    }

    let proposed_principal_id = format!("principal-{}", random_hex_256());
    let (principal, _) = store
        .resolve_principal(ResolvePrincipalRequest {
            issuer: identity.issuer,
            subject: identity.subject,
            proposed_principal_id,
            email_snapshot: identity.email_snapshot,
            now_ms,
        })
        .await?;

    let idle_expires_at_ms = checked_expiry(now_ms, policy.idle_ttl)?;
    let absolute_expires_at_ms = checked_expiry(now_ms, policy.absolute_ttl)?;
    let session_token = random_hex_256();
    let csrf_token = random_hex_256();

    store
        .create_session(CreateSessionRequest {
            principal_id: principal.principal_id.clone(),
            raw_session_token: session_token.as_bytes(),
            raw_csrf_token: csrf_token.as_bytes(),
            session_generation: 1,
            created_at_ms: now_ms,
            idle_expires_at_ms,
            absolute_expires_at_ms,
        })
        .await?;

    Ok(IssuedSession {
        principal_id: principal.principal_id,
        session_token,
        csrf_token,
        idle_expires_at_ms,
        absolute_expires_at_ms,
        return_path: identity.return_path,
    })
}

fn checked_expiry(now_ms: i64, ttl: Duration) -> Result<i64, AuthnError> {
    let ttl_ms = duration_millis(ttl)?;
    now_ms.checked_add(ttl_ms).ok_or_else(|| {
        AuthnError::new(
            "session_expiry_overflow",
            "session expiry exceeds supported timestamp range",
        )
    })
}

fn duration_millis(duration: Duration) -> Result<i64, AuthnError> {
    i64::try_from(duration.as_millis()).map_err(|_| {
        AuthnError::new(
            "session_ttl_invalid",
            "session TTL exceeds supported millisecond range",
        )
    })
}

fn random_hex_256() -> String {
    let mut random = [0_u8; OPAQUE_RANDOM_BYTES];
    OsRng.fill_bytes(&mut random);
    hex_encode(&random)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
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
            "chaptera-authn-session-{label}-{}-{serial}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[test]
    fn policy_rejects_zero_idle_and_absolute_before_idle() {
        assert_eq!(
            SessionPolicy::new(Duration::ZERO, Duration::from_secs(60))
                .unwrap_err()
                .code,
            "session_idle_ttl_invalid"
        );
        assert_eq!(
            SessionPolicy::new(Duration::from_secs(60), Duration::from_secs(30))
                .unwrap_err()
                .code,
            "session_absolute_ttl_invalid"
        );
    }

    #[tokio::test]
    async fn issued_session_is_random_persisted_and_debug_redacted() {
        let path = temp_db("issue");
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let store = SqliteAuthnStore::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();

        let issued = issue_verified_login_session(
            &store,
            OidcVerifiedIdentity {
                issuer: "https://id.example".into(),
                subject: "subject-a".into(),
                email_snapshot: Some("a@example.test".into()),
                return_path: "/projects/demo".into(),
            },
            100,
            SessionPolicy::new(Duration::from_secs(60), Duration::from_secs(3600)).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(issued.session_token.len(), 64);
        assert_eq!(issued.csrf_token.len(), 64);
        assert_ne!(issued.session_token, issued.csrf_token);
        assert!(issued.session_token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(issued.csrf_token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(issued.return_path, "/projects/demo");

        let debug = format!("{issued:?}");
        assert!(!debug.contains(&issued.session_token));
        assert!(!debug.contains(&issued.csrf_token));

        let session = store
            .authenticate_session(
                issued.session_token.as_bytes(),
                110,
                issued.idle_expires_at_ms,
            )
            .await
            .unwrap();
        assert_eq!(session.principal_id, issued.principal_id);

        store
            .verify_csrf(
                issued.session_token.as_bytes(),
                issued.csrf_token.as_bytes(),
                110,
            )
            .await
            .unwrap();

        store.close().await;
        cleanup(&path);
    }
}
