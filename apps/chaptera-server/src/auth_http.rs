use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{
        HeaderMap, StatusCode,
        header::{HOST, ORIGIN},
    },
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::{Position, Url};

use crate::{
    authn::{AuthnError, LoginFlowStore, SqliteAuthnStore},
    authn_session::{SessionPolicy, issue_verified_login_session, rotate_session_csrf},
    oidc_authn::{OidcAdapterError, OidcAuthorizationAdapter},
};

pub const SESSION_COOKIE: &str = "__Host-chaptera_session";
pub const CSRF_HEADER: &str = "x-csrf-token";
pub const AUTH_CALLBACK_PATH: &str = "/v1/auth/callback";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub principal_id: String,
}

#[derive(Clone)]
pub struct AuthHttpState {
    oidc: OidcAuthorizationAdapter,
    flows: LoginFlowStore,
    store: SqliteAuthnStore,
    session_policy: SessionPolicy,
    login_ttl: Duration,
    origin: OriginPolicy,
}

impl AuthHttpState {
    pub fn new(
        oidc: OidcAuthorizationAdapter,
        flows: LoginFlowStore,
        store: SqliteAuthnStore,
        session_policy: SessionPolicy,
        login_ttl: Duration,
        public_origin: &str,
    ) -> Result<Self, AuthHttpError> {
        if login_ttl.is_zero() {
            return Err(AuthHttpError::internal("auth_login_ttl_invalid"));
        }
        Ok(Self {
            oidc,
            flows,
            store,
            session_policy,
            login_ttl,
            origin: OriginPolicy::new(public_origin)?,
        })
    }

    pub async fn authenticate_read_request(
        &self,
        headers: &HeaderMap,
        jar: &CookieJar,
    ) -> Result<AuthenticatedPrincipal, AuthHttpError> {
        self.authenticate_api_request(headers, jar, false).await
    }

    pub async fn authenticate_mutation_request(
        &self,
        headers: &HeaderMap,
        jar: &CookieJar,
    ) -> Result<AuthenticatedPrincipal, AuthHttpError> {
        self.authenticate_api_request(headers, jar, true).await
    }

    async fn authenticate_api_request(
        &self,
        headers: &HeaderMap,
        jar: &CookieJar,
        require_csrf: bool,
    ) -> Result<AuthenticatedPrincipal, AuthHttpError> {
        self.origin.require_host(headers)?;
        if require_csrf {
            self.origin.require_origin(headers)?;
        }

        let session_token = session_token(jar)?;
        let now = now_ms_i64()?;
        let refresh_idle_expires_at_ms = self
            .session_policy
            .refreshed_idle_expiry(now)
            .map_err(map_authn_error)?;
        let record = self
            .store
            .authenticate_session(session_token.as_bytes(), now, refresh_idle_expires_at_ms)
            .await
            .map_err(map_authn_error)?;

        if require_csrf {
            let csrf = headers
                .get(CSRF_HEADER)
                .ok_or_else(|| AuthHttpError::forbidden("csrf_missing"))?
                .to_str()
                .map_err(|_| AuthHttpError::forbidden("csrf_invalid"))?;
            self.store
                .verify_csrf(session_token.as_bytes(), csrf.as_bytes(), now)
                .await
                .map_err(map_authn_error)?;
        }

        Ok(AuthenticatedPrincipal {
            principal_id: record.principal_id,
        })
    }
}

