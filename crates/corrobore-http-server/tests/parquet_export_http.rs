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

//! Issue #327: Parquet export through the admin route.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{ADMIN_TOKEN, cypher_write, persistent_app, post, send_as, unique_path};
use serde_json::json;

#[tokio::test]
async fn admin_exports_the_main_head_to_parquet() {
    let storage = unique_path("parquet-storage");
    let destination = unique_path("parquet-export");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'a', score: 1})").await;
    cypher_write(&app, "CREATE (n:Indicator {name: 'b', score: 2})").await;

    let (status, _) = post(
        &app,
        "/v1/admin/storage/export/parquet",
        json!({"destination": destination}),
    )
    .await;
    assert!(status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN);

    let (status, body) = send_as(
        &app,
        ADMIN_TOKEN,
        Method::POST,
        "/v1/admin/storage/export/parquet",
        Some(json!({"destination": destination})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["total_rows"], 2);
    assert_eq!(
        body["result"]["files"][0]["path"],
        "nodes/Indicator.parquet"
    );
    assert!(destination.join("nodes/Indicator.parquet").is_file());
    assert!(destination.join("manifest.json").is_file());

    let _ = std::fs::remove_dir_all(storage);
    let _ = std::fs::remove_dir_all(destination);
}
