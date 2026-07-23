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
#[cfg(all(unix, feature = "enterprise-cti"))]
use std::process::Command;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use ed25519_dalek::pkcs8::EncodePublicKey;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
#[cfg(all(unix, feature = "enterprise-cti"))]
use sha2::{Digest, Sha256};
use tower::ServiceExt;

#[derive(serde::Serialize)]
struct TestUnsignedLicenseClaims<'a> {
    client_uuid: &'a str,
    client_email: &'a str,
    modules: &'a [String],
    valid_until: &'a str,
    tags: &'a [String],
}

fn test_app() -> axum::Router {
    test_app_with_store_dir(unique_store_dir("default"))
}

fn test_app_with_store_dir(store_dir: PathBuf) -> axum::Router {
    test_app_with_store_dir_and_extra_env(store_dir, HashMap::new())
}

fn test_app_with_store_dir_and_extra_env(
    store_dir: PathBuf,
    extra_env: HashMap<String, String>,
) -> axum::Router {
    let legacy_modules = extra_env
        .get("CORROBORE_HTTP_LICENSED_MODULES")
        .cloned()
        .unwrap_or_else(|| "cti".to_owned());
    let mut vars = HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            store_dir.display().to_string(),
        ),
    ]);
    vars.extend(extra_env);
    vars.remove("CORROBORE_HTTP_LICENSED_MODULES");
    vars.extend(test_signed_license_env(&legacy_modules));

    let config = ServerConfig::from_map(&vars).expect("config should parse");

    build_router(AppState::new(config).expect("app state should initialize"))
}

fn unique_store_dir(suffix: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis();

    std::env::temp_dir().join(format!(
        "corrobore-http-session-store-{}-{}",
        suffix, millis
    ))
}

fn test_signed_license_env(modules_csv: &str) -> HashMap<String, String> {
    let signing = SigningKey::from_bytes(&[23_u8; 32]);
    let public_key_der = signing
        .verifying_key()
        .to_public_key_der()
        .expect("public key der should serialize")
        .as_bytes()
        .to_vec();
    let verifying_pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
        STANDARD.encode(public_key_der)
    );

    let mut modules = modules_csv
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    modules.sort();
    modules.dedup();

    let mut tags = vec!["NFR".to_owned()]
        .into_iter()
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();

    let canonical = serde_json::to_vec(&TestUnsignedLicenseClaims {
        client_uuid: "11111111-2222-4333-8444-555555555555",
        client_email: "tests@corrobore.dev",
        modules: &modules,
        valid_until: "2099-01-01T00:00:00Z",
        tags: &tags,
    })
    .expect("canonical payload should serialize");
    let signature = signing.sign(&canonical);

    let license_json = serde_json::to_vec(&json!({
        "client_uuid": "11111111-2222-4333-8444-555555555555",
        "client_email": "tests@corrobore.dev",
        "modules": modules,
        "valid_until": "2099-01-01T00:00:00Z",
        "tags": ["NFR"],
        "signature": STANDARD.encode(signature.to_bytes()),
    }))
    .expect("license payload should serialize");
    let license_pem = format!(
        "-----BEGIN CORROBORE LICENSE-----\n{}\n-----END CORROBORE LICENSE-----",
        STANDARD.encode(license_json)
    );

    HashMap::from([
        ("CORROBORE_HTTP_LICENSE_PEM".to_owned(), license_pem),
        (
            "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM".to_owned(),
            verifying_pem,
        ),
    ])
}

#[tokio::test]
async fn domain_validation_contract_rejects_unknown_domain() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/domains/unknown/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"payload": {}}).to_string()))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "INVALID_DOMAIN");
}

#[cfg(feature = "enterprise-fimi")]
#[tokio::test]
async fn domain_validation_contract_rejects_unlicensed_module() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/domains/fimi/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"payload": {}}).to_string()))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "LICENSE_MODULE_MISSING");
}

#[cfg(feature = "enterprise-fimi")]
#[tokio::test]
async fn domain_validation_contract_rejects_missing_provider() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("domain-fimi-provider-missing"),
        HashMap::from([(
            "CORROBORE_HTTP_LICENSED_MODULES".to_owned(),
            "fimi".to_owned(),
        )]),
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/domains/fimi/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"payload": {}}).to_string()))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "DOMAIN_PROVIDER_NOT_READY");
}

#[cfg(all(unix, feature = "enterprise-cti"))]
#[tokio::test]
async fn domain_validation_contract_invokes_real_c_provider() {
    let (provider_dir, manifest_file) = compile_c_provider_fixture();
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("domain-cti-provider-success"),
        HashMap::from([
            (
                "CORROBORE_DOMAIN_PROVIDER_DIR".to_owned(),
                provider_dir.display().to_string(),
            ),
            (
                "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE".to_owned(),
                manifest_file.display().to_string(),
            ),
        ]),
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/domains/cti/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "request_id": "http-c-provider",
                "workspace_id": "workspace--test",
                "payload": {"labels": ["ThreatActor"]}
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["request_id"], "http-c-provider");
    assert_eq!(payload["result"]["status"], "accepted");
    assert_eq!(payload["result"]["issues"], json!([]));
}

