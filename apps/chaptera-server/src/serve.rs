use std::{
    collections::VecDeque,
    future::Future,
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::Instant,
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use serde_json::json;
use tokio::net::TcpListener;

use crate::{
    auth_http::{self, AuthHttpState},
    build_info::{BUILD_GIT_SHA, BUILD_IDENTITY},
    config::RuntimeConfig,
    edge::{self, EdgePolicy},
    shutdown,
    state::AppState,
};

const LOCAL_DASHBOARD_HTML: &str = include_str!("../assets/local-dashboard.html");
static LOCAL_EVENT_SEQ: AtomicU64 = AtomicU64::new(1);
static LOCAL_EVENTS: OnceLock<Mutex<VecDeque<LocalRequestEvent>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
struct LocalRequestEvent {
    seq: u64,
    level: &'static str,
    request_id: String,
    method: String,
    path: String,
    status: u16,
    duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<String>,
}

fn local_events_store() -> &'static Mutex<VecDeque<LocalRequestEvent>> {
    LOCAL_EVENTS.get_or_init(|| Mutex::new(VecDeque::with_capacity(256)))
}

pub fn router(state: AppState) -> Router {
    router_with_edge(state, EdgePolicy::development())
}

pub fn router_with_edge(state: AppState, edge_policy: EdgePolicy) -> Router {
    router_with_edge_and_auth(state, edge_policy, None)
}

pub fn router_with_edge_and_auth(
    state: AppState,
    edge_policy: EdgePolicy,
    auth: Option<AuthHttpState>,
) -> Router {
    router_with_edge_auth_and_local(state, edge_policy, auth, false)
}

pub fn router_with_edge_auth_and_local(
    state: AppState,
    edge_policy: EdgePolicy,
    auth: Option<AuthHttpState>,
    local_ui: bool,
) -> Router {
    let base = Router::new()
        .route("/live", get(live))
        .route("/ready", get(ready))
        .route("/version", get(version))
        .with_state(state.clone());

    let base = if local_ui {
        base.route("/local", get(local_dashboard))
            .route("/local/api/status", get(local_status))
            .route("/local/api/events", get(local_events))
    } else {
        base
    };

    let base = match auth {
        Some(auth) => base.merge(auth_http::router(auth)),
        None => base,
    };

    let base = base.layer(middleware::from_fn_with_state(edge_policy, edge::enforce));
    if local_ui {
        base.layer(middleware::from_fn(local_request_log))
    } else {
        base
    }
}

fn local_access_allowed(peer: SocketAddr, headers: &HeaderMap) -> bool {
    if !peer.ip().is_loopback() {
        return false;
    }
    let Some(host) = headers.get("host").and_then(|value| value.to_str().ok()) else {
        return false;
    };
    host.starts_with("127.0.0.1:")
        || host.starts_with("localhost:")
        || host.starts_with("[::1]:")
        || host == "127.0.0.1"
        || host == "localhost"
        || host == "[::1]"
}

async fn local_dashboard(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !local_access_allowed(peer, &headers) {
        return (StatusCode::NOT_FOUND, Html("not found")).into_response();
    }
    (StatusCode::OK, Html(LOCAL_DASHBOARD_HTML)).into_response()
}

async fn local_status(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !local_access_allowed(peer, &headers) {
        return (StatusCode::NOT_FOUND, Json(json!({"error":"not_found"}))).into_response();
    }
    Json(json!({
        "version": {"identity": BUILD_IDENTITY, "git_sha": BUILD_GIT_SHA},
        "readiness": state.readiness_report(),
    }))
    .into_response()
}

async fn local_events(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !local_access_allowed(peer, &headers) {
        return (StatusCode::NOT_FOUND, Json(json!({"error":"not_found"}))).into_response();
    }
    let events = local_events_store()
        .lock()
        .map(|items| items.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    Json(json!({"events": events})).into_response()
}

async fn local_request_log(request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let trace_id = request
        .headers()
        .get("x-chaptera-trace-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 160)
        .map(str::to_owned);
    let seq = LOCAL_EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
    let request_id = format!("req:{seq:08}");
    let start = Instant::now();
    let response = next.run(request).await;
    let status = response.status().as_u16();

    if path != "/local/api/events" && path != "/local/api/status" {
        let level = if status >= 500 {
            "error"
        } else if status >= 400 {
            "warn"
        } else {
            "info"
        };
        let event = LocalRequestEvent {
            seq,
            level,
            request_id,
            method,
            path,
            status,
            duration_ms: start.elapsed().as_millis(),
            trace_id,
        };
        if let Ok(mut items) = local_events_store().lock() {
            if items.len() >= 256 {
                items.pop_front();
            }
            items.push_back(event.clone());
        }
        if let Ok(encoded) = serde_json::to_string(&event) {
            eprintln!("chaptera_http {encoded}");
        }
    }

    response
}

async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "live" })))
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let report = state.readiness_report();
    let status = if report.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(report))
}

async fn version() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "version": BUILD_IDENTITY,
            "git_sha": BUILD_GIT_SHA,
        })),
    )
}

pub async fn run(
    config: RuntimeConfig,
    edge_policy: EdgePolicy,
    state: AppState,
) -> io::Result<()> {
    run_with_auth(config, edge_policy, state, None).await
}

pub async fn run_with_auth(
    config: RuntimeConfig,
    edge_policy: EdgePolicy,
    state: AppState,
    auth: Option<AuthHttpState>,
) -> io::Result<()> {
    run_with_auth_local(config, edge_policy, state, auth, false).await
}

pub async fn run_with_auth_local(
    config: RuntimeConfig,
    edge_policy: EdgePolicy,
    state: AppState,
    auth: Option<AuthHttpState>,
    local_ui: bool,
) -> io::Result<()> {
    config
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    let listener = TcpListener::bind(config.listen).await?;
    eprintln!(
        "chaptera serve listening on {} local_ui={}",
        listener.local_addr()?,
        local_ui
    );
    run_with_listener_policy_auth_and_local(
        listener,
        state,
        edge_policy,
        auth,
        local_ui,
        shutdown::signal(),
    )
    .await
}

pub async fn run_with_listener<F>(
    listener: TcpListener,
    state: AppState,
    shutdown: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    run_with_listener_policy_auth_and_local(
        listener,
        state,
        EdgePolicy::development(),
        None,
        false,
        shutdown,
    )
    .await
}

async fn run_with_listener_policy_auth_and_local<F>(
    listener: TcpListener,
    state: AppState,
    edge_policy: EdgePolicy,
    auth: Option<AuthHttpState>,
    local_ui: bool,
    shutdown: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(
        listener,
        router_with_edge_auth_and_local(state, edge_policy, auth, local_ui)
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}
