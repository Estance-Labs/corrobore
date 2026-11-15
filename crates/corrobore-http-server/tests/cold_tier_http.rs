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

//! Issue #392: cleanup archives pruned history to the Lance cold tier, and
//! pruned commits stay readable.
#![cfg(feature = "lance-cold-tier")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{ADMIN_TOKEN, TOKEN, persistent_app_with, send_as, unique_path};
use serde_json::{Value, json};

async fn commits(app: &axum::Router) -> Vec<String> {
    let (status, body) = send_as(
        app,
        TOKEN,
        Method::GET,
        "/v1/graph/commits?branch=main&limit=100",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["commit_id"].as_str().unwrap().to_owned())
        .collect()
}

async fn names_at(app: &axum::Router, commit: &str) -> (StatusCode, Value) {
    send_as(
        app,
        TOKEN,
        Method::POST,
        "/v1/cypher/read",
        Some(json!({"query": "MATCH (n:Indicator) RETURN n.name", "commit": commit})),
        &[],
    )
    .await
}

fn names(body: &Value) -> Vec<String> {
    let mut names: Vec<String> = body["result"]["data"]["Records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["fields"]["n.name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn pruned_commits_are_archived_and_stay_readable() {
    let archive = unique_path("cold-archive");
    let archive_text = archive.display().to_string();
    let app = persistent_app_with(
        &unique_path("cold-store"),
        &[("CORROBORE_STORAGE_COLD_TIER_URL", archive_text.as_str())],
    );
    for index in 0..6 {
        let (status, body) = send_as(
            &app,
            TOKEN,
            Method::POST,
            "/v1/cypher/write",
            Some(json!({"query": format!("CREATE (n:Indicator {{name: 'n{index}'}})")})),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let before = commits(&app).await;
    // Newest first: before[3] holds n0..n2.
    let (status, body) = names_at(&app, &before[3]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let expected = names(&body);
    assert_eq!(expected, ["n0", "n1", "n2"]);

    let (status, body) = send_as(
        &app,
        ADMIN_TOKEN,
        Method::POST,
        "/v1/admin/storage/cleanup",
        Some(json!({"keep_commits": 2})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    let pruned = result["pruned_commits"].as_u64().unwrap();
    assert!(pruned >= 4, "{body}");
    assert_eq!(
        result["cold_tier"]["archived"].as_u64().unwrap(),
        pruned,
        "{body}"
    );
    assert_eq!(result["cold_tier"]["uri"], archive_text.as_str());

    // The hot store no longer has it; the archive does, exactly.
    let (status, body) = send_as(
        &app,
        TOKEN,
        Method::GET,
        &format!("/v1/graph/commits/{}", before[3]),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["archived"], true);
    assert_eq!(body["result"]["commit_id"], before[3].as_str());
    let (status, body) = names_at(&app, &before[3]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(names(&body), expected);
    assert_eq!(body["graph_commit_id"], before[3].as_str());

    let (status, body) = send_as(
        &app,
        TOKEN,
        Method::GET,
        "/v1/graph/cold/commits",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["configured"], true);
    assert_eq!(
        body["result"]["commits"].as_array().unwrap().len() as u64,
        pruned
    );

    // A commit that never existed is still unknown.
    let (status, _) = names_at(&app, "gc0000000000000000000000000000000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Nothing new to prune: nothing new archived.
    let (status, body) = send_as(
        &app,
        ADMIN_TOKEN,
        Method::POST,
        "/v1/admin/storage/cleanup",
        Some(json!({"keep_commits": 2})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["cold_tier"]["archived"], 0);
}