#[cfg(all(unix, feature = "enterprise-cti"))]
fn compile_c_provider_fixture() -> (PathBuf, PathBuf) {
    let root = unique_store_dir("c-domain-provider");
    fs::create_dir_all(&root).expect("provider root should be created");
    let source = root.join("provider.c");
    fs::write(&source, include_str!("fixtures/domain_provider_v1.c"))
        .expect("provider source should be written");
    let library_name = if cfg!(target_os = "macos") {
        "libcorrobore_domain_cti.dylib"
    } else {
        "libcorrobore_domain_cti.so"
    };
    let library = root.join(library_name);
    let include_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../domain-provider-abi/include");
    let mut command = Command::new("cc");
    if cfg!(target_os = "macos") {
        command.arg("-dynamiclib");
    } else {
        command.args(["-shared", "-fPIC"]);
    }
    let output = command
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(include_dir)
        .arg(&source)
        .arg("-o")
        .arg(&library)
        .output()
        .expect("C compiler should run");
    assert!(
        output.status.success(),
        "C provider fixture compilation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let hash = format!(
        "{:x}",
        Sha256::digest(fs::read(&library).expect("provider library should be readable"))
    );
    let manifest = root.join("providers.json");
    fs::write(
        &manifest,
        json!({
            "schema_version": "1",
            "providers": [{
                "domain": "cti",
                "library": library_name,
                "sha256": hash,
                "required": true,
                "capabilities": [{"name": "node.validate", "version": "1"}]
            }]
        })
        .to_string(),
    )
    .expect("provider manifest should be written");
    (root, manifest)
}

fn looks_like_uuid(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 5 {
        return false;
    }

    let expected_lengths = [8, 4, 4, 4, 12];
    for (part, expected_len) in parts.iter().zip(expected_lengths) {
        if part.len() != expected_len {
            return false;
        }

        if !part.chars().all(|character| character.is_ascii_hexdigit()) {
            return false;
        }
    }

    true
}

#[tokio::test]
async fn health_contract_returns_ok_payload() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("health should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("health payload should be json");

    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["service"], "corrobore-http-server");
    assert_eq!(payload["storage_mode"], "ephemeral");
    assert_eq!(payload["durability"]["recovery"]["outcome"], "ephemeral");
    assert_eq!(payload["durability"]["controls"]["require_fsync"], false);
    assert_eq!(payload["durability"]["controls"]["strict_recovery"], false);
    assert_eq!(payload["durability"]["wal_bytes"], 0);
    assert_eq!(payload["durability"]["wal_lag_sequences"], 0);
    assert_eq!(payload["durability"]["checkpoint_sequence"], Value::Null);
    assert_eq!(payload["durability"]["checkpoint_age_seconds"], Value::Null);
    assert_eq!(payload["durability"]["compaction_backlog_bytes"], 0);
    assert_eq!(payload["domain_providers"]["configured"], 0);
    assert_eq!(payload["domain_providers"]["ready"], 0);
    assert_eq!(payload["session_ttl_metrics"]["total_expired_sessions"], 0);
    assert_eq!(
        payload["session_ttl_metrics"]["expired_last_5m_sessions"],
        0
    );
}

#[tokio::test]
async fn health_contract_exposes_persistent_recovery_and_controls() {
    let storage_root = unique_store_dir("durability-persistent-root");
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("durability-persistent-sessions"),
        HashMap::from([
            ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
            (
                "CORROBORE_STORAGE_DIR".to_owned(),
                storage_root.display().to_string(),
            ),
        ]),
    );
    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("health should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("health payload should be json");

    assert_eq!(payload["storage_mode"], "persistent");
    assert_eq!(payload["durability"]["controls"]["require_fsync"], true);
    assert_eq!(payload["durability"]["controls"]["strict_recovery"], true);
    assert_eq!(payload["durability"]["recovery"]["outcome"], "recovered");
    assert_eq!(payload["durability"]["storage_version"], "V1");
    assert_eq!(payload["durability"]["record_format"], "JsonLinesV1");
    assert_eq!(
        payload["durability"]["recovery"]["manifest_validated"],
        true
    );
    assert_eq!(payload["durability"]["wal_bytes"].as_u64(), Some(0));

    let _ = fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn cypher_contract_rejects_missing_auth_header() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "query": "MATCH (n) RETURN n LIMIT 1"
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cypher_contract_executes_read_query_with_auth() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "query": "MATCH (n) RETURN n LIMIT 1",
                "mode": "read"
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["status"], "Success");
}

#[tokio::test]
async fn cypher_read_contract_rejects_mutation_query() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "query": "MATCH (n) DELETE n"
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["status"], "Rejected");
    assert_eq!(
        payload["result"]["validation_errors"][0]["code"],
        "WRITE_PERMISSION_REQUIRED"
    );
}

#[tokio::test]
async fn cypher_contract_restores_session_idle_when_request_is_invalid() {
    let store_dir = unique_store_dir("cypher-audit-invalid-request");
    let app = test_app_with_store_dir(store_dir);

    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-cypher-invalid",
                "actor_id": "actor--http-cypher-invalid",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    let invalid_query_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "query": "   ",
                "session_id": session_id,
                "workspace_id": "workspace--http-cypher-invalid"
            })
            .to_string(),
        ))
        .expect("request should build");

    let invalid_query_response = app
        .clone()
        .oneshot(invalid_query_request)
        .await
        .expect("invalid query request should respond");
    assert_eq!(invalid_query_response.status(), StatusCode::BAD_REQUEST);

    let health_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/health"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let health_response = app
        .clone()
        .oneshot(health_request)
        .await
        .expect("health should respond");
    assert_eq!(health_response.status(), StatusCode::OK);

    let health_body = to_bytes(health_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let health_payload: Value =
        serde_json::from_slice(&health_body).expect("payload should be json");
    assert_eq!(health_payload["result"]["status"], "idle");
}

#[tokio::test]
async fn export_contract_uses_default_snapshot_when_missing() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/export/stix")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["type"], "bundle");
    assert_eq!(
        payload["export_metadata"]["snapshot_id"],
        "snapshot--current"
    );
}