pub fn router(state: AuthHttpState) -> Router {
    Router::new()
        .route("/v1/auth/login", get(login))
        .route(AUTH_CALLBACK_PATH, get(callback))
        .route("/v1/session", get(session))
        .route("/v1/auth/logout", post(logout))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct LoginQuery {
    return_path: Option<String>,
}

async fn login(
    State(state): State<AuthHttpState>,
    headers: HeaderMap,
    Query(query): Query<LoginQuery>,
) -> Result<Redirect, AuthHttpError> {
    state.origin.require_host(&headers)?;
    let now = now_ms_u64()?;
    let start = state
        .oidc
        .begin_login(
            &state.flows,
            query.return_path.as_deref().unwrap_or("/"),
            now,
            state.login_ttl,
        )
        .map_err(map_oidc_error)?;
    Ok(Redirect::to(&start.authorization_url))
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(state): State<AuthHttpState>,
    headers: HeaderMap,
    jar: CookieJar,
    Query(query): Query<CallbackQuery>,
) -> Result<(CookieJar, Redirect), AuthHttpError> {
    state.origin.require_host(&headers)?;
    if query.error.is_some() {
        return Err(AuthHttpError::unauthorized("oidc_provider_error"));
    }
    let state_token = query
        .state
        .as_deref()
        .ok_or_else(|| AuthHttpError::bad_request("oidc_state_missing"))?;
    let code = query
        .code
        .as_deref()
        .ok_or_else(|| AuthHttpError::bad_request("oidc_code_missing"))?;
    let now_u64 = now_ms_u64()?;
    let identity = state
        .oidc
        .finish_login(&state.flows, state_token, code, now_u64)
        .await
        .map_err(map_oidc_error)?;
    let issued = issue_verified_login_session(
        &state.store,
        identity,
        i64::try_from(now_u64).map_err(|_| AuthHttpError::internal("clock_out_of_range"))?,
        state.session_policy,
    )
    .await
    .map_err(map_authn_error)?;

    let redirect = Redirect::to(&issued.return_path);
    Ok((jar.add(session_cookie(issued.session_token)), redirect))
}

#[derive(Debug, Serialize)]
struct SessionPayload {
    principal_id: String,
    csrf_token: String,
    idle_expires_at_ms: i64,
    absolute_expires_at_ms: i64,
}

async fn session(
    State(state): State<AuthHttpState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<SessionPayload>, AuthHttpError> {
    state.origin.require_host(&headers)?;
    let session_token = session_token(&jar)?;
    let now = now_ms_i64()?;
    let refresh_idle_expires_at_ms = state
        .session_policy
        .refreshed_idle_expiry(now)
        .map_err(map_authn_error)?;
    let record = state
        .store
        .authenticate_session(session_token.as_bytes(), now, refresh_idle_expires_at_ms)
        .await
        .map_err(map_authn_error)?;
    let csrf_token = rotate_session_csrf(&state.store, &session_token, now)
        .await
        .map_err(map_authn_error)?;

    Ok(Json(SessionPayload {
        principal_id: record.principal_id,
        csrf_token,
        idle_expires_at_ms: record.idle_expires_at_ms,
        absolute_expires_at_ms: record.absolute_expires_at_ms,
    }))
}

async fn logout(
    State(state): State<AuthHttpState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), AuthHttpError> {
    state.origin.require_host(&headers)?;
    state.origin.require_origin(&headers)?;
    let session_token = session_token(&jar)?;
    let csrf = headers
        .get(CSRF_HEADER)
        .ok_or_else(|| AuthHttpError::forbidden("csrf_missing"))?
        .to_str()
        .map_err(|_| AuthHttpError::forbidden("csrf_invalid"))?;

    let now = now_ms_i64()?;
    state
        .store
        .verify_csrf(session_token.as_bytes(), csrf.as_bytes(), now)
        .await
        .map_err(map_authn_error)?;
    let revoked = state
        .store
        .revoke_session(session_token.as_bytes(), now)
        .await
        .map_err(map_authn_error)?;
    if !revoked {
        return Err(AuthHttpError::unauthorized("session_invalid"));
    }

    Ok((jar.remove(session_cookie_removal()), StatusCode::NO_CONTENT))
}

fn session_token(jar: &CookieJar) -> Result<String, AuthHttpError> {
    jar.get(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AuthHttpError::unauthorized("session_missing"))
}

fn session_cookie(value: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, value))
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .build()
}

fn session_cookie_removal() -> Cookie<'static> {
    Cookie::build(SESSION_COOKIE)
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::Lax)
        .build()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginPolicy {
    expected_origin: String,
    expected_host: String,
}

