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

//! Issue #318: reads pinned to a commit over HTTP.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, get, persistent_app, post, unique_path};
use serde_json::{Value, json};

async fn head(app: &axum::Router) -> String {
    let (_, body) = get(app, "/v1/graph/commits?limit=1").await;
    body["result"]["head"].as_str().unwrap().to_owned()
}

async fn read_names(app: &axum::Router, commit: Option<&str>) -> (Vec<String>, Value) {
    let mut request = json!({"query": "MATCH (n:Indicator) RETURN n.name"});
    if let Some(commit) = commit {
        request["commit"] = Value::String(commit.to_owned());
    }
    let (status, body) = post(app, "/v1/cypher/read", request).await;
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
    (names, body)
}

#[tokio::test]
async fn cypher_read_at_a_commit_sees_the_versions_of_that_commit() {
    let storage = unique_path("pinned-cypher");
    let app = persistent_app(&storage);
    let written = cypher_write(&app, "CREATE (n:Indicator {name: 'old.example'})").await;
    let seeded = head(&app).await;
    assert_eq!(
        written["graph_commit_id"], seeded,
        "writes report the new head"
    );
    cypher_write(
        &app,
        "MATCH (n:Indicator {name: 'old.example'}) SET n.name = 'new.example'",
    )
    .await;
    cypher_write(&app, "CREATE (n:Indicator {name: 'late.example'})").await;

    let (pinned, body) = read_names(&app, Some(&seeded)).await;
    assert_eq!(pinned, vec!["old.example".to_owned()], "{body}");
    assert_eq!(body["graph_commit_id"], seeded);

    let (current, body) = read_names(&app, None).await;
    assert_eq!(
        current,
        vec!["late.example".to_owned(), "new.example".to_owned()]
    );
    assert_eq!(body["graph_commit_id"], head(&app).await);

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn writes_pinned_to_a_commit_are_refused() {
    let storage = unique_path("pinned-write");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'a'})").await;
    let commit = head(&app).await;

    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({"query": "CREATE (n:Indicator {name: 'b'})", "commit": commit}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "GRAPH_COMMIT_READ_ONLY");

    let (status, body) = post(
        &app,
        "/v1/cypher/read",
        json!({"query": "MATCH (n) RETURN n", "commit": "gc-nope"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn record_pages_are_typed_and_pinned_to_their_commit() {
    let storage = unique_path("records");
    let app = persistent_app(&storage);
    for name in ["a", "b", "c"] {
        cypher_write(
            &app,
            &format!("CREATE (n:Indicator {{name: '{name}', score: 7, tags: ['x', 'y']}})"),
        )
        .await;
    }
    cypher_write(&app, "CREATE (n:Malware {name: 'm'})").await;
    let pinned = head(&app).await;

    let (status, first) = get(
        &app,
        &format!("/v1/graph/records?label=Indicator&limit=2&commit={pinned}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let result = &first["result"];
    assert_eq!(result["commit_id"], pinned);
    assert_eq!(result["kind"], "node");
    assert_eq!(result["total"], 3);
    let nodes = result["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["labels"][0], "Indicator");
    assert_eq!(nodes[0]["properties"]["score"], 7);
    assert_eq!(nodes[0]["properties"]["tags"], json!(["x", "y"]));

    cypher_write(&app, "CREATE (n:Indicator {name: 'later'})").await;
    let cursor = result["next_cursor"].as_str().unwrap();
    let (_, second) = get(
        &app,
        &format!("/v1/graph/records?label=Indicator&limit=2&commit={pinned}&cursor={cursor}"),
    )
    .await;
    assert_eq!(second["result"]["total"], 3);
    assert_eq!(second["result"]["nodes"].as_array().unwrap().len(), 1);
    assert!(second["result"]["next_cursor"].is_null());

    let (_, latest) = get(&app, "/v1/graph/records?label=Indicator").await;
    assert_eq!(latest["result"]["total"], 4);

    let (status, body) = get(&app, "/v1/graph/records?kind=edge").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn node_detail_groups_relationships_with_neighbours() {
    let storage = unique_path("node-detail");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'i'})").await;
    cypher_write(
        &app,
        "MATCH (i:Indicator {name: 'i'}) CREATE (i)-[:INDICATES]->(m:Malware {name: 'm'})",
    )
    .await;
    let (_, records) = get(&app, "/v1/graph/records?label=Malware").await;
    let malware = records["result"]["nodes"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, body) = get(&app, &format!("/v1/graph/nodes/{malware}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let group = &body["result"]["groups"][0];
    assert_eq!(group["type"], "INDICATES");
    assert_eq!(group["direction"], "incoming");
    assert_eq!(group["count"], 1);
    assert_eq!(group["items"][0]["node"]["properties"]["name"], "i");

    let (status, body) = get(&app, "/v1/graph/nodes/node--999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "GRAPH_NODE_NOT_FOUND");

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn explorer_projects_the_commit_it_is_asked_for() {
    let storage = unique_path("explorer");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'first'})").await;
    let first = head(&app).await;
    cypher_write(&app, "CREATE (n:Indicator {name: 'second'})").await;
    let (status, session) = post(
        &app,
        "/v1/sessions/start",
        json!({"workspace_id": "workspace--graph", "actor_id": "actor--viewer"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let session_id = session["result"]["session_id"].as_str().unwrap();

    let (status, current) = get(&app, &format!("/v1/explorer/sessions/{session_id}/graph")).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(current["result"]["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(current["graph_commit_id"], head(&app).await);

    let (status, pinned) = get(
        &app,
        &format!("/v1/explorer/sessions/{session_id}/graph?commit={first}&max_nodes=1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pinned}");
    assert_eq!(pinned["graph_commit_id"], first);
    assert_eq!(pinned["result"]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(pinned["result"]["metadata"]["omitted_nodes"], 0);

    let _ = std::fs::remove_dir_all(storage);
}