#[tokio::test]
async fn import_stix_contract_rejects_missing_auth_header() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/import/stix")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "bundle": {
                    "type": "bundle",
                    "objects": [
                        {
                            "type": "identity",
                            "id": "identity--import-auth"
                        }
                    ]
                }
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stix_validate_contract_rejects_missing_auth_header() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "source": "bundle",
                "bundle": {
                    "type": "bundle",
                    "objects": []
                }
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn license_status_contract_rejects_missing_auth_header() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/license/status")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn license_status_contract_returns_runtime_license_summary() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/license/status")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["source"], "signed_pem");
    assert_eq!(
        payload["result"]["client_uuid"],
        json!("11111111-2222-4333-8444-555555555555")
    );
    assert_eq!(
        payload["result"]["client_email"],
        json!("tests@corrobore.dev")
    );
    assert_eq!(
        payload["result"]["valid_until"],
        json!("2099-01-01T00:00:00Z")
    );
    assert_eq!(payload["result"]["is_nfr"], json!(true));
    assert_eq!(payload["result"]["modules"], json!(["cti"]));
}

#[tokio::test]
async fn admin_license_status_contract_rejects_when_admin_token_not_configured() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/admin/license/status")
        .header(header::AUTHORIZATION, "Bearer admin-token-123")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "ADMIN_AUTH_NOT_CONFIGURED");
}

#[tokio::test]
async fn admin_license_status_contract_rejects_missing_auth_header() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("admin-license-status-missing-auth"),
        HashMap::from([(
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            "admin-token-123".to_owned(),
        )]),
    );
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/admin/license/status")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_license_status_contract_rejects_invalid_token() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("admin-license-status-invalid-auth"),
        HashMap::from([(
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            "admin-token-123".to_owned(),
        )]),
    );
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/admin/license/status")
        .header(header::AUTHORIZATION, "Bearer wrong-token")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_license_status_contract_returns_license_summary_with_admin_token() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("admin-license-status-success"),
        HashMap::from([(
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            "admin-token-123".to_owned(),
        )]),
    );
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/admin/license/status")
        .header(header::AUTHORIZATION, "Bearer admin-token-123")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["source"], "signed_pem");
    assert_eq!(
        payload["result"]["client_uuid"],
        json!("11111111-2222-4333-8444-555555555555")
    );
    assert_eq!(
        payload["result"]["client_email"],
        json!("tests@corrobore.dev")
    );
    assert_eq!(
        payload["result"]["valid_until"],
        json!("2099-01-01T00:00:00Z")
    );
    assert_eq!(payload["result"]["is_nfr"], json!(true));
    assert_eq!(payload["result"]["modules"], json!(["cti"]));
}

#[tokio::test]
async fn admin_domain_provider_status_contract_returns_non_sensitive_summary() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("admin-domain-provider-status"),
        HashMap::from([(
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            "admin-token-123".to_owned(),
        )]),
    );
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/admin/domain-providers/status")
        .header(header::AUTHORIZATION, "Bearer admin-token-123")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["providers"], json!([]));
    assert!(payload.to_string().find("library").is_none());
    assert!(payload.to_string().find("path").is_none());
}

#[tokio::test]
async fn stix_validate_contract_valid_bundle_reports_no_issues() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "source": "bundle",
                "bundle": {
                    "type": "bundle",
                    "objects": [
                        {
                            "type": "identity",
                            "id": "identity--validate-contract-1",
                            "name": "Validation Contract"
                        }
                    ]
                }
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["source_mode"], "bundle");
    assert_eq!(payload["result"]["valid"], true);
    assert!(
        payload["result"]["issues"]
            .as_array()
            .map(Vec::is_empty)
            .unwrap_or(false)
    );
    assert_eq!(payload["result"]["corrections_summary"], Value::Null);
    assert_eq!(payload["result"]["persistence"], Value::Null);
}

#[tokio::test]
async fn stix_validate_contract_rejects_missing_bundle_for_bundle_source() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "source": "bundle"
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "MISSING_BUNDLE");
}

#[cfg(feature = "enterprise-cti")]
#[tokio::test]
async fn stix_validate_contract_graph_source_requires_ready_provider() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "source": "graph" }).to_string()))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "DOMAIN_PROVIDER_NOT_READY");
}

#[cfg(feature = "enterprise-cti")]
#[tokio::test]
async fn stix_validate_contract_graph_source_rejected_without_cti_license() {
    let store_dir = unique_store_dir("graph-native-license-missing");
    let app = test_app_with_store_dir_and_extra_env(
        store_dir,
        HashMap::from([(
            "CORROBORE_HTTP_LICENSED_MODULES".to_owned(),
            "fimi".to_owned(),
        )]),
    );

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "source": "graph" }).to_string()))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "LICENSE_MODULE_MISSING");
}

#[cfg(not(feature = "enterprise-cti"))]
#[tokio::test]
async fn stix_validate_contract_graph_source_rejected_when_enterprise_cti_disabled() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "source": "graph" }).to_string()))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "FEATURE_NOT_AVAILABLE");
}

#[tokio::test]
async fn stix_validate_contract_bundle_identity_missing_name_is_flagged_and_autocorrected() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "source": "bundle",
                "bundle": {
                    "type": "bundle",
                    "objects": [
                        {
                            "type": "identity",
                            "id": "identity--validate-happy-1"
                        }
                    ]
                }
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["source_mode"], "bundle");
    // The identity is missing 'name' → structural issue reported
    let issues = payload["result"]["issues"]
        .as_array()
        .expect("issues should be array");
    assert!(
        issues
            .iter()
            .any(|i| i["code"] == "STIX_IDENTITY_NAME_REQUIRED")
    );
    // Autofix playbook applied
    assert_eq!(
        payload["result"]["playbooks_applied"][0]["id"],
        "PLAYBOOK_FIX_IDENTITY_NAME"
    );
    // Bundle was corrected and persisted
    assert_ne!(payload["result"]["persistence"], Value::Null);
    assert_eq!(payload["result"]["persistence"]["processed_objects"], 1);

    // Aggregated machine-readable correction summary for dashboards.
    assert_eq!(
        payload["result"]["corrections_summary"]["total_corrections"],
        1
    );
    assert_eq!(
        payload["result"]["corrections_summary"]["by_field"]["name"],
        1
    );
    assert_eq!(
        payload["result"]["corrections_summary"]["by_strategy"]["playbook_default"],
        1
    );
    assert_eq!(
        payload["result"]["corrections_summary"]["by_playbook_id"]["PLAYBOOK_FIX_IDENTITY_NAME"],
        1
    );
}

