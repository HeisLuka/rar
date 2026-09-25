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
    let base = Router::new()
        .route("/live", get(live))
        .route("/ready", get(ready))
        .route("/version", get(version))
        .with_state(state);
    let base = match auth {
        Some(auth) => base.merge(auth_http::router(auth)),
        None => base,
    };
    base.layer(middleware::from_fn_with_state(edge_policy, edge::enforce))
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
    config
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    let listener = TcpListener::bind(config.listen).await?;
    eprintln!("chaptera serve listening on {}", listener.local_addr()?);
    run_with_listener_policy_and_auth(
        listener,
        state,
        edge_policy,
        auth,
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
        shutdown,
    )
    .await
}

async fn run_with_listener_policy_and_auth<F>(
    listener: TcpListener,
    state: AppState,
    edge_policy: EdgePolicy,
    auth: Option<AuthHttpState>,
    shutdown: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(
        listener,
        router_with_edge_and_auth(state, edge_policy, auth)
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}
