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

//! Issue #353: `POST /v1/nlq/compile` compiles a French or English request
//! into a validated `nlq/v1` action and executes nothing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{ephemeral_app, get, post};
use serde_json::{Value, json};

async fn compile(app: &axum::Router, body: Value) -> (StatusCode, Value) {
    post(app, "/v1/nlq/compile", body).await
}

#[tokio::test]
async fn a_question_compiles_to_a_bounded_read() {
    let app = ephemeral_app();
    let (status, body) = compile(
        &app,
        json!({"text": "Quels acteurs de menace utilisent le malware \"X-Agent\" ?"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_eq!(result["kind"], "cypher_read");
    assert_eq!(result["language"], "fr");
    assert_eq!(
        result["canonical"],
        "MATCH (a:ThreatActor)-[r:USES]->(m:Malware) WHERE m.name = 'X-Agent' RETURN a.name LIMIT 50"
    );
    assert_eq!(result["writes"], false);
    assert_eq!(result["envelope"]["schema_version"], "nlq/v1");
    assert_eq!(result["envelope"]["action"]["kind"], "cypher_read");

    let (status, body) = compile(&app, json!({"text": "How many campaigns?"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["language"], "en");
    assert_eq!(
        body["result"]["canonical"],
        "MATCH (n:Campaign) RETURN count(n)"
    );
}

#[tokio::test]
async fn compiling_executes_nothing() {
    let app = ephemeral_app();
    let (_, before) = get(&app, "/v1/explorer/sessions").await;
    let (status, body) = compile(
        &app,
        json!({
            "text": "Remember that \"APT28 uses X-Agent\" (source span--7)",
            "evidence_refs": ["span--7"],
            "allow_writes": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["kind"], "memory_operation");
    assert_eq!(body["result"]["writes"], true);
    let (_, after) = get(&app, "/v1/explorer/sessions").await;
    assert_eq!(before, after);
}

#[tokio::test]
async fn writes_and_unknown_requests_end_in_typed_terminal_answers() {
    let app = ephemeral_app();
    // A write under a read-only request abstains rather than guessing.
    let (status, body) = compile(
        &app,
        json!({"text": "Remember that \"APT28 uses X-Agent\" (source span--7)", "evidence_refs": ["span--7"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["kind"], "abstain");
    assert!(
        body["result"]["canonical"]
            .as_str()
            .unwrap()
            .starts_with("abstain:")
    );

    let (status, body) = compile(&app, json!({"text": "Tell me about X"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["kind"], "clarification_required");
    assert!(
        body["result"]["envelope"]["action"]["options"]
            .as_array()
            .unwrap()
            .len()
            >= 2
    );

    let (status, body) = compile(
        &app,
        json!({"text": "Delete all indicators", "allow_writes": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["kind"], "unsupported");
}

#[tokio::test]
async fn invalid_requests_are_rejected() {
    let app = ephemeral_app();
    for body in [
        json!({"text": "   "}),
        json!({"text": "List the indicators", "language": "de"}),
        json!({"text": "x".repeat(4001)}),
        json!({"text": "List the indicators", "evidence_refs": [""]}),
    ] {
        let (status, response) = compile(&app, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} → {response}");
        assert_eq!(
            response["error"]["code"], "INVALID_NLQ_REQUEST",
            "{response}"
        );
    }
    // An explicit language wins over detection.
    let (status, body) = compile(
        &app,
        json!({"text": "List the indicators", "language": "fr"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["language"], "fr");
}
