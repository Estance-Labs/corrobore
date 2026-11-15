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

//! Issue #323: the schema route.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, get, persistent_app, unique_path};

#[tokio::test]
async fn schema_lists_labels_types_and_property_value_types() {
    let storage = unique_path("schema");
    let app = persistent_app(&storage);
    cypher_write(&app, "CREATE (n:Indicator {name: 'a', score: 3})").await;
    cypher_write(
        &app,
        "MATCH (i:Indicator {name: 'a'}) CREATE (i)-[:INDICATES]->(m:Malware {name: 'm'})",
    )
    .await;

    let (status, body) = get(&app, "/v1/graph/schema").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    let labels = result["labels"].as_array().unwrap();
    let indicator = labels
        .iter()
        .find(|label| label["name"] == "Indicator")
        .unwrap();
    assert_eq!(indicator["count"], 1);
    let score = indicator["properties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|property| property["key"] == "score")
        .unwrap();
    assert_eq!(score["value_types"][0], "integer");
    let indicates = &result["relationship_types"][0];
    assert_eq!(indicates["name"], "INDICATES");
    assert_eq!(indicates["endpoints"][0]["source_label"], "Indicator");
    assert_eq!(indicates["endpoints"][0]["target_label"], "Malware");
    assert!(indicator["id"].as_u64().unwrap() > 0);

    let (status, _) = get(&app, "/v1/graph/schema?branch=missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(storage);
}
