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
use std::{collections::BTreeMap, time::Duration};

use axum::{
    Json,
    extract::{Multipart, State},
};
#[cfg(test)]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use corrobore_engine::{EngineError, EngineMutationContext};
use graph_core::{
    Confidence, EvidenceId, EvidenceInput, EvidenceRecordStore, EvidenceSourceType, Graph,
    GraphError, NodeId, NodeInput, PropertyValue, RecordStatus, RelationshipId, RelationshipInput,
};
use opencti_adapter::{MappedRecord, OpenCtiAdapter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{app::AppState, error::ApiError};

#[derive(Debug, Deserialize)]
pub struct ImportStixRequest {
    pub bundle: Value,
    #[serde(default)]
    pub evidence: Option<ImportEvidenceEnvelope>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub budget_ref: Option<String>,
}

/// Versioned evidence/provenance extension for one STIX import transaction.
#[derive(Clone, Debug, Deserialize)]
pub struct ImportEvidenceEnvelope {
    pub schema_version: String,
    pub records: Vec<ImportEvidenceRecord>,
    pub annotations: BTreeMap<String, ImportRecordAnnotation>,
}

/// Caller-owned durable evidence record supplied alongside a STIX bundle.
#[derive(Clone, Debug, Deserialize)]
pub struct ImportEvidenceRecord {
    pub id: String,
    pub source_id: String,
    pub content_sha256: String,
    pub payload: String,
    pub locator: graph_core::EvidenceLocator,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub extractor_id: Option<String>,
    #[serde(default)]
    pub model_version: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

/// Native metadata to apply to one STIX object or relationship by STIX ID.
#[derive(Clone, Debug, Deserialize)]
pub struct ImportRecordAnnotation {
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportStixResponse {
    pub ok: bool,
    pub result: ImportStixResult,
}

#[derive(Debug, Serialize)]
pub struct ImportStixResult {
    pub processed_objects: usize,
    pub applied_mutations: usize,
    pub rejected_mutations: usize,
    pub errors: Vec<String>,
}

pub async fn import_stix_bundle(
    State(state): State<AppState>,
    Json(payload): Json<ImportStixRequest>,
) -> Result<Json<ImportStixResponse>, ApiError> {
    let result = import_bundle_with_evidence_context(
        &state,
        payload.bundle,
        payload.evidence,
        payload.workspace_id,
        payload.session_id,
        payload.budget_ref,
    )
    .await?;

    Ok(Json(ImportStixResponse { ok: true, result }))
}

async fn import_bundle_with_evidence_context(
    state: &AppState,
    bundle: Value,
    evidence: Option<ImportEvidenceEnvelope>,
    workspace_id: Option<String>,
    session_id: Option<String>,
    budget_ref: Option<String>,
) -> Result<ImportStixResult, ApiError> {
    let prepared = prepare_typed_import(bundle, evidence)?;
    let processed_objects = prepared.records.len();
    let workspace_id = workspace_id.unwrap_or_else(|| "workspace--http-default".to_owned());
    let session_id = session_id.unwrap_or_else(|| "session--http-import-stix".to_owned());
    let budget_ref = budget_ref.unwrap_or_else(|| "budget--http-import-stix".to_owned());
    let context = EngineMutationContext::new(workspace_id, session_id, budget_ref);
    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    let engine = state.engine.clone();

    let applied_mutations = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut locked = engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
            locked
                .mutate_graph_atomically(context, move |graph| apply_typed_import(graph, prepared))
                .map_err(map_typed_import_error)
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "stix import timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;

    Ok(ImportStixResult {
        processed_objects,
        applied_mutations,
        rejected_mutations: 0,
        errors: Vec::new(),
    })
}

#[derive(Clone, Debug)]
struct NativeImportAnnotation {
    evidence_refs: Vec<EvidenceId>,
    confidence: Option<Confidence>,
}

#[derive(Clone, Debug)]
struct PreparedTypedImport {
    records: Vec<MappedRecord>,
    evidence: Vec<EvidenceInput>,
    annotations: BTreeMap<String, NativeImportAnnotation>,
}

fn prepare_typed_import(
    bundle: Value,
    evidence: Option<ImportEvidenceEnvelope>,
) -> Result<PreparedTypedImport, ApiError> {
    let bundle_type = bundle
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !bundle_type.eq_ignore_ascii_case("bundle") {
        return Err(ApiError::bad_request(
            "INVALID_STIX_BUNDLE",
            "bundle.type must be 'bundle'",
        ));
    }
    let objects = bundle
        .get("objects")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ApiError::bad_request("INVALID_STIX_BUNDLE", "bundle.objects must be an array")
        })?;
    if objects.is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_STIX_BUNDLE",
            "bundle.objects cannot be empty",
        ));
    }

    let adapter = OpenCtiAdapter::pinned();
    let records = objects
        .iter()
        .cloned()
        .map(|object| {
            adapter
                .map(object)
                .map_err(|error| ApiError::bad_request("INVALID_STIX_OBJECT", error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let canonical_ids = records
        .iter()
        .map(|record| record.record_ref().canonical_id().to_owned())
        .collect::<std::collections::BTreeSet<_>>();

    let Some(envelope) = evidence else {
        return Ok(PreparedTypedImport {
            records,
            evidence: Vec::new(),
            annotations: BTreeMap::new(),
        });
    };
    if envelope.schema_version != "1.0" {
        return Err(ApiError::bad_request(
            "UNSUPPORTED_EVIDENCE_SCHEMA_VERSION",
            "evidence.schema_version must be '1.0'",
        ));
    }

    let mut validation_store = EvidenceRecordStore::new();
    let mut evidence_inputs = Vec::with_capacity(envelope.records.len());
    for record in envelope.records {
        let evidence_id = EvidenceId::new(record.id)
            .map_err(|error| ApiError::bad_request("INVALID_EVIDENCE_ID", error.to_string()))?;
        let mut input = EvidenceInput::new(evidence_id, record.source_id, record.payload)
            .with_source_type(EvidenceSourceType::Document)
            .with_content_sha256(record.content_sha256)
            .with_locator(record.locator);
        if let Some(source_url) = record.source_url {
            input = input.with_source_url(source_url);
        }
        if let Some(extractor_id) = record.extractor_id {
            input = input.with_extractor_id(extractor_id);
        }
        if let Some(model_version) = record.model_version {
            input = input.with_model_version(model_version);
        }
        if let Some(language) = record.language {
            input = input.with_language(language);
        }
        validation_store
            .create_evidence(input.clone())
            .map_err(map_evidence_input_error)?;
        evidence_inputs.push(input);
    }

    let mut annotations = BTreeMap::new();
    for (stix_id, annotation) in envelope.annotations {
        if !canonical_ids.contains(&stix_id) {
            return Err(ApiError::bad_request(
                "ANNOTATION_TARGET_NOT_FOUND",
                format!("annotation target is not present in bundle: {stix_id}"),
            ));
        }
        if let Some(status) = annotation.status.as_deref()
            && status != "candidate"
        {
            return Err(ApiError::bad_request(
                "INVALID_IMPORT_STATUS",
                "import annotations may request only 'candidate' status",
            ));
        }
        let confidence = annotation
            .confidence
            .map(normalize_stix_confidence)
            .transpose()?;
        let evidence_refs = annotation
            .evidence_refs
            .into_iter()
            .map(|value| {
                EvidenceId::new(value).map_err(|error| {
                    ApiError::bad_request("INVALID_EVIDENCE_ID", error.to_string())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        annotations.insert(
            stix_id,
            NativeImportAnnotation {
                evidence_refs,
                confidence,
            },
        );
    }

    Ok(PreparedTypedImport {
        records,
        evidence: evidence_inputs,
        annotations,
    })
}

fn map_evidence_input_error(error: GraphError) -> ApiError {
    let message = error.to_string();
    let code = if message.contains("conflicting evidence record") {
        "DUPLICATE_EVIDENCE_ID"
    } else if message.contains("content_sha256") {
        "INVALID_EVIDENCE_DIGEST"
    } else if message.contains("locator") {
        "INVALID_EVIDENCE_LOCATOR"
    } else {
        "INVALID_EVIDENCE_RECORD"
    };
    ApiError::bad_request(code, message)
}

fn normalize_stix_confidence(value: f64) -> Result<Confidence, ApiError> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(ApiError::bad_request(
            "INVALID_STIX_CONFIDENCE",
            "STIX confidence must be finite and within 0..=100",
        ));
    }
    Confidence::new(value / 100.0)
        .map_err(|error| ApiError::bad_request("INVALID_STIX_CONFIDENCE", error.to_string()))
}

fn apply_typed_import(
    graph: &mut Graph,
    prepared: PreparedTypedImport,
) -> Result<usize, GraphError> {
    for input in prepared.evidence {
        graph.create_evidence(input)?;
    }
    for annotation in prepared.annotations.values() {
        for evidence_id in &annotation.evidence_refs {
            if graph.evidence_by_id(evidence_id).is_none() {
                return Err(GraphError::EvidenceNotFound(evidence_id.clone()));
            }
        }
    }

    let mut node_index = index_nodes_by_canonical_id(graph)?;
    let mut relationship_index = index_relationships_by_canonical_id(graph)?;
    let mut relationships = Vec::new();
    let applied = prepared.records.len();

    for record in prepared.records {
        let canonical_id = record.record_ref().canonical_id().to_owned();
        let raw = record.raw().clone();
        match record {
            MappedRecord::Object(object) => {
                let annotation = prepared.annotations.get(&canonical_id);
                let confidence = native_confidence(annotation, &raw)?;
                let input = decorate_node_input(object.to_node_input(), annotation, confidence);
                let node_id = if let Some(existing) = node_index.get(&canonical_id) {
                    graph.replace_node(existing, input)?
                } else {
                    graph.create_node(input)?
                };
                node_index.insert(canonical_id, node_id);
            }
            relationship @ MappedRecord::Relationship(_) => relationships.push(relationship),
        }
    }

    for record in relationships {
        let canonical_id = record.record_ref().canonical_id().to_owned();
        let raw = record.raw().clone();
        let MappedRecord::Relationship(relationship) = record else {
            unreachable!("relationship partition contains only relationships")
        };
        let source = node_index
            .get(relationship.source_ref())
            .cloned()
            .ok_or_else(|| {
                GraphError::InvalidPropertyValue(format!(
                    "relationship source is unavailable: {}",
                    relationship.source_ref()
                ))
            })?;
        let target = node_index
            .get(relationship.target_ref())
            .cloned()
            .ok_or_else(|| {
                GraphError::InvalidPropertyValue(format!(
                    "relationship target is unavailable: {}",
                    relationship.target_ref()
                ))
            })?;
        let annotation = prepared.annotations.get(&canonical_id);
        let confidence = native_confidence(annotation, &raw)?;
        let input = relationship
            .to_relationship_input(source, target)
            .map_err(|error| GraphError::InvalidPropertyValue(error.to_string()))?;
        let input = decorate_relationship_input(input, annotation, confidence);
        let relationship_id = if let Some(existing) = relationship_index.get(&canonical_id) {
            graph.replace_relationship(existing, input)?
        } else {
            graph.create_relationship(input)?
        };
        relationship_index.insert(canonical_id, relationship_id);
    }
    Ok(applied)
}

fn native_confidence(
    annotation: Option<&NativeImportAnnotation>,
    raw: &Value,
) -> Result<Option<Confidence>, GraphError> {
    if let Some(confidence) = annotation.and_then(|annotation| annotation.confidence) {
        return Ok(Some(confidence));
    }
    let Some(value) = raw.get("confidence").and_then(Value::as_f64) else {
        return Ok(None);
    };
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(GraphError::InvalidPropertyValue(
            "STIX confidence must be finite and within 0..=100".to_owned(),
        ));
    }
    Confidence::new(value / 100.0).map(Some)
}

fn decorate_node_input(
    mut input: NodeInput,
    annotation: Option<&NativeImportAnnotation>,
    confidence: Option<Confidence>,
) -> NodeInput {
    input = input.with_status(RecordStatus::Candidate);
    if let Some(confidence) = confidence {
        input = input.with_confidence(confidence);
    }
    if let Some(annotation) = annotation {
        for evidence_id in &annotation.evidence_refs {
            input = input.with_evidence_ref(evidence_id.clone());
        }
    }
    input
}

fn decorate_relationship_input(
    mut input: RelationshipInput,
    annotation: Option<&NativeImportAnnotation>,
    confidence: Option<Confidence>,
) -> RelationshipInput {
    input = input.with_status(RecordStatus::Candidate);
    if let Some(confidence) = confidence {
        input = input.with_confidence(confidence);
    }
    if let Some(annotation) = annotation {
        for evidence_id in &annotation.evidence_refs {
            input = input.with_evidence_ref(evidence_id.clone());
        }
    }
    input
}

fn index_nodes_by_canonical_id(graph: &Graph) -> Result<BTreeMap<String, NodeId>, GraphError> {
    let mut index = BTreeMap::new();
    for node in graph.list_nodes()? {
        if let Some(canonical_id) = canonical_id_property(
            node.property("opencti.canonical_id")
                .or_else(|| node.property("stix_id")),
        ) {
            index.insert(canonical_id, node.id().clone());
        }
    }
    Ok(index)
}

fn index_relationships_by_canonical_id(
    graph: &Graph,
) -> Result<BTreeMap<String, RelationshipId>, GraphError> {
    let mut index = BTreeMap::new();
    for relationship in graph.list_relationships()? {
        if let Some(canonical_id) = canonical_id_property(
            relationship
                .property("opencti.canonical_id")
                .or_else(|| relationship.property("stix_id")),
        ) {
            index.insert(canonical_id, relationship.id().clone());
        }
    }
    Ok(index)
}

fn canonical_id_property(value: Option<&PropertyValue>) -> Option<String> {
    match value {
        Some(PropertyValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn map_typed_import_error(error: EngineError) -> ApiError {
    match error {
        EngineError::InvalidConfiguration { field, reason } => {
            let code = match field {
                "workspace_id" => "INVALID_WORKSPACE_ID",
                "session_id" => "INVALID_SESSION_ID",
                "budget_ref" => "INVALID_BUDGET_REF",
                "mutation_policy" => "MUTATION_FORBIDDEN",
                _ => "INVALID_REQUEST",
            };
            ApiError::bad_request(code, reason)
        }
        EngineError::Graph(GraphError::EvidenceNotFound(evidence_id)) => ApiError::bad_request(
            "EVIDENCE_NOT_FOUND",
            format!(
                "referenced evidence does not exist: {}",
                evidence_id.as_str()
            ),
        ),
        EngineError::Graph(GraphError::InvalidPropertyValue(message))
            if message.contains("conflicting evidence record") =>
        {
            ApiError::bad_request("DUPLICATE_EVIDENCE_ID", message)
        }
        EngineError::Graph(GraphError::InvalidPropertyValue(message))
            if message.contains("STIX confidence") =>
        {
            ApiError::bad_request("INVALID_STIX_CONFIDENCE", message)
        }
        other => ApiError::bad_request(
            "TYPED_STIX_IMPORT_FAILED",
            format!("typed STIX import failed: {other}"),
        ),
    }
}

pub async fn import_stix_bundle_file(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ImportStixResponse>, ApiError> {
    let mut bundle: Option<Value> = None;
    let mut workspace_id: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut budget_ref: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::bad_request("INVALID_MULTIPART", error.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_owned();

        match name.as_str() {
            "file" => {
                let filename = field.file_name().unwrap_or_default().to_owned();
                if !filename.is_empty() && !has_supported_stix_extension(&filename) {
                    return Err(ApiError::bad_request(
                        "UNSUPPORTED_FILE_EXTENSION",
                        "file must use .json or .stix extension",
                    ));
                }

                let bytes = field.bytes().await.map_err(|error| {
                    ApiError::bad_request("INVALID_MULTIPART", error.to_string())
                })?;

                let parsed = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
                    ApiError::bad_request("INVALID_STIX_FILE", error.to_string())
                })?;
                bundle = Some(parsed);
            }
            "workspace_id" => {
                let value = field.text().await.map_err(|error| {
                    ApiError::bad_request("INVALID_MULTIPART", error.to_string())
                })?;
                if !value.trim().is_empty() {
                    workspace_id = Some(value.trim().to_owned());
                }
            }
            "session_id" => {
                let value = field.text().await.map_err(|error| {
                    ApiError::bad_request("INVALID_MULTIPART", error.to_string())
                })?;
                if !value.trim().is_empty() {
                    session_id = Some(value.trim().to_owned());
                }
            }
            "budget_ref" => {
                let value = field.text().await.map_err(|error| {
                    ApiError::bad_request("INVALID_MULTIPART", error.to_string())
                })?;
                if !value.trim().is_empty() {
                    budget_ref = Some(value.trim().to_owned());
                }
            }
            _ => {
                // Ignore unknown multipart fields to keep endpoint forward-compatible.
            }
        }
    }

    let bundle = bundle.ok_or_else(|| {
        ApiError::bad_request("MISSING_FILE_FIELD", "multipart field 'file' is required")
    })?;

    let result =
        import_bundle_with_context(&state, bundle, workspace_id, session_id, budget_ref).await?;

    Ok(Json(ImportStixResponse { ok: true, result }))
}

pub(crate) async fn import_bundle_with_context(
    state: &AppState,
    bundle: Value,
    workspace_id: Option<String>,
    session_id: Option<String>,
    budget_ref: Option<String>,
) -> Result<ImportStixResult, ApiError> {
    import_bundle_with_evidence_context(state, bundle, None, workspace_id, session_id, budget_ref)
        .await
}

fn has_supported_stix_extension(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.ends_with(".json") || lower.ends_with(".stix")
}

#[cfg(test)]
mod evidence_aware_tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use axum::{Json, extract::State};
    use graph_core::{EvidenceId, PropertyValue, RecordStatus};
    use serde_json::json;

    use super::{ImportStixRequest, import_bundle_with_context, import_stix_bundle};
    use crate::{app::AppState, config::ServerConfig};

    fn test_state() -> AppState {
        let config = ServerConfig::from_map(&HashMap::from([(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        )]))
        .expect("test config should parse");
        AppState::new(config).expect("test state should initialize")
    }

    fn unique_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("corrobore-{label}-{nanos}"))
    }

    fn persistent_state(storage: &Path, sessions: &Path) -> AppState {
        let config = ServerConfig::from_map(&HashMap::from([
            (
                "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
                "token-123".to_owned(),
            ),
            (
                "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
                sessions.display().to_string(),
            ),
            ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
            (
                "CORROBORE_STORAGE_DIR".to_owned(),
                storage.display().to_string(),
            ),
        ]))
        .expect("persistent test config should parse");
        AppState::new(config).expect("persistent state should initialize")
    }

    // Regression: the legacy scalar-Cypher route stored STIX confidence
    // as a generic property. Typed import must normalize it into native graph
    // metadata while preserving the complete raw object and candidate status.
    #[tokio::test]
    async fn plain_stix_import_uses_native_candidate_metadata_and_typed_raw_json() {
        let state = test_state();
        let raw = json!({
            "type": "threat-actor",
            "id": "threat-actor--typed-import-1",
            "name": "Typed actor",
            "confidence": 50,
            "aliases": ["TA-1", "Typed actor alias"],
            "x_nested": {"active": true, "score": 7}
        });

        import_bundle_with_context(
            &state,
            json!({"type": "bundle", "objects": [raw.clone()]}),
            None,
            None,
            None,
        )
        .await
        .expect("plain STIX import should succeed");

        let engine = state.engine.lock().expect("engine lock should succeed");
        let nodes = engine
            .graph()
            .list_nodes()
            .expect("nodes should be readable");
        let node = nodes
            .iter()
            .find(|node| {
                node.property("opencti.canonical_id")
                    == Some(&PropertyValue::String(
                        "threat-actor--typed-import-1".to_owned(),
                    ))
            })
            .expect("typed OpenCTI node should exist");

        assert_eq!(node.status(), RecordStatus::Candidate);
        assert_eq!(
            node.confidence().map(|confidence| confidence.value()),
            Some(0.5)
        );
        assert!(node.evidence_refs().is_empty());
        assert_eq!(
            node.property("opencti.raw"),
            Some(&PropertyValue::Json(raw))
        );
    }

    fn evidence_aware_payload() -> serde_json::Value {
        json!({
            "bundle": {
                "type": "bundle",
                "objects": [{
                    "type": "threat-actor",
                    "id": "threat-actor--evidence-aware-1",
                    "name": "Grounded actor",
                    "confidence": 90,
                    "aliases": ["Grounded", "TA-G"],
                    "created": "2026-01-02T03:04:05Z",
                    "object_marking_refs": ["marking-definition--tlp-amber"],
                    "hashes": {"SHA-256": "abc123"},
                    "extensions": {"extension-definition--demo": {"enabled": true}}
                }]
            },
            "evidence": {
                "schema_version": "1.0",
                "records": [{
                    "id": "evidence--apt-k-47-p7-paragraph-2",
                    "source_id": "document--apt-k-47",
                    "content_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "payload": "Grounding excerpt",
                    "locator": {"type": "paragraph", "page": 7, "paragraph": 2}
                }],
                "annotations": {
                    "threat-actor--evidence-aware-1": {
                        "evidence_refs": ["evidence--apt-k-47-p7-paragraph-2"],
                        "confidence": 50,
                        "status": "candidate"
                    }
                }
            }
        })
    }

    #[tokio::test]
    async fn evidence_aware_import_persists_native_metadata_and_is_idempotent() {
        let state = test_state();
        let request: ImportStixRequest = serde_json::from_value(evidence_aware_payload())
            .expect("evidence-aware request should deserialize");
        let _ = import_stix_bundle(State(state.clone()), Json(request))
            .await
            .expect("evidence-aware import should succeed");

        let replay: ImportStixRequest = serde_json::from_value(evidence_aware_payload())
            .expect("replay request should deserialize");
        let _ = import_stix_bundle(State(state.clone()), Json(replay))
            .await
            .expect("idempotent replay should succeed");

        let evidence_id = EvidenceId::new("evidence--apt-k-47-p7-paragraph-2")
            .expect("evidence id should be valid");
        let engine = state.engine.lock().expect("engine lock should succeed");
        let graph = engine.graph();
        let nodes = graph.list_nodes().expect("nodes should be readable");
        assert_eq!(nodes.len(), 1, "replay must preserve one native node");
        let node = &nodes[0];
        assert_eq!(node.status(), RecordStatus::Candidate);
        assert_eq!(
            node.confidence().map(|confidence| confidence.value()),
            Some(0.5)
        );
        assert_eq!(node.evidence_refs(), std::slice::from_ref(&evidence_id));
        assert_eq!(
            graph.evidence_count(),
            1,
            "replay must not duplicate evidence"
        );
        let evidence = graph
            .evidence_by_id(&evidence_id)
            .expect("evidence should remain addressable");
        assert_eq!(evidence.source_ref(), "document--apt-k-47");
        assert_eq!(
            evidence.content_sha256(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            node.property("opencti.field.aliases"),
            Some(&PropertyValue::StringList(vec![
                "Grounded".to_owned(),
                "TA-G".to_owned(),
            ]))
        );
        assert!(matches!(
            node.property("opencti.field.extensions"),
            Some(PropertyValue::Json(_))
        ));
    }

    #[tokio::test]
    async fn invalid_evidence_reference_rejects_the_whole_import() {
        let state = test_state();
        let mut payload = evidence_aware_payload();
        payload["evidence"]["annotations"]["threat-actor--evidence-aware-1"]["evidence_refs"] =
            json!(["evidence--missing"]);
        let request: ImportStixRequest =
            serde_json::from_value(payload).expect("request should deserialize");

        let error = import_stix_bundle(State(state.clone()), Json(request))
            .await
            .expect_err("missing referenced evidence must fail");

        assert_eq!(error.code, "EVIDENCE_NOT_FOUND");
        let engine = state.engine.lock().expect("engine lock should succeed");
        assert!(
            engine
                .graph()
                .list_nodes()
                .expect("nodes should be readable")
                .is_empty(),
            "invalid requests must not leave partial state"
        );
        assert_eq!(engine.graph().evidence_count(), 0);
    }

    #[tokio::test]
    async fn conflicting_duplicate_evidence_and_authoritative_status_are_rejected_atomically() {
        let state = test_state();
        let mut duplicate = evidence_aware_payload();
        let mut conflicting = duplicate["evidence"]["records"][0].clone();
        conflicting["payload"] = json!("Conflicting excerpt");
        duplicate["evidence"]["records"]
            .as_array_mut()
            .expect("records should be an array")
            .push(conflicting);
        let duplicate_request: ImportStixRequest =
            serde_json::from_value(duplicate).expect("request should deserialize");
        let duplicate_error = import_stix_bundle(State(state.clone()), Json(duplicate_request))
            .await
            .expect_err("conflicting duplicate evidence must fail");
        assert_eq!(duplicate_error.code, "DUPLICATE_EVIDENCE_ID");

        let mut authoritative = evidence_aware_payload();
        authoritative["evidence"]["annotations"]["threat-actor--evidence-aware-1"]["status"] =
            json!("exported");
        let authoritative_request: ImportStixRequest =
            serde_json::from_value(authoritative).expect("request should deserialize");
        let status_error = import_stix_bundle(State(state.clone()), Json(authoritative_request))
            .await
            .expect_err("payload must not assert exported authority");
        assert_eq!(status_error.code, "INVALID_IMPORT_STATUS");

        let engine = state.engine.lock().expect("engine lock should succeed");
        assert!(
            engine
                .graph()
                .list_nodes()
                .expect("nodes should load")
                .is_empty()
        );
        assert_eq!(engine.graph().evidence_count(), 0);
    }

    #[tokio::test]
    async fn invalid_locator_and_confidence_are_stable_atomic_errors() {
        let state = test_state();
        let mut invalid_locator = evidence_aware_payload();
        invalid_locator["evidence"]["records"][0]["locator"] =
            json!({"type": "paragraph", "page": 0, "paragraph": 2});
        let request: ImportStixRequest =
            serde_json::from_value(invalid_locator).expect("request should deserialize");
        let locator_error = import_stix_bundle(State(state.clone()), Json(request))
            .await
            .expect_err("zero-based page must fail");
        assert_eq!(locator_error.code, "INVALID_EVIDENCE_LOCATOR");

        let mut invalid_confidence = evidence_aware_payload();
        invalid_confidence["evidence"]["annotations"]["threat-actor--evidence-aware-1"]["confidence"] =
            json!(101);
        let request: ImportStixRequest =
            serde_json::from_value(invalid_confidence).expect("request should deserialize");
        let confidence_error = import_stix_bundle(State(state.clone()), Json(request))
            .await
            .expect_err("out-of-range confidence must fail");
        assert_eq!(confidence_error.code, "INVALID_STIX_CONFIDENCE");

        let engine = state.engine.lock().expect("engine lock should succeed");
        assert!(
            engine
                .graph()
                .list_nodes()
                .expect("nodes should load")
                .is_empty()
        );
        assert_eq!(engine.graph().evidence_count(), 0);
    }

    #[tokio::test]
    async fn evidence_records_survive_persistent_engine_restart() {
        let storage = unique_dir("typed-import-storage");
        let sessions = unique_dir("typed-import-sessions");
        {
            let state = persistent_state(&storage, &sessions);
            let request: ImportStixRequest = serde_json::from_value(evidence_aware_payload())
                .expect("request should deserialize");
            let _ = import_stix_bundle(State(state.clone()), Json(request))
                .await
                .expect("persistent import should succeed");
        }

        let restored = persistent_state(&storage, &sessions);
        let evidence_id = EvidenceId::new("evidence--apt-k-47-p7-paragraph-2")
            .expect("evidence id should be valid");
        let mut engine = restored.engine.lock().expect("engine lock should succeed");
        engine
            .read("MATCH (n) RETURN n")
            .expect("persistent projection should hydrate");
        assert_eq!(engine.graph().evidence_count(), 1);
        assert!(engine.graph().evidence_by_id(&evidence_id).is_some());
        let node = engine
            .graph()
            .list_nodes()
            .expect("nodes should load")
            .into_iter()
            .next()
            .expect("typed node should survive restart");
        assert_eq!(node.evidence_refs(), &[evidence_id]);
        drop(engine);
        drop(restored);
        let _ = fs::remove_dir_all(storage);
        let _ = fs::remove_dir_all(sessions);
    }

    #[tokio::test]
    async fn relationship_annotations_use_native_typed_metadata() {
        let state = test_state();
        let payload = json!({
            "bundle": {
                "type": "bundle",
                "objects": [
                    {"type": "identity", "id": "identity--source", "name": "Source"},
                    {"type": "identity", "id": "identity--target", "name": "Target"},
                    {
                        "type": "relationship",
                        "id": "relationship--grounded-1",
                        "relationship_type": "related-to",
                        "source_ref": "identity--source",
                        "target_ref": "identity--target",
                        "confidence": 75
                    }
                ]
            },
            "evidence": {
                "schema_version": "1.0",
                "records": [{
                    "id": "evidence--relationship-1",
                    "source_id": "document--relationship-test",
                    "content_sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "payload": "Source is related to target",
                    "locator": {"type": "table_cell", "page": 3, "table": 1, "row": 2, "column": 4}
                }],
                "annotations": {
                    "relationship--grounded-1": {
                        "evidence_refs": ["evidence--relationship-1"],
                        "confidence": 50,
                        "status": "candidate"
                    }
                }
            }
        });
        let request: ImportStixRequest =
            serde_json::from_value(payload).expect("request should deserialize");
        let _ = import_stix_bundle(State(state.clone()), Json(request))
            .await
            .expect("relationship import should succeed");

        let evidence_id =
            EvidenceId::new("evidence--relationship-1").expect("evidence id should be valid");
        let engine = state.engine.lock().expect("engine lock should succeed");
        let relationships = engine
            .graph()
            .list_relationships()
            .expect("relationships should load");
        assert_eq!(relationships.len(), 1);
        assert_eq!(relationships[0].status(), RecordStatus::Candidate);
        assert_eq!(
            relationships[0]
                .confidence()
                .map(|confidence| confidence.value()),
            Some(0.5)
        );
        assert_eq!(relationships[0].evidence_refs(), &[evidence_id]);
        assert!(matches!(
            relationships[0].property("opencti.raw"),
            Some(PropertyValue::Json(_))
        ));
    }
}

#[cfg(test)]
fn build_query_for_object(object: &Value) -> Result<String, ApiError> {
    let object_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("INVALID_STIX_OBJECT", "object.type is required"))?;
    let mapped = OpenCtiAdapter::pinned()
        .map(object.clone())
        .map_err(|error| ApiError::bad_request("INVALID_STIX_OBJECT", error.to_string()))?;
    // The MVP Cypher SET parser separates assignments on commas without
    // interpreting quoted JSON. Base64 keeps the lossless payload scalar until
    // the HTTP importer can write typed `PropertyValue::Json` directly.
    let raw = STANDARD.encode(object.to_string());
    let mapping_version = mapped.mapping_version();
    let mapping_version = format!("{}.{}", mapping_version.major, mapping_version.minor);

    if let MappedRecord::Relationship(relationship) = &mapped {
        let relationship_id = escape_cypher_string(required_str_field(object, "id")?);
        let source_ref = escape_cypher_string(relationship.source_ref());
        let target_ref = escape_cypher_string(relationship.target_ref());
        let relationship_label = sanitize_relationship_type(relationship.relationship_type());
        let family = mapped.family().as_str();

        return Ok(format!(
            "MATCH (source {{stix_id: '{source_ref}'}}) MATCH (target {{stix_id: '{target_ref}'}}) MERGE (source)-[r:{relationship_label}]->(target) SET r.stix_id = '{relationship_id}', r.opencti_raw = '{raw}', r.opencti_raw_encoding = 'base64-json', r.opencti_type = '{object_type}', r.opencti_family = '{family}', r.opencti_mapping_version = '{mapping_version}' RETURN r"
        ));
    }

    let stix_id = escape_cypher_string(required_str_field(object, "id")?);
    let label = map_stix_type_to_label(object_type);
    let family = mapped.family().as_str();
    let mut query = format!("MERGE (n:{label} {{stix_id: '{stix_id}'}})");
    let mut assignments = vec![
        format!("n.opencti_raw = '{raw}'"),
        "n.opencti_raw_encoding = 'base64-json'".to_owned(),
        format!("n.opencti_type = '{}'", escape_cypher_string(object_type)),
        format!("n.opencti_family = '{family}'"),
        format!("n.opencti_mapping_version = '{mapping_version}'"),
    ];

    if let Some(name) = object.get("name").and_then(Value::as_str) {
        assignments.push(format!("n.name = '{}'", escape_cypher_string(name)));
    }

    query.push_str(&format!(" SET {}", assignments.join(", ")));
    query.push_str(" RETURN n");
    Ok(query)
}

#[cfg(test)]
fn required_str_field<'a>(object: &'a Value, field: &'static str) -> Result<&'a str, ApiError> {
    object.get(field).and_then(Value::as_str).ok_or_else(|| {
        ApiError::bad_request("INVALID_STIX_OBJECT", format!("object.{field} is required"))
    })
}

#[cfg(test)]
fn map_stix_type_to_label(object_type: &str) -> &'static str {
    match object_type.to_ascii_lowercase().as_str() {
        "threat-actor" | "intrusion-set" => "ThreatActor",
        "indicator" => "Indicator",
        "malware" => "Malware",
        "tool" => "Tool",
        "campaign" => "Campaign",
        "infrastructure" => "Infrastructure",
        "vulnerability" => "Vulnerability",
        "identity" => "Identity",
        "location" => "Location",
        "report" => "Report",
        _ => "OpenCtiObject",
    }
}

#[cfg(test)]
fn sanitize_relationship_type(value: &str) -> String {
    let mut normalized = value
        .to_ascii_uppercase()
        .chars()
        .map(|ch| match ch {
            'A'..='Z' | '0'..='9' => ch,
            _ => '_',
        })
        .collect::<String>();

    if normalized.is_empty() {
        normalized = "RELATED_TO".to_owned();
    }

    normalized
}

#[cfg(test)]
fn escape_cypher_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use corrobore_engine::{CorroboreEngine, CypherResponseStatus};
    use graph_core::PropertyValue;
    use serde_json::json;

    use super::build_query_for_object;

    #[test]
    fn unknown_opencti_type_uses_generic_label_and_preserves_raw_record() {
        let raw = json!({
            "type": "future-opencti-type",
            "id": "future-opencti-type--1",
            "x_future": {"nested": [true, 42]}
        });

        let query = build_query_for_object(&raw).expect("future type should map generically");

        assert!(query.contains("MERGE (n:OpenCtiObject"));
        assert!(!query.contains("MERGE (n:Identity"));
        assert!(query.contains("n.opencti_raw"));
        assert!(query.contains("n.opencti_raw_encoding = 'base64-json'"));
        assert!(query.contains("n.opencti_mapping_version = '1.0'"));

        let mut engine = CorroboreEngine::strict_default();
        let response = engine
            .write(&query)
            .expect("generated query should execute");
        assert_eq!(response.status, CypherResponseStatus::Success);
        let nodes = engine
            .graph()
            .list_nodes()
            .expect("generated query should create one graph node");
        assert_eq!(nodes.len(), 1, "query={query}; response={response:?}");
        assert!(nodes[0].has_label("OpenCtiObject"));
        assert!(!nodes[0].has_label("Identity"));
        assert_eq!(
            nodes[0].property("opencti_raw"),
            Some(&PropertyValue::String(STANDARD.encode(raw.to_string())))
        );
    }
}
