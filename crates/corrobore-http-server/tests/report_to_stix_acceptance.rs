// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Release-gating acceptance for the supported already-extracted report-to-STIX
//! boundary. No PDF parser, OCR engine, or extraction model is involved.

use std::{collections::BTreeSet, fs, path::PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/report-to-stix")
        .join(name)
}

fn fixture_json(name: &str) -> Value {
    let path = fixture_path(name);
    let bytes = fs::read(&path)
        .unwrap_or_else(|error| panic!("report-to-STIX fixture {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("report-to-STIX fixture {}: {error}", path.display()))
}

#[test]
fn report_to_stix_corpus_covers_the_supported_boundary() {
    let input = fixture_json("input.json");
    let expected = fixture_json("expected.json");
    let objects = input["bundle"]["objects"]
        .as_array()
        .expect("bundle objects must be an array");

    assert_eq!(
        objects.first().and_then(|value| value["type"].as_str()),
        Some("relationship")
    );
    let relationship_count = objects
        .iter()
        .filter(|object| object["type"] == "relationship")
        .count();
    assert_eq!(relationship_count, 30);
    assert_eq!(
        expected["expected_import"]["relationships"],
        relationship_count
    );

    for required_type in [
        "intrusion-set",
        "malware",
        "attack-pattern",
        "identity",
        "location",
        "file",
        "domain-name",
        "report",
    ] {
        assert!(
            objects.iter().any(|object| object["type"] == required_type),
            "corpus must contain {required_type}"
        );
    }

    let locators = input["evidence"]["records"]
        .as_array()
        .expect("evidence records must be an array");
    for locator_type in ["page", "paragraph", "table_cell"] {
        assert!(
            locators
                .iter()
                .any(|record| record["locator"]["type"] == locator_type),
            "corpus must contain a {locator_type} locator"
        );
    }

    let fixture_text = [
        "input.json",
        "expected.json",
        "PROVENANCE.md",
        "GENERATION.md",
    ]
    .into_iter()
    .map(|name| fs::read_to_string(fixture_path(name)).expect("fixture text must load"))
    .collect::<String>();
    for forbidden in ["/Users/", "BEGIN CORROBORE LICENSE", "Bearer ", "APT_K_47"] {
        assert!(
            !fixture_text.contains(forbidden),
            "fixture leaks forbidden marker {forbidden}"
        );
    }
}

#[test]
fn report_to_stix_checksums_match_committed_artifacts() {
    let manifest =
        fs::read_to_string(fixture_path("checksums.sha256")).expect("checksum manifest must load");
    for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
        let (expected, name) = line
            .split_once("  ")
            .expect("checksum rows use sha256sum format");
        let actual =
            Sha256::digest(fs::read(fixture_path(name)).expect("checksummed fixture must load"))
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
        assert_eq!(actual, expected, "checksum mismatch for {name}");
    }
}

#[cfg(all(unix, feature = "enterprise-cti"))]
mod e2e {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use corrobore_http_server::{AppState, ServerConfig, build_router};
    use ed25519_dalek::pkcs8::EncodePublicKey;
    use ed25519_dalek::{Signer, SigningKey};
    use serde::Serialize;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::{BTreeSet, Digest, Sha256, fixture_json, fixture_path, fs};

    const AUTHORIZATION: &str = "Bearer report-to-stix-acceptance";
    const EXPORT_URI: &str = "/v1/export/stix?mode=strict&profile=stix-mvp&snapshot_id=snapshot--report-to-stix&transaction_id=transaction--report-to-stix&exporter_version=acceptance-v1";

    #[derive(Serialize)]
    struct UnsignedLicenseClaims<'a> {
        client_uuid: &'a str,
        client_email: &'a str,
        modules: &'a [String],
        valid_until: &'a str,
        tags: &'a [String],
    }

    struct HttpResponse {
        status: StatusCode,
        bytes: Vec<u8>,
        json: Value,
    }

    #[tokio::test]
    async fn report_to_stix_acceptance_is_deterministic_across_replay_and_restart() {
        let input = fixture_json("input.json");
        let expected = fixture_json("expected.json");
        run_report_to_stix_acceptance(input, expected).await;
    }

    async fn run_report_to_stix_acceptance(input: Value, expected: Value) {
        let root = unique_root("report-to-stix");
        let provider_root = root.join("provider");
        let storage_root = root.join("storage");
        let session_root = root.join("sessions");
        fs::create_dir_all(&root).expect("acceptance root must be created");
        let manifest = compile_verified_provider(&provider_root);

        let app = acceptance_app(
            &storage_root,
            &session_root,
            Some((&provider_root, &manifest)),
            "cti",
        );
        assert_provider_release(&app).await;
        assert_fail_closed_boundaries(&root, &provider_root, &manifest).await;
        assert_negative_imports(&app).await;

        let imported = send_json(
            &app,
            Method::POST,
            "/v1/import/stix",
            Some(input.clone()),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(
            imported.status,
            StatusCode::OK,
            "initial import: {}",
            imported.json
        );
        assert_eq!(
            imported.json["result"]["processed_objects"],
            expected["expected_import"]["requested"]
        );
        assert_eq!(
            imported.json["result"]["applied_mutations"],
            expected["expected_import"]["requested"]
        );
        assert_eq!(
            imported.json["result"]["metrics"]["created"],
            expected["expected_import"]["requested"]
        );
        assert!(
            imported.json["result"]["outcomes"]
                .as_array()
                .is_some_and(|outcomes| outcomes
                    .iter()
                    .all(|outcome| outcome["status"] == "created"))
        );

        assert_duplicate_replay(&app, input.clone(), &expected).await;

        assert_candidate_metadata(&app, &expected).await;
        assert_relationship_metadata(&app, "candidate").await;
        let validation = validate_graph(&app).await;
        let mut issue_codes = validation["result"]["issues"]
            .as_array()
            .expect("issues must be an array")
            .iter()
            .filter_map(|issue| issue["code"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        issue_codes.sort();
        let mut expected_codes = expected["validation_issue_codes"]
            .as_array()
            .expect("expected issue codes must be an array")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        expected_codes.sort();
        assert_eq!(issue_codes, expected_codes);
        assert_eq!(validation["result"]["valid"], false);

        let blocked = send_json(&app, Method::GET, EXPORT_URI, None, AUTHORIZATION).await;
        assert_eq!(blocked.status, StatusCode::BAD_REQUEST);
        assert_eq!(blocked.json["error"]["code"], "EXPORT_PLAN_FAILED");
        let blocked_message = blocked.json["error"]["message"]
            .as_str()
            .unwrap_or_default();
        for code in &expected_codes {
            assert!(
                blocked_message.contains(code),
                "strict failure must name {code}: {blocked_message}"
            );
        }

        let guide = fs::read_to_string(repository_path("docs/acceptance/report-to-stix.md"))
            .expect("acceptance guide must load");
        let queries = cypher_examples(&guide);
        assert_eq!(
            queries.len(),
            4,
            "every published Cypher example must be executed"
        );
        for query in queries {
            let response = send_json(
                &app,
                Method::POST,
                "/v1/cypher/write",
                Some(json!({"query": query})),
                AUTHORIZATION,
            )
            .await;
            assert_eq!(
                response.status,
                StatusCode::OK,
                "documented Cypher failed: {}",
                response.json
            );
            assert_eq!(response.json["result"]["status"], "Success");
        }
        assert_corrected_metadata(&app, &expected).await;
        assert_relationship_metadata(&app, "exportable").await;

        let validated = validate_graph(&app).await;
        assert_eq!(
            validated["result"]["valid"], true,
            "post-correction validation: {validated}"
        );
        assert_eq!(validated["result"]["issues"], json!([]));

        let first_export = send_json(&app, Method::GET, EXPORT_URI, None, AUTHORIZATION).await;
        assert_eq!(
            first_export.status,
            StatusCode::OK,
            "strict export: {}",
            first_export.json
        );
        assert_export_golden(&first_export.json, &expected);

        drop(app);
        let restarted = acceptance_app(
            &storage_root,
            &root.join("sessions-restarted"),
            Some((&provider_root, &manifest)),
            "cti",
        );
        assert_duplicate_replay(&restarted, input, &expected).await;
        let second_export =
            send_json(&restarted, Method::GET, EXPORT_URI, None, AUTHORIZATION).await;
        assert_eq!(
            second_export.status,
            StatusCode::OK,
            "restart export: {}",
            second_export.json
        );
        assert_eq!(
            second_export.bytes, first_export.bytes,
            "restart replay must produce byte-identical STIX"
        );
        assert_export_golden(&second_export.json, &expected);

        drop(restarted);
        fs::remove_dir_all(&root).expect("temporary acceptance root must be removable");
    }

    async fn assert_duplicate_replay(app: &Router, input: Value, expected: &Value) {
        let replay = send_json(
            app,
            Method::POST,
            "/v1/import/stix",
            Some(input),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(
            replay.status,
            StatusCode::OK,
            "duplicate replay: {}",
            replay.json
        );
        assert_eq!(replay.json["result"]["applied_mutations"], 0);
        assert_eq!(
            replay.json["result"]["metrics"]["duplicate"],
            expected["expected_import"]["requested"]
        );
        assert_eq!(replay.json["result"]["metrics"]["created"], 0);
        assert_eq!(replay.json["result"]["metrics"]["updated"], 0);
    }

    fn repository_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    fn unique_root(suffix: &str) -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after epoch")
            .as_millis();
        std::env::temp_dir().join(format!(
            "corrobore-{suffix}-{millis}-{}",
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn compile_verified_provider(root: &Path) -> PathBuf {
        fs::create_dir_all(root).expect("provider root must be created");
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
            .arg(fixture_path("provider.c"))
            .arg("-o")
            .arg(&library)
            .output()
            .expect("C compiler must run");
        assert!(
            output.status.success(),
            "provider compilation: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let digest = Sha256::digest(fs::read(&library).expect("provider library must load"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let manifest = root.join("providers.json");
        fs::write(
            &manifest,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "1",
                "providers": [{
                    "domain": "cti",
                    "library": library_name,
                    "sha256": digest,
                    "required": true,
                    "capabilities": [{"name": "node.validate", "version": "1"}]
                }]
            }))
            .expect("provider manifest must serialize"),
        )
        .expect("provider manifest must be written");
        manifest
    }

    fn acceptance_app(
        storage_root: &Path,
        session_root: &Path,
        provider: Option<(&Path, &Path)>,
        licensed_modules: &str,
    ) -> Router {
        let mut vars = HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "report-to-stix-acceptance".to_owned(),
            ),
            (
                "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
                "report-to-stix-admin".to_owned(),
            ),
            (
                "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
                session_root.display().to_string(),
            ),
            ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
            (
                "CORROBORE_STORAGE_DIR".to_owned(),
                storage_root.display().to_string(),
            ),
        ]);
        if let Some((provider_root, manifest)) = provider {
            vars.insert(
                "CORROBORE_DOMAIN_PROVIDER_DIR".to_owned(),
                provider_root.display().to_string(),
            );
            vars.insert(
                "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE".to_owned(),
                manifest.display().to_string(),
            );
        }
        vars.extend(signed_license_env(licensed_modules));
        let config = ServerConfig::from_map(&vars).expect("acceptance config must parse");
        build_router(AppState::new(config).expect("acceptance server must initialize"))
    }

    fn signed_license_env(modules_csv: &str) -> HashMap<String, String> {
        let signing = SigningKey::from_bytes(&[37_u8; 32]);
        let public_key = signing
            .verifying_key()
            .to_public_key_der()
            .expect("public key must encode");
        let verifying_pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
            STANDARD.encode(public_key.as_bytes())
        );
        let mut modules = modules_csv
            .split(',')
            .map(|module| module.trim().to_ascii_lowercase())
            .filter(|module| !module.is_empty())
            .collect::<Vec<_>>();
        modules.sort();
        modules.dedup();
        let tags = vec!["nfr".to_owned()];
        let canonical = serde_json::to_vec(&UnsignedLicenseClaims {
            client_uuid: "11111111-2222-4333-8444-555555555555",
            client_email: "report-to-stix-tests@corrobore.dev",
            modules: &modules,
            valid_until: "2099-01-01T00:00:00Z",
            tags: &tags,
        })
        .expect("license claims must serialize");
        let signature = signing.sign(&canonical);
        let license = serde_json::to_vec(&json!({
            "client_uuid": "11111111-2222-4333-8444-555555555555",
            "client_email": "report-to-stix-tests@corrobore.dev",
            "modules": modules,
            "valid_until": "2099-01-01T00:00:00Z",
            "tags": ["NFR"],
            "signature": STANDARD.encode(signature.to_bytes()),
        }))
        .expect("license must serialize");
        HashMap::from([
            (
                "CORROBORE_HTTP_LICENSE_PEM".to_owned(),
                format!(
                    "-----BEGIN CORROBORE LICENSE-----\n{}\n-----END CORROBORE LICENSE-----",
                    STANDARD.encode(license)
                ),
            ),
            (
                "CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM".to_owned(),
                verifying_pem,
            ),
        ])
    }

    async fn send_json(
        app: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
        authorization: &str,
    ) -> HttpResponse {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, authorization);
        if body.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        let request = builder
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .expect("request must build");
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("request must complete");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body must load")
            .to_vec();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        HttpResponse {
            status,
            bytes,
            json,
        }
    }

    async fn assert_provider_release(app: &Router) {
        let health = send_json(app, Method::GET, "/health", None, AUTHORIZATION).await;
        assert_eq!(health.status, StatusCode::OK);
        assert_eq!(health.json["domain_providers"]["configured"], 1);
        assert_eq!(health.json["domain_providers"]["ready"], 1);
        let status = send_json(
            app,
            Method::GET,
            "/v1/admin/domain-providers/status",
            None,
            "Bearer report-to-stix-admin",
        )
        .await;
        assert_eq!(status.status, StatusCode::OK);
        let provider = &status.json["result"]["providers"][0];
        assert_eq!(
            provider["provider_id"],
            "fr.estance.corrobore.domain.cti.report-to-stix-acceptance"
        );
        assert_eq!(provider["provider_version"], "1.0.0-test");
        assert_eq!(provider["ready"], true);
    }

    async fn assert_fail_closed_boundaries(root: &Path, provider_root: &Path, manifest: &Path) {
        let unavailable = acceptance_app(
            &root.join("unavailable-storage"),
            &root.join("unavailable-sessions"),
            None,
            "cti",
        );
        let response = send_json(&unavailable, Method::GET, EXPORT_URI, None, AUTHORIZATION).await;
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.json["error"]["code"], "DOMAIN_PROVIDER_NOT_READY");
        drop(unavailable);

        let unlicensed = acceptance_app(
            &root.join("unlicensed-storage"),
            &root.join("unlicensed-sessions"),
            Some((provider_root, manifest)),
            "fimi",
        );
        let response = send_json(&unlicensed, Method::GET, EXPORT_URI, None, AUTHORIZATION).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);
        assert_eq!(response.json["error"]["code"], "LICENSE_MODULE_MISSING");
    }

    async fn assert_negative_imports(app: &Router) {
        let dangling = json!({
            "bundle": {"type": "bundle", "objects": [
                {"type": "identity", "id": "identity--00000000-0000-4000-8000-000000008001", "name": "Atomic endpoint"},
                {"type": "relationship", "id": "relationship--00000000-0000-4000-8000-000000008002", "relationship_type": "related-to", "source_ref": "identity--00000000-0000-4000-8000-000000008001", "target_ref": "identity--00000000-0000-4000-8000-000000008099"}
            ]}
        });
        let response = send_json(
            app,
            Method::POST,
            "/v1/import/stix",
            Some(dangling),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.json["result"]["applied_mutations"], 0);
        assert_eq!(
            response.json["result"]["metrics"]["unresolved_reference"],
            1
        );
        assert_eq!(response.json["result"]["metrics"]["rejected"], 1);

        let contradictory = json!({
            "bundle": {"type": "bundle", "objects": [
                {"type": "malware", "id": "malware--00000000-0000-4000-8000-000000008101", "name": "First", "is_family": true},
                {"type": "malware", "id": "malware--00000000-0000-4000-8000-000000008101", "name": "Contradiction", "is_family": true}
            ]}
        });
        let response = send_json(
            app,
            Method::POST,
            "/v1/import/stix",
            Some(contradictory),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert_eq!(response.json["error"]["code"], "CONFLICTING_STIX_ID");

        let invalid_list = send_json(
            app,
            Method::POST,
            "/v1/cypher/write",
            Some(json!({"query": "MATCH (n) SET n.tags = $tags", "params": {"tags": ["synthetic", 7]}})),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(invalid_list.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            invalid_list.json["error"]["code"],
            "UNSUPPORTED_PARAMETER_TYPE"
        );
    }

    async fn validate_graph(app: &Router) -> Value {
        let response = send_json(
            app,
            Method::POST,
            "/v1/stix/validate",
            Some(json!({"source": "graph", "snapshot_id": "snapshot--report-to-stix"})),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "graph validation: {}",
            response.json
        );
        response.json
    }

    async fn native_metadata(app: &Router, label: Option<&str>, confidence: f64) -> Vec<Value> {
        let selector = label.map_or_else(|| "(n)".to_owned(), |label| format!("(n:{label})"));
        let query = format!(
            "MATCH {selector} WHERE n.confidence = {confidence} RETURN n.confidence, n.evidence_refs, n.status"
        );
        let response = send_json(
            app,
            Method::POST,
            "/v1/cypher/read",
            Some(json!({"query": query})),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "metadata read: {}",
            response.json
        );
        let records = response.json["result"]["data"]["Records"]
            .as_array()
            .unwrap_or_else(|| {
                panic!(
                    "metadata records missing for {label:?}/{confidence}: {}",
                    response.json
                )
            });
        records
            .iter()
            .map(|record| record["fields"].clone())
            .collect()
    }

    async fn assert_candidate_metadata(app: &Router, _expected: &Value) {
        let low_records = native_metadata(app, Some("OpenCtiType_malware"), 0.4).await;
        assert_eq!(low_records.len(), 1);
        let low = &low_records[0];
        assert_eq!(low["n.confidence"], "0.4");
        assert_eq!(low["n.status"], "candidate");
        assert!(
            low["n.evidence_refs"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        let missing_records = native_metadata(app, None, 0.9).await;
        assert_eq!(missing_records.len(), 1);
        let missing = &missing_records[0];
        assert_eq!(missing["n.confidence"], "0.9");
        assert_eq!(missing["n.status"], "candidate");
        assert_eq!(missing["n.evidence_refs"], "");
    }

    async fn assert_corrected_metadata(app: &Router, _expected: &Value) {
        let records = native_metadata(app, None, 0.95).await;
        assert_eq!(records.len(), 2);
        for metadata in records {
            assert_eq!(metadata["n.confidence"], "0.95");
            assert_eq!(metadata["n.status"], "exportable");
            assert!(
                metadata["n.evidence_refs"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
        }
    }

    async fn assert_relationship_metadata(app: &Router, expected_status: &str) {
        let response = send_json(
            app,
            Method::POST,
            "/v1/cypher/read",
            Some(json!({
                "query": "MATCH (source)-[r]->(target) RETURN r.status, r.confidence, r.evidence_refs"
            })),
            AUTHORIZATION,
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "relationship read: {}",
            response.json
        );
        let records = response.json["result"]["data"]["Records"]
            .as_array()
            .expect("relationship records must be an array");
        assert_eq!(
            records.len(),
            30,
            "all directed relationships must be readable: {}",
            response.json
        );
        assert!(records.iter().all(|record| {
            record["fields"]["r.status"] == expected_status
                && record["fields"]["r.confidence"].as_str().is_some()
                && record["fields"]["r.evidence_refs"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
        }));
    }

    fn cypher_examples(markdown: &str) -> Vec<String> {
        let mut examples = Vec::new();
        let mut lines = markdown.lines();
        while let Some(line) = lines.next() {
            if line.trim() != "```cypher" {
                continue;
            }
            let mut query = Vec::new();
            for query_line in lines.by_ref() {
                if query_line.trim() == "```" {
                    break;
                }
                query.push(query_line);
            }
            examples.push(query.join("\n"));
        }
        examples
    }

    fn assert_export_golden(bundle: &Value, expected: &Value) {
        assert_eq!(bundle["type"], "bundle");
        assert_eq!(bundle["export_metadata"]["mode"], "strict");
        assert_eq!(bundle["export_metadata"]["profile"], "stix-mvp");
        let objects = bundle["objects"]
            .as_array()
            .expect("export objects must be an array");
        assert_eq!(
            objects.len(),
            expected["golden"]["object_count"]
                .as_u64()
                .expect("golden object count must be an integer") as usize,
            "unexpected export cardinality: {}",
            serde_json::to_string_pretty(bundle).expect("export bundle must serialize")
        );
        let ids = objects
            .iter()
            .map(|object| {
                object["id"]
                    .as_str()
                    .expect("every STIX object must preserve id")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), objects.len());
        assert!(
            ids.contains(
                expected["golden"]["intrusion_set_id"]
                    .as_str()
                    .expect("intrusion-set golden id must be a string")
            )
        );
        for malware_id in expected["golden"]["malware_ids"]
            .as_array()
            .expect("malware golden ids must be an array")
        {
            assert!(
                ids.contains(
                    malware_id
                        .as_str()
                        .expect("malware golden id must be a string")
                )
            );
        }
        for object in objects {
            let object_type = object["type"].as_str().expect("STIX type must be present");
            assert!(!matches!(object_type, "node" | "edge" | "generic"));
            assert!(
                object["x_corrobore_evidence_refs"]
                    .as_array()
                    .is_some_and(|refs| !refs.is_empty())
            );
            if object_type == "relationship" {
                assert!(
                    ids.contains(
                        object["source_ref"]
                            .as_str()
                            .expect("relationship source_ref must be a string")
                    )
                );
                assert!(
                    ids.contains(
                        object["target_ref"]
                            .as_str()
                            .expect("relationship target_ref must be a string")
                    )
                );
            }
        }
        let report = objects
            .iter()
            .find(|object| object["id"] == expected["golden"]["report_id"])
            .expect("report must be exported");
        let report_refs = report["object_refs"]
            .as_array()
            .expect("report refs must be preserved");
        assert_eq!(
            report_refs.len(),
            expected["golden"]["report_object_ref_count"]
                .as_u64()
                .expect("report ref golden count must be an integer") as usize
        );
        assert!(report_refs.iter().all(|reference| {
            ids.contains(
                reference
                    .as_str()
                    .expect("report object_ref must be a string"),
            )
        }));

        let extension_object = objects
            .iter()
            .find(|object| object["id"] == expected["golden"]["unknown_extension_object_id"])
            .expect("unknown extension object must be exported");
        assert_eq!(
            extension_object["extensions"]["extension-definition--00000000-0000-4000-8000-000000009999"]
                ["synthetic_score"],
            7
        );
        let attack_ids = objects
            .iter()
            .filter(|object| object["type"] == "attack-pattern")
            .flat_map(|object| {
                object["external_references"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .filter_map(|reference| reference["external_id"].as_str())
            .collect::<BTreeSet<_>>();
        for external_id in expected["golden"]["technique_external_ids"]
            .as_array()
            .expect("technique external ids must be an array")
        {
            assert!(
                attack_ids.contains(
                    external_id
                        .as_str()
                        .expect("technique external id must be a string")
                )
            );
        }
    }
}
