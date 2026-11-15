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

//! Issue #384: a Cedar policy decides what each actor may do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::{Method, StatusCode};
use graph_api_support::{TOKEN, persistent_app_with, send_as, unique_path};
use serde_json::{Value, json};

const READER: &str = "reader-secret-token-0001";
const WRITER: &str = "writer-secret-token-0002";
const MAINTAINER: &str = "maintainer-secret-token-03";

const POLICY: &str = r#"
version: 1
groups:
  readers: [act-reader, act-writer]
  writers: [act-writer]
protected_branches: [main]
rules:
  - id: readers-read
    allow:
      actors: { group: readers }
      actions: [read]
      branch_scope: any
  - id: writers-change-drafts
    allow:
      actors: { group: writers }
      actions: [change, review]
      branch_scope: unprotected
  - id: writers-open-drafts
    allow:
      actors: { group: writers }
      actions: [branch_create]
      target_branch_scope: unprotected
  - id: maintainer-reads
    allow:
      actors: { ids: [act-maintainer] }
      actions: [read, change]
  - id: maintainer-merges
    allow:
      actors: { ids: [act-maintainer] }
      actions: [branch_merge, branch_delete]
      target_branch_scope: any
"#;

static FILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn write_policy(text: &str) -> String {
    // A counter, not the clock: tests run concurrently.
    let n = FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = unique_path(&format!("policy-{n}")).with_extension("yaml");
    std::fs::write(&path, text).unwrap();
    path.display().to_string()
}

fn tokens() -> String {
    json!({"act-reader": READER, "act-writer": WRITER, "act-maintainer": MAINTAINER}).to_string()
}

fn policy_app(extra: &[(&str, &str)]) -> axum::Router {
    let policy = write_policy(POLICY);
    let tokens = tokens();
    let mut env = vec![
        ("CORROBORE_HTTP_POLICY_FILE", policy.as_str()),
        ("CORROBORE_HTTP_BEARER_TOKENS_JSON", tokens.as_str()),
    ];
    env.extend_from_slice(extra);
    persistent_app_with(&unique_path("policy-store"), &env)
}

async fn call(
    app: &axum::Router,
    token: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_as(app, token, method, uri, body, &[]).await
}

async fn cypher_write(
    app: &axum::Router,
    token: &str,
    query: &str,
    branch: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({"query": query});
    if let Some(branch) = branch {
        body["branch"] = json!(branch);
    }
    call(app, token, Method::POST, "/v1/cypher/write", Some(body)).await
}

fn assert_denied(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "POLICY_DENIED", "{body}");
}

