// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! Shared harness for the graph history HTTP contract tests.
#![allow(dead_code, clippy::unwrap_used)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use serde_json::Value;
use tower::ServiceExt;

pub const TOKEN: &str = "graph-token";

pub fn unique_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-graph-api-{name}-{}-{unique}",
        std::process::id()
    ))
}

fn config(storage: Option<&Path>) -> ServerConfig {
    let mut values = HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), TOKEN.to_owned()),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            unique_path("sessions").display().to_string(),
        ),
    ]);
    if let Some(storage) = storage {
        values.insert("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned());
        values.insert(
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage.display().to_string(),
        );
    }
    ServerConfig::from_map(&values).unwrap()
}

pub fn persistent_app(storage: &Path) -> axum::Router {
    build_router(AppState::new(config(Some(storage))).unwrap())
}

pub fn ephemeral_app() -> axum::Router {
    build_router(AppState::new(config(None)).unwrap())
}

pub async fn send(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

pub async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    send(app, Method::GET, uri, None, &[]).await
}

pub async fn post(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    send(app, Method::POST, uri, Some(body), &[]).await
}

pub async fn cypher_write(app: &axum::Router, query: &str) -> Value {
    let (status, body) = post(
        app,
        "/v1/cypher/write",
        serde_json::json!({ "query": query }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