impl OriginPolicy {
    fn new(public_origin: &str) -> Result<Self, AuthHttpError> {
        let parsed = Url::parse(public_origin)
            .map_err(|_| AuthHttpError::internal("public_origin_invalid"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(AuthHttpError::internal("public_origin_invalid"));
        }
        let expected_origin = parsed.origin().ascii_serialization();
        let expected_host = parsed[Position::BeforeHost..Position::AfterPort].to_owned();
        if expected_host.is_empty() {
            return Err(AuthHttpError::internal("public_origin_invalid"));
        }
        Ok(Self {
            expected_origin,
            expected_host,
        })
    }

    fn require_host(&self, headers: &HeaderMap) -> Result<(), AuthHttpError> {
        let host = headers
            .get(HOST)
            .ok_or_else(|| AuthHttpError::forbidden("host_missing"))?
            .to_str()
            .map_err(|_| AuthHttpError::forbidden("host_invalid"))?;
        if host != self.expected_host {
            return Err(AuthHttpError::forbidden("host_mismatch"));
        }
        Ok(())
    }

    fn require_origin(&self, headers: &HeaderMap) -> Result<(), AuthHttpError> {
        let origin = headers
            .get(ORIGIN)
            .ok_or_else(|| AuthHttpError::forbidden("origin_missing"))?
            .to_str()
            .map_err(|_| AuthHttpError::forbidden("origin_invalid"))?;
        if origin != self.expected_origin {
            return Err(AuthHttpError::forbidden("origin_mismatch"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthHttpError {
    status: StatusCode,
    code: &'static str,
}

impl AuthHttpError {
    fn bad_request(code: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
        }
    }

    fn unauthorized(code: &'static str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code,
        }
    }

    fn forbidden(code: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
        }
    }

    fn internal(code: &'static str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
        }
    }
}

impl fmt::Display for AuthHttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for AuthHttpError {}

impl IntoResponse for AuthHttpError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.code }))).into_response()
    }
}

fn map_oidc_error(error: OidcAdapterError) -> AuthHttpError {
    match error.code {
        "login_flow_not_found"
        | "login_flow_expired"
        | "login_flow_token_invalid"
        | "oidc_authorization_code_invalid"
        | "oidc_id_token_missing"
        | "oidc_id_token_invalid" => AuthHttpError::unauthorized(error.code),
        "return_path_invalid" | "oidc_login_ttl_invalid" => AuthHttpError::bad_request(error.code),
        "oidc_code_exchange_failed" => AuthHttpError {
            status: StatusCode::BAD_GATEWAY,
            code: error.code,
        },
        _ => AuthHttpError::internal(error.code),
    }
}

fn map_authn_error(error: AuthnError) -> AuthHttpError {
    match error.code {
        "session_invalid" | "session_missing" => AuthHttpError::unauthorized(error.code),
        "csrf_invalid" => AuthHttpError::forbidden(error.code),
        _ => AuthHttpError::internal(error.code),
    }
}

fn now_ms_u64() -> Result<u64, AuthHttpError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthHttpError::internal("clock_invalid"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| AuthHttpError::internal("clock_out_of_range"))
}

fn now_ms_i64() -> Result<i64, AuthHttpError> {
    let millis = now_ms_u64()?;
    i64::try_from(millis).map_err(|_| AuthHttpError::internal("clock_out_of_range"))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn session_cookie_is_host_only_secure_http_only_and_lax() {
        let rendered = session_cookie("secret".to_owned()).to_string();
        assert!(rendered.starts_with("__Host-chaptera_session=secret"));
        assert!(rendered.contains("Path=/"));
        assert!(rendered.contains("Secure"));
        assert!(rendered.contains("HttpOnly"));
        assert!(rendered.contains("SameSite=Lax"));
        assert!(!rendered.contains("Domain="));
    }

    #[test]
    fn origin_policy_rejects_forged_host_and_origin() {
        let policy = OriginPolicy::new("https://cloud.example.test").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_static("cloud.example.test"));
        headers.insert(
            ORIGIN,
            HeaderValue::from_static("https://cloud.example.test"),
        );
        policy.require_host(&headers).unwrap();
        policy.require_origin(&headers).unwrap();

        headers.insert(HOST, HeaderValue::from_static("evil.example.test"));
        assert_eq!(
            policy.require_host(&headers).unwrap_err().code,
            "host_mismatch"
        );
        headers.insert(HOST, HeaderValue::from_static("cloud.example.test"));
        headers.insert(
            ORIGIN,
            HeaderValue::from_static("https://evil.example.test"),
        );
        assert_eq!(
            policy.require_origin(&headers).unwrap_err().code,
            "origin_mismatch"
        );
    }

    #[test]
    fn origin_policy_rejects_path_query_and_credentials() {
        for value in [
            "https://cloud.example.test/path",
            "https://cloud.example.test/?x=1",
            "https://user@cloud.example.test/",
        ] {
            assert_eq!(
                OriginPolicy::new(value).unwrap_err().code,
                "public_origin_invalid"
            );
        }
    }
}
