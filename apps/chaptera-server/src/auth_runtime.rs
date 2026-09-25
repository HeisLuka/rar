use std::{fmt, time::Duration};

use url::Url;
use zeroize::Zeroizing;

use crate::{
    auth_http::{AUTH_CALLBACK_PATH, AuthHttpState},
    authn::{LoginFlowStore, SqliteAuthnStore},
    authn_session::SessionPolicy,
    config::{ChapteraConfig, ResolvedSecrets},
    oidc_authn::OidcAuthorizationAdapter,
};

const LOGIN_FLOW_CAPACITY: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRuntimeError {
    pub code: &'static str,
    pub message: String,
}

impl AuthRuntimeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AuthRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AuthRuntimeError {}

#[derive(Clone)]
pub struct AuthRuntime {
    http_state: AuthHttpState,
    _store: SqliteAuthnStore,
}

impl AuthRuntime {
    pub async fn open(
        config: &ChapteraConfig,
        secrets: &ResolvedSecrets,
    ) -> Result<Self, AuthRuntimeError> {
        let auth = config.auth.as_ref().ok_or_else(|| {
            AuthRuntimeError::new("auth_config_missing", "auth configuration is missing")
        })?;
        let public_origin = config.public_origin.as_deref().ok_or_else(|| {
            AuthRuntimeError::new(
                "public_origin_missing",
                "public_origin is required for AuthN",
            )
        })?;

        require_canonical_callback_path(&auth.oidc.redirect_path)?;

        let client_secret = secrets.oidc_client_secret.as_ref().ok_or_else(|| {
            AuthRuntimeError::new(
                "oidc_client_secret_missing",
                "resolved OIDC client secret is missing",
            )
        })?;
        let client_secret = Zeroizing::new(
            String::from_utf8(client_secret.expose().to_vec()).map_err(|_| {
                AuthRuntimeError::new(
                    "oidc_client_secret_not_utf8",
                    "OIDC client secret must be valid UTF-8",
                )
            })?,
        );

        let redirect_url = redirect_url(public_origin, &auth.oidc.redirect_path)?;
        let oidc = OidcAuthorizationAdapter::discover(
            &auth.oidc.issuer,
            auth.oidc.client_id.clone(),
            Some(client_secret.as_str().to_owned()),
            redirect_url,
        )
        .await
        .map_err(|error| AuthRuntimeError::new(error.code, error.message))?;

        let store = SqliteAuthnStore::open(
            &config.sqlite.path,
            config.sqlite.pool_max,
            Duration::from_millis(config.sqlite.busy_timeout_ms),
        )
        .await
        .map_err(|error| AuthRuntimeError::new(error.code, error.message))?;

        let session_policy = SessionPolicy::new(
            Duration::from_secs(auth.session_idle_ttl_seconds),
            Duration::from_secs(auth.session_absolute_ttl_seconds),
        )
        .map_err(|error| AuthRuntimeError::new(error.code, error.message))?;
        let flows = LoginFlowStore::new(LOGIN_FLOW_CAPACITY)
            .map_err(|error| AuthRuntimeError::new(error.code, error.message))?;
        let http_state = AuthHttpState::new(
            oidc,
            flows,
            store.clone(),
            session_policy,
            Duration::from_secs(auth.login_flow_ttl_seconds),
            public_origin,
        )
        .map_err(|error| AuthRuntimeError::new("auth_http_state_invalid", error.to_string()))?;

        Ok(Self {
            http_state,
            _store: store,
        })
    }

    pub fn http_state(&self) -> AuthHttpState {
        self.http_state.clone()
    }
}

fn require_canonical_callback_path(redirect_path: &str) -> Result<(), AuthRuntimeError> {
    if redirect_path == AUTH_CALLBACK_PATH {
        Ok(())
    } else {
        Err(AuthRuntimeError::new(
            "oidc_redirect_path_mismatch",
            format!(
                "auth.oidc.redirect_path must be {AUTH_CALLBACK_PATH:?} for the mounted V0 callback route"
            ),
        ))
    }
}

fn redirect_url(public_origin: &str, redirect_path: &str) -> Result<String, AuthRuntimeError> {
    let mut origin = Url::parse(public_origin).map_err(|_| {
        AuthRuntimeError::new("public_origin_invalid", "public_origin is not a valid URL")
    })?;
    origin.set_path(redirect_path);
    origin.set_query(None);
    origin.set_fragment(None);
    Ok(origin.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_callback_path_must_match_mounted_route() {
        require_canonical_callback_path(AUTH_CALLBACK_PATH).unwrap();
        let error = require_canonical_callback_path("/different/callback").unwrap_err();
        assert_eq!(error.code, "oidc_redirect_path_mismatch");
    }

    #[test]
    fn redirect_url_is_derived_from_validated_public_origin() {
        assert_eq!(
            redirect_url("https://cloud.example.test", "/v1/auth/callback").unwrap(),
            "https://cloud.example.test/v1/auth/callback"
        );
    }
}
