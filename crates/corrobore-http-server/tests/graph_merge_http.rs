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

//! Issue #321: branch merges over HTTP.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, get, persistent_app, post, unique_path};
use serde_json::{Value, json};

async fn branch_write(app: &axum::Router, branch: &str, query: &str) {
    let (status, body) = post(
        app,
        "/v1/cypher/write",
        json!({"query": query, "branch": branch}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn main_names(app: &axum::Router) -> Vec<String> {
    let (_, body) = post(
        app,
        "/v1/cypher/read",
        json!({"query": "MATCH (n:Indicator) RETURN n.name"}),
    )
    .await;
    let mut names: Vec<String> = body["result"]["data"]["Records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row: &Value| {
            row["fields"]["n.name"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn previewed_merge_then_merge_publishes_on_main() {
    let storage = unique_path("merge");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'base'})").await;
    post(&app, "/v1/graph/branches", json!({"name": "review/7"})).await;
    branch_write(&app, "review/7", "CREATE (n:Indicator {name: 'proposed'})").await;

    let (status, preview) = post(
        &app,
        "/v1/graph/merge",
        json!({"source": "review/7", "dry_run": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["result"]["outcome"], "fast_forward");
    assert_eq!(preview["result"]["dry_run"], true);
    assert!(preview["result"]["commit_id"].is_null());
    assert_eq!(preview["result"]["changes"]["nodes"], 1);
    assert_eq!(main_names(&app).await, vec!["base".to_owned()]);

    let (status, merged) = post(
        &app,
        "/v1/graph/merge",
        json!({
            "source": "review/7",
            "expected_target_head": preview["result"]["target_head"],
            "delete_source": true,
            "actor": "reviewer"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{merged}");
    let commit = merged["result"]["commit_id"].as_str().unwrap();
    let (_, detail) = get(&app, &format!("/v1/graph/commits/{commit}")).await;
    assert_eq!(detail["result"]["parents"].as_array().unwrap().len(), 2);
    assert_eq!(detail["result"]["actor"], "reviewer");
    assert_eq!(
        main_names(&app).await,
        vec!["base".to_owned(), "proposed".to_owned()]
    );
    let (_, branches) = get(&app, "/v1/graph/branches").await;
    assert_eq!(branches["result"]["branches"].as_array().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn conflicting_merge_answers_409_with_every_conflict() {
    let storage = unique_path("merge-conflict");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'shared', score: 1})").await;
    post(&app, "/v1/graph/branches", json!({"name": "draft"})).await;
    branch_write(
        &app,
        "draft",
        "MATCH (n:Indicator {name: 'shared'}) SET n.score = 2",
    )
    .await;
    cypher_write(&app, "MATCH (n:Indicator {name: 'shared'}) SET n.score = 3").await;

    let (status, body) = post(&app, "/v1/graph/merge", json!({"source": "draft"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"]["code"], "MERGE_CONFLICT");
    assert_eq!(body["result"]["outcome"], "conflicted");
    let conflict = &body["result"]["conflicts"][0];
    assert_eq!(conflict["kind"], "concurrent_update");
    assert_eq!(conflict["record_kind"], "node");

    let (status, moved) = post(
        &app,
        "/v1/graph/merge",
        json!({"source": "draft", "expected_target_head": "gc0000000000000000000000000000000000000000"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{moved}");
    assert_eq!(moved["error"]["code"], "GRAPH_COMMIT_PRECONDITION_FAILED");

    let (status, _) = post(&app, "/v1/graph/merge", json!({"source": "missing"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(storage);
}
