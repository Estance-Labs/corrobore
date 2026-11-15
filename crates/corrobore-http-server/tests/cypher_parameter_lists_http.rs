// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Issue #378: typed parameter lists are long enough for embeddings, while
//! list literals in query text keep their tighter bound.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{persistent_app, post, unique_path};
use serde_json::json;

fn embedding(dimensions: usize, seed: f64) -> Vec<f64> {
    (0..dimensions)
        .map(|index| ((index as f64 + seed) * 0.37).sin())
        .collect()
}

#[tokio::test]
async fn a_model_sized_embedding_is_written_through_cypher_parameters() {
    let app = persistent_app(&unique_path("parameter-lists"));
    let vector = embedding(768, 1.0);
    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({
            "query": "CREATE (n:Doc {name: $name, embedding: $embedding})",
            "params": {"name": "nomic-sized", "embedding": vector}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({
            "query": "CREATE (n:Doc {name: $name, embedding: $embedding})",
            "params": {"name": "other", "embedding": embedding(768, 50.0)}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = post(
        &app,
        "/v1/graph/search/vector",
        json!({"vector": embedding(768, 1.0), "k": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hits = body["result"]["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{body}");
    assert!(
        (hits[0]["score"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "{body}"
    );
}

#[tokio::test]
async fn list_bounds_still_apply() {
    let app = persistent_app(&unique_path("parameter-list-bounds"));
    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({
            "query": "CREATE (n:Doc {embedding: $embedding})",
            "params": {"embedding": embedding(8193, 1.0)}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["error"]["code"], "UNSUPPORTED_PARAMETER_TYPE",
        "{body}"
    );

    let literal = (0..257)
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let (status, body) = post(
        &app,
        "/v1/cypher/write",
        json!({"query": format!("CREATE (n:Doc {{values: [{literal}]}})")}),
    )
    .await;
    // Refused through the Cypher API's validation result, as other malformed queries are.
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["result"]["status"], "ValidationFailed",
        "a 257-item literal list stays refused: {body}"
    );
}
