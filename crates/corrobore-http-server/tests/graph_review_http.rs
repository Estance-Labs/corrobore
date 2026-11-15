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

//! Issue #348: revert one change of a branch and record review decisions over
//! HTTP, in the `{ ok, result }` envelope of the other graph routes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, ephemeral_app, get, persistent_app, post, unique_path};
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

async fn head(app: &axum::Router, branch: &str) -> String {
    let (status, body) = get(app, &format!("/v1/graph/commits?branch={branch}&limit=1")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["commits"][0]["commit_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Records the branch changed since it split from main.
async fn changes(app: &axum::Router, branch: &str) -> Vec<Value> {
    let (status, body) = get(app, &format!("/v1/graph/merge-base?a=main&b={branch}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let base = body["result"]["merge_base"].as_str().unwrap().to_owned();
    let (status, body) = get(app, &format!("/v1/graph/diff?from={base}&to={branch}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["entries"].as_array().unwrap().clone()
}

async fn fixture(name: &str) -> axum::Router {
    let app = persistent_app(&unique_path(name));
    cypher_write(&app, "CREATE (n:Indicator {name: 'base'})").await;
    let (status, body) = post(&app, "/v1/graph/branches", json!({"name": "review"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    branch_write(&app, "review", "CREATE (n:Indicator {name: 'proposed'})").await;
    branch_write(
        &app,
        "review",
        "MATCH (n:Indicator {name: 'base'}) SET n.name = 'edited'",
    )
    .await;
    app
}

#[tokio::test]
async fn reverting_one_change_leaves_the_others_on_the_branch() {
    let app = fixture("revert-http").await;
    let entries = changes(&app, "review").await;
    assert_eq!(entries.len(), 2, "{entries:?}");
    let edited = entries
        .iter()
        .find(|entry| entry["change"] == "changed")
        .expect("the renamed node");
    let record_id = edited["record_id"].as_str().unwrap().to_owned();
    let before = head(&app, "review").await;

    let (status, body) = post(
        &app,
        "/v1/graph/revert",
        json!({
            "branch": "review",
            "record_kind": "node",
            "record_id": record_id,
            "expected_head": before,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    let result = &body["result"];
    assert_eq!(result["branch"], "review");
    assert_eq!(result["record_kind"], "node");
    assert_eq!(result["record_id"], record_id.as_str());
    assert_eq!(result["restored"], "base_version");
    assert_eq!(result["commit_id"], head(&app, "review").await);

    let remaining = changes(&app, "review").await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["change"], "added");

    // The same request again: the branch moved and nothing differs any more.
    let (status, body) = post(
        &app,
        "/v1/graph/revert",
        json!({"branch": "review", "record_kind": "node", "record_id": record_id, "expected_head": before}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_PRECONDITION_FAILED");
    let (status, body) = post(
        &app,
        "/v1/graph/revert",
        json!({"branch": "review", "record_kind": "node", "record_id": record_id}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_REVERT_NO_CHANGE");
}

#[tokio::test]
async fn invalid_reverts_are_rejected() {
    let app = fixture("revert-invalid").await;
    for (body, status, code) in [
        (
            json!({"branch": "main", "record_kind": "node", "record_id": "x"}),
            StatusCode::BAD_REQUEST,
            "INVALID_REVERT",
        ),
        (
            json!({"branch": "review", "record_kind": "edge", "record_id": "x"}),
            StatusCode::BAD_REQUEST,
            "INVALID_REVERT",
        ),
        (
            json!({"branch": "review", "record_kind": "node", "record_id": " "}),
            StatusCode::BAD_REQUEST,
            "INVALID_REVERT",
        ),
        (
            json!({"branch": "missing", "record_kind": "node", "record_id": "x"}),
            StatusCode::NOT_FOUND,
            "GRAPH_BRANCH_NOT_FOUND",
        ),
        (
            json!({"branch": "review", "record_kind": "node", "record_id": "x", "expected_head": "not-a-commit"}),
            StatusCode::NOT_FOUND,
            "GRAPH_COMMIT_NOT_FOUND",
        ),
    ] {
        let (actual, response) = post(&app, "/v1/graph/revert", body.clone()).await;
        assert_eq!(actual, status, "{body} → {response}");
        assert_eq!(response["error"]["code"], code, "{body} → {response}");
    }
    let (status, body) = post(
        &ephemeral_app(),
        "/v1/graph/revert",
        json!({"branch": "review", "record_kind": "node", "record_id": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "PERSISTENT_STORAGE_REQUIRED");
}

#[tokio::test]
async fn reviews_name_the_head_they_looked_at() {
    let app = fixture("reviews-http").await;
    let first = head(&app, "review").await;
    let (status, body) = post(
        &app,
        "/v1/graph/reviews",
        json!({"branch": "review", "head": first, "decision": "changes_requested", "comment": "rename it back"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let review = &body["result"];
    assert_eq!(review["branch"], "review");
    assert_eq!(review["head"], first.as_str());
    assert_eq!(review["decision"], "changes_requested");
    assert_eq!(review["comment"], "rename it back");
    assert!(!review["reviewer"].as_str().unwrap().is_empty());
    assert!(review["at_ms"].as_u64().unwrap() > 0);

    branch_write(&app, "review", "CREATE (n:Indicator {name: 'third'})").await;
    let second = head(&app, "review").await;
    let (status, body) = post(
        &app,
        "/v1/graph/reviews",
        json!({"branch": "review", "head": first, "decision": "approved"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_PRECONDITION_FAILED");
    let (status, body) = post(
        &app,
        "/v1/graph/reviews",
        json!({"branch": "review", "head": second, "decision": "approved", "reviewer": "alice"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["result"]["reviewer"], "alice");
    assert!(body["result"]["comment"].is_null());

    let (status, body) = get(&app, "/v1/graph/reviews?branch=review").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reviews = body["result"]["reviews"].as_array().unwrap();
    assert_eq!(reviews.len(), 2);
    assert_eq!(reviews[0]["reviewer"], "alice");
    assert_eq!(reviews[0]["current"], true);
    assert_eq!(reviews[1]["decision"], "changes_requested");
    assert_eq!(reviews[1]["current"], false);

    for invalid in [
        json!({"branch": "review", "head": second, "decision": "maybe"}),
        json!({"branch": "main", "head": second, "decision": "approved"}),
        json!({"branch": "review", "head": second, "decision": "approved", "comment": "x".repeat(4001)}),
    ] {
        let (status, body) = post(&app, "/v1/graph/reviews", invalid.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid} → {body}");
        assert_eq!(body["error"]["code"], "INVALID_REVIEW", "{body}");
    }
    let (status, body) = get(&app, "/v1/graph/reviews").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
