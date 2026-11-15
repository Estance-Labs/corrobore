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

//! Issue #343: an embedding provider turns the seed-search objective into a
//! vector, and the vector, semantic and hybrid modes resolve through the
//! canonical vector and hybrid retrieval of #326.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::{Json, Router, http::StatusCode, routing::post as post_route};
use graph_api_support::{app_state_error, app_with, get, post, unique_path};
use serde_json::{Value, json};

const HASHING: &[(&str, &str)] = &[
    ("CORROBORE_EMBEDDING_PROVIDER", "hashing"),
    ("CORROBORE_EMBEDDING_DIMENSIONS", "128"),
];

const DOCUMENTS: &[(&str, &str)] = &[
    (
        "Campaign",
        "acme phishing campaign targeting regional banks",
    ),
    ("Malware", "ransomware loader for windows endpoints"),
    ("Infrastructure", "north relay command and control server"),
];

async fn embed(app: &Router, texts: &[&str]) -> Vec<Vec<f64>> {
    let (status, body) = post(app, "/v1/embeddings", json!({ "texts": texts })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|vector| {
            vector
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap())
                .collect()
        })
        .collect()
}

async fn create(app: &Router, label: &str, name: &str, embedding: Option<Vec<f64>>) {
    let (query, params) = match embedding {
        Some(embedding) => (
            format!("CREATE (n:{label} {{name: $name, embedding: $embedding}})"),
            json!({ "name": name, "embedding": embedding }),
        ),
        None => (
            format!("CREATE (n:{label} {{name: $name}})"),
            json!({ "name": name }),
        ),
    };
    let (status, body) = post(
        app,
        "/v1/cypher/write",
        json!({ "query": query, "params": params }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn seed_embedded_documents(app: &Router) {
    let texts: Vec<&str> = DOCUMENTS.iter().map(|(_, name)| *name).collect();
    let vectors = embed(app, &texts).await;
    for ((label, name), vector) in DOCUMENTS.iter().zip(vectors) {
        create(app, label, name, Some(vector)).await;
    }
}

async fn seed(app: &Router, objective: &str, mode: &str) -> (StatusCode, Value) {
    post(
        app,
        "/v1/seed/search",
        json!({ "objective": objective, "mode": mode, "top_k": 3 }),
    )
    .await
}

async fn labels_of(app: &Router, node_id: &str) -> Vec<String> {
    let (status, body) = get(app, &format!("/v1/graph/nodes/{node_id}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["node"]["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn the_hashing_provider_embeds_text_deterministically() {
    let app = app_with(None, HASHING);
    let first = embed(&app, &["phishing campaign", "ransomware"]).await;
    let second = embed(&app, &["phishing campaign"]).await;
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].len(), 128);
    assert_eq!(first[0], second[0], "same text, same vector");
    let norm: f64 = first[0]
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-6,
        "vectors are unit length, got {norm}"
    );
    assert_ne!(first[0], first[1]);

    let (status, body) = post(&app, "/v1/embeddings", json!({ "texts": [] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "INVALID_EMBEDDING_REQUEST", "{body}");
}

#[tokio::test]
async fn embeddings_need_a_configured_provider() {
    let app = app_with(None, &[]);
    let (status, body) = post(&app, "/v1/embeddings", json!({ "texts": ["x"] })).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body["error"]["code"], "EMBEDDING_PROVIDER_NOT_CONFIGURED",
        "{body}"
    );
}

#[tokio::test]
async fn vector_mode_ranks_seeds_by_embedding_similarity() {
    let storage = unique_path("seed-vector");
    let app = app_with(Some(&storage), HASHING);
    seed_embedded_documents(&app).await;

    let (status, body) = seed(&app, "phishing campaign against banks", "vector").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["retrieval"], "vector");
    assert!(
        body["result"]["commit_id"]
            .as_str()
            .unwrap()
            .starts_with("gc")
    );
    let candidates = body["result"]["candidates"].as_array().unwrap();
    // At most top_k; the default threshold of 0 drops dissimilar nodes.
    assert!((1..=3).contains(&candidates.len()), "{candidates:?}");
    assert!(
        candidates
            .iter()
            .all(|c| c["score"].as_f64().unwrap() >= 0.0)
    );
    let top = candidates[0]["node_id"].as_str().unwrap();
    assert_eq!(labels_of(&app, top).await, ["Campaign"]);
    let rationale = candidates[0]["explanation"]["rationale"].as_str().unwrap();
    assert!(rationale.contains("cosine"), "{rationale}");
    assert!(rationale.contains("hashing"), "{rationale}");
    let scores: Vec<f64> = candidates
        .iter()
        .map(|c| c["score"].as_f64().unwrap())
        .collect();
    assert!(
        scores.windows(2).all(|pair| pair[0] >= pair[1]),
        "{scores:?}"
    );
    for candidate in candidates {
        let notes = candidate["explanation"]["boundary_notes"]
            .as_array()
            .unwrap();
        assert!(
            notes
                .iter()
                .all(|note| !note.as_str().unwrap().contains("not configured")),
            "{notes:?}"
        );
    }

    // `semantic` resolves the same way.
    let (status, body) = seed(&app, "phishing campaign against banks", "semantic").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["retrieval"], "vector");

    // A threshold above every score leaves no seed.
    let (status, body) = post(
        &app,
        "/v1/seed/search",
        json!({ "objective": "phishing campaign", "mode": "vector", "score_threshold": 1.5 }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["code"], "NO_SEED");
}

#[tokio::test]
async fn hybrid_mode_fuses_text_and_vector_rankings() {
    let storage = unique_path("seed-hybrid");
    let app = app_with(Some(&storage), HASHING);
    seed_embedded_documents(&app).await;

    let (status, body) = seed(&app, "ransomware loader", "hybrid").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["retrieval"], "hybrid");
    let candidates = body["result"]["candidates"].as_array().unwrap();
    let top = candidates[0]["node_id"].as_str().unwrap();
    assert_eq!(labels_of(&app, top).await, ["Malware"]);
    let rationale = candidates[0]["explanation"]["rationale"].as_str().unwrap();
    assert!(rationale.contains("reciprocal rank fusion"), "{rationale}");
    assert!(rationale.contains("vector #1"), "{rationale}");

    // The default mode is hybrid.
    let (status, body) = post(
        &app,
        "/v1/seed/search",
        json!({ "objective": "ransomware loader" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["retrieval"], "hybrid");
}

#[tokio::test]
async fn without_embedded_nodes_vector_mode_falls_back_to_lexical_retrieval() {
    let storage = unique_path("seed-unembedded");
    let app = app_with(Some(&storage), HASHING);
    for (label, name) in DOCUMENTS {
        create(&app, label, name, None).await;
    }
    let (status, body) = seed(&app, "phishing campaign", "vector").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["retrieval"], "lexical");
    let notes = body["result"]["candidates"][0]["explanation"]["boundary_notes"]
        .as_array()
        .unwrap();
    assert!(
        notes
            .iter()
            .any(|note| note.as_str().unwrap().contains("no node carries")),
        "{notes:?}"
    );
}

#[tokio::test]
async fn without_a_provider_seed_search_keeps_the_lexical_resolver() {
    let storage = unique_path("seed-no-provider");
    let app = app_with(Some(&storage), &[]);
    for (label, name) in DOCUMENTS {
        create(&app, label, name, None).await;
    }
    let (status, body) = seed(&app, "phishing campaign", "vector").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["retrieval"], "lexical");
    let notes = body["result"]["candidates"][0]["explanation"]["boundary_notes"]
        .as_array()
        .unwrap();
    assert!(
        notes.iter().any(|note| note
            .as_str()
            .unwrap()
            .contains("vector retrieval is not configured")),
        "{notes:?}"
    );
}

/// An OpenAI-compatible `/v1/embeddings` endpoint embedding by keyword.
async fn mock_embedding_server(fail: bool) -> String {
    let app = Router::new().route(
        "/v1/embeddings",
        post_route(move |Json(body): Json<Value>| async move {
            if fail {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "down" })),
                );
            }
            assert_eq!(body["model"], "keyword-model");
            let inputs = body["input"].as_array().unwrap();
            let data: Vec<Value> = inputs
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let text = text.as_str().unwrap();
                    let vector = [
                        if text.contains("phishing") { 1.0 } else { 0.0 },
                        if text.contains("ransomware") {
                            1.0
                        } else {
                            0.0
                        },
                        if text.contains("relay") { 1.0 } else { 0.0 },
                        0.1,
                    ];
                    json!({ "object": "embedding", "index": index, "embedding": vector })
                })
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "object": "list", "data": data })),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}/v1/embeddings")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_http_provider_embeds_through_an_openai_compatible_endpoint() {
    let endpoint = mock_embedding_server(false).await;
    let storage = unique_path("seed-http-provider");
    let app = app_with(
        Some(&storage),
        &[
            ("CORROBORE_EMBEDDING_PROVIDER", "http"),
            ("CORROBORE_EMBEDDING_ENDPOINT", endpoint.as_str()),
            ("CORROBORE_EMBEDDING_MODEL", "keyword-model"),
        ],
    );
    seed_embedded_documents(&app).await;
    let (status, body) = seed(&app, "relay", "vector").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let top = body["result"]["candidates"][0]["node_id"].as_str().unwrap();
    assert_eq!(labels_of(&app, top).await, ["Infrastructure"]);
    let rationale = body["result"]["candidates"][0]["explanation"]["rationale"]
        .as_str()
        .unwrap();
    assert!(rationale.contains("keyword-model"), "{rationale}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_provider_is_reported_as_a_bad_gateway() {
    let endpoint = mock_embedding_server(true).await;
    let app = app_with(
        None,
        &[
            ("CORROBORE_EMBEDDING_PROVIDER", "http"),
            ("CORROBORE_EMBEDDING_ENDPOINT", endpoint.as_str()),
            ("CORROBORE_EMBEDDING_MODEL", "keyword-model"),
        ],
    );
    let (status, body) = post(&app, "/v1/embeddings", json!({ "texts": ["x"] })).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["error"]["code"], "EMBEDDING_PROVIDER_FAILED");
}

