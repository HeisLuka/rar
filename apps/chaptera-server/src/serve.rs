use std::{future::Future, io};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde_json::json;
use tokio::net::TcpListener;

use crate::{
    build_info::{BUILD_GIT_SHA, BUILD_IDENTITY},
    config::RuntimeConfig,
    shutdown,
    state::AppState,
};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/live", get(live))
        .route("/ready", get(ready))
        .route("/version", get(version))
        .with_state(state)
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

pub async fn run(config: RuntimeConfig, state: AppState) -> io::Result<()> {
    config
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    let listener = TcpListener::bind(config.listen).await?;
    eprintln!("chaptera serve listening on {}", listener.local_addr()?);

    run_with_listener(listener, state, shutdown::signal()).await
}

pub async fn run_with_listener<F>(
    listener: TcpListener,
    state: AppState,
    shutdown: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}
