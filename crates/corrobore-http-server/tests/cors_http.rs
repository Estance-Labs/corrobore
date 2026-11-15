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

//! Issue #359: opt-in CORS so a browser viewer on another origin can call
//! the server with its bearer token.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;

use axum::http::{HeaderMap, Method, StatusCode};
use graph_api_support::{TOKEN, app_state_error, app_with, ephemeral_app, send_bytes};

const VIEWER: &str = "http://localhost:5173";
const ALLOWED: &[(&str, &str)] = &[(
    "CORROBORE_HTTP_CORS_ALLOWED_ORIGINS",
    "https://viewer.example, http://localhost:5173",
)];

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).map(|value| value.to_str().unwrap())
}

async fn preflight(
    app: &axum::Router,
    origin: &str,
    path: &str,
    method: &str,
) -> (StatusCode, HeaderMap) {
    let (status, headers, _) = send_bytes(
        app,
        None,
        Method::OPTIONS,
        path,
        Vec::new(),
        &[
            ("origin", origin),
            ("access-control-request-method", method),
            (
                "access-control-request-headers",
                "authorization,content-type,if-graph-commit",
            ),
        ],
    )
    .await;
    (status, headers)
}

#[tokio::test]
async fn an_allowed_origin_gets_a_preflight_without_a_token() {
    let app = app_with(None, ALLOWED);
    for (path, method) in [
        ("/v1/cypher/write", "POST"),
        ("/health/ready", "GET"),
        ("/v1/graph/branches/x", "DELETE"),
    ] {
        let (status, headers) = preflight(&app, VIEWER, path, method).await;
        assert!(status.is_success(), "{path}: {status}");
        assert_eq!(
            header(&headers, "access-control-allow-origin"),
            Some(VIEWER),
            "{path}"
        );
        let methods = header(&headers, "access-control-allow-methods")
            .unwrap()
            .to_ascii_uppercase();
        assert!(methods.contains(method), "{path}: {methods}");
        let allowed = header(&headers, "access-control-allow-headers")
            .unwrap()
            .to_ascii_lowercase();
        for name in ["authorization", "content-type", "if-graph-commit"] {
            assert!(allowed.contains(name), "{path}: {allowed}");
        }
        assert_eq!(header(&headers, "access-control-allow-credentials"), None);
    }
}

#[tokio::test]
async fn responses_to_an_allowed_origin_carry_cors_headers() {
    let app = app_with(None, ALLOWED);
    let (status, headers, _) = send_bytes(
        &app,
        Some(TOKEN),
        Method::GET,
        "/v1/explorer/sessions",
        Vec::new(),
        &[("origin", "https://viewer.example")],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header(&headers, "access-control-allow-origin"),
        Some("https://viewer.example")
    );
    let exposed = header(&headers, "access-control-expose-headers")
        .unwrap()
        .to_ascii_lowercase();
    for name in ["content-range", "etag", "accept-ranges"] {
        assert!(exposed.contains(name), "{exposed}");
    }

    // The token is still required: CORS only lets the browser read answers.
    let (status, headers, _) = send_bytes(
        &app,
        None,
        Method::GET,
        "/v1/explorer/sessions",
        Vec::new(),
        &[("origin", VIEWER)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        header(&headers, "access-control-allow-origin"),
        Some(VIEWER)
    );
}

#[tokio::test]
async fn other_origins_and_the_default_get_no_cors_headers() {
    let app = app_with(None, ALLOWED);
    let (_, headers) = preflight(&app, "https://evil.example", "/v1/cypher/write", "POST").await;
    assert_eq!(header(&headers, "access-control-allow-origin"), None);

    let app = ephemeral_app();
    let (_, headers) = preflight(&app, VIEWER, "/v1/cypher/write", "POST").await;
    assert_eq!(header(&headers, "access-control-allow-origin"), None);
    let (_, headers, _) = send_bytes(
        &app,
        Some(TOKEN),
        Method::GET,
        "/v1/explorer/sessions",
        Vec::new(),
        &[("origin", VIEWER)],
    )
    .await;
    assert_eq!(header(&headers, "access-control-allow-origin"), None);
}

#[test]
fn origins_must_be_exact() {
    use corrobore_http_server::ServerConfig;
    use std::collections::HashMap;
    let config = |origins: &str| {
        ServerConfig::from_map(&HashMap::from([
            ("CORROBORE_HTTP_AUTH_TOKEN".to_owned(), "token".to_owned()),
            (
                "CORROBORE_HTTP_CORS_ALLOWED_ORIGINS".to_owned(),
                origins.to_owned(),
            ),
        ]))
    };
    for invalid in [
        "*",
        "viewer.example",
        "ftp://viewer.example",
        "https://viewer.example/app",
        "https://viewer.example/",
        "https://viewer.example?x=1",
        "https://user@viewer.example",
    ] {
        assert!(config(invalid).is_err(), "{invalid} must be refused");
    }
    let parsed = config(" https://viewer.example , http://localhost:5173 ").unwrap();
    assert_eq!(
        parsed.cors_allowed_origins,
        ["https://viewer.example", "http://localhost:5173"]
    );
    assert!(config("").unwrap().cors_allowed_origins.is_empty());
    assert!(app_state_error(None, &[("CORROBORE_HTTP_CORS_ALLOWED_ORIGINS", VIEWER)]).is_none());
}