#[test]
fn provider_configuration_is_validated() {
    // The configuration rejects these before the server starts; the harness
    // panics on a configuration error, so check them through the config.
    use corrobore_http_server::ServerConfig;
    use std::collections::HashMap;
    let config = |pairs: &[(&str, &str)]| {
        let mut vars: HashMap<String, String> =
            HashMap::from([("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), "token".to_owned())]);
        for (name, value) in pairs {
            vars.insert((*name).to_owned(), (*value).to_owned());
        }
        ServerConfig::from_map(&vars)
    };
    assert!(config(&[("CORROBORE_EMBEDDING_PROVIDER", "telepathy")]).is_err());
    assert!(config(&[("CORROBORE_EMBEDDING_PROVIDER", "http")]).is_err());
    assert!(
        config(&[
            ("CORROBORE_EMBEDDING_PROVIDER", "http"),
            (
                "CORROBORE_EMBEDDING_ENDPOINT",
                "http://127.0.0.1:1/v1/embeddings"
            ),
        ])
        .is_err(),
        "an HTTP provider names its model"
    );
    assert!(config(&[("CORROBORE_EMBEDDING_DIMENSIONS", "0")]).is_err());
    assert!(config(&[("CORROBORE_EMBEDDING_PROVIDER", "hashing")]).is_ok());
    assert!(app_state_error(None, &[("CORROBORE_EMBEDDING_PROVIDER", "none")]).is_none());
}