#[cfg(all(feature = "enterprise-cti", not(feature = "enterprise-cti-binary")))]
#[tokio::test]
async fn stix_validate_contract_graph_source_with_preloaded_nodes_validates_natively() {
    // Seed the graph with one valid ThreatActor via the write endpoint first.
    let store_dir = unique_store_dir("graph-native-validate");
    let app = test_app_with_store_dir(store_dir);

    // Import a valid ThreatActor with all required fields.
    let import_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/import/stix")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "bundle": {
                    "type": "bundle",
                    "objects": [
                        {
                            "type": "threat-actor",
                            "id": "threat-actor--graph-native-1",
                            "name": "GraphNativeActor"
                        }
                    ]
                }
            })
            .to_string(),
        ))
        .expect("import request should build");

    let import_response = app
        .clone()
        .oneshot(import_request)
        .await
        .expect("import should respond");
    assert_eq!(import_response.status(), StatusCode::OK);

    // Now validate the graph natively.
    let validate_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/stix/validate")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "source": "graph" }).to_string()))
        .expect("validate request should build");

    let validate_response = app
        .oneshot(validate_request)
        .await
        .expect("validate should respond");
    assert_eq!(validate_response.status(), StatusCode::OK);

    let body = to_bytes(validate_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["source_mode"], "graph");
    assert_eq!(payload["result"]["persistence"], Value::Null);
}

#[tokio::test]
async fn import_stix_contract_imports_bundle_with_auth() {
    let app = test_app();
    let import_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/import/stix")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "bundle": {
                    "type": "bundle",
                    "objects": [
                        {
                            "type": "identity",
                            "id": "identity--demo-import-1",
                            "name": "Imported identity"
                        }
                    ]
                }
            })
            .to_string(),
        ))
        .expect("request should build");

    let import_response = app
        .clone()
        .oneshot(import_request)
        .await
        .expect("import should respond");
    assert_eq!(import_response.status(), StatusCode::OK);

    let import_body = to_bytes(import_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let import_payload: Value =
        serde_json::from_slice(&import_body).expect("payload should be json");

    assert_eq!(import_payload["ok"], true);
    assert_eq!(import_payload["result"]["processed_objects"], 1);
    assert_eq!(import_payload["result"]["applied_mutations"], 1);

    let query_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "query": "MATCH (n:Identity {stix_id: 'identity--demo-import-1'}) RETURN n LIMIT 1"
            })
            .to_string(),
        ))
        .expect("request should build");

    let query_response = app
        .oneshot(query_request)
        .await
        .expect("query should respond");
    assert_eq!(query_response.status(), StatusCode::OK);

    let query_body = to_bytes(query_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let query_payload: Value = serde_json::from_slice(&query_body).expect("payload should be json");

    assert_eq!(query_payload["ok"], true);
    assert_eq!(query_payload["result"]["status"], "Success");
}

#[tokio::test]
async fn import_stix_file_contract_imports_json_file_with_auth() {
    let app = test_app();
    let boundary = "----corrobore-boundary-001";
    let bundle_json = r#"{"type":"bundle","objects":[{"type":"identity","id":"identity--demo-file-1","name":"Imported from file"}]}"#;
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"bundle.stix\"\r\nContent-Type: application/json\r\n\r\n{bundle_json}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"workspace_id\"\r\n\r\nworkspace--http-import-file\r\n--{boundary}--\r\n"
    );

    let import_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/import/stix/file")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .expect("request should build");

    let import_response = app
        .clone()
        .oneshot(import_request)
        .await
        .expect("import should respond");
    assert_eq!(import_response.status(), StatusCode::OK);

    let import_body = to_bytes(import_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let import_payload: Value =
        serde_json::from_slice(&import_body).expect("payload should be json");

    assert_eq!(import_payload["ok"], true);
    assert_eq!(import_payload["result"]["processed_objects"], 1);
    assert_eq!(import_payload["result"]["applied_mutations"], 1);
}

#[tokio::test]
async fn import_stix_file_contract_rejects_unsupported_extension() {
    let app = test_app();
    let boundary = "----corrobore-boundary-002";
    let bundle_json =
        r#"{"type":"bundle","objects":[{"type":"identity","id":"identity--demo-file-2"}]}"#;
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"bundle.txt\"\r\nContent-Type: text/plain\r\n\r\n{bundle_json}\r\n--{boundary}--\r\n"
    );

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/import/stix/file")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_start_contract_rejects_missing_auth_header() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-contract",
                "actor_id": "actor--http-session-contract",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn session_start_contract_returns_uuid_with_auth() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-contract",
                "actor_id": "actor--http-session-contract",
                "actor_kind": "Agent",
                "metadata": {
                    "source": "http-contract"
                }
            })
            .to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    let session_id = payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string");
    assert!(looks_like_uuid(session_id), "session_id should be uuid");
    assert_eq!(payload["result"]["status"], "idle");
}

#[tokio::test]
async fn session_health_contract_returns_not_found_for_unknown_session() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/sessions/00000000-0000-0000-0000-000000000000/health")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_health_contract_returns_status_for_created_session() {
    let app = test_app();
    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-health",
                "actor_id": "actor--http-session-health",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    let health_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/health"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let health_response = app
        .clone()
        .oneshot(health_request)
        .await
        .expect("health should respond");
    assert_eq!(health_response.status(), StatusCode::OK);

    let health_body = to_bytes(health_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let health_payload: Value =
        serde_json::from_slice(&health_body).expect("health payload should be json");

    assert_eq!(health_payload["ok"], true);
    assert_eq!(health_payload["result"]["session_id"], session_id);
    assert_eq!(health_payload["result"]["status"], "idle");
    assert_eq!(health_payload["result"]["stop_reason"], Value::Null);
}

