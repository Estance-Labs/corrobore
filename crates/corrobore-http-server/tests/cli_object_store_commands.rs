// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Issue #376: offline storage commands that use an object store run outside
//! the binary's async runtime, so they work instead of panicking.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use tower::ServiceExt;

static DIRS: AtomicUsize = AtomicUsize::new(0);

fn corrobore() -> Command {
    Command::new(env!("CARGO_BIN_EXE_corrobore"))
}

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-cli-376-{name}-{}-{unique}-{}",
        std::process::id(),
        DIRS.fetch_add(1, Ordering::Relaxed)
    ))
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the corrobore binary runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A persistent storage root holding one committed write, so it has a
/// checkpoint to snapshot.
async fn storage(root: &Path) -> PathBuf {
    let storage = root.join("graph");
    let vars = HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), "token".to_owned()),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage.display().to_string(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            root.join("sessions").display().to_string(),
        ),
    ]);
    let app = build_router(AppState::new(ServerConfig::from_map(&vars).unwrap()).unwrap());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/cypher/write")
                .header(header::AUTHORIZATION, "Bearer token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"query": "CREATE (n:Indicator {name: 'cli-376'})"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    storage
}

#[tokio::test]
async fn import_snapshot_reads_an_object_store_url_through_the_binary() {
    let root = temp_dir("import");
    let storage = storage(&root).await;
    let exported = root.join("bucket").join("backups").join("2026-11-15");
    let output = run(corrobore().args([
        "server",
        "snapshot",
        "--storage-dir",
        storage.to_str().unwrap(),
        "--destination",
        exported.to_str().unwrap(),
    ]));
    assert!(output.status.success(), "snapshot: {}", stderr(&output));

    // file:// goes through the same object_store client as s3://.
    let imported = root.join("imported");
    let source = format!("file://{}", root.join("bucket").display());
    let output = run(corrobore().args([
        "server",
        "import-snapshot",
        "--source",
        source.as_str(),
        "--prefix",
        "backups/2026-11-15",
        "--destination",
        imported.to_str().unwrap(),
    ]));
    let message = stderr(&output);
    assert!(!message.contains("panicked"), "{message}");
    assert!(output.status.success(), "import-snapshot: {message}");

    let output = run(corrobore().args([
        "server",
        "validate-snapshot",
        "--snapshot",
        imported.to_str().unwrap(),
    ]));
    assert!(
        output.status.success(),
        "validate-snapshot: {}",
        stderr(&output)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn export_snapshot_s3_reports_an_unreachable_endpoint_without_panicking() {
    let root = temp_dir("export");
    let storage = storage(&root).await;
    let snapshot = root.join("snapshot");
    let output = run(corrobore().args([
        "server",
        "snapshot",
        "--storage-dir",
        storage.to_str().unwrap(),
        "--destination",
        snapshot.to_str().unwrap(),
    ]));
    assert!(output.status.success(), "snapshot: {}", stderr(&output));

    let output = run(corrobore()
        .env("CORROBORE_S3_ACCESS_KEY", "test-access")
        .env("CORROBORE_S3_SECRET_KEY", "test-secret")
        .args([
            "server",
            "export-snapshot-s3",
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--endpoint",
            "http://127.0.0.1:1",
            "--bucket",
            "corrobore-backups",
            "--prefix",
            "unreachable",
        ]));
    let message = stderr(&output);
    assert!(!message.contains("panicked"), "{message}");
    assert!(!output.status.success());
    assert!(message.contains("database operation failed"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}
