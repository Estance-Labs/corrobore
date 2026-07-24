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
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
};
use corrobore_ingest::{
    CorroboreImportClient, CursorStore, IngestConfig, IngestConfigError, TaxiiAuth, run_poll_cycle,
};
use opencti_adapter::{
    MutationClass, OpenCtiMutation, OpenCtiSyncBatch, OperationStatus, SyncPhase,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn unique_state_dir(suffix: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();

    let path: PathBuf =
        std::env::temp_dir().join(format!("corrobore-ingest-tests-{}-{}", suffix, millis));
    path.display().to_string()
}

fn base_config_map() -> HashMap<String, String> {
    let mut vars = HashMap::new();
    vars.insert(
        "CORROBORE_INGEST_TAXII_ROOT_URL".to_owned(),
        "http://taxii.example".to_owned(),
    );
    vars.insert(
        "CORROBORE_INGEST_TAXII_COLLECTION_ID".to_owned(),
        "collection-1".to_owned(),
    );
    vars.insert(
        "CORROBORE_INGEST_CORROBORE_BASE_URL".to_owned(),
        "http://corrobore.example".to_owned(),
    );
    vars.insert(
        "CORROBORE_INGEST_CORROBORE_AUTH_TOKEN".to_owned(),
        "corrobore-token".to_owned(),
    );
    vars
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
}

#[derive(Clone, Default)]
struct TaxiiMockState {
    pages: Arc<Vec<(Value, Option<String>)>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

async fn taxii_mock_handler(
    State(state): State<TaxiiMockState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let recorded_headers = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();

    let page_index = {
        let mut requests = state.requests.lock().expect("taxii mock lock");
        requests.push(RecordedRequest {
            query,
            headers: recorded_headers,
        });
        requests.len() - 1
    };

    let (body, date_added_last) = state
        .pages
        .get(page_index)
        .cloned()
        .unwrap_or_else(|| (json!({"objects": [], "more": false}), None));

    let mut response_headers = HeaderMap::new();
    if let Some(value) = date_added_last {
        response_headers.insert(
            "X-TAXII-Date-Added-Last",
            value.parse().expect("header value should parse"),
        );
    }

    (response_headers, Json(body))
}

/// Starts a mock TAXII server serving `pages` in request order.
async fn spawn_mock_taxii(pages: Vec<(Value, Option<String>)>) -> (SocketAddr, TaxiiMockState) {
    let state = TaxiiMockState {
        pages: Arc::new(pages),
        requests: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route(
            "/collections/{collection_id}/objects",
            get(taxii_mock_handler),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock taxii should bind");
    let addr = listener
        .local_addr()
        .expect("mock taxii should expose addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock taxii serve");
    });

    (addr, state)
}

type RecordedImport = (Value, HashMap<String, String>);

#[derive(Clone)]
struct CorroboreMockState {
    response: Arc<Value>,
    imports: Arc<Mutex<Vec<RecordedImport>>>,
}

async fn corrobore_mock_handler(
    State(state): State<CorroboreMockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let recorded_headers = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();

    state
        .imports
        .lock()
        .expect("corrobore mock lock")
        .push((body, recorded_headers));

    Json(state.response.as_ref().clone())
}

/// Starts a mock Corrobore server answering `POST /v1/import/stix` with `response`.
async fn spawn_mock_corrobore(response: Value) -> (SocketAddr, CorroboreMockState) {
    let state = CorroboreMockState {
        response: Arc::new(response),
        imports: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/v1/import/stix", post(corrobore_mock_handler))
        .route("/v1/opencti/sync/batches", post(corrobore_mock_handler))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock corrobore should bind");
    let addr = listener
        .local_addr()
        .expect("mock corrobore should expose addr");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock corrobore serve");
    });

    (addr, state)
}

fn import_success_response(processed: usize) -> Value {
    json!({
        "ok": true,
        "result": {
            "processed_objects": processed,
            "applied_mutations": processed,
            "rejected_mutations": 0,
            "errors": []
        }
    })
}

#[tokio::test]
async fn opencti_sync_client_preserves_batch_boundary_auth_and_checkpoint_response() {
    let (addr, state) = spawn_mock_corrobore(json!({
        "ok": true,
        "result": {
            "batch": {
                "transaction_id": "tx--opencti-sync-test",
                "operations": [{
                    "operation_id": "operation--1",
                    "sequence": 1,
                    "status": "applied",
                    "diagnostic": null
                }],
                "acknowledged_sequence": 1,
                "queue_depth": 0
            },
            "checkpoint": {
                "source_id": "opencti--connector",
                "snapshot_id": "snapshot--connector",
                "phase": "catch_up",
                "last_acknowledged_sequence": 1,
                "high_water_mark": 1,
                "queue_depth": 0,
                "retry_count": 0,
                "rejected_operations": 0,
                "quarantined_operations": 0,
                "replay_identities": [],
                "dead_letters": []
            },
            "validation": null
        }
    }))
    .await;
    let client = CorroboreImportClient::new(
        format!("http://{addr}"),
        "corrobore-token",
        "workspace--unused",
    );
    let batch = OpenCtiSyncBatch::new(
        "opencti--connector",
        "snapshot--connector",
        SyncPhase::Snapshot,
        1,
        true,
        vec![
            OpenCtiMutation::new(
                "operation--1",
                1,
                MutationClass::Upsert,
                json!({"id": "indicator--1", "type": "indicator"}),
            )
            .expect("mutation should be valid"),
        ],
    )
    .expect("batch should be valid");

    let response = client
        .synchronize_opencti(batch, None)
        .await
        .expect("synchronization should succeed");
    assert_eq!(
        response.batch.operations[0].status,
        OperationStatus::Applied
    );
    assert_eq!(response.checkpoint.last_acknowledged_sequence, 1);

    let imports = state.imports.lock().expect("corrobore mock lock");
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].0["batch"]["high_water_mark"], 1);
    assert_eq!(
        imports[0].1.get("authorization").map(String::as_str),
        Some("Bearer corrobore-token")
    );
}

