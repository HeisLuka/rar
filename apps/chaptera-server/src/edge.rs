use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;
use tokio::time::timeout;
use url::Url;

use crate::config::{ChapteraConfig, ConfigError, EnvironmentMode};

const REQUEST_ID_HEADER: &str = "x-request-id";
const CSRF_HEADER: &str = "x-csrf-token";
const FORWARDED_HEADERS: [&str; 4] = [
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
];
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(pub String);

#[derive(Debug, Clone)]
pub struct EdgePolicy {
    public_origin: Option<String>,
    public_host: Option<String>,
    trusted_proxy_ips: BTreeSet<IpAddr>,
    require_https_proxy: bool,
    max_header_bytes: usize,
    max_api_body_bytes: u64,
    max_upload_body_bytes: u64,
    request_timeout: Duration,
}

impl EdgePolicy {
    pub fn from_config(config: &ChapteraConfig) -> Result<Self, ConfigError> {
        let (public_origin, public_host) = config
            .public_origin
            .as_deref()
            .map(canonical_origin)
            .transpose()?
            .map_or((None, None), |(origin, host)| (Some(origin), Some(host)));

        Ok(Self {
            public_origin,
            public_host,
            trusted_proxy_ips: config.edge.trusted_proxy_ips.iter().copied().collect(),
            require_https_proxy: config.environment == EnvironmentMode::Prod,
            max_header_bytes: usize::try_from(config.edge.max_header_bytes).map_err(|_| {
                ConfigError::new(
                    "edge_header_limit_invalid",
                    "edge header limit does not fit usize",
                )
            })?,
            max_api_body_bytes: config.edge.max_api_body_bytes,
            max_upload_body_bytes: config.edge.max_upload_body_bytes,
            request_timeout: Duration::from_millis(config.edge.request_timeout_ms),
        })
    }

    pub fn development() -> Self {
        Self {
            public_origin: Some("http://127.0.0.1:8080".to_owned()),
            public_host: Some("127.0.0.1:8080".to_owned()),
            trusted_proxy_ips: [
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ]
            .into_iter()
            .collect(),
            require_https_proxy: false,
            max_header_bytes: 32 * 1024,
            max_api_body_bytes: 8 * 1024 * 1024,
            max_upload_body_bytes: 256 * 1024 * 1024,
            request_timeout: Duration::from_secs(30),
        }
    }

    #[cfg(test)]
    fn test_prod() -> Self {
        Self {
            public_origin: Some("https://cloud.example.invalid".to_owned()),
            public_host: Some("cloud.example.invalid".to_owned()),
            trusted_proxy_ips: [IpAddr::V4(Ipv4Addr::LOCALHOST)].into_iter().collect(),
            require_https_proxy: true,
            max_header_bytes: 32 * 1024,
            max_api_body_bytes: 8 * 1024 * 1024,
            max_upload_body_bytes: 256 * 1024 * 1024,
            request_timeout: Duration::from_secs(30),
        }
    }
}

