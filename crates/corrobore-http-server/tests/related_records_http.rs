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

//! Issue #361: `GET /v1/graph/records` filters nodes across relationships.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{cypher_write, get, persistent_app, unique_path};
use serde_json::Value;

async fn names(app: &axum::Router, query: &str) -> (Vec<String>, u64) {
    let (status, body) = get(app, &format!("/v1/graph/records?kind=node&{query}")).await;
    assert_eq!(status, StatusCode::OK, "{query}: {body}");
    let mut names: Vec<String> = body["result"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node: &Value| node["properties"]["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    (names, body["result"]["total"].as_u64().unwrap())
}

#[tokio::test]
async fn records_are_filtered_through_their_relationships() {
    let app = persistent_app(&unique_path("related-records"));
    for name in ["lonely", "i1", "i2"] {
        cypher_write(&app, &format!("CREATE (n:Indicator {{name: '{name}'}})")).await;
    }
    cypher_write(
        &app,
        "MATCH (x:Indicator {name: 'i1'}) CREATE (x)-[:INDICATES]->(m:Malware {name: 'm'})",
    )
    .await;
    cypher_write(
        &app,
        "MATCH (y:Indicator {name: 'i2'}) CREATE (y)-[:INDICATES]->(t:Tool {name: 't'})",
    )
    .await;
    let (all, _) = names(&app, "label=Indicator").await;
    assert_eq!(all, ["i1", "i2", "lonely"]);

    assert_eq!(
        names(
            &app,
            "label=Indicator&related_type=INDICATES&related_direction=out&related_label=Malware"
        )
        .await,
        (vec!["i1".to_owned()], 1)
    );
    assert_eq!(
        names(&app, "label=Indicator&related_type=INDICATES").await,
        (vec!["i1".to_owned(), "i2".to_owned()], 2)
    );
    assert_eq!(
        names(&app, "label=Malware&related_direction=in").await,
        (vec!["m".to_owned()], 1)
    );
    assert_eq!(
        names(&app, "label=Indicator&related_direction=in").await.1,
        0
    );

    for uri in [
        "/v1/graph/records?kind=node&related_direction=sideways",
        "/v1/graph/records?kind=relationship&related_type=INDICATES",
    ] {
        let (status, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert_eq!(
            body["error"]["code"], "INVALID_RECORD_FILTER",
            "{uri}: {body}"
        );
    }
}