#[tokio::test]
async fn session_health_contract_auto_stops_inactive_session_when_ttl_is_configured() {
    let store_dir = unique_store_dir("session-ttl");
    let app = test_app_with_store_dir_and_extra_env(
        store_dir.clone(),
        HashMap::from([(
            "CORROBORE_HTTP_SESSION_IDLE_TTL_MS".to_owned(),
            "1".to_owned(),
        )]),
    );

    let log_dir = store_dir.join("logs");
    fs::create_dir_all(&log_dir).expect("log dir should be created");
    fs::write(log_dir.join("http-server.session.log.jsonl"), "")
        .expect("empty log file should be created");

    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-ttl",
                "actor_id": "actor--http-session-ttl",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let health_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/health"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let health_response = app
        .clone()
        .oneshot(health_request)
        .await
        .expect("health should respond");
    assert_eq!(health_response.status(), StatusCode::OK);

    let health_body = to_bytes(health_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let health_payload: Value =
        serde_json::from_slice(&health_body).expect("health payload should be json");

    assert_eq!(health_payload["result"]["status"], "stopped");
    assert_eq!(health_payload["result"]["stop_reason"], "idle_ttl_expired");

    let logs_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/logs?limit=10"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");
    let logs_response = app
        .clone()
        .oneshot(logs_request)
        .await
        .expect("logs should respond");
    assert_eq!(logs_response.status(), StatusCode::OK);
    let logs_body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let logs_payload: Value = serde_json::from_slice(&logs_body).expect("payload should be json");
    assert_eq!(logs_payload["result"]["stop_reason"], "idle_ttl_expired");

    let service_health_request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .expect("request should build");
    let service_health_response = app
        .oneshot(service_health_request)
        .await
        .expect("service health should respond");
    assert_eq!(service_health_response.status(), StatusCode::OK);
    let service_health_body = to_bytes(service_health_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let service_health_payload: Value =
        serde_json::from_slice(&service_health_body).expect("payload should be json");
    assert_eq!(
        service_health_payload["session_ttl_metrics"]["total_expired_sessions"],
        1
    );
    assert_eq!(
        service_health_payload["session_ttl_metrics"]["expired_last_5m_sessions"],
        1
    );
}

#[tokio::test]
async fn session_health_contract_keeps_active_session_alive_with_ttl_enabled() {
    let store_dir = unique_store_dir("session-ttl-active");
    // Use a generous idle TTL so the assertion under test ("a recently-used
    // session stays idle, not expired") is decided by activity, not by wall-clock
    // scheduling jitter. The two short sleeps below (~60ms total) stay far under
    // this bound even when the process runs under slow, coverage-instrumented
    // builds, keeping the test deterministic.
    let app = test_app_with_store_dir_and_extra_env(
        store_dir,
        HashMap::from([(
            "CORROBORE_HTTP_SESSION_IDLE_TTL_MS".to_owned(),
            "5000".to_owned(),
        )]),
    );

    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-ttl-active",
                "actor_id": "actor--http-session-ttl-active",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    let read_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "query": "MATCH (n) RETURN n LIMIT 1",
                "session_id": session_id.clone(),
                "workspace_id": "workspace--http-session-ttl-active"
            })
            .to_string(),
        ))
        .expect("request should build");

    let read_response = app
        .clone()
        .oneshot(read_request)
        .await
        .expect("read should respond");
    assert_eq!(read_response.status(), StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    let health_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/health"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");
    let health_response = app
        .oneshot(health_request)
        .await
        .expect("health should respond");
    assert_eq!(health_response.status(), StatusCode::OK);

    let health_body = to_bytes(health_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let health_payload: Value =
        serde_json::from_slice(&health_body).expect("payload should be json");
    assert_eq!(health_payload["result"]["status"], "idle");
    assert_eq!(health_payload["result"]["stop_reason"], Value::Null);
}

