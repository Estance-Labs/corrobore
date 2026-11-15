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

//! Issue #326: vector and hybrid search over HTTP.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{persistent_app, persistent_app_with, post, unique_path};
use serde_json::json;

async fn create(app: &axum::Router, name: &str, embedding: [f64; 2]) {
    let (status, body) = post(
        app,
        "/v1/cypher/write",
        json!({
            "query": "CREATE (n:Doc {name: $name, embedding: $embedding})",
            "params": {"name": name, "embedding": embedding}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn vector_and_hybrid_routes_rank_nodes() {
    let storage = unique_path("search");
    let app = persistent_app(&storage);
    create(&app, "east", [1.0, 0.0]).await;
    create(&app, "north", [0.0, 1.0]).await;

    let (status, body) = post(
        &app,
        "/v1/graph/search/vector",
        json!({"vector": [0.9, 0.1], "k": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hits = body["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["labels"][0], "Doc");
    assert_eq!(hits[0]["ranks"]["vector"], 1);

    let (status, body) = post(
        &app,
        "/v1/graph/search/hybrid",
        json!({"vector": [0.0, 1.0], "k": 2, "graph_expansion": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["hits"].as_array().unwrap().len(), 2);

    let (status, body) = post(
        &app,
        "/v1/graph/search/vector",
        json!({"vector": [1.0, 0.0, 0.0]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "INVALID_SEARCH");
    let (status, _) = post(&app, "/v1/graph/search/hybrid", json!({"k": 2})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(
        &app,
        "/v1/graph/search/vector",
        json!({"vector": [1.0], "metric": "dot"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let _ = std::fs::remove_dir_all(storage);
}

/// Issue #342: the HNSW index answers the same routes when configured.
#[tokio::test]
async fn vector_and_hybrid_routes_use_the_configured_hnsw_index() {
    let storage = unique_path("search-hnsw");
    let app = persistent_app_with(
        &storage,
        &[
            ("CORROBORE_STORAGE_VECTOR_INDEX", "hnsw"),
            ("CORROBORE_STORAGE_VECTOR_HNSW_MIN_VECTORS", "1"),
        ],
    );
    create(&app, "east", [1.0, 0.0]).await;
    create(&app, "north", [0.0, 1.0]).await;
    create(&app, "north-east", [0.7, 0.7]).await;

    for metric in ["cosine", "l2"] {
        let (status, body) = post(
            &app,
            "/v1/graph/search/vector",
            json!({"vector": [0.0, 1.0], "k": 2, "metric": metric}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let hits = body["result"]["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2, "{metric}: {body}");
        assert_eq!(hits[0]["ranks"]["vector"], 1);
        assert!(hits[0]["score"].as_f64().unwrap() >= hits[1]["score"].as_f64().unwrap());
    }

    let (status, body) = post(
        &app,
        "/v1/graph/search/hybrid",
        json!({"vector": [1.0, 0.0], "k": 3, "graph_expansion": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["hits"].as_array().unwrap().len(), 3);

    let _ = std::fs::remove_dir_all(storage);
}
