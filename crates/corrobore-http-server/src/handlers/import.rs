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
use std::time::Duration;

use axum::{
    Json,
    extract::{Multipart, State},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use corrobore_engine::{CypherResponseStatus, EngineError, EngineRequest, EngineRequestMode};
use opencti_adapter::{MappedRecord, OpenCtiAdapter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{app::AppState, error::ApiError};

#[derive(Debug, Deserialize)]
pub struct ImportStixRequest {
    pub bundle: Value,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub budget_ref: Option<String>,
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
    let result = import_bundle_with_context(
        &state,
        payload.bundle,
        payload.workspace_id,
        payload.session_id,
        payload.budget_ref,
    )
    .await?;

    Ok(Json(ImportStixResponse { ok: true, result }))
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
    let object_queries = build_queries_from_bundle(&bundle)?;

    let workspace_id = workspace_id.unwrap_or_else(|| "workspace--http-default".to_owned());
    let session_id = session_id.unwrap_or_else(|| "session--http-import-stix".to_owned());
    let budget_ref = budget_ref.unwrap_or_else(|| "budget--http-import-stix".to_owned());

    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    let engine = state.engine.clone();

    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut locked = engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;

            let mut applied_mutations = 0usize;
            let mut rejected_mutations = 0usize;
            let mut errors = Vec::new();

            for query in &object_queries {
                let request = EngineRequest::new(query, EngineRequestMode::Mutation)
                    .with_workspace_id(workspace_id.clone())
                    .with_session_id(session_id.clone())
                    .with_budget_ref(budget_ref.clone());

                let response = locked.execute_request(request).map_err(map_engine_error)?;

                match response.status {
                    CypherResponseStatus::Success => {
                        applied_mutations += 1;
                    }
                    CypherResponseStatus::Rejected | CypherResponseStatus::ValidationFailed => {
                        rejected_mutations += 1;
                        if let Some(first_error) = response.validation_errors.first() {
                            errors.push(first_error.message.clone());
                        } else {
                            errors.push("mutation rejected during import".to_owned());
                        }
                    }
                }
            }

            Ok::<_, ApiError>(ImportStixResult {
                processed_objects: object_queries.len(),
                applied_mutations,
                rejected_mutations,
                errors,
            })
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "stix import timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;

    Ok(result)
}

fn map_engine_error(error: EngineError) -> ApiError {
    match error {
        EngineError::InvalidConfiguration { field, reason } => {
            let code = match field {
                "workspace_id" => "INVALID_WORKSPACE_ID",
                "session_id" => "INVALID_SESSION_ID",
                "budget_ref" => "INVALID_BUDGET_REF",
                _ => "INVALID_REQUEST",
            };
            ApiError::bad_request(code, reason)
        }
        other => ApiError::bad_request(
            "RUNTIME_ERROR",
            format!("stix import mutation failed: {other}"),
        ),
    }
}

fn has_supported_stix_extension(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.ends_with(".json") || lower.ends_with(".stix")
}

fn build_queries_from_bundle(bundle: &Value) -> Result<Vec<String>, ApiError> {
    let bundle_type = bundle
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if bundle_type != "bundle" {
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

    let mut queries = Vec::with_capacity(objects.len());
    for object in objects {
        queries.push(build_query_for_object(object)?);
    }

    Ok(queries)
}

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

fn required_str_field<'a>(object: &'a Value, field: &'static str) -> Result<&'a str, ApiError> {
    object.get(field).and_then(Value::as_str).ok_or_else(|| {
        ApiError::bad_request("INVALID_STIX_OBJECT", format!("object.{field} is required"))
    })
}

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