#[tokio::test]
async fn session_persistence_contract_survives_app_restart() {
    let store_dir = unique_store_dir("restart");

    let first_app = test_app_with_store_dir(store_dir.clone());
    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-restart",
                "actor_id": "actor--http-session-restart",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = first_app
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    let restarted_app = test_app_with_store_dir(store_dir);
    let health_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/health"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let health_response = restarted_app
        .oneshot(health_request)
        .await
        .expect("health should respond");
    assert_eq!(health_response.status(), StatusCode::OK);

    let health_body = to_bytes(health_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let health_payload: Value =
        serde_json::from_slice(&health_body).expect("health payload should be json");

    assert_eq!(health_payload["result"]["session_id"], session_id);
    assert_eq!(health_payload["result"]["status"], "idle");
}

#[tokio::test]
async fn session_stop_contract_sets_stopped_and_persists_after_restart() {
    let store_dir = unique_store_dir("stop");
    let app = test_app_with_store_dir(store_dir.clone());

    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-stop",
                "actor_id": "actor--http-session-stop",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    let stop_request = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/sessions/{session_id}/stop"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let stop_response = app
        .clone()
        .oneshot(stop_request)
        .await
        .expect("stop should respond");
    assert_eq!(stop_response.status(), StatusCode::OK);

    let stop_body = to_bytes(stop_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let stop_payload: Value =
        serde_json::from_slice(&stop_body).expect("stop payload should be json");

    assert_eq!(stop_payload["ok"], true);
    assert_eq!(stop_payload["result"]["session_id"], session_id);
    assert_eq!(stop_payload["result"]["status"], "stopped");

    let restarted_app = test_app_with_store_dir(store_dir);
    let health_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/health"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let health_response = restarted_app
        .oneshot(health_request)
        .await
        .expect("health should respond");
    assert_eq!(health_response.status(), StatusCode::OK);

    let health_body = to_bytes(health_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let health_payload: Value =
        serde_json::from_slice(&health_body).expect("health payload should be json");

    assert_eq!(health_payload["result"]["session_id"], session_id);
    assert_eq!(health_payload["result"]["status"], "stopped");
}

#[tokio::test]
async fn session_logs_contract_returns_entries_for_requested_session() {
    let store_dir = unique_store_dir("session-logs");
    let app = test_app_with_store_dir(store_dir.clone());

    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-session-logs",
                "actor_id": "actor--http-session-logs",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    let log_dir = store_dir.join("logs");
    fs::create_dir_all(&log_dir).expect("log dir should be created");
    let log_file = log_dir.join("http-server.session.log.jsonl");
    let content = format!(
        "{{\"timestamp\":\"2026-07-11T17:00:00Z\",\"fields\":{{\"message\":\"session started\",\"session_id\":\"{}\"}}}}\n{{\"timestamp\":\"2026-07-11T17:00:01Z\",\"fields\":{{\"message\":\"session started\",\"session_id\":\"session--other\"}}}}\n{{\"timestamp\":\"2026-07-11T17:00:02Z\",\"fields\":{{\"message\":\"session stopped\",\"session_id\":\"{}\"}}}}\n",
        session_id, session_id
    );
    fs::write(&log_file, content).expect("log file should be written");

    let from_ms = 0_u64;
    let to_ms = 9_999_999_999_999_u64;
    let logs_request = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/v1/sessions/{session_id}/logs?limit=1&from_ms={from_ms}&to_ms={to_ms}"
        ))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let logs_response = app
        .clone()
        .oneshot(logs_request)
        .await
        .expect("logs should respond");
    assert_eq!(logs_response.status(), StatusCode::OK);

    let logs_body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let logs_payload: Value = serde_json::from_slice(&logs_body).expect("payload should be json");

    assert_eq!(logs_payload["ok"], true);
    assert_eq!(logs_payload["result"]["session_id"], session_id);
    assert_eq!(logs_payload["result"]["matched_entries"], 1);
    let entries = logs_payload["result"]["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].to_string().contains(&session_id));

    let ndjson_request = Request::builder()
        .method(Method::GET)
        .uri(format!(
            "/v1/sessions/{session_id}/logs?format=ndjson&from_ms={from_ms}&to_ms={to_ms}"
        ))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let ndjson_response = app
        .oneshot(ndjson_request)
        .await
        .expect("ndjson logs should respond");
    assert_eq!(ndjson_response.status(), StatusCode::OK);
    assert_eq!(
        ndjson_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/x-ndjson")
    );

    let ndjson_body = to_bytes(ndjson_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let ndjson_text = String::from_utf8(ndjson_body.to_vec()).expect("body should be utf8");
    assert!(ndjson_text.contains(&session_id));
}

#[tokio::test]
async fn session_logs_contract_contains_cypher_audit_input_and_output() {
    let store_dir = unique_store_dir("cypher-audit");
    let app = test_app_with_store_dir(store_dir.clone());

    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-cypher-audit",
                "actor_id": "actor--http-cypher-audit",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    let log_dir = store_dir.join("logs");
    fs::create_dir_all(&log_dir).expect("log dir should be created");
    let log_file = log_dir.join("http-server.session.log.jsonl");
    let log_content = format!(
        "{{\"timestamp\":\"2026-07-11T18:00:00Z\",\"level\":\"INFO\",\"fields\":{{\"event\":\"cypher_audit_input\",\"session_id\":\"{}\",\"query\":\"MATCH (n) RETURN n LIMIT 1\"}}}}\n{{\"timestamp\":\"2026-07-11T18:00:01Z\",\"level\":\"INFO\",\"fields\":{{\"event\":\"cypher_audit_output\",\"session_id\":\"{}\",\"status\":\"Success\"}}}}\n",
        session_id, session_id
    );
    fs::write(&log_file, log_content).expect("log file should be written");

    let logs_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/logs?limit=200"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let logs_response = app
        .oneshot(logs_request)
        .await
        .expect("logs should respond");
    assert_eq!(logs_response.status(), StatusCode::OK);

    let logs_body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let logs_payload: Value = serde_json::from_slice(&logs_body).expect("payload should be json");

    let entries = logs_payload["result"]["entries"]
        .as_array()
        .expect("entries should be an array");

    let has_audit_input = entries
        .iter()
        .any(|entry| entry.to_string().contains("cypher_audit_input"));
    let has_audit_output = entries
        .iter()
        .any(|entry| entry.to_string().contains("cypher_audit_output"));

    assert!(
        has_audit_input,
        "session logs should contain cypher audit input event"
    );
    assert!(
        has_audit_output,
        "session logs should contain cypher audit output event"
    );

    let audit_parity = &logs_payload["result"]["audit_parity"];
    assert_eq!(audit_parity["parity_ok"], true);
    assert_eq!(audit_parity["input_events"], 1);
    assert_eq!(audit_parity["output_events"], 1);
    assert_eq!(audit_parity["missing_output_event_ids"], json!([]));
    assert_eq!(audit_parity["orphan_output_event_ids"], json!([]));
}

#[tokio::test]
async fn session_logs_contract_reports_audit_parity_mismatch() {
    let store_dir = unique_store_dir("cypher-audit-parity-mismatch");
    let app = test_app_with_store_dir(store_dir.clone());

    let start_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/sessions/start")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "workspace_id": "workspace--http-cypher-audit-mismatch",
                "actor_id": "actor--http-cypher-audit-mismatch",
                "actor_kind": "Agent"
            })
            .to_string(),
        ))
        .expect("request should build");

    let start_response = app
        .clone()
        .oneshot(start_request)
        .await
        .expect("start should respond");
    assert_eq!(start_response.status(), StatusCode::OK);

    let start_body = to_bytes(start_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let start_payload: Value = serde_json::from_slice(&start_body).expect("payload should be json");
    let session_id = start_payload["result"]["session_id"]
        .as_str()
        .expect("session_id should be a string")
        .to_owned();

    let log_dir = store_dir.join("logs");
    fs::create_dir_all(&log_dir).expect("log dir should be created");
    let log_file = log_dir.join("http-server.session.log.jsonl");
    let log_content = format!(
        "{{\"timestamp\":\"2026-07-11T18:00:00Z\",\"level\":\"INFO\",\"fields\":{{\"event\":\"cypher_audit_input\",\"audit_event_id\":\"audit-1\",\"session_id\":\"{}\"}}}}\n{{\"timestamp\":\"2026-07-11T18:00:01Z\",\"level\":\"INFO\",\"fields\":{{\"event\":\"cypher_audit_output\",\"audit_event_id\":\"audit-1\",\"session_id\":\"{}\"}}}}\n{{\"timestamp\":\"2026-07-11T18:00:02Z\",\"level\":\"INFO\",\"fields\":{{\"event\":\"cypher_audit_input\",\"audit_event_id\":\"audit-2\",\"session_id\":\"{}\"}}}}\n",
        session_id, session_id, session_id
    );
    fs::write(&log_file, log_content).expect("log file should be written");

    let logs_request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/sessions/{session_id}/logs?limit=200"))
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build");

    let logs_response = app
        .oneshot(logs_request)
        .await
        .expect("logs should respond");
    assert_eq!(logs_response.status(), StatusCode::OK);

    let logs_body = to_bytes(logs_response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let logs_payload: Value = serde_json::from_slice(&logs_body).expect("payload should be json");

    assert_eq!(logs_payload["result"]["matched_entries"], 3);
    assert_eq!(logs_payload["result"]["total_matched_entries"], 3);
    assert_eq!(logs_payload["result"]["audit_parity"]["parity_ok"], false);
    assert_eq!(logs_payload["result"]["audit_parity"]["input_events"], 2);
    assert_eq!(logs_payload["result"]["audit_parity"]["output_events"], 1);
    assert_eq!(
        logs_payload["result"]["audit_parity"]["missing_output_event_ids"],
        json!(["audit-2"])
    );
    assert_eq!(
        logs_payload["result"]["audit_parity"]["orphan_output_event_ids"],
        json!([])
    );
}

#[tokio::test]
async fn seed_search_contract_rejects_missing_auth_header() {
    let app = test_app();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "phishing campaign"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn seed_search_contract_returns_ranked_candidates_with_explanations() {
    let app = test_app();

    let write_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/write")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"query": "CREATE (n:Campaign {name: 'acme phishing campaign'})"}).to_string(),
        ))
        .expect("request should build");

    let write_response = app
        .clone()
        .oneshot(write_request)
        .await
        .expect("write should respond");
    assert_eq!(write_response.status(), StatusCode::OK);

    let search_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "phishing campaign", "top_k": 5}).to_string(),
        ))
        .expect("request should build");

    let response = app
        .oneshot(search_request)
        .await
        .expect("seed search should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], true);
    let candidates = payload["result"]["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert_eq!(candidates.len(), 1);
    assert!(
        candidates[0]["node_id"]
            .as_str()
            .is_some_and(|node_id| !node_id.is_empty())
    );
    assert!(
        candidates[0]["score"]
            .as_f64()
            .is_some_and(|score| score > 0.0)
    );
    assert!(
        candidates[0]["explanation"]["rationale"]
            .as_str()
            .is_some_and(|rationale| rationale.contains("matched terms"))
    );
}

#[tokio::test]
async fn seed_search_contract_maps_no_seed_to_unprocessable() {
    let app = test_app();

    let search_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "phishing campaign"}).to_string(),
        ))
        .expect("request should build");

    let response = app
        .oneshot(search_request)
        .await
        .expect("seed search should respond");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");

    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "NO_SEED");
}

