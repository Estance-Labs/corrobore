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

//! Issue #382: named identities and an OAuth 2.0 resource server.
//!
//! The server accepts JWT access tokens from a configured issuer, publishes
//! RFC 9728 metadata so a client can find where to sign in, and records the
//! authenticated actor on the commits it writes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod graph_api_support;
mod oauth_support;

use axum::http::{HeaderMap, Method, StatusCode};
use corrobore_http_server::ServerConfig;
use graph_api_support::{TOKEN, app_with, persistent_app_with, send_as, send_bytes, unique_path};
use oauth_support::{TestIssuer, hmac_token, now, unsigned_token};
use serde_json::{Value, json};

const AUDIENCE: &str = "https://graph.example";
const METADATA_URL: &str = "https://graph.example/.well-known/oauth-protected-resource";

fn oauth_env(issuer: &TestIssuer) -> Vec<(&'static str, String)> {
    vec![
        ("CORROBORE_HTTP_OAUTH_ISSUER", issuer.url().to_owned()),
        ("CORROBORE_HTTP_OAUTH_AUDIENCE", AUDIENCE.to_owned()),
        (
            "CORROBORE_HTTP_OAUTH_SCOPES",
            "graph:read graph:write".to_owned(),
        ),
    ]
}

fn oauth_app(issuer: &TestIssuer, extra: &[(&str, &str)]) -> axum::Router {
    let env = oauth_env(issuer);
    let mut all: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    all.extend_from_slice(extra);
    app_with(None, &all)
}

async fn whoami(app: &axum::Router, token: &str) -> (StatusCode, Value) {
    send_as(app, token, Method::GET, "/v1/whoami", None, &[]).await
}

fn challenge(headers: &HeaderMap) -> String {
    headers
        .get("www-authenticate")
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default()
}