#[tokio::test]
async fn readers_read_and_cannot_write() {
    let app = policy_app(&[]);
    let (status, body) =
        cypher_write(&app, MAINTAINER, "CREATE (:Indicator {name: 'a'})", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = call(
        &app,
        READER,
        Method::GET,
        "/v1/graph/records?kind=node",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        &app,
        READER,
        Method::POST,
        "/v1/cypher/read",
        Some(json!({"query": "MATCH (n) RETURN count(n)"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = cypher_write(&app, READER, "CREATE (:Indicator {name: 'b'})", None).await;
    assert_denied(status, &body);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("act-reader may not change on main"),
        "{body}"
    );
    // `execute` decides by the statement: a write is a change.
    let (status, body) = call(
        &app,
        READER,
        Method::POST,
        "/v1/cypher/execute",
        Some(json!({"query": "CREATE (:Indicator {name: 'c'})"})),
    )
    .await;
    assert_denied(status, &body);
    let (status, body) = call(
        &app,
        READER,
        Method::POST,
        "/v1/graph/edits",
        Some(json!({"edits": [{"op": "create_node", "labels": ["Indicator"], "properties": {}}]})),
    )
    .await;
    assert_denied(status, &body);
}

#[tokio::test]
async fn writers_change_drafts_but_not_the_protected_branch() {
    let app = policy_app(&[]);
    // main is protected: no direct write.
    let (status, body) = cypher_write(&app, WRITER, "CREATE (:Indicator {name: 'x'})", None).await;
    assert_denied(status, &body);

    // A draft branch from main, written and reviewed.
    let (status, body) = call(
        &app,
        WRITER,
        Method::POST,
        "/v1/graph/branches",
        Some(json!({"name": "drafts"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = cypher_write(
        &app,
        WRITER,
        "CREATE (:Indicator {name: 'x'})",
        Some("drafts"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        &app,
        WRITER,
        Method::POST,
        "/v1/graph/edits",
        Some(json!({"branch": "drafts", "edits": [{"op": "create_node", "labels": ["Indicator"], "properties": {"name": "y"}}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A branch named like a protected one cannot be created.
    let (status, body) = call(
        &app,
        WRITER,
        Method::POST,
        "/v1/graph/branches",
        Some(json!({"name": "main"})),
    )
    .await;
    assert_denied(status, &body);

    // Merging into main and deleting branches is the maintainer's.
    let (status, body) = call(
        &app,
        WRITER,
        Method::POST,
        "/v1/graph/merge",
        Some(json!({"source": "drafts"})),
    )
    .await;
    assert_denied(status, &body);
    let (status, body) = call(
        &app,
        WRITER,
        Method::DELETE,
        "/v1/graph/branches/drafts",
        None,
    )
    .await;
    assert_denied(status, &body);
    let (status, body) = call(
        &app,
        MAINTAINER,
        Method::POST,
        "/v1/graph/merge",
        Some(json!({"source": "drafts"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        &app,
        MAINTAINER,
        Method::DELETE,
        "/v1/graph/branches/drafts",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_shared_token_needs_an_actor_under_a_policy() {
    let app = policy_app(&[]);
    let (status, body) = call(&app, TOKEN, Method::GET, "/v1/graph/branches", None).await;
    assert_denied(status, &body);
    let app = policy_app(&[("CORROBORE_HTTP_POLICY_SHARED_TOKEN_ACTOR", "act-reader")]);
    let (status, body) = call(&app, TOKEN, Method::GET, "/v1/graph/branches", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = cypher_write(&app, TOKEN, "CREATE (:Indicator)", None).await;
    assert_denied(status, &body);
    // whoami needs no permission.
    let app = policy_app(&[]);
    let (status, _) = call(&app, TOKEN, Method::GET, "/v1/whoami", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_access_route_lists_what_the_caller_may_do() {
    let app = policy_app(&[]);
    let (status, body) = call(
        &app,
        READER,
        Method::GET,
        "/v1/graph/access?branch=main",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["actor_id"], "act-reader");
    assert_eq!(body["result"]["graph_wide"], json!(["read"]));
    assert_eq!(body["result"]["actions"], json!(["read"]));

    let (status, body) = call(
        &app,
        WRITER,
        Method::GET,
        "/v1/graph/access?branch=drafts&target_branch=feature",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["result"]["actions"],
        json!(["read", "change", "review", "branch_create"])
    );
    let (_, body) = call(
        &app,
        WRITER,
        Method::GET,
        "/v1/graph/access?branch=main",
        None,
    )
    .await;
    assert_eq!(body["result"]["actions"], json!(["read"]));

    // Without a policy every action is open.
    let app = persistent_app_with(&unique_path("policy-none"), &[]);
    let (_, body) = call(&app, TOKEN, Method::GET, "/v1/graph/access", None).await;
    assert_eq!(body["result"]["graph_wide"].as_array().unwrap().len(), 11);
}

#[tokio::test]
async fn reads_of_other_branches_and_commits_follow_the_scope() {
    let policy = write_policy(
        r#"
version: 1
protected_branches: [main]
rules:
  - id: public-reads
    allow:
      actors: any
      actions: [read]
      branch_scope: unprotected
"#,
    );
    let tokens = tokens();
    let app = persistent_app_with(
        &unique_path("policy-scope"),
        &[
            ("CORROBORE_HTTP_POLICY_FILE", policy.as_str()),
            ("CORROBORE_HTTP_BEARER_TOKENS_JSON", tokens.as_str()),
            ("CORROBORE_HTTP_POLICY_SHARED_TOKEN_ACTOR", "act-operator"),
        ],
    );
    // Nobody may read main here, nor read graph-wide (a commit by id).
    let (status, body) = call(&app, READER, Method::GET, "/v1/graph/records", None).await;
    assert_denied(status, &body);
    let (status, body) = call(
        &app,
        READER,
        Method::GET,
        "/v1/graph/records?branch=main",
        None,
    )
    .await;
    assert_denied(status, &body);
    let (status, body) = call(&app, READER, Method::GET, "/v1/graph/branches", None).await;
    assert_denied(status, &body);
    let (status, body) = call(
        &app,
        READER,
        Method::POST,
        "/v1/graph/search/hybrid",
        Some(json!({"text": "x", "branch": "main"})),
    )
    .await;
    assert_denied(status, &body);
    // An unprotected branch is readable (it need not exist to be judged).
    let (status, body) = call(
        &app,
        READER,
        Method::GET,
        "/v1/graph/diff?from=feature&to=main",
        None,
    )
    .await;
    assert_denied(status, &body);
    let (status, body) = call(
        &app,
        READER,
        Method::GET,
        "/v1/graph/changes?branch=feature",
        None,
    )
    .await;
    assert_ne!(status, StatusCode::FORBIDDEN, "{body}");
}

#[test]
fn an_invalid_policy_stops_the_server() {
    for bad in [
        "version: 2\n",
        "version: 1\nrules:\n  - id: r\n    allow:\n      actors: { group: missing }\n      actions: [read]\n",
        "version: 1\nrules:\n  - id: r\n    allow:\n      actors: any\n      actions: [fly]\n",
        "version: 1\nrules:\n  - id: r\n    allow:\n      actors: any\n      actions: [reclaim]\n      branch_scope: any\n",
        "version: 1\nprotected_branches: [' main']\n",
        "not: [yaml",
    ] {
        let path = write_policy(bad);
        let error = graph_api_support::app_state_error(
            Some(&unique_path("policy-bad")),
            &[("CORROBORE_HTTP_POLICY_FILE", path.as_str())],
        );
        assert!(
            error
                .as_deref()
                .is_some_and(|error| error.contains("policy")),
            "{bad}: {error:?}"
        );
    }
}
