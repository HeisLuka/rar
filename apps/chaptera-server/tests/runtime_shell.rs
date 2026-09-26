use std::{process::Command, sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    routing::get,
};
use chaptera_server::{
    build_info::BUILD_IDENTITY,
    config::RuntimeConfig,
    serve,
    state::{AppState, DependencyFailure, RuntimeDependency, RuntimePorts},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};
use tower::ServiceExt;

struct ReadyDependency;

impl RuntimeDependency for ReadyDependency {
    fn check(&self) -> Result<(), DependencyFailure> {
        Ok(())
    }
}

fn ready_dependency() -> Arc<dyn RuntimeDependency> {
    Arc::new(ReadyDependency)
}

fn ready_state() -> AppState {
    AppState::new(RuntimePorts::new(
        ready_dependency(),
        ready_dependency(),
        ready_dependency(),
        ready_dependency(),
        ready_dependency(),
        ready_dependency(),
    ))
}

#[tokio::test]
async fn unconfigured_runtime_is_live_but_not_ready() {
    let state = AppState::new(RuntimePorts::unconfigured());
    let app = serve::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn injected_required_dependencies_make_runtime_ready() {
    let response = serve::router(ready_state())
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn local_dashboard_is_disabled_by_default() {
    let response = serve::router(ready_state())
        .oneshot(
            Request::builder()
                .uri("/local")
                .header("host", "127.0.0.1:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn local_dashboard_requires_explicit_local_mode_and_loopback_peer() {
    let app = serve::router_with_edge_auth_and_local(
        ready_state(),
        chaptera_server::edge::EdgePolicy::development(),
        None,
        true,
    );

    let mut request = Request::builder()
        .uri("/local")
        .header("host", "127.0.0.1:8080")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:49000".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut remote = Request::builder()
        .uri("/local")
        .header("host", "127.0.0.1:8080")
        .body(Body::empty())
        .unwrap();
    remote.extensions_mut().insert(ConnectInfo(
        "192.168.10.20:49000"
            .parse::<std::net::SocketAddr>()
            .unwrap(),
    ));
    let response = app.oneshot(remote).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn product_router_is_merged_under_the_same_edge_boundary() {
    let product = Router::new().route("/product-probe", get(|| async { StatusCode::NO_CONTENT }));
    let app = serve::router_with_edge_auth_local_and_product(
        ready_state(),
        chaptera_server::edge::EdgePolicy::development(),
        None,
        false,
        Some(product),
    );

    let mut request = Request::builder()
        .uri("/product-probe")
        .header("host", "127.0.0.1:8080")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:49000".parse::<std::net::SocketAddr>().unwrap(),
    ));

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn loopback_server_starts_answers_live_and_shuts_down() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let task = tokio::spawn(serve::run_with_listener(
        listener,
        AppState::new(RuntimePorts::unconfigured()),
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8(response).unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

    shutdown_tx.send(()).unwrap();
    timeout(Duration::from_secs(2), task)
        .await
        .expect("server did not shut down in time")
        .unwrap()
        .unwrap();
}

#[test]
fn config_rejects_public_listener() {
    let config = RuntimeConfig {
        listen: "0.0.0.0:8080".parse().unwrap(),
    };
    assert!(config.validate().is_err());
}

#[test]
fn version_flag_contains_build_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_chaptera"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(BUILD_IDENTITY), "{stdout}");
}

#[test]
fn unconfigured_commands_fail_closed() {
    for args in [
        vec!["worker"],
        vec!["doctor"],
        vec!["migrate", "status"],
        vec!["migrate", "up"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chaptera"))
            .args(&args)
            .output()
            .unwrap();

        assert!(
            !output.status.success(),
            "command {:?} unexpectedly succeeded",
            args
        );
    }
}