#[tokio::test]
async fn seed_search_contract_rejects_unknown_mode() {
    let app = test_app();

    let search_request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "phishing campaign", "mode": "quantum"}).to_string(),
        ))
        .expect("request should build");

    let response = app
        .oneshot(search_request)
        .await
        .expect("seed search should respond");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "INVALID_RETRIEVAL_MODE");
}

#[cfg(feature = "enterprise-fimi")]
#[tokio::test]
async fn seed_search_contract_rejects_fimi_profile_without_license() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("seed-fimi-license-missing"),
        HashMap::from([(
            "CORROBORE_HTTP_LICENSED_MODULES".to_owned(),
            "cti,crisis".to_owned(),
        )]),
    );

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "coordinated messaging", "domain_profile": "fimi"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "LICENSE_MODULE_MISSING");
}

#[cfg(feature = "enterprise-fimi")]
#[tokio::test]
async fn seed_search_contract_rejects_fimi_profile_without_provider() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("seed-fimi-provider-missing"),
        HashMap::from([(
            "CORROBORE_HTTP_LICENSED_MODULES".to_owned(),
            "fimi".to_owned(),
        )]),
    );

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "coordinated messaging", "domain_profile": "fimi"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "DOMAIN_PROVIDER_NOT_READY");
}

#[cfg(not(feature = "enterprise-fimi"))]
#[tokio::test]
async fn seed_search_contract_rejects_fimi_profile_when_feature_disabled() {
    let app = test_app();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "coordinated messaging", "domain_profile": "fimi"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "FEATURE_NOT_AVAILABLE");
}

#[cfg(feature = "enterprise-crisis")]
#[tokio::test]
async fn seed_search_contract_rejects_crisis_profile_without_license() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("seed-crisis-license-missing"),
        HashMap::from([(
            "CORROBORE_HTTP_LICENSED_MODULES".to_owned(),
            "cti,fimi".to_owned(),
        )]),
    );

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "humanitarian needs", "domain_profile": "crisis"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "LICENSE_MODULE_MISSING");
}

#[cfg(not(feature = "enterprise-crisis"))]
#[tokio::test]
async fn seed_search_contract_rejects_crisis_profile_when_feature_disabled() {
    let app = test_app();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "humanitarian needs", "domain_profile": "crisis"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "FEATURE_NOT_AVAILABLE");
}

