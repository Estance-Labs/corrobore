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

//! Issue #319: branches over HTTP, with Cypher writes isolated on a branch.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{cypher_write, get, persistent_app, post, send, unique_path};
use serde_json::{Value, json};

async fn names(app: &axum::Router, branch: &str) -> Vec<String> {
    let (status, body) = post(
        app,
        "/v1/cypher/read",
        json!({"query": "MATCH (n:Indicator) RETURN n.name", "branch": branch}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut names: Vec<String> = body["result"]["data"]["Records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row["fields"]["n.name"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    names.sort();
    names
}

fn branch<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["result"]["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["name"] == name)
        .unwrap()
}

#[tokio::test]
async fn cypher_writes_on_a_branch_stay_off_main_until_deleted() {
    let storage = unique_path("branch-writes");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'base'})").await;

    let (status, created) = post(
        &app,
        "/v1/graph/branches",
        json!({"name": "agent/ingest-42", "actor": "agent--ingest"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["result"]["created_by"], "agent--ingest");

    let (status, written) = post(
        &app,
        "/v1/cypher/write",
        json!({"query": "CREATE (n:Indicator {name: 'proposed'})", "branch": "agent/ingest-42"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{written}");
    let (_, list) = get(&app, "/v1/graph/branches").await;
    assert_eq!(list["result"]["branches"][0]["name"], "main");
    let agent = branch(&list, "agent/ingest-42");
    assert_eq!(written["graph_commit_id"], agent["head"]);
    assert_eq!(agent["merged"], false);
    assert_eq!(agent["commits_ahead"], 1);

    assert_eq!(names(&app, "main").await, vec!["base".to_owned()]);
    assert_eq!(
        names(&app, "agent/ingest-42").await,
        vec!["base".to_owned(), "proposed".to_owned()]
    );

    let (status, refused) = send(
        &app,
        Method::DELETE,
        "/v1/graph/branches/agent/ingest-42",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"]["code"], "GRAPH_BRANCH_CONFLICT");
    let (status, deleted) = send(
        &app,
        Method::DELETE,
        "/v1/graph/branches/agent/ingest-42?force=true",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert_eq!(deleted["result"]["deleted"], "agent/ingest-42");
    let (_, list) = get(&app, "/v1/graph/branches").await;
    assert_eq!(list["result"]["branches"].as_array().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn branch_creation_validates_names_and_origins() {
    let storage = unique_path("branch-validation");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'first'})").await;
    let (_, commits) = get(&app, "/v1/graph/commits?limit=1").await;
    let first = commits["result"]["head"].as_str().unwrap().to_owned();
    cypher_write(&app, "CREATE (n:Indicator {name: 'second'})").await;

    let (status, body) = post(&app, "/v1/graph/branches", json!({"name": "bad name"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "INVALID_BRANCH_NAME");

    let (status, body) = post(
        &app,
        "/v1/graph/branches",
        json!({"name": "replay", "from_commit": first}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["head"], first);
    assert_eq!(names(&app, "replay").await, vec!["first".to_owned()]);

    let (status, body) = post(&app, "/v1/graph/branches", json!({"name": "replay"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, body) = post(
        &app,
        "/v1/graph/branches",
        json!({"name": "ghost", "from_commit": "gc0000000000000000000000000000000000000000"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({"query": "CREATE (n:Indicator {name: 'x'})", "branch": "missing"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_BRANCH_NOT_FOUND");

    let _ = std::fs::remove_dir_all(storage);
}