pub(crate) async fn enforce(
    State(policy): State<EdgePolicy>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    let health_route = matches!(path.as_str(), "/live" | "/ready" | "/version");
    let request_id = match request_id(request.headers()) {
        Ok(value) => value,
        Err(code) => {
            return reject(StatusCode::BAD_REQUEST, code, "chaptera-invalid-request-id");
        }
    };

    if header_bytes(request.headers()) > policy.max_header_bytes {
        return reject(
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "request_headers_too_large",
            &request_id,
        );
    }
    if request.method() == Method::CONNECT || request.method() == Method::TRACE {
        return reject(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            &request_id,
        );
    }

    let peer_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0.ip());
    let forwarded = FORWARDED_HEADERS
        .iter()
        .any(|name| request.headers().contains_key(*name));

    let client_ip = if forwarded {
        let Some(peer_ip) = peer_ip else {
            return reject(
                StatusCode::BAD_REQUEST,
                "forwarded_peer_unknown",
                &request_id,
            );
        };
        if !policy.trusted_proxy_ips.contains(&peer_ip) {
            return reject(
                StatusCode::FORBIDDEN,
                "untrusted_forwarded_peer",
                &request_id,
            );
        }
        if request.headers().contains_key("forwarded") {
            return reject(
                StatusCode::BAD_REQUEST,
                "forwarded_header_unsupported",
                &request_id,
            );
        }

        let proto = match single_header(request.headers(), "x-forwarded-proto") {
            Ok(Some(value)) => value,
            Ok(None) => {
                return reject(
                    StatusCode::BAD_REQUEST,
                    "forwarded_proto_missing",
                    &request_id,
                );
            }
            Err(code) => return reject(StatusCode::BAD_REQUEST, code, &request_id),
        };
        if proto != "https" {
            return reject(
                StatusCode::BAD_REQUEST,
                "forwarded_proto_invalid",
                &request_id,
            );
        }

        if let Some(expected_host) = policy.public_host.as_deref() {
            let host = match single_header(request.headers(), "x-forwarded-host") {
                Ok(Some(value)) => value,
                Ok(None) => {
                    return reject(
                        StatusCode::BAD_REQUEST,
                        "forwarded_host_missing",
                        &request_id,
                    );
                }
                Err(code) => return reject(StatusCode::BAD_REQUEST, code, &request_id),
            };
            if !host.eq_ignore_ascii_case(expected_host) {
                return reject(
                    StatusCode::MISDIRECTED_REQUEST,
                    "forwarded_host_mismatch",
                    &request_id,
                );
            }
        }

        let raw_client = match single_header(request.headers(), "x-forwarded-for") {
            Ok(Some(value)) => value,
            Ok(None) => {
                return reject(
                    StatusCode::BAD_REQUEST,
                    "forwarded_for_missing",
                    &request_id,
                );
            }
            Err(code) => return reject(StatusCode::BAD_REQUEST, code, &request_id),
        };
        if raw_client.contains(',') {
            return reject(
                StatusCode::BAD_REQUEST,
                "forwarded_for_ambiguous",
                &request_id,
            );
        }
        match raw_client.parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) => {
                return reject(
                    StatusCode::BAD_REQUEST,
                    "forwarded_for_invalid",
                    &request_id,
                );
            }
        }
    } else {
        if policy.require_https_proxy && !health_route {
            return reject(StatusCode::FORBIDDEN, "https_edge_required", &request_id);
        }
        peer_ip.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
    };

    if !health_route && let Some(expected_host) = policy.public_host.as_deref() {
        let host = match single_header(request.headers(), "host") {
            Ok(Some(value)) => value,
            Ok(None) => return reject(StatusCode::BAD_REQUEST, "host_missing", &request_id),
            Err(code) => return reject(StatusCode::BAD_REQUEST, code, &request_id),
        };
        if !host.eq_ignore_ascii_case(expected_host) {
            return reject(
                StatusCode::MISDIRECTED_REQUEST,
                "host_mismatch",
                &request_id,
            );
        }
    }

    let origin = match single_header(request.headers(), "origin") {
        Ok(value) => value,
        Err(code) => return reject(StatusCode::BAD_REQUEST, code, &request_id),
    };
    if let Some(origin) = origin.as_deref()
        && policy.public_origin.as_deref() != Some(origin)
    {
        return reject(StatusCode::FORBIDDEN, "origin_denied", &request_id);
    }

    let unsafe_browser_method = request.method() == Method::POST
        || request.method() == Method::PUT
        || request.method() == Method::PATCH
        || request.method() == Method::DELETE;
    if !health_route && unsafe_browser_method {
        if origin.is_none() {
            return reject(StatusCode::FORBIDDEN, "origin_required", &request_id);
        }
        let csrf = match single_header(request.headers(), CSRF_HEADER) {
            Ok(Some(value)) => value,
            Ok(None) => return reject(StatusCode::FORBIDDEN, "csrf_token_required", &request_id),
            Err(code) => return reject(StatusCode::BAD_REQUEST, code, &request_id),
        };
        if csrf.is_empty() || csrf.len() > 256 || csrf.bytes().any(|byte| byte.is_ascii_control()) {
            return reject(StatusCode::BAD_REQUEST, "csrf_token_invalid", &request_id);
        }
    }

    if request.headers().contains_key("transfer-encoding") {
        return reject(
            StatusCode::BAD_REQUEST,
            "transfer_encoding_rejected",
            &request_id,
        );
    }

    let max_body_bytes = if is_stream_upload(&path) {
        policy.max_upload_body_bytes
    } else {
        policy.max_api_body_bytes
    };
    let content_length = match single_header(request.headers(), "content-length") {
        Ok(value) => value,
        Err(code) => return reject(StatusCode::BAD_REQUEST, code, &request_id),
    };
    if (request.method() == Method::POST
        || request.method() == Method::PUT
        || request.method() == Method::PATCH)
        && content_length.is_none()
    {
        return reject(
            StatusCode::LENGTH_REQUIRED,
            "content_length_required",
            &request_id,
        );
    }
    if let Some(raw) = content_length {
        let Ok(length) = raw.parse::<u64>() else {
            return reject(
                StatusCode::BAD_REQUEST,
                "content_length_invalid",
                &request_id,
            );
        };
        if length > max_body_bytes {
            return reject(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_body_too_large",
                &request_id,
            );
        }
    }

    if request.method() == Method::OPTIONS && origin.is_some() && !health_route {
        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_response_headers(&mut response, &policy, origin.as_deref(), &request_id, true);
        return response;
    }

    request.extensions_mut().insert(ClientIp(client_ip));
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    let mut response = match timeout(policy.request_timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => reject(StatusCode::REQUEST_TIMEOUT, "request_timeout", &request_id),
    };
    apply_response_headers(
        &mut response,
        &policy,
        origin.as_deref(),
        &request_id,
        false,
    );
    response
}

