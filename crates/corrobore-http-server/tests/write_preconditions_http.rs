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

//! Issue #322: optimistic write preconditions over HTTP.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{cypher_write, get, persistent_app, post, send, unique_path};
use serde_json::json;

async fn head(app: &axum::Router, branch: &str) -> String {
    let (_, body) = get(app, &format!("/v1/graph/commits?branch={branch}&limit=1")).await;
    body["result"]["head"].as_str().unwrap().to_owned()
}

async fn count(app: &axum::Router) -> usize {
    let (_, body) = get(app, "/v1/graph/records?label=Indicator").await;
    body["result"]["total"].as_u64().unwrap() as usize
}

#[tokio::test]
async fn main_write_with_if_graph_commit_commits_only_on_the_read_head() {
    let storage = unique_path("precondition-main");
    let app = persistent_app(&storage);
    let read = head(&app, "main").await;

    let (status, body) = send(
        &app,
        Method::POST,
        "/v1/cypher/write",
        Some(json!({"query": "CREATE (n:Indicator {name: 'first'})"})),
        &[("If-Graph-Commit", read.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = head(&app, "main").await;
    assert_ne!(after, read);
    assert_eq!(body["graph_commit_id"], after);

    let (status, body) = send(
        &app,
        Method::POST,
        "/v1/cypher/write",
        Some(json!({"query": "CREATE (n:Indicator {name: 'stale'})"})),
        &[("If-Graph-Commit", read.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_PRECONDITION_FAILED");
    assert_eq!(count(&app).await, 1, "a refused write changes nothing");

    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({"query": "CREATE (n:Indicator {name: 'fresh'})", "expected_commit": after}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(count(&app).await, 2);

    let (status, body) = send(
        &app,
        Method::POST,
        "/v1/cypher/write",
        Some(json!({"query": "CREATE (n:Indicator)"})),
        &[("If-Graph-Commit", "yesterday")],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "INVALID_PRECONDITION");

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn branch_write_with_a_stale_precondition_is_refused() {
    let storage = unique_path("precondition-branch");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'base'})").await;
    post(&app, "/v1/graph/branches", json!({"name": "draft"})).await;
    let read = head(&app, "draft").await;
    let (status, _) = post(
        &app,
        "/v1/cypher/write",
        json!({"query": "CREATE (n:Indicator {name: 'one'})", "branch": "draft", "expected_commit": read}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({"query": "CREATE (n:Indicator {name: 'two'})", "branch": "draft", "expected_commit": read}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_PRECONDITION_FAILED");

    let _ = std::fs::remove_dir_all(storage);
}
