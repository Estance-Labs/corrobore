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

//! Issue #352: `POST /v1/graph/edits` publishes a batch of edits as one commit.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, ephemeral_app, get, persistent_app, post, unique_path};
use serde_json::{Value, json};

async fn head(app: &axum::Router, branch: &str) -> String {
    let (status, body) = get(app, &format!("/v1/graph/commits?branch={branch}&limit=1")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["commits"][0]["commit_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn names(app: &axum::Router, branch: Option<&str>) -> Vec<String> {
    let mut request = json!({"query": "MATCH (n:Indicator) RETURN n.name"});
    if let Some(branch) = branch {
        request["branch"] = json!(branch);
    }
    let (status, body) = post(app, "/v1/cypher/read", request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
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

async fn node_id(app: &axum::Router, name: &str) -> String {
    let (status, body) = get(app, "/v1/graph/records?kind=node&label=Indicator&limit=50").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["properties"]["name"] == name)
        .unwrap_or_else(|| panic!("no Indicator named {name}: {body}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn fixture(name: &str) -> axum::Router {
    let app = persistent_app(&unique_path(name));
    cypher_write(&app, "CREATE (n:Indicator {name: 'alpha', alias: 'a'})").await;
    cypher_write(&app, "CREATE (n:Indicator {name: 'beta'})").await;
    app
}

#[tokio::test]
async fn a_batch_on_main_is_one_commit_visible_to_cypher() {
    let app = fixture("edits-main").await;
    let alpha = node_id(&app, "alpha").await;
    let beta = node_id(&app, "beta").await;
    let before = head(&app, "main").await;
    let (status, body) = post(
        &app,
        "/v1/graph/edits",
        json!({
            "expected_head": before,
            "message": "tidy",
            "edits": [
                {"op": "set_properties", "node_id": alpha, "set": {"name": "alpha-2", "score": 0.5, "tags": ["x", "y"]}, "remove": ["alias"]},
                {"op": "create_node", "labels": ["Indicator"], "properties": {"name": "gamma", "count": 3}},
                {"op": "delete_node", "node_id": beta}
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_eq!(result["branch"], "main");
    assert_eq!(result["changed"], 3);
    assert_eq!(result["created"].as_array().unwrap().len(), 1);
    let after = head(&app, "main").await;
    assert_eq!(result["commit_id"], after.as_str());
    let (_, commit) = get(&app, &format!("/v1/graph/commits/{after}")).await;
    assert_eq!(commit["result"]["parents"][0], before.as_str());
    assert_eq!(names(&app, None).await, ["alpha-2", "gamma"]);

    // Stale head: refused, nothing written.
    let (status, body) = post(
        &app,
        "/v1/graph/edits",
        json!({"expected_head": before, "edits": [{"op": "delete_node", "node_id": alpha}]}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_PRECONDITION_FAILED");
    assert_eq!(head(&app, "main").await, after);
}

#[tokio::test]
async fn a_batch_on_a_branch_stays_on_the_branch() {
    let app = fixture("edits-branch").await;
    let alpha = node_id(&app, "alpha").await;
    let (status, body) = post(&app, "/v1/graph/branches", json!({"name": "drafts"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = post(
        &app,
        "/v1/graph/edits",
        json!({"branch": "drafts", "edits": [{"op": "set_properties", "node_id": alpha, "set": {"name": "drafted"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["branch"], "drafts");
    assert_eq!(names(&app, Some("drafts")).await, ["beta", "drafted"]);
    assert_eq!(names(&app, None).await, ["alpha", "beta"]);
}

#[tokio::test]
async fn invalid_batches_are_rejected_without_writing() {
    let app = fixture("edits-invalid").await;
    let alpha = node_id(&app, "alpha").await;
    let before = head(&app, "main").await;
    cypher_write(
        &app,
        "MATCH (a:Indicator {name: 'alpha'}) CREATE (a)-[:INDICATES]->(m:Malware {name: 'm'})",
    )
    .await;
    let linked = head(&app, "main").await;
    assert_ne!(before, linked);
    for (edits, status, code) in [
        (json!([]), StatusCode::BAD_REQUEST, "INVALID_EDIT"),
        (
            json!([{"op": "rename_node", "node_id": alpha}]),
            StatusCode::BAD_REQUEST,
            "INVALID_EDIT",
        ),
        (
            json!([{"op": "set_properties", "node_id": alpha, "set": {"nested": {"a": 1}}}]),
            StatusCode::BAD_REQUEST,
            "INVALID_EDIT",
        ),
        (
            json!([{"op": "set_properties", "node_id": alpha, "set": {"mixed": [1, "a"]}}]),
            StatusCode::BAD_REQUEST,
            "INVALID_EDIT",
        ),
        (
            json!([{"op": "create_node", "labels": [], "properties": {}}]),
            StatusCode::BAD_REQUEST,
            "INVALID_EDIT",
        ),
        (
            json!([{"op": "set_properties", "node_id": "node--missing", "set": {"name": "x"}}]),
            StatusCode::NOT_FOUND,
            "GRAPH_NODE_NOT_FOUND",
        ),
        (
            json!([{"op": "delete_node", "node_id": alpha}]),
            StatusCode::CONFLICT,
            "GRAPH_EDIT_BLOCKED",
        ),
    ] {
        let (actual, body) = post(&app, "/v1/graph/edits", json!({ "edits": edits })).await;
        assert_eq!(actual, status, "{edits} → {body}");
        assert_eq!(body["error"]["code"], code, "{edits} → {body}");
    }
    assert_eq!(head(&app, "main").await, linked);

    let (status, body) = post(
        &ephemeral_app(),
        "/v1/graph/edits",
        json!({"edits": [{"op": "delete_node", "node_id": alpha}]}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "PERSISTENT_STORAGE_REQUIRED");
}
