use std::{future::Future, io, net::SocketAddr};

use axum::{
    Json, Router, extract::State, http::StatusCode, middleware, response::IntoResponse,
    routing::get,
};
use serde_json::json;
use tokio::net::TcpListener;

use crate::{
    auth_http::{self, AuthHttpState},
    build_info::{BUILD_GIT_SHA, BUILD_IDENTITY},
    config::RuntimeConfig,
    diagnostics::{self, Diagnostics},
    edge::{self, EdgePolicy},
    shutdown,
    state::AppState,
};

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
    router_with_edge_auth_and_console(state, edge_policy, auth, false)
}

pub fn router_with_edge_auth_and_console(
    state: AppState,
    edge_policy: EdgePolicy,
    auth: Option<AuthHttpState>,
    local_console: bool,
) -> Router {
    let diagnostics = Diagnostics::default();
    let base = Router::new()
        .route("/live", get(live))
        .route("/ready", get(ready))
        .route("/version", get(version))
        .with_state(state);
    let base = match auth {
        Some(auth) => base.merge(auth_http::router(auth)),
        None => base,
    };
    let base = if local_console {
        base.merge(diagnostics::local_console_router(diagnostics.clone()))
    } else {
        base
    };

    base.layer(middleware::from_fn_with_state(edge_policy, edge::enforce))
        .layer(middleware::from_fn_with_state(
            diagnostics,
            diagnostics::record_requests,
        ))
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
    run_mode(config, edge_policy, state, None, false).await
}

pub async fn run_with_auth(
    config: RuntimeConfig,
    edge_policy: EdgePolicy,
    state: AppState,
    auth: Option<AuthHttpState>,
) -> io::Result<()> {
    run_mode(config, edge_policy, state, auth, false).await
}

pub async fn run_local(
    config: RuntimeConfig,
    edge_policy: EdgePolicy,
    state: AppState,
) -> io::Result<()> {
    run_mode(config, edge_policy, state, None, true).await
}

pub async fn run_local_with_auth(
    config: RuntimeConfig,
    edge_policy: EdgePolicy,
    state: AppState,
    auth: Option<AuthHttpState>,
) -> io::Result<()> {
    run_mode(config, edge_policy, state, auth, true).await
}

async fn run_mode(
    config: RuntimeConfig,
    edge_policy: EdgePolicy,
    state: AppState,
    auth: Option<AuthHttpState>,
    local_console: bool,
) -> io::Result<()> {
    config
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    let listener = TcpListener::bind(config.listen).await?;
    let address = listener.local_addr()?;
    eprintln!(
        "{}",
        json!({
            "level": "info",
            "component": "server",
            "event": "listening",
            "address": address.to_string(),
            "local_console": local_console,
        })
    );
    run_with_listener_policy_and_auth(
        listener,
        state,
        edge_policy,
        auth,
        local_console,
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
    run_with_listener_policy_and_auth(
        listener,
        state,
        EdgePolicy::development(),
        None,
        false,
        shutdown,
    )
    .await
}

async fn run_with_listener_policy_and_auth<F>(
    listener: TcpListener,
    state: AppState,
    edge_policy: EdgePolicy,
    auth: Option<AuthHttpState>,
    local_console: bool,
    shutdown: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(
        listener,
        router_with_edge_auth_and_console(state, edge_policy, auth, local_console)
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}