fn canonical_origin(raw: &str) -> Result<(String, String), ConfigError> {
    let url = Url::parse(raw)
        .map_err(|_| ConfigError::new("public_origin_invalid", "public_origin is invalid"))?;
    let host = url
        .host_str()
        .ok_or_else(|| ConfigError::new("public_origin_invalid", "public_origin has no host"))?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let port = url
        .port()
        .filter(|port| !matches!((url.scheme(), *port), ("https", 443) | ("http", 80)));
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    Ok((format!("{}://{authority}", url.scheme()), authority))
}

fn request_id(headers: &HeaderMap) -> Result<String, &'static str> {
    if let Some(value) = single_header(headers, REQUEST_ID_HEADER)? {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
        {
            return Err("request_id_invalid");
        }
        return Ok(value);
    }
    let serial = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    Ok(format!("chaptera-{}-{serial}", std::process::id()))
}

fn single_header(headers: &HeaderMap, name: &'static str) -> Result<Option<String>, &'static str> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err("duplicate_header_rejected");
    }
    first
        .to_str()
        .map(|value| Some(value.trim().to_owned()))
        .map_err(|_| "header_encoding_invalid")
}

fn header_bytes(headers: &HeaderMap) -> usize {
    headers.iter().fold(0usize, |total, (name, value)| {
        total
            .saturating_add(name.as_str().len())
            .saturating_add(value.as_bytes().len())
            .saturating_add(4)
    })
}

fn is_stream_upload(path: &str) -> bool {
    path.starts_with("/v1/uploads/") && path.ends_with("/content")
}