fn config_for(taxii_addr: SocketAddr, corrobore_addr: SocketAddr, state_dir: &str) -> IngestConfig {
    let mut vars = base_config_map();
    vars.insert(
        "CORROBORE_INGEST_TAXII_ROOT_URL".to_owned(),
        format!("http://{taxii_addr}"),
    );
    vars.insert(
        "CORROBORE_INGEST_CORROBORE_BASE_URL".to_owned(),
        format!("http://{corrobore_addr}"),
    );
    vars.insert(
        "CORROBORE_INGEST_STATE_DIR".to_owned(),
        state_dir.to_owned(),
    );

    IngestConfig::from_map(&vars).expect("test config should be valid")
}

// ---------------------------------------------------------------------------
// Config contract
// ---------------------------------------------------------------------------

#[test]
fn ingest_config_contract_loads_defaults_and_required_values() {
    let config = IngestConfig::from_map(&base_config_map()).expect("config should load");

    assert_eq!(config.taxii_root_url, "http://taxii.example");
    assert_eq!(config.taxii_collection_id, "collection-1");
    assert_eq!(config.corrobore_base_url, "http://corrobore.example");
    assert_eq!(config.corrobore_auth_token, "corrobore-token");
    assert!(matches!(config.taxii_auth, TaxiiAuth::None));
    assert_eq!(config.workspace_id, "workspace--ingest-taxii");
    assert_eq!(config.poll_interval_ms, 300_000);
    assert_eq!(config.page_limit, 100);
    assert_eq!(config.state_dir, ".corrobore-runtime/ingest");
}

#[test]
fn ingest_config_contract_rejects_missing_taxii_root_url() {
    let mut vars = base_config_map();
    vars.remove("CORROBORE_INGEST_TAXII_ROOT_URL");

    let error = IngestConfig::from_map(&vars).expect_err("missing root url must be rejected");
    assert_eq!(
        error,
        IngestConfigError::MissingEnv("CORROBORE_INGEST_TAXII_ROOT_URL")
    );
}

#[test]
fn ingest_config_contract_rejects_blank_corrobore_token() {
    let mut vars = base_config_map();
    vars.insert(
        "CORROBORE_INGEST_CORROBORE_AUTH_TOKEN".to_owned(),
        "   ".to_owned(),
    );

    let error = IngestConfig::from_map(&vars).expect_err("blank token must be rejected");
    assert!(matches!(
        error,
        IngestConfigError::InvalidValue {
            name: "CORROBORE_INGEST_CORROBORE_AUTH_TOKEN",
            ..
        }
    ));
}

#[test]
fn ingest_config_contract_parses_bearer_auth() {
    let mut vars = base_config_map();
    vars.insert(
        "CORROBORE_INGEST_TAXII_TOKEN".to_owned(),
        "taxii-token".to_owned(),
    );

    let config = IngestConfig::from_map(&vars).expect("config should load");
    assert!(matches!(config.taxii_auth, TaxiiAuth::Bearer(token) if token == "taxii-token"));
}

#[test]
fn ingest_config_contract_parses_basic_auth() {
    let mut vars = base_config_map();
    vars.insert(
        "CORROBORE_INGEST_TAXII_USERNAME".to_owned(),
        "user".to_owned(),
    );
    vars.insert(
        "CORROBORE_INGEST_TAXII_PASSWORD".to_owned(),
        "pass".to_owned(),
    );

    let config = IngestConfig::from_map(&vars).expect("config should load");
    assert!(matches!(
        config.taxii_auth,
        TaxiiAuth::Basic { username, password } if username == "user" && password == "pass"
    ));
}

