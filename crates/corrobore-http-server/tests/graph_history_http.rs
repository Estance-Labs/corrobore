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

//! Issue #317: the commit history HTTP contract.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, ephemeral_app, get, persistent_app, unique_path};

#[tokio::test]
async fn cypher_writes_are_listed_as_main_commits_newest_first() {
    let storage = unique_path("commits");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'first.example'})").await;
    cypher_write(&app, "CREATE (n:Indicator {name: 'second.example'})").await;

    let (status, body) = get(&app, "/v1/graph/commits?branch=main&limit=10").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_eq!(result["branch"], "main");
    let commits = result["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 3, "two writes and the genesis: {body}");
    assert_eq!(result["head"], commits[0]["commit_id"]);
    assert_eq!(commits[0]["node_changes"], 1);
    assert_eq!(commits[0]["parents"][0], commits[1]["commit_id"]);
    assert_eq!(commits[2]["generation"], 0);
    assert!(result["next_cursor"].is_null());

    let (status, page) = get(&app, "/v1/graph/commits?limit=1").await;
    assert_eq!(status, StatusCode::OK);
    let cursor = page["result"]["next_cursor"].as_str().unwrap().to_owned();
    let (status, next) = get(&app, &format!("/v1/graph/commits?limit=1&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        next["result"]["commits"][0]["commit_id"],
        commits[1]["commit_id"]
    );

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn commit_detail_lists_its_record_changes() {
    let storage = unique_path("commit-detail");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (a:Indicator {name: 'a.example'})").await;
    cypher_write(
        &app,
        "MATCH (a:Indicator {name: 'a.example'}) CREATE (a)-[:INDICATES]->(m:Malware {name: 'm'})",
    )
    .await;
    let (_, list) = get(&app, "/v1/graph/commits?limit=1").await;
    let head = list["result"]["commits"][0]["commit_id"].as_str().unwrap();

    let (status, body) = get(&app, &format!("/v1/graph/commits/{head}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let commit = &body["result"];
    assert_eq!(commit["commit_id"], head);
    let changes = commit["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{body} {list}");
    let relationship = changes
        .iter()
        .find(|change| change["kind"] == "relationship")
        .unwrap();
    assert_eq!(relationship["labels"][0], "INDICATES");
    assert!(relationship["source"].is_string());
    assert!(changes.iter().all(|change| change.get("payload").is_none()));

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn unknown_commit_and_branch_answer_not_found() {
    let storage = unique_path("commit-missing");
    let app = persistent_app(&storage);

    let (status, body) = get(
        &app,
        "/v1/graph/commits/gc0000000000000000000000000000000000000000",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_NOT_FOUND");

    let (status, body) = get(&app, "/v1/graph/commits/not-a-commit").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_NOT_FOUND");

    let (status, body) = get(&app, "/v1/graph/commits?branch=nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "GRAPH_BRANCH_NOT_FOUND");

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn ephemeral_mode_explains_that_history_needs_persistent_storage() {
    let app = ephemeral_app();
    let (status, body) = get(&app, "/v1/graph/commits").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "PERSISTENT_STORAGE_REQUIRED");
}