async fn rejected(app: &axum::Router, token: &str) -> String {
    let (status, headers, body) =
        send_bytes(app, Some(token), Method::GET, "/v1/whoami", Vec::new(), &[]).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    challenge(&headers)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_valid_access_token_names_its_principal() {
    let issuer = TestIssuer::start().await;
    let app = oauth_app(&issuer, &[]);
    let (status, body) = whoami(&app, &issuer.token("alice", AUDIENCE)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["actor"], "principal:alice");
    assert_eq!(body["result"]["client_id"], "corrobore-viewer");
    assert_eq!(body["result"]["method"], "oauth");
    // The static token still works beside OAuth, as an unnamed trusted client.
    let (status, body) = whoami(&app, TOKEN).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["actor"], Value::Null);
    assert_eq!(body["result"]["method"], "static");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_resource_metadata_is_public_and_names_the_issuer() {
    let issuer = TestIssuer::start().await;
    let app = oauth_app(&issuer, &[]);
    let (status, headers, body) = send_bytes(
        &app,
        None,
        Method::GET,
        "/.well-known/oauth-protected-resource",
        Vec::new(),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["resource"], AUDIENCE);
    assert_eq!(body["authorization_servers"], json!([issuer.url()]));
    assert_eq!(body["bearer_methods_supported"], json!(["header"]));
    assert_eq!(
        body["scopes_supported"],
        json!(["graph:read", "graph:write"])
    );

    // Without OAuth there is no metadata document.
    let app = app_with(None, &[]);
    let (status, _, _) = send_bytes(
        &app,
        None,
        Method::GET,
        "/.well-known/oauth-protected-resource",
        Vec::new(),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_credential_points_the_client_to_the_metadata() {
    let issuer = TestIssuer::start().await;
    let app = oauth_app(&issuer, &[]);
    let (status, headers, _) =
        send_bytes(&app, None, Method::GET, "/v1/whoami", Vec::new(), &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let value = challenge(&headers);
    assert_eq!(
        value,
        format!("Bearer resource_metadata=\"{METADATA_URL}\"")
    );

    // Static-token servers keep their plain 401 without a challenge pointer.
    let app = app_with(None, &[]);
    let (status, headers, _) =
        send_bytes(&app, None, Method::GET, "/v1/whoami", Vec::new(), &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!challenge(&headers).contains("resource_metadata"));
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_and_misaddressed_tokens_are_refused() {
    let issuer = TestIssuer::start().await;
    let app = oauth_app(&issuer, &[]);

    let mut expired = issuer.claims("alice", AUDIENCE);
    expired["exp"] = json!(now() - 600);
    expired["nbf"] = json!(now() - 900);
    let value = rejected(&app, &issuer.sign("k1", &expired)).await;
    assert!(value.contains("error=\"invalid_token\""), "{value}");
    assert!(value.contains(METADATA_URL), "{value}");

    let mut not_yet = issuer.claims("alice", AUDIENCE);
    not_yet["nbf"] = json!(now() + 600);
    rejected(&app, &issuer.sign("k1", &not_yet)).await;

    let wrong_audience = issuer.claims("alice", "https://other.example");
    rejected(&app, &issuer.sign("k1", &wrong_audience)).await;

    let mut wrong_issuer = issuer.claims("alice", AUDIENCE);
    wrong_issuer["iss"] = json!("https://evil.example");
    rejected(&app, &issuer.sign("k1", &wrong_issuer)).await;

    let mut no_subject = issuer.claims("alice", AUDIENCE);
    no_subject.as_object_mut().unwrap().remove("sub");
    rejected(&app, &issuer.sign("k1", &no_subject)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unsigned_and_hmac_tokens_are_refused() {
    let issuer = TestIssuer::start().await;
    let app = oauth_app(&issuer, &[]);
    let claims = issuer.claims("alice", AUDIENCE);
    rejected(&app, &unsigned_token(&claims)).await;
    rejected(&app, &hmac_token(&claims, b"guessable-secret")).await;
    // A token signed by a key the issuer never published.
    let other = TestIssuer::start().await;
    let mut foreign = other.claims("alice", AUDIENCE);
    foreign["iss"] = json!(issuer.url());
    rejected(&app, &other.sign("k1", &foreign)).await;
    rejected(&app, "not.a.jwt").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rotated_key_is_fetched_once_and_cached() {
    let issuer = TestIssuer::start().await;
    let app = oauth_app(
        &issuer,
        &[("CORROBORE_HTTP_OAUTH_JWKS_MIN_REFRESH_SECONDS", "0")],
    );
    let (status, _) = whoami(&app, &issuer.token("alice", AUDIENCE)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = whoami(&app, &issuer.token("bob", AUDIENCE)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(issuer.jwks_fetches(), 1, "keys are cached");

    issuer.add_key("k2");
    let rotated = issuer.sign("k2", &issuer.claims("carol", AUDIENCE));
    let (status, body) = whoami(&app, &rotated).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["actor"], "principal:carol");
    assert_eq!(
        issuer.jwks_fetches(),
        2,
        "an unknown kid refreshes the keys"
    );

    // With the default interval, unknown key ids cannot make the server
    // hammer the issuer: one fetch, then the cached set answers.
    let issuer = TestIssuer::start().await;
    let app = oauth_app(&issuer, &[]);
    let (status, _) = whoami(&app, &issuer.token("alice", AUDIENCE)).await;
    assert_eq!(status, StatusCode::OK);
    issuer.add_key("k3");
    for _ in 0..3 {
        rejected(&app, &issuer.sign("k3", &issuer.claims("dave", AUDIENCE))).await;
    }
    assert_eq!(issuer.jwks_fetches(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_explicit_jwks_url_and_rfc8414_discovery_both_work() {
    let issuer = TestIssuer::start_with_discovery("/.well-known/oauth-authorization-server").await;
    let app = oauth_app(&issuer, &[]);
    let (status, body) = whoami(&app, &issuer.token("alice", AUDIENCE)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let issuer = TestIssuer::start_with_discovery("/nowhere").await;
    let jwks = issuer.jwks_url();
    let app = oauth_app(&issuer, &[("CORROBORE_HTTP_OAUTH_JWKS_URL", jwks.as_str())]);
    let (status, body) = whoami(&app, &issuer.token("alice", AUDIENCE)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_client_allowlist_refuses_other_clients() {
    let issuer = TestIssuer::start().await;
    let app = oauth_app(
        &issuer,
        &[("CORROBORE_HTTP_OAUTH_CLIENT_IDS", "corrobore-viewer, cli")],
    );
    let (status, _) = whoami(&app, &issuer.token("alice", AUDIENCE)).await;
    assert_eq!(status, StatusCode::OK);
    let mut other = issuer.claims("alice", AUDIENCE);
    other["client_id"] = json!("someone-else");
    rejected(&app, &issuer.sign("k1", &other)).await;
    // `azp` names the client when `client_id` is absent.
    let mut azp = issuer.claims("alice", AUDIENCE);
    azp.as_object_mut().unwrap().remove("client_id");
    azp["azp"] = json!("cli");
    let (status, body) = whoami(&app, &issuer.sign("k1", &azp)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["client_id"], "cli");
}

#[tokio::test]
async fn named_static_tokens_identify_their_actor() {
    let app = app_with(
        None,
        &[(
            "CORROBORE_HTTP_BEARER_TOKENS_JSON",
            r#"{"act-alice":"alice-secret-token","act-bot":"bot-secret-token"}"#,
        )],
    );
    let (status, body) = whoami(&app, "alice-secret-token").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["actor"], "act-alice");
    assert_eq!(body["result"]["method"], "static");
    let (status, _) = whoami(&app, "unknown-token").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// The configuration error for `extra` on top of a minimal valid setup.
fn config_error(extra: &[(&str, &str)]) -> Option<String> {
    let mut vars = std::collections::HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        TOKEN.to_owned(),
    )]);
    for (name, value) in extra {
        vars.insert((*name).to_owned(), (*value).to_owned());
    }
    ServerConfig::from_map(&vars)
        .err()
        .map(|error| error.to_string())
}

#[tokio::test]
async fn identity_configuration_is_validated() {
    // The audience is the resource identifier and is required with an issuer.
    let error = config_error(&[("CORROBORE_HTTP_OAUTH_ISSUER", "https://id.example")]).unwrap();
    assert!(error.contains("CORROBORE_HTTP_OAUTH_AUDIENCE"), "{error}");
    // Plain http is only accepted for a loopback issuer.
    let error = config_error(&[
        ("CORROBORE_HTTP_OAUTH_ISSUER", "http://id.example"),
        ("CORROBORE_HTTP_OAUTH_AUDIENCE", AUDIENCE),
    ])
    .unwrap();
    assert!(error.contains("CORROBORE_HTTP_OAUTH_ISSUER"), "{error}");
    for bad in [
        r#"{"act alice":"token-with-space-actor"}"#,
        r#"{"act-alice":"short"}"#,
        r#"{"a":"same-token-twice-0","b":"same-token-twice-0"}"#,
        r#"["not","a","map"]"#,
    ] {
        let error = config_error(&[("CORROBORE_HTTP_BEARER_TOKENS_JSON", bad)]);
        assert!(
            error
                .as_deref()
                .is_some_and(|error| error.contains("CORROBORE_HTTP_BEARER_TOKENS_JSON")),
            "{bad}: {error:?}"
        );
    }
    // OAuth alone is enough to require authentication: no shared token.
    let vars = std::collections::HashMap::from([
        (
            "CORROBORE_HTTP_OAUTH_ISSUER".to_owned(),
            "https://id.example".to_owned(),
        ),
        (
            "CORROBORE_HTTP_OAUTH_AUDIENCE".to_owned(),
            AUDIENCE.to_owned(),
        ),
    ]);
    let config = ServerConfig::from_map(&vars).unwrap();
    assert!(config.auth_token.is_none());
    assert_eq!(
        config.identity.oauth.unwrap().resource_metadata_url(),
        METADATA_URL
    );
    // Audience settings without an issuer are a mistake, not ignored.
    assert!(config_error(&[("CORROBORE_HTTP_OAUTH_AUDIENCE", AUDIENCE)]).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_record_the_authenticated_actor() {
    let issuer = TestIssuer::start().await;
    let mut env = oauth_env(&issuer);
    env.push((
        "CORROBORE_HTTP_BEARER_TOKENS_JSON",
        r#"{"act-alice":"alice-secret-token"}"#.to_owned(),
    ));
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let app = persistent_app_with(&unique_path("oauth-actor"), &env);
    let bearer = issuer.token("bob", AUDIENCE);

    // A Cypher write on main through an access token.
    let (status, body) = send_as(
        &app,
        &bearer,
        Method::POST,
        "/v1/cypher/write",
        Some(json!({"query": "CREATE (n:Indicator {name: 'alpha'})"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(head_actor(&app, "main").await, "principal:bob");

    // Edits with a named static token and no body actor.
    let (status, body) = send_as(
        &app,
        "alice-secret-token",
        Method::POST,
        "/v1/graph/edits",
        Some(json!({"edits": [{"op": "create_node", "labels": ["Indicator"], "properties": {"name": "beta"}}]})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(head_actor(&app, "main").await, "act-alice");

    // A body actor that contradicts the identity is refused, nothing written.
    let before = head_actor(&app, "main").await;
    let (status, body) = send_as(
        &app,
        "alice-secret-token",
        Method::POST,
        "/v1/graph/edits",
        Some(json!({"actor": "act-mallory", "edits": [{"op": "create_node", "labels": ["Indicator"], "properties": {"name": "gamma"}}]})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "ACTOR_MISMATCH");
    assert_eq!(head_actor(&app, "main").await, before);

    // The same actor in the body is accepted.
    let (status, body) = send_as(
        &app,
        "alice-secret-token",
        Method::POST,
        "/v1/graph/branches",
        Some(json!({"name": "drafts", "actor": "act-alice"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A Cypher write on a branch records the token's principal.
    let (status, body) = send_as(
        &app,
        &bearer,
        Method::POST,
        "/v1/cypher/write",
        Some(json!({"query": "CREATE (n:Indicator {name: 'delta'})", "branch": "drafts"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(head_actor(&app, "drafts").await, "principal:bob");

    // The unnamed static token keeps naming its actor in the body.
    let (status, body) = send_as(
        &app,
        TOKEN,
        Method::POST,
        "/v1/graph/edits",
        Some(json!({"actor": "actor--importer", "edits": [{"op": "create_node", "labels": ["Indicator"], "properties": {"name": "epsilon"}}]})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(head_actor(&app, "main").await, "actor--importer");
}

async fn head_actor(app: &axum::Router, branch: &str) -> String {
    let (status, body) = send_as(
        app,
        TOKEN,
        Method::GET,
        &format!("/v1/graph/commits?branch={branch}&limit=1"),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["result"]["commits"][0]["actor"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