#[test]
fn ingest_config_contract_rejects_ambiguous_taxii_auth() {
    let mut vars = base_config_map();
    vars.insert(
        "CORROBORE_INGEST_TAXII_TOKEN".to_owned(),
        "taxii-token".to_owned(),
    );
    vars.insert(
        "CORROBORE_INGEST_TAXII_USERNAME".to_owned(),
        "user".to_owned(),
    );
    vars.insert(
        "CORROBORE_INGEST_TAXII_PASSWORD".to_owned(),
        "pass".to_owned(),
    );

    let error = IngestConfig::from_map(&vars).expect_err("ambiguous auth must be rejected");
    assert!(matches!(
        error,
        IngestConfigError::InvalidValue {
            name: "CORROBORE_INGEST_TAXII_TOKEN",
            ..
        }
    ));
}

// ---------------------------------------------------------------------------
// Cursor store contract
// ---------------------------------------------------------------------------

#[test]
fn cursor_store_round_trips_cursor_per_collection() {
    let state_dir = unique_state_dir("cursor-roundtrip");

    let mut store = CursorStore::new(&state_dir);
    store
        .save_cursor("collection-1", "2026-07-15T10:00:00.000Z")
        .expect("cursor should persist");

    let reloaded = CursorStore::new(&state_dir);
    assert_eq!(
        reloaded
            .load_cursor("collection-1")
            .expect("cursor should load"),
        Some("2026-07-15T10:00:00.000Z".to_owned())
    );
}

#[test]
fn cursor_store_returns_none_for_unknown_collection() {
    let state_dir = unique_state_dir("cursor-unknown");

    let store = CursorStore::new(&state_dir);
    assert_eq!(
        store
            .load_cursor("collection-unknown")
            .expect("load should succeed"),
        None
    );
}

// ---------------------------------------------------------------------------
// Poll cycle contract
// ---------------------------------------------------------------------------

#[tokio::test]
async fn poll_cycle_imports_paginated_objects_and_advances_cursor() {
    let (taxii_addr, taxii_state) = spawn_mock_taxii(vec![
        (
            json!({
                "objects": [
                    {"type": "indicator", "id": "indicator--1", "name": "one"},
                    {"type": "indicator", "id": "indicator--2", "name": "two"}
                ],
                "more": true,
                "next": "cursor-page-2"
            }),
            Some("2026-07-15T10:00:00.000Z".to_owned()),
        ),
        (
            json!({
                "objects": [
                    {"type": "malware", "id": "malware--3", "name": "three"}
                ],
                "more": false
            }),
            Some("2026-07-15T11:00:00.000Z".to_owned()),
        ),
    ])
    .await;
    let (corrobore_addr, corrobore_state) = spawn_mock_corrobore(import_success_response(3)).await;

    let state_dir = unique_state_dir("paginated");
    let config = config_for(taxii_addr, corrobore_addr, &state_dir);
    let mut store = CursorStore::new(&state_dir);

    let outcome = run_poll_cycle(&config, &mut store)
        .await
        .expect("poll cycle should succeed");

    assert_eq!(outcome.fetched_objects, 3);
    let summary = outcome.import.expect("import should have run");
    assert_eq!(summary.processed_objects, 3);
    assert_eq!(summary.applied_mutations, 3);
    assert_eq!(summary.rejected_mutations, 0);
    assert_eq!(
        outcome.cursor,
        Some("2026-07-15T11:00:00.000Z".to_owned()),
        "cursor should advance to the last page's X-TAXII-Date-Added-Last"
    );

    // The TAXII requests: first page without pagination cursor, second with it.
    let requests = taxii_state.requests.lock().expect("taxii mock lock");
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].query.contains_key("added_after"));
    assert!(!requests[0].query.contains_key("next"));
    assert_eq!(
        requests[0].query.get("limit").map(String::as_str),
        Some("100")
    );
    assert_eq!(
        requests[1].query.get("next").map(String::as_str),
        Some("cursor-page-2")
    );
    assert!(
        requests[0]
            .headers
            .get("accept")
            .is_some_and(|accept| accept.contains("application/taxii+json")),
        "TAXII media type must be requested"
    );

    // One bundle import carrying all paged objects, with bearer auth.
    let imports = corrobore_state.imports.lock().expect("corrobore mock lock");
    assert_eq!(imports.len(), 1);
    let (body, headers) = &imports[0];
    assert_eq!(body["bundle"]["type"], "bundle");
    assert_eq!(
        body["bundle"]["objects"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        3
    );
    assert_eq!(body["workspace_id"], "workspace--ingest-taxii");
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer corrobore-token")
    );

    // Cursor must survive a restart.
    let reloaded = CursorStore::new(&state_dir);
    assert_eq!(
        reloaded
            .load_cursor("collection-1")
            .expect("cursor should load"),
        Some("2026-07-15T11:00:00.000Z".to_owned())
    );
}

