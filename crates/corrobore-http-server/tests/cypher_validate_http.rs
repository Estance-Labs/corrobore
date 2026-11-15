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

//! Issue #336: parse-only Cypher validation for editors.
#![allow(clippy::unwrap_used)]

mod graph_api_support;

use axum::http::StatusCode;
use graph_api_support::{ephemeral_app, get, persistent_app, post, unique_path};
use serde_json::json;

#[tokio::test]
async fn validation_classifies_queries_and_reports_diagnostics() {
    let app = ephemeral_app();

    let (status, read) = post(
        &app,
        "/v1/cypher/validate",
        json!({"query": "MATCH (n:Indicator) RETURN n.name LIMIT 5"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["result"]["valid"], true);
    assert_eq!(read["result"]["kind"], "read");
    assert!(
        read["result"]["clauses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|clause| clause == "MATCH")
    );
    assert!(read["result"]["diagnostics"].as_array().unwrap().is_empty());

    let (_, write) = post(
        &app,
        "/v1/cypher/validate",
        json!({"query": "CREATE (n:Indicator {name: 'x'})"}),
    )
    .await;
    assert_eq!(write["result"]["kind"], "mutation");

    let (_, unsupported) = post(
        &app,
        "/v1/cypher/validate",
        json!({"query": "MATCH (n) CALL { WITH n RETURN n } RETURN n"}),
    )
    .await;
    assert_eq!(unsupported["result"]["valid"], false, "{unsupported}");
    assert!(unsupported["result"]["kind"].is_null());
    assert_eq!(unsupported["result"]["diagnostics"][0]["severity"], "error");

    let (_, empty) = post(&app, "/v1/cypher/validate", json!({"query": "   "})).await;
    assert_eq!(empty["result"]["diagnostics"][0]["code"], "EMPTY_QUERY");

    let (_, partial) = post(
        &app,
        "/v1/cypher/validate",
        json!({"query": "MATCH (n) RETURN n LIMIT nope"}),
    )
    .await;
    assert_eq!(partial["result"]["valid"], true);
    assert_eq!(
        partial["result"]["diagnostics"][0]["code"],
        "UNSTRUCTURED_QUERY"
    );

    let (_, unbound) = post(
        &app,
        "/v1/cypher/validate",
        json!({"query": "MATCH (n:Indicator) RETURN n LIMIT $size"}),
    )
    .await;
    let diagnostics = unbound["result"]["diagnostics"].as_array().unwrap();
    if !diagnostics.is_empty() {
        assert_eq!(diagnostics[0]["severity"], "warning");
        assert_eq!(unbound["result"]["valid"], true);
    }

    let huge = "MATCH (n) RETURN n ".repeat(4_000);
    let (status, _) = post(&app, "/v1/cypher/validate", json!({"query": huge})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn validating_a_write_never_executes_it() {
    let storage = unique_path("validate-no-write");
    let app = persistent_app(&storage);
    let (_, before) = get(&app, "/v1/graph/commits").await;

    for _ in 0..3 {
        let (status, body) = post(
            &app,
            "/v1/cypher/validate",
            json!({"query": "CREATE (n:Indicator {name: 'should-not-exist'})"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (_, after) = get(&app, "/v1/graph/commits").await;
    assert_eq!(before["result"]["head"], after["result"]["head"]);
    let (_, records) = get(&app, "/v1/graph/records?label=Indicator").await;
    assert_eq!(records["result"]["total"], 0);

    let _ = std::fs::remove_dir_all(storage);
}

/// Issue #360: a valid query's result carries its logical plan.
#[tokio::test]
async fn a_valid_query_explains_its_logical_plan() {
    let app = ephemeral_app();
    let (status, body) = post(
        &app,
        "/v1/cypher/validate",
        json!({"query": "MATCH (a:Actor)-[r:USES]->(m:Malware) WHERE m.name = 'X' RETURN a, m LIMIT 5"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plan = &body["result"]["plan"];
    assert_eq!(plan["query_kind"], "read");
    assert_eq!(
        plan["operators"],
        json!([
            "node_scan",
            "expand_relationship",
            "filter",
            "projection",
            "limit"
        ])
    );
    let bindings = plan["bindings"].as_array().unwrap();
    let find = |variable: &str| {
        bindings
            .iter()
            .find(|binding| binding["variable"] == variable)
            .unwrap_or_else(|| panic!("no binding {variable}: {plan}"))
            .clone()
    };
    assert_eq!(
        find("a"),
        json!({"variable": "a", "element": "node", "projected": true})
    );
    assert_eq!(
        find("r"),
        json!({"variable": "r", "element": "relationship", "projected": false})
    );
    assert_eq!(find("m")["projected"], true);

    let (_, body) = post(
        &app,
        "/v1/cypher/validate",
        json!({"query": "CREATE (n:Indicator {name: 'x'})"}),
    )
    .await;
    assert_eq!(body["result"]["plan"]["query_kind"], "mutation");
    assert!(
        body["result"]["plan"]["operators"]
            .as_array()
            .unwrap()
            .contains(&json!("mutation"))
    );

    let (_, body) = post(&app, "/v1/cypher/validate", json!({"query": "CALL {"})).await;
    assert_eq!(body["result"]["valid"], false);
    assert!(body["result"]["plan"].is_null());
}
