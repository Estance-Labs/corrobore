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

//! Issue #324: history cleanup through the admin route.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{
    ADMIN_TOKEN, cypher_write, get, persistent_app, post, send_as, unique_path,
};
use serde_json::json;

#[tokio::test]
async fn admin_cleanup_prunes_history_outside_the_window() {
    let storage = unique_path("cleanup");
    let app = persistent_app(&storage);
    for index in 0..4 {
        cypher_write(&app, &format!("CREATE (n:Indicator {{name: 'n{index}'}})")).await;
    }

    let (status, refused) = post(
        &app,
        "/v1/admin/storage/cleanup",
        json!({"keep_commits": 1}),
    )
    .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "{status} {refused}"
    );

    let (status, preview) = send_as(
        &app,
        ADMIN_TOKEN,
        Method::POST,
        "/v1/admin/storage/cleanup",
        Some(json!({"keep_commits": 1, "dry_run": true})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["result"]["pruned_commits"], 3);
    assert_eq!(preview["result"]["dry_run"], true);

    let (status, report) = send_as(
        &app,
        ADMIN_TOKEN,
        Method::POST,
        "/v1/admin/storage/cleanup",
        Some(json!({"keep_commits": 1})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["result"]["pruned_commits"], 3);

    let (_, commits) = get(&app, "/v1/graph/commits").await;
    assert_eq!(commits["result"]["commits"].as_array().unwrap().len(), 2);
    let (_, records) = get(&app, "/v1/graph/records?label=Indicator").await;
    assert_eq!(
        records["result"]["total"], 4,
        "the head still sees every record"
    );

    let _ = std::fs::remove_dir_all(storage);
}