#[cfg(feature = "enterprise-cti")]
#[tokio::test]
async fn seed_search_contract_rejects_cti_profile_without_license() {
    let app = test_app_with_store_dir_and_extra_env(
        unique_store_dir("seed-cti-license-missing"),
        HashMap::from([(
            "CORROBORE_HTTP_LICENSED_MODULES".to_owned(),
            "fimi,crisis".to_owned(),
        )]),
    );

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "phishing campaign", "domain_profile": "cti"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "LICENSE_MODULE_MISSING");
}

#[cfg(not(feature = "enterprise-cti"))]
#[tokio::test]
async fn seed_search_contract_rejects_cti_profile_when_feature_disabled() {
    let app = test_app();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/seed/search")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"objective": "phishing campaign", "domain_profile": "cti"}).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("payload should be json");
    assert_eq!(payload["error"]["code"], "FEATURE_NOT_AVAILABLE");
}

#[tokio::test]
async fn auth_contract_rejects_invalid_token() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer wrong-token-000")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "query": "MATCH (n) RETURN n LIMIT 1", "mode": "read" }).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn body_limit_contract_rejects_oversized_cypher_body() {
    // 2.3: a small standard body limit must reject an oversized payload with 413.
    let extra_env = HashMap::from([("CORROBORE_HTTP_MAX_BODY_BYTES".to_owned(), "16".to_owned())]);
    let app = test_app_with_store_dir_and_extra_env(unique_store_dir("bodylimit"), extra_env);

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer token-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "query": "MATCH (n) RETURN n LIMIT 1", "mode": "read" }).to_string(),
        ))
        .expect("request should build");

    let response = app.oneshot(request).await.expect("request should respond");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn rate_limit_contract_returns_429_when_burst_exceeded() {
    // 2.3: a burst of 1 means the second immediate request is throttled.
    let extra_env = HashMap::from([
        (
            "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND".to_owned(),
            "1".to_owned(),
        ),
        ("CORROBORE_HTTP_RATE_LIMIT_BURST".to_owned(), "1".to_owned()),
    ]);
    let app = test_app_with_store_dir_and_extra_env(unique_store_dir("ratelimit"), extra_env);

    let build_request = || {
        Request::builder()
            .method(Method::POST)
            .uri("/v1/cypher/read")
            .header(header::AUTHORIZATION, "Bearer token-123")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "query": "MATCH (n) RETURN n LIMIT 1", "mode": "read" }).to_string(),
            ))
            .expect("request should build")
    };

    let first = app
        .clone()
        .oneshot(build_request())
        .await
        .expect("first request should respond");
    assert_eq!(first.status(), StatusCode::OK);

    let second = app
        .oneshot(build_request())
        .await
        .expect("second request should respond");
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}

// Issue #239 (§7 observability): the `/metrics` endpoint completes the
// observability story by exposing the existing session-expiration health
// metrics in the Prometheus text exposition format. Intent validated here:
// - the endpoint is scrape-friendly and unauthenticated (like `/health`), so a
//   Prometheus server can reach it without a bearer token;
// - it responds `200 OK` with a `text/plain` content type carrying the
//   Prometheus exposition version;
// - the body follows the exposition format (`# HELP` / `# TYPE` lines) and
//   surfaces the uptime, build info, and session-expiration counters.
#[tokio::test]
async fn metrics_contract_exposes_prometheus_exposition_without_auth() {
    let app = test_app();
    let request = Request::builder()
        .method(Method::GET)
        .uri("/metrics")
        .body(Body::empty())
        .expect("request should build");

    let response = app.oneshot(request).await.expect("metrics should respond");
    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .expect("metrics response should carry a content type")
        .to_owned();
    assert!(
        content_type.starts_with("text/plain"),
        "metrics content type should be text/plain, got {content_type}"
    );
    assert!(
        content_type.contains("version=0.0.4"),
        "metrics content type should advertise the Prometheus exposition version, got {content_type}"
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    let exposition = String::from_utf8(body.to_vec()).expect("metrics body should be utf-8");

    assert!(
        exposition.contains("# HELP corrobore_uptime_seconds"),
        "exposition should document uptime, got:\n{exposition}"
    );
    assert!(
        exposition.contains("# TYPE corrobore_uptime_seconds gauge"),
        "exposition should type uptime as a gauge, got:\n{exposition}"
    );
    assert!(
        exposition.contains("# TYPE corrobore_sessions_expired_total counter"),
        "exposition should type expired sessions as a counter, got:\n{exposition}"
    );
    assert!(
        exposition.contains("corrobore_sessions_expired_total 0"),
        "a fresh server should report zero expired sessions, got:\n{exposition}"
    );
    assert!(
        exposition.contains("corrobore_sessions_expired_last_5m 0"),
        "a fresh server should report zero recent expirations, got:\n{exposition}"
    );
    assert!(
        exposition.contains(&format!(
            "corrobore_build_info{{version=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION")
        )),
        "exposition should expose build info with the crate version, got:\n{exposition}"
    );
    for expected in [
        "# TYPE corrobore_storage_mode gauge",
        "corrobore_storage_mode{mode=\"ephemeral\"} 1",
        "# TYPE corrobore_storage_wal_bytes gauge",
        "# TYPE corrobore_storage_wal_lag_sequences gauge",
        "# TYPE corrobore_storage_checkpoint_age_seconds gauge",
        "# TYPE corrobore_storage_compaction_backlog_bytes gauge",
        "# TYPE corrobore_storage_recovery_warning_count gauge",
        "# TYPE corrobore_domain_providers_configured gauge",
        "corrobore_domain_providers_configured 0",
        "# TYPE corrobore_domain_providers_ready gauge",
        "corrobore_domain_providers_ready 0",
    ] {
        assert!(
            exposition.contains(expected),
            "exposition should include {expected}, got:\n{exposition}"
        );
    }
}