fn reject(status: StatusCode, code: &'static str, request_id: &str) -> Response {
    let mut response = (
        status,
        Json(json!({"error": {"code": code, "request_id": request_id}})),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

fn apply_response_headers(
    response: &mut Response,
    policy: &EdgePolicy,
    origin: Option<&str>,
    request_id: &str,
    preflight: bool,
) {
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'"));
    if policy.require_https_proxy {
        headers.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert(REQUEST_ID_HEADER, value);
    }
    if let Some(origin) = origin
        && policy.public_origin.as_deref() == Some(origin)
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        headers.insert("access-control-allow-origin", value);
        headers.insert(
            "access-control-allow-credentials",
            HeaderValue::from_static("true"),
        );
        headers.append("vary", HeaderValue::from_static("Origin"));
        if preflight {
            headers.insert(
                "access-control-allow-methods",
                HeaderValue::from_static("GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS"),
            );
            headers.insert(
                "access-control-allow-headers",
                HeaderValue::from_static("Content-Type, X-CSRF-Token, X-Request-ID"),
            );
            headers.insert("access-control-max-age", HeaderValue::from_static("600"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        extract::Extension,
        http::Request as HttpRequest,
        middleware,
        routing::{get, post},
    };
    use tower::ServiceExt;

    fn app() -> Router {
        app_with_policy(EdgePolicy::test_prod())
    }

    fn app_with_policy(policy: EdgePolicy) -> Router {
        Router::new()
            .route("/v1/read", get(|| async { StatusCode::OK }))
            .route(
                "/v1/client-ip",
                get(|Extension(ClientIp(ip)): Extension<ClientIp>| async move {
                    if ip == "203.0.113.9".parse::<IpAddr>().unwrap() {
                        StatusCode::OK
                    } else {
                        StatusCode::BAD_REQUEST
                    }
                }),
            )
            .route("/v1/write", post(|| async { StatusCode::NO_CONTENT }))
            .route(
                "/v1/uploads/test/content",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .route(
                "/v1/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    StatusCode::OK
                }),
            )
            .layer(middleware::from_fn_with_state(policy, enforce))
    }

    fn proxied(method: Method, uri: &str) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header("host", "cloud.example.invalid")
            .header("x-forwarded-for", "203.0.113.9")
            .header("x-forwarded-host", "cloud.example.invalid")
            .header("x-forwarded-proto", "https")
            .header("content-length", "0")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:43123".parse::<SocketAddr>().unwrap(),
        ));
        request
    }

    #[tokio::test]
    async fn trusted_proxy_derives_client_ip_and_adds_security_headers() {
        let response = app()
            .oneshot(proxied(Method::GET, "/v1/client-ip"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("strict-transport-security"));
        assert!(response.headers().contains_key(REQUEST_ID_HEADER));
    }

    #[tokio::test]
    async fn forwarded_headers_from_unknown_peer_fail_closed() {
        let mut request = proxied(Method::GET, "/v1/read");
        request
            .extensions_mut()
            .insert(ConnectInfo("10.0.0.7:43123".parse::<SocketAddr>().unwrap()));
        assert_eq!(
            app().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn direct_non_health_request_is_not_an_https_edge_request() {
        let mut request = HttpRequest::builder()
            .uri("/v1/read")
            .header("host", "cloud.example.invalid")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:43123".parse::<SocketAddr>().unwrap(),
        ));
        assert_eq!(
            app().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn origin_and_csrf_preconditions_are_strict() {
        let mut cross_origin = proxied(Method::GET, "/v1/read");
        cross_origin
            .headers_mut()
            .insert("origin", HeaderValue::from_static("https://evil.example"));
        assert_eq!(
            app().oneshot(cross_origin).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let mut write = proxied(Method::POST, "/v1/write");
        write.headers_mut().insert(
            "origin",
            HeaderValue::from_static("https://cloud.example.invalid"),
        );
        assert_eq!(
            app().oneshot(write).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );

        let mut allowed = proxied(Method::POST, "/v1/write");
        allowed.headers_mut().insert(
            "origin",
            HeaderValue::from_static("https://cloud.example.invalid"),
        );
        allowed
            .headers_mut()
            .insert(CSRF_HEADER, HeaderValue::from_static("csrf-token"));
        assert_eq!(
            app().oneshot(allowed).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn body_and_forwarded_header_ambiguity_are_bounded() {
        let mut too_large = proxied(Method::POST, "/v1/write");
        too_large.headers_mut().insert(
            "origin",
            HeaderValue::from_static("https://cloud.example.invalid"),
        );
        too_large
            .headers_mut()
            .insert(CSRF_HEADER, HeaderValue::from_static("csrf-token"));
        too_large
            .headers_mut()
            .insert("content-length", HeaderValue::from_static("8388609"));
        assert_eq!(
            app().oneshot(too_large).await.unwrap().status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );

        let mut ambiguous = proxied(Method::GET, "/v1/read");
        ambiguous.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 198.51.100.4"),
        );
        assert_eq!(
            app().oneshot(ambiguous).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn header_method_upload_class_and_timeout_limits_fail_closed() {
        let mut oversized_header = proxied(Method::GET, "/v1/read");
        oversized_header.headers_mut().insert(
            "x-noise",
            HeaderValue::from_str(&"a".repeat(33 * 1024)).unwrap(),
        );
        assert_eq!(
            app().oneshot(oversized_header).await.unwrap().status(),
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
        );

        assert_eq!(
            app()
                .oneshot(proxied(Method::TRACE, "/v1/read"))
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );

        assert_eq!(
            app()
                .oneshot(proxied(Method::GET, "/v1/unknown"))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );

        let mut upload = proxied(Method::POST, "/v1/uploads/test/content");
        upload.headers_mut().insert(
            "origin",
            HeaderValue::from_static("https://cloud.example.invalid"),
        );
        upload
            .headers_mut()
            .insert(CSRF_HEADER, HeaderValue::from_static("csrf-token"));
        upload
            .headers_mut()
            .insert("content-length", HeaderValue::from_static("9437184"));
        assert_eq!(
            app().oneshot(upload).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );

        let mut policy = EdgePolicy::test_prod();
        policy.request_timeout = Duration::from_millis(5);
        assert_eq!(
            app_with_policy(policy)
                .oneshot(proxied(Method::GET, "/v1/slow"))
                .await
                .unwrap()
                .status(),
            StatusCode::REQUEST_TIMEOUT
        );
    }

    #[tokio::test]
    async fn same_origin_preflight_is_explicit() {
        let mut request = proxied(Method::OPTIONS, "/v1/write");
        request.headers_mut().insert(
            "origin",
            HeaderValue::from_static("https://cloud.example.invalid"),
        );
        let response = app().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "https://cloud.example.invalid"
        );
    }
}
