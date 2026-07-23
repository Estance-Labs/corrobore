// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    fs,
    io::Write,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use corrobore_http_server::{AppState, LifecycleState, ServerConfig, build_router};
use serde_json::Value;
use tower::ServiceExt;

const FORCED_SHUTDOWN_EXIT_CODE: i32 = 7;

fn temp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-lifecycle-{name}-{suffix}"));
    fs::create_dir_all(&path).expect("test directory should be created");
    path
}

fn ephemeral_config() -> ServerConfig {
    ServerConfig::from_map(&HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        "lifecycle-secret".to_owned(),
    )]))
    .expect("ephemeral configuration should parse")
}

fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test port should be reserved");
    listener.local_addr().expect("local address").port()
}

fn spawn_server(
    storage_dir: &Path,
    runtime_dir: &Path,
    port: u16,
    shutdown_timeout_ms: u64,
) -> Child {
    Command::new(env!("CARGO_BIN_EXE_corrobore"))
        .args([
            "server",
            "start",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--auth-token",
            "lifecycle-process-secret",
            "--data-dir",
            runtime_dir
                .join(format!("runtime-{port}"))
                .to_str()
                .expect("UTF-8 path"),
            "--log-dir",
            runtime_dir
                .join(format!("logs-{port}"))
                .to_str()
                .expect("UTF-8 path"),
            "--storage-mode",
            "persistent",
            "--storage-dir",
            storage_dir.to_str().expect("UTF-8 path"),
            "--shutdown-timeout-ms",
            &shutdown_timeout_ms.to_string(),
        ])
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server should spawn")
}

fn wait_for_listener(child: &mut Child, port: u16) -> bool {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        if child
            .try_wait()
            .expect("server status should be readable")
            .is_some()
        {
            return false;
        }
        thread::sleep(Duration::from_millis(40));
    }
    false
}

fn wait_for_exit(mut child: Child) -> Output {
    for _ in 0..100 {
        if child
            .try_wait()
            .expect("server status should be readable")
            .is_some()
        {
            return child
                .wait_with_output()
                .expect("server output should be collected");
        }
        thread::sleep(Duration::from_millis(40));
    }
    let _ = child.kill();
    child
        .wait_with_output()
        .expect("timed-out server output should be collected")
}

fn send_signal(child: &Child, signal: &str) {
    let status = Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .expect("signal command should run");
    assert!(status.success(), "signal should be delivered");
}

#[test]
fn lifecycle_enters_ready_only_after_state_initialization_and_tracks_active_work() {
    let state = AppState::new(ephemeral_config()).expect("state should initialize");

    assert_eq!(state.lifecycle.state(), LifecycleState::Ready);
    let active = state
        .lifecycle
        .try_begin_request()
        .expect("ready lifecycle should accept work");
    assert_eq!(state.lifecycle.active_requests(), 1);

    state.lifecycle.begin_draining();
    assert_eq!(state.lifecycle.state(), LifecycleState::Draining);
    assert!(
        state.lifecycle.try_begin_request().is_err(),
        "draining lifecycle must reject new work"
    );

    drop(active);
    assert_eq!(state.lifecycle.active_requests(), 0);
}

#[tokio::test]
async fn protected_requests_receive_stable_service_unavailable_during_draining() {
    let state = AppState::new(ephemeral_config()).expect("state should initialize");
    state.lifecycle.begin_draining();
    let app = build_router(state);
    let health_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("health request should build"),
        )
        .await
        .expect("health should remain observable while draining");
    let health_payload: Value = serde_json::from_slice(
        &to_bytes(health_response.into_body(), usize::MAX)
            .await
            .expect("health body should be readable"),
    )
    .expect("health should be JSON");
    assert_eq!(health_payload["lifecycle_state"], "draining");
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header("authorization", "Bearer lifecycle-secret")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"query":"MATCH (n) RETURN n"}"#))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("router should respond");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable"),
    )
    .expect("response should be JSON");
    assert_eq!(payload["error"]["code"], "SERVICE_DRAINING");
}

#[test]
fn sigterm_and_sigint_share_clean_shutdown_and_allow_persistent_restart() {
    let directory = temp_dir("signals");
    let storage = directory.join("graph");

    for signal in ["-TERM", "-INT"] {
        let port = reserve_port();
        let mut child = spawn_server(&storage, &directory, port, 2_000);
        assert!(wait_for_listener(&mut child, port), "server should listen");

        send_signal(&child, signal);
        let output = wait_for_exit(child);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{signal} shutdown failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn shutdown_timeout_cancels_active_connection_and_uses_forced_exit_code() {
    let directory = temp_dir("timeout");
    let storage = directory.join("graph");
    let port = reserve_port();
    let mut child = spawn_server(&storage, &directory, port, 0);
    assert!(wait_for_listener(&mut child, port), "server should listen");

    let mut connection =
        TcpStream::connect(("127.0.0.1", port)).expect("active connection should open");
    connection
        .write_all(
            b"POST /v1/cypher/read HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer lifecycle-process-secret\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{",
        )
        .expect("partial active request should be written");
    thread::sleep(Duration::from_millis(50));

    send_signal(&child, "-TERM");
    let output = wait_for_exit(child);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(FORCED_SHUTDOWN_EXIT_CODE),
        "{stderr}"
    );
    assert!(stderr.contains("shutdown timeout"));
    assert!(!stderr.contains("lifecycle-process-secret"));

    drop(connection);
    let restart_port = reserve_port();
    let mut restarted = spawn_server(&storage, &directory, restart_port, 2_000);
    assert!(
        wait_for_listener(&mut restarted, restart_port),
        "forced shutdown must release ownership after the durability flush"
    );
    send_signal(&restarted, "-TERM");
    assert_eq!(wait_for_exit(restarted).status.code(), Some(0));
}
