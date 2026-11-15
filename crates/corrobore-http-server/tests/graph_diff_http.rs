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

//! Issue #320: diffs, merge bases and the change feed over HTTP.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, get, persistent_app, post, unique_path};
use serde_json::json;

#[tokio::test]
async fn diff_between_main_and_a_branch_names_each_change() {
    let storage = unique_path("diff");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'kept', score: 1})").await;
    cypher_write(&app, "CREATE (n:Indicator {name: 'edited', score: 1})").await;
    post(&app, "/v1/graph/branches", json!({"name": "review/1"})).await;
    for query in [
        "MATCH (n:Indicator {name: 'edited'}) SET n.score = 5",
        "CREATE (n:Indicator {name: 'proposed'})",
    ] {
        let (status, body) = post(
            &app,
            "/v1/cypher/write",
            json!({"query": query, "branch": "review/1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (status, body) = get(
        &app,
        "/v1/graph/diff?from=main&to=review/1&include_values=true",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_eq!(
        result["totals"],
        json!({"added": 1, "changed": 1, "removed": 0})
    );
    let entries = result["entries"].as_array().unwrap();
    let changed = entries
        .iter()
        .find(|entry| entry["change"] == "changed")
        .unwrap();
    assert_eq!(changed["kind"], "node");
    assert_eq!(
        changed["property_changes"],
        json!([{"key": "score", "before": 1, "after": 5}])
    );
    assert_eq!(changed["before"]["properties"]["score"], 1);
    assert_eq!(changed["after"]["properties"]["score"], 5);
    let added = entries
        .iter()
        .find(|entry| entry["change"] == "added")
        .unwrap();
    assert!(added.get("before").is_none());
    assert_eq!(added["after"]["properties"]["name"], "proposed");

    let (_, base) = get(&app, "/v1/graph/merge-base?a=main&b=review/1").await;
    let (_, main_commits) = get(&app, "/v1/graph/commits?limit=1").await;
    assert_eq!(base["result"]["merge_base"], main_commits["result"]["head"]);

    let (status, body) = get(&app, "/v1/graph/diff?from=main&to=review/1&cursor=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _) = get(&app, "/v1/graph/diff?from=main&to=nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(storage);
}

#[tokio::test]
async fn change_feed_resumes_from_its_cursor_and_reports_gaps() {
    let storage = unique_path("feed");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'one'})").await;

    let (status, first) = get(&app, "/v1/graph/changes?limit=10").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let commits = first["result"]["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2, "genesis then one write");
    assert_eq!(commits[1]["changes"][0]["labels"][0], "Indicator");
    let cursor = first["result"]["next_cursor"].as_str().unwrap().to_owned();

    cypher_write(&app, "CREATE (n:Indicator {name: 'two'})").await;
    let (_, next) = get(&app, &format!("/v1/graph/changes?after={cursor}")).await;
    assert_eq!(next["result"]["commits"].as_array().unwrap().len(), 1);
    assert_eq!(next["result"]["has_more"], false);

    let (status, gap) = get(
        &app,
        "/v1/graph/changes?after=gc0000000000000000000000000000000000000000",
    )
    .await;
    assert_eq!(status, StatusCode::GONE, "{gap}");
    assert_eq!(gap["error"]["code"], "CHANGE_FEED_GAP");
    let (status, _) = get(&app, "/v1/graph/changes?after=not-a-commit").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let _ = std::fs::remove_dir_all(storage);
}