#[tokio::test]
async fn poll_cycle_sends_added_after_from_persisted_cursor() {
    let (taxii_addr, taxii_state) =
        spawn_mock_taxii(vec![(json!({"objects": [], "more": false}), None)]).await;
    let (corrobore_addr, _corrobore_state) = spawn_mock_corrobore(import_success_response(0)).await;

    let state_dir = unique_state_dir("added-after");
    let config = config_for(taxii_addr, corrobore_addr, &state_dir);
    let mut store = CursorStore::new(&state_dir);
    store
        .save_cursor("collection-1", "2026-07-14T09:00:00.000Z")
        .expect("cursor should persist");

    run_poll_cycle(&config, &mut store)
        .await
        .expect("poll cycle should succeed");

    let requests = taxii_state.requests.lock().expect("taxii mock lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].query.get("added_after").map(String::as_str),
        Some("2026-07-14T09:00:00.000Z")
    );
}

#[tokio::test]
async fn poll_cycle_with_empty_envelope_skips_import() {
    let (taxii_addr, _taxii_state) =
        spawn_mock_taxii(vec![(json!({"objects": [], "more": false}), None)]).await;
    let (corrobore_addr, corrobore_state) = spawn_mock_corrobore(import_success_response(0)).await;

    let state_dir = unique_state_dir("empty");
    let config = config_for(taxii_addr, corrobore_addr, &state_dir);
    let mut store = CursorStore::new(&state_dir);

    let outcome = run_poll_cycle(&config, &mut store)
        .await
        .expect("poll cycle should succeed");

    assert_eq!(outcome.fetched_objects, 0);
    assert!(outcome.import.is_none(), "empty cycles must not import");
    assert_eq!(outcome.cursor, None);
    assert!(
        corrobore_state
            .imports
            .lock()
            .expect("corrobore mock lock")
            .is_empty(),
        "no import request must reach Corrobore"
    );
}

#[tokio::test]
async fn poll_cycle_sends_basic_auth_to_taxii() {
    let (taxii_addr, taxii_state) =
        spawn_mock_taxii(vec![(json!({"objects": [], "more": false}), None)]).await;
    let (corrobore_addr, _corrobore_state) = spawn_mock_corrobore(import_success_response(0)).await;

    let state_dir = unique_state_dir("basic-auth");
    let mut vars = base_config_map();
    vars.insert(
        "CORROBORE_INGEST_TAXII_ROOT_URL".to_owned(),
        format!("http://{taxii_addr}"),
    );
    vars.insert(
        "CORROBORE_INGEST_CORROBORE_BASE_URL".to_owned(),
        format!("http://{corrobore_addr}"),
    );
    vars.insert("CORROBORE_INGEST_STATE_DIR".to_owned(), state_dir.clone());
    vars.insert(
        "CORROBORE_INGEST_TAXII_USERNAME".to_owned(),
        "user".to_owned(),
    );
    vars.insert(
        "CORROBORE_INGEST_TAXII_PASSWORD".to_owned(),
        "pass".to_owned(),
    );
    let config = IngestConfig::from_map(&vars).expect("config should load");
    let mut store = CursorStore::new(&state_dir);

    run_poll_cycle(&config, &mut store)
        .await
        .expect("poll cycle should succeed");

    let requests = taxii_state.requests.lock().expect("taxii mock lock");
    assert!(
        requests[0]
            .headers
            .get("authorization")
            .is_some_and(|value| value.starts_with("Basic ")),
        "TAXII request must carry HTTP Basic credentials"
    );
}

#[tokio::test]
async fn poll_cycle_surfaces_corrobore_rejection_summary() {
    let (taxii_addr, _taxii_state) = spawn_mock_taxii(vec![(
        json!({
            "objects": [{"type": "indicator", "id": "indicator--x", "name": "x"}],
            "more": false
        }),
        Some("2026-07-15T12:00:00.000Z".to_owned()),
    )])
    .await;
    let (corrobore_addr, _corrobore_state) = spawn_mock_corrobore(json!({
        "ok": true,
        "result": {
            "processed_objects": 1,
            "applied_mutations": 0,
            "rejected_mutations": 1,
            "errors": ["mutation rejected during import"]
        }
    }))
    .await;

    let state_dir = unique_state_dir("rejection");
    let config = config_for(taxii_addr, corrobore_addr, &state_dir);
    let mut store = CursorStore::new(&state_dir);

    let outcome = run_poll_cycle(&config, &mut store)
        .await
        .expect("poll cycle should succeed");

    let summary = outcome.import.expect("import should have run");
    assert_eq!(summary.rejected_mutations, 1);
    assert_eq!(summary.errors, vec!["mutation rejected during import"]);
}
