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

//! Issue #388: declarative deployments, stored queries and live apply.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{persistent_app_with, send_as, unique_path};
use serde_json::{Value, json};

const ADMIN: &str = "admin-secret-token-00001";
const READER: &str = "reader-secret-token-0001";

const POLICY: &str = "version: 1
groups:
  admins: [act-admin]
  readers: [act-reader, act-admin]
rules:
  - id: admins-everything
    allow: { actors: { group: admins }, actions: [read, change, config_manage, invoke_query] }
  - id: readers-read
    allow: { actors: { group: readers }, actions: [read, invoke_query] }
";

fn bundle(policy: &str) -> Value {
    json!({
        "cluster.yaml": "version: 1\nmetadata: { name: intel }\npolicy: graph.policy.yaml\nqueries: queries/\n",
        "graph.policy.yaml": policy,
        "queries/indicator_names.cypher": "// Names of the indicators.\nMATCH (n:Indicator) RETURN n.name\n",
        "queries/add_indicator.cypher": "CREATE (n:Indicator {name: $name})\n",
    })
}

fn env(tokens: &str) -> Vec<(&'static str, String)> {
    vec![("CORROBORE_HTTP_BEARER_TOKENS_JSON", tokens.to_owned())]
}

fn tokens() -> String {
    json!({"act-admin": ADMIN, "act-reader": READER}).to_string()
}

fn app(root: &std::path::Path) -> axum::Router {
    let env = env(&tokens());
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    persistent_app_with(root, &env)
}

async fn deploy(app: &axum::Router, token: &str, id: &str, files: Value) -> (StatusCode, Value) {
    send_as(
        app,
        token,
        Method::POST,
        "/v1/cluster/deployments",
        Some(json!({"deployment_id": id, "files": files})),
        &[],
    )
    .await
}

#[tokio::test]
async fn a_live_deployment_installs_policy_and_queries() {
    let root = unique_path("cluster-live");
    let app = app(&root);
    let (status, body) = deploy(&app, ADMIN, "first", bundle(POLICY)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_eq!(result["applied"], true);
    assert_eq!(result["deployment"]["sequence"], 1);
    assert_eq!(result["deployment"]["actor"], "act-admin");
    assert_eq!(
        result["changes"],
        json!([
            {"kind": "policy_install"},
            {"kind": "query_add", "name": "add_indicator"},
            {"kind": "query_add", "name": "indicator_names"}
        ])
    );

    // The stored queries are listed with their mode and parameters.
    let (status, body) = send_as(&app, READER, Method::GET, "/v1/queries", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let queries = body["result"]["queries"].as_array().unwrap();
    assert_eq!(queries.len(), 2);
    assert_eq!(queries[0]["name"], "add_indicator");
    assert_eq!(queries[0]["writes"], true);
    assert_eq!(queries[0]["parameters"], json!(["name"]));
    assert_eq!(queries[1]["name"], "indicator_names");
    assert_eq!(queries[1]["description"], "Names of the indicators.");

    // The policy is in force: the reader runs reads, not writes.
    let (status, body) = send_as(
        &app,
        ADMIN,
        Method::POST,
        "/v1/queries/add_indicator",
        Some(json!({"params": {"name": "alpha"}})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send_as(
        &app,
        READER,
        Method::POST,
        "/v1/queries/add_indicator",
        Some(json!({"params": {"name": "beta"}})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = send_as(
        &app,
        READER,
        Method::POST,
        "/v1/queries/indicator_names",
        Some(json!({})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["result"]["data"]["Records"],
        json!([{"fields": {"n.name": "alpha"}}]),
        "{body}"
    );
    let (status, _) = send_as(
        &app,
        READER,
        Method::POST,
        "/v1/queries/missing",
        Some(json!({})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Only config_manage deploys; a proposed policy cannot grant it to its author.
    let generous = POLICY.replace(
        "[read, invoke_query]",
        "[read, invoke_query, config_manage]",
    );
    let (status, body) = deploy(&app, READER, "takeover", bundle(&generous)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Same id, same input: the recorded outcome. Same id, other input: refused.
    let (status, body) = deploy(&app, ADMIN, "first", bundle(POLICY)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["applied"], false);
    let (status, body) = deploy(&app, ADMIN, "first", bundle(&generous)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "DEPLOYMENT_CONFLICT");

    let (status, body) = send_as(&app, ADMIN, Method::GET, "/v1/cluster/status", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["applied"]["name"], "intel");
    assert_eq!(body["result"]["applied"]["queries"], 2);
    assert_eq!(body["result"]["applied"]["policy"], true);
    assert_eq!(body["result"]["deployments"].as_array().unwrap().len(), 1);

    // A restart installs the applied revision from the storage root.
    drop(app);
    let app = self::app(&root);
    let (status, body) = send_as(&app, READER, Method::GET, "/v1/queries", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["queries"].as_array().unwrap().len(), 2);
    let (status, _) = deploy(&app, READER, "second", bundle(POLICY)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_invalid_bundle_is_refused_whole() {
    let app = app(&unique_path("cluster-invalid"));
    for (files, needle) in [
        (
            json!({"queries/x.cypher": "MATCH (n) RETURN n"}),
            "cluster.yaml",
        ),
        (json!({"cluster.yaml": "version: 9\n"}), "version"),
        (
            json!({"cluster.yaml": "version: 1\nsurprise: 1\n"}),
            "surprise",
        ),
        (
            json!({"cluster.yaml": "version: 1\npolicy: missing.yaml\n"}),
            "missing.yaml",
        ),
        (
            json!({"cluster.yaml": "version: 1\nqueries: q/\n", "q/bad.cypher": "MATCH (n RETURN"}),
            "bad.cypher",
        ),
        (
            json!({"cluster.yaml": "version: 1\n", "../escape.yaml": "x"}),
            "relative path",
        ),
    ] {
        let (status, body) = deploy(&app, ADMIN, "bad", files).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "CLUSTER_BUNDLE_INVALID");
        assert!(
            body["error"]["message"].as_str().unwrap().contains(needle),
            "{needle}: {body}"
        );
    }
    let (_, body) = send_as(&app, ADMIN, Method::GET, "/v1/queries", None, &[]).await;
    assert_eq!(body["result"]["queries"], json!([]));
}
