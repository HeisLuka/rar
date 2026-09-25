use std::{fmt, time::Duration};

use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    reqwest,
};

use crate::authn::{AuthnError, LoginFlow, LoginFlowStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcAdapterError {
    pub code: &'static str,
    pub message: String,
}

impl OidcAdapterError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for OidcAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for OidcAdapterError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcLoginStart {
    pub authorization_url: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcVerifiedIdentity {
    pub issuer: String,
    pub subject: String,
    pub email_snapshot: Option<String>,
    pub return_path: String,
}

#[derive(Clone)]
pub struct OidcAuthorizationAdapter {
    provider_metadata: CoreProviderMetadata,
    client_id: ClientId,
    client_secret: Option<ClientSecret>,
    redirect_url: RedirectUrl,
}

impl OidcAuthorizationAdapter {
    pub async fn discover(
        issuer: &str,
        client_id: String,
        client_secret: Option<String>,
        redirect_url: String,
    ) -> Result<Self, OidcAdapterError> {
        let issuer = IssuerUrl::new(issuer.to_owned()).map_err(|error| {
            OidcAdapterError::new("oidc_issuer_invalid", bounded_message(error))
        })?;
        let redirect_url = RedirectUrl::new(redirect_url).map_err(|error| {
            OidcAdapterError::new("oidc_redirect_invalid", bounded_message(error))
        })?;

        let http_client = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                OidcAdapterError::new("oidc_http_client_failed", bounded_message(error))
            })?;

        let provider_metadata = CoreProviderMetadata::discover_async(issuer, &http_client)
            .await
            .map_err(|error| {
                OidcAdapterError::new("oidc_discovery_failed", bounded_message(error))
            })?;

        Ok(Self {
            provider_metadata,
            client_id: ClientId::new(client_id),
            client_secret: client_secret.map(ClientSecret::new),
            redirect_url,
        })
    }

    pub fn begin_login(
        &self,
        flows: &LoginFlowStore,
        return_path: &str,
        now_ms: u64,
        ttl: Duration,
    ) -> Result<OidcLoginStart, OidcAdapterError> {
        if ttl.is_zero() {
            return Err(OidcAdapterError::new(
                "oidc_login_ttl_invalid",
                "OIDC login flow TTL must be positive",
            ));
        }

        let ttl_ms = u64::try_from(ttl.as_millis()).map_err(|_| {
            OidcAdapterError::new(
                "oidc_login_ttl_invalid",
                "OIDC login flow TTL does not fit milliseconds",
            )
        })?;
        let expires_at_ms = now_ms.checked_add(ttl_ms).ok_or_else(|| {
            OidcAdapterError::new(
                "oidc_login_ttl_invalid",
                "OIDC login flow expiry overflows milliseconds",
            )
        })?;

        let client = CoreClient::from_provider_metadata(
            self.provider_metadata.clone(),
            self.client_id.clone(),
            self.client_secret.clone(),
        )
        .set_redirect_uri(self.redirect_url.clone());

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (authorization_url, state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("openid".to_owned()))
            .set_pkce_challenge(pkce_challenge)
            .url();

        flows
            .insert(
                LoginFlow {
                    state: state.secret().to_owned(),
                    nonce: nonce.secret().to_owned(),
                    pkce_verifier: pkce_verifier.secret().to_owned(),
                    return_path: return_path.to_owned(),
                    expires_at_ms,
                },
                now_ms,
            )
            .map_err(authn_error)?;

        Ok(OidcLoginStart {
            authorization_url: authorization_url.to_string(),
            expires_at_ms,
        })
    }

    pub async fn finish_login(
        &self,
        flows: &LoginFlowStore,
        state: &str,
        authorization_code: &str,
        now_ms: u64,
    ) -> Result<OidcVerifiedIdentity, OidcAdapterError> {
        if authorization_code.is_empty()
            || authorization_code.len() > 4096
            || authorization_code.chars().any(char::is_control)
        {
            return Err(OidcAdapterError::new(
                "oidc_authorization_code_invalid",
                "OIDC authorization code must be a bounded non-control string",
            ));
        }

        let flow = flows.consume(state, now_ms).map_err(authn_error)?;
        let client = CoreClient::from_provider_metadata(
            self.provider_metadata.clone(),
            self.client_id.clone(),
            self.client_secret.clone(),
        )
        .set_redirect_uri(self.redirect_url.clone());

        let http_client = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| {
                OidcAdapterError::new("oidc_http_client_failed", bounded_message(error))
            })?;

        let token_request = client
            .exchange_code(AuthorizationCode::new(authorization_code.to_owned()))
            .map_err(|error| {
                OidcAdapterError::new("oidc_token_endpoint_unavailable", bounded_message(error))
            })?;

        let token_response = token_request
            .set_pkce_verifier(PkceCodeVerifier::new(flow.pkce_verifier))
            .request_async(&http_client)
            .await
            .map_err(|error| {
                OidcAdapterError::new("oidc_code_exchange_failed", bounded_message(error))
            })?;

        let id_token = token_response.id_token().ok_or_else(|| {
            OidcAdapterError::new(
                "oidc_id_token_missing",
                "OIDC token response did not contain an ID token",
            )
        })?;
        let verifier = client.id_token_verifier();
        let nonce = Nonce::new(flow.nonce);
        let claims = id_token.claims(&verifier, &nonce).map_err(|error| {
            OidcAdapterError::new("oidc_id_token_invalid", bounded_message(error))
        })?;

        Ok(OidcVerifiedIdentity {
            issuer: claims.issuer().as_str().to_owned(),
            subject: claims.subject().as_str().to_owned(),
            email_snapshot: claims.email().map(|email| email.as_str().to_owned()),
            return_path: flow.return_path,
        })
    }
}

fn authn_error(error: AuthnError) -> OidcAdapterError {
    OidcAdapterError::new(error.code, error.message)
}

fn bounded_message(error: impl fmt::Display) -> String {
    error.to_string().chars().take(512).collect()
}
