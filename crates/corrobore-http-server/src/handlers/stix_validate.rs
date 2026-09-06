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

use axum::{Json, extract::State};
use chrono::{SecondsFormat, Utc};
use corrobore_engine::{EngineError, ExportMode, ExportProfile, StixExportOptions};
#[cfg(feature = "enterprise-cti")]
use domain_provider_abi::{
    DomainName, InvokeRequest, IssueSeverity, ProviderResponseStatus, SCHEMA_V1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::{
    app::AppState,
    error::ApiError,
    handlers::import::{ImportStixResult, import_bundle_with_context},
};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ValidateStixRequest {
    pub source: Option<String>,
    pub bundle: Option<Value>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub budget_ref: Option<String>,
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidateStixResponse {
    pub ok: bool,
    pub result: ValidateStixResult,
}

#[derive(Debug, Serialize)]
pub struct ValidateStixResult {
    pub source_mode: String,
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
    pub playbooks_applied: Vec<AppliedPlaybook>,
    pub corrections_summary: Option<CorrectionsSummary>,
    pub persistence: Option<ImportStixResult>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CorrectionsSummary {
    pub total_corrections: u64,
    pub by_field: BTreeMap<String, u64>,
    pub by_strategy: BTreeMap<String, u64>,
    pub by_playbook_id: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationIssue {
    pub code: String,
    pub message: String,
    pub field: Option<String>,
    pub severity: String,
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppliedPlaybook {
    pub id: String,
    pub description: String,
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct StructuredCorrection {
    field: String,
    strategy: String,
    value: Value,
    reason: String,
    playbook_id: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn validate_stix(
    State(state): State<AppState>,
    Json(payload): Json<ValidateStixRequest>,
) -> Result<Json<ValidateStixResponse>, ApiError> {
    let source_mode = parse_source_mode(payload.source.as_deref())?;

    if source_mode == "graph" {
        require_cti_provider(&state)?;
    }

    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    let engine = state.engine.clone();
    let domain_providers = state.domain_providers.clone();

    let workspace_id = payload.workspace_id.clone();
    let session_id = payload.session_id.clone();
    let budget_ref = payload.budget_ref.clone();
    let snapshot_id = payload.snapshot_id.clone();
    let source_mode_clone = source_mode.clone();
    let bundle_payload = payload.bundle.clone();

    // Run graph inspection + autofix on the blocking thread pool to avoid
    // starving the async runtime.
    let (issues, playbooks, corrections) = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            if source_mode_clone == "bundle" {
                // --- source=bundle: lightweight structural + field validation ---
                let bundle = bundle_payload.ok_or_else(|| {
                    ApiError::bad_request("MISSING_BUNDLE", "bundle is required for source=bundle")
                })?;

                validate_bundle_shape(&bundle)?;
                let (issues, playbooks, corrected_objects) =
                    validate_and_fix_bundle_objects(&bundle);
                Ok::<_, ApiError>((issues, playbooks, Some(corrected_objects)))
            } else {
                // --- source=graph: native domain validation ---
                let mut engine = engine
                    .lock()
                    .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
                engine.hydrate_full_graph().map_err(|error| {
                    ApiError::internal("GRAPH_HYDRATION_FAILED", error.to_string())
                })?;
                let (issues, playbooks) = validate_graph_nodes(
                    engine.graph(),
                    snapshot_id.as_deref(),
                    domain_providers.as_deref(),
                    true,
                )?;
                Ok::<_, ApiError>((issues, playbooks, None))
            }
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "stix validation timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;

    // Persist auto-corrected objects when source=bundle produced corrections.
    let corrections_summary = if source_mode == "bundle" {
        corrections
            .as_ref()
            .map(|objects| summarize_corrections(objects))
            .filter(|summary| summary.total_corrections > 0)
    } else {
        None
    };

    let persistence = if source_mode == "bundle" {
        if let Some(corrected_objects) = corrections {
            if !playbooks.is_empty() {
                let corrected_bundle = serde_json::json!({
                    "type": "bundle",
                    "objects": corrected_objects
                });
                Some(
                    import_bundle_with_context(
                        &state,
                        corrected_bundle,
                        workspace_id,
                        session_id,
                        budget_ref,
                    )
                    .await?,
                )
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let has_errors = issues.iter().any(|issue| issue.severity == "error");

    Ok(Json(ValidateStixResponse {
        ok: true,
        result: ValidateStixResult {
            source_mode,
            valid: !has_errors,
            issues,
            playbooks_applied: playbooks,
            corrections_summary,
            persistence,
            errors: Vec::new(),
        },
    }))
}

#[cfg(feature = "enterprise-cti")]
pub(crate) fn require_cti_provider(
    state: &AppState,
) -> Result<&crate::enterprise::registry::DomainProviderRegistry, ApiError> {
    if !state.config.is_module_licensed("cti") {
        return Err(ApiError::forbidden(
            "LICENSE_MODULE_MISSING",
            "graph-native STIX validation requires a valid cti enterprise license",
        ));
    }
    let provider = state.domain_providers.as_deref().ok_or_else(|| {
        ApiError::service_unavailable(
            "DOMAIN_PROVIDER_NOT_READY",
            "graph-native STIX validation requires a configured CTI provider",
        )
    })?;
    let status = provider.status(DomainName::Cti).ok_or_else(|| {
        ApiError::service_unavailable(
            "DOMAIN_PROVIDER_NOT_READY",
            "graph-native STIX validation requires a loaded CTI provider",
        )
    })?;
    if !status.ready {
        return Err(ApiError::service_unavailable(
            "DOMAIN_PROVIDER_NOT_READY",
            "graph-native STIX validation requires a ready CTI provider",
        ));
    }
    if !status.has_capability("node.validate", SCHEMA_V1) {
        return Err(ApiError::service_unavailable(
            "DOMAIN_PROVIDER_CAPABILITY_MISSING",
            "CTI provider does not expose node.validate/v1",
        ));
    }
    Ok(provider)
}

#[cfg(not(feature = "enterprise-cti"))]
pub(crate) fn require_cti_provider(
    _state: &AppState,
) -> Result<&crate::enterprise::registry::DomainProviderRegistry, ApiError> {
    Err(ApiError::forbidden(
        "FEATURE_NOT_AVAILABLE",
        "graph-native STIX validation requires enterprise-cti",
    ))
}

pub(crate) fn collect_cti_export_findings(
    state: &AppState,
    graph: &graph_core::Graph,
) -> Result<Vec<graph_core::ValidationErrorRecord>, ApiError> {
    let provider = require_cti_provider(state)?;
    let (issues, _) = validate_graph_nodes(graph, None, Some(provider), false)?;
    Ok(issues
        .into_iter()
        .filter_map(|issue| {
            let node_id = issue.node_id?;
            // Preserve legacy provider findings as diagnostics on export only.
            // The public validation endpoint still uses the original contract.
            if matches!(
                issue.code.as_str(),
                "CTI_CONFIDENCE_REQUIRED" | "CTI_CONFIDENCE_TOO_LOW"
            ) {
                return Some(graph_core::ValidationErrorRecord::new(
                    "EXPORT_LEGACY_CONFIDENCE_DIAGNOSTIC",
                    graph_core::ValidationErrorSeverity::Warning,
                    format!(
                        "{}: {} (display-only criterion; permission is governed by actionability)",
                        issue.code, issue.message
                    ),
                    graph_core::ValidationTarget::node(node_id),
                ));
            }

            Some(graph_core::ValidationErrorRecord::new(
                issue.code,
                if issue.severity == "error" {
                    graph_core::ValidationErrorSeverity::Error
                } else {
                    graph_core::ValidationErrorSeverity::Warning
                },
                issue.message,
                graph_core::ValidationTarget::node(node_id),
            ))
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Graph-native validation (source=graph)
// ---------------------------------------------------------------------------

#[cfg(feature = "enterprise-cti")]
fn validate_graph_nodes(
    graph: &graph_core::Graph,
    _snapshot_id: Option<&str>,
    providers: Option<&crate::enterprise::registry::DomainProviderRegistry>,
    legacy_scalar_validation: bool,
) -> Result<(Vec<ValidationIssue>, Vec<AppliedPlaybook>), ApiError> {
    let providers = providers.ok_or_else(|| {
        ApiError::service_unavailable(
            "DOMAIN_PROVIDER_NOT_READY",
            "graph-native STIX validation requires a ready CTI provider",
        )
    })?;
    let nodes = graph
        .list_nodes()
        .map_err(|error| ApiError::internal("GRAPH_LIST_FAILED", error.to_string()))?;

    let mut issues = Vec::new();

    for node in &nodes {
        let labels = node.labels().to_vec();
        if !graph_core::node_eligible_for_export_profile(&ExportProfile::StixMvp, node) {
            continue;
        }
        let external_id = node
            .property("stix_id")
            .or_else(|| node.property("external_id"))
            .and_then(|value| match value {
                graph_core::PropertyValue::String(value) => Some(value.clone()),
                _ => None,
            })
            .or_else(|| {
                node.property("opencti.raw").and_then(|value| match value {
                    graph_core::PropertyValue::Json(value) => {
                        value.get("id").and_then(Value::as_str).map(str::to_owned)
                    }
                    _ => None,
                })
            });
        let response = providers
            .invoke(InvokeRequest {
                schema_version: SCHEMA_V1.to_owned(),
                request_id: uuid::Uuid::new_v4().to_string(),
                domain: DomainName::Cti,
                operation: "node.validate".to_owned(),
                workspace_id: None,
                snapshot_id: None,
                payload: serde_json::json!({
                    "labels": labels,
                    "external_id": external_id,
                    "evidence_refs": node
                        .evidence_refs()
                        .iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>(),
                    "confidence": if legacy_scalar_validation { node.confidence().map(|value| value.value()) } else { None },
                }),
            })
            .map_err(|error| ApiError::bad_gateway("DOMAIN_PROVIDER_ERROR", error.to_string()))?;
        if response.status == ProviderResponseStatus::Failed {
            return Err(ApiError::bad_gateway(
                "DOMAIN_PROVIDER_ERROR",
                "CTI provider reported a failed validation operation",
            ));
        }

        for domain_issue in response.issues {
            issues.push(ValidationIssue {
                code: domain_issue.code,
                message: domain_issue.message,
                field: domain_issue.field,
                severity: match domain_issue.severity {
                    IssueSeverity::Error => "error".to_owned(),
                    IssueSeverity::Warning => "warning".to_owned(),
                },
                node_id: domain_issue
                    .node_id
                    .or_else(|| Some(node.id().as_str().to_owned())),
            });
        }
    }

    // Graph-native mode: no in-place corrections are applied automatically
    // (the agent should use targeted Cypher mutations based on the issues).
    Ok((issues, Vec::new()))
}

#[cfg(not(feature = "enterprise-cti"))]
fn validate_graph_nodes(
    _graph: &graph_core::Graph,
    _snapshot_id: Option<&str>,
    _providers: Option<&crate::enterprise::registry::DomainProviderRegistry>,
    _legacy_scalar_validation: bool,
) -> Result<(Vec<ValidationIssue>, Vec<AppliedPlaybook>), ApiError> {
    Err(ApiError::forbidden(
        "FEATURE_NOT_AVAILABLE",
        "graph-native STIX validation requires enterprise-cti",
    ))
}

// ---------------------------------------------------------------------------
// Bundle structural validation + field-level autofix (source=bundle)
// ---------------------------------------------------------------------------

fn validate_bundle_shape(bundle: &Value) -> Result<(), ApiError> {
    let bundle_type = bundle
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    if bundle_type != "bundle" {
        return Err(ApiError::bad_request(
            "INVALID_STIX_BUNDLE",
            "expected a STIX bundle object with type='bundle'",
        ));
    }

    if !bundle.get("objects").map(Value::is_array).unwrap_or(false) {
        return Err(ApiError::bad_request(
            "INVALID_STIX_BUNDLE",
            "bundle.objects must be an array",
        ));
    }

    Ok(())
}

/// Required fields per STIX object type (field name, issue code, message).
const REQUIRED_FIELDS: &[(&str, &str, &str)] = &[
    (
        "type",
        "STIX_TYPE_REQUIRED",
        "object must have a 'type' field",
    ),
    ("id", "STIX_ID_REQUIRED", "object must have an 'id' field"),
];

/// Object-type-specific required fields: (stix_type, field, code, message).
const TYPE_REQUIRED_FIELDS: &[(&str, &str, &str, &str)] = &[
    (
        "identity",
        "name",
        "STIX_IDENTITY_NAME_REQUIRED",
        "identity object requires 'name'",
    ),
    (
        "indicator",
        "pattern",
        "STIX_INDICATOR_PATTERN_REQUIRED",
        "indicator object requires 'pattern'",
    ),
    (
        "indicator",
        "pattern_type",
        "STIX_INDICATOR_PATTERN_TYPE_REQUIRED",
        "indicator object requires 'pattern_type'",
    ),
    (
        "indicator",
        "valid_from",
        "STIX_INDICATOR_VALID_FROM_REQUIRED",
        "indicator object requires 'valid_from'",
    ),
    (
        "malware",
        "is_family",
        "STIX_MALWARE_IS_FAMILY_REQUIRED",
        "malware object requires 'is_family'",
    ),
    (
        "relationship",
        "relationship_type",
        "STIX_REL_TYPE_REQUIRED",
        "relationship object requires 'relationship_type'",
    ),
    (
        "relationship",
        "source_ref",
        "STIX_REL_SOURCE_REQUIRED",
        "relationship object requires 'source_ref'",
    ),
    (
        "relationship",
        "target_ref",
        "STIX_REL_TARGET_REQUIRED",
        "relationship object requires 'target_ref'",
    ),
];

/// Autofix playbooks: (stix_type, missing_field, playbook_id, description, default_value).
const AUTOFIX_PLAYBOOKS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "identity",
        "name",
        "PLAYBOOK_FIX_IDENTITY_NAME",
        "fill missing identity.name with placeholder",
        "Unknown Identity",
    ),
    (
        "malware",
        "is_family",
        "PLAYBOOK_FIX_MALWARE_IS_FAMILY",
        "set malware.is_family to false",
        "false",
    ),
];

/// Temporal autofix playbooks: (stix_type, missing_field, playbook_id).
const TEMPORAL_AUTOFIX_PLAYBOOKS: &[(&str, &str, &str)] = &[
    (
        "indicator",
        "valid_from",
        "PLAYBOOK_FIX_INDICATOR_VALID_FROM_PROCESSING_UTC",
    ),
    (
        "report",
        "published",
        "PLAYBOOK_FIX_REPORT_PUBLISHED_PROCESSING_UTC",
    ),
    (
        "observed-data",
        "first_observed",
        "PLAYBOOK_FIX_OBSERVED_DATA_FIRST_OBSERVED_PROCESSING_UTC",
    ),
    (
        "observed-data",
        "last_observed",
        "PLAYBOOK_FIX_OBSERVED_DATA_LAST_OBSERVED_PROCESSING_UTC",
    ),
];

fn validate_and_fix_bundle_objects(
    bundle: &Value,
) -> (Vec<ValidationIssue>, Vec<AppliedPlaybook>, Vec<Value>) {
    let objects = bundle
        .get("objects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut all_issues = Vec::new();
    let mut all_playbooks = Vec::new();
    let mut corrected_objects = Vec::new();
    let processing_utc = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    for (idx, object) in objects.iter().enumerate() {
        let object_id = object
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| format!("object[{}]", idx));

        let object_type = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();

        // Universal required fields.
        for (field, code, message) in REQUIRED_FIELDS {
            if object.get(field).is_none() {
                all_issues.push(ValidationIssue {
                    code: code.to_string(),
                    message: message.to_string(),
                    field: Some(field.to_string()),
                    severity: "error".to_owned(),
                    node_id: Some(object_id.clone()),
                });
            }
        }

        // Type-specific required fields.
        for (stix_type, field, code, message) in TYPE_REQUIRED_FIELDS {
            if *stix_type == object_type && object.get(*field).is_none() {
                all_issues.push(ValidationIssue {
                    code: code.to_string(),
                    message: message.to_string(),
                    field: Some(field.to_string()),
                    severity: "error".to_owned(),
                    node_id: Some(object_id.clone()),
                });
            }
        }

        // Apply autofix playbooks.
        let mut patched = object.clone();
        for (stix_type, field, playbook_id, description, default_value) in AUTOFIX_PLAYBOOKS {
            if *stix_type == object_type
                && patched.get(field).is_none()
                && let Some(obj) = patched.as_object_mut()
            {
                // Parse the default_value into the appropriate JSON type.
                let json_value = if *default_value == "false" {
                    Value::Bool(false)
                } else if *default_value == "true" {
                    Value::Bool(true)
                } else {
                    Value::String(default_value.to_string())
                };
                obj.insert(field.to_string(), json_value);
                append_structured_correction(
                    obj,
                    StructuredCorrection {
                        field: (*field).to_owned(),
                        strategy: "playbook_default".to_owned(),
                        value: obj.get(*field).cloned().unwrap_or(Value::Null),
                        reason: format!("missing required field '{}'", field),
                        playbook_id: (*playbook_id).to_owned(),
                    },
                );
                all_playbooks.push(AppliedPlaybook {
                    id: playbook_id.to_string(),
                    description: description.to_string(),
                    node_id: object_id.clone(),
                });
            }
        }

        apply_temporal_autofix_playbooks(
            &mut patched,
            &object_type,
            &object_id,
            &processing_utc,
            &mut all_playbooks,
        );

        corrected_objects.push(patched);
    }

    (all_issues, all_playbooks, corrected_objects)
}

fn apply_temporal_autofix_playbooks(
    patched: &mut Value,
    object_type: &str,
    object_id: &str,
    processing_utc: &str,
    all_playbooks: &mut Vec<AppliedPlaybook>,
) {
    for (stix_type, field, playbook_id) in TEMPORAL_AUTOFIX_PLAYBOOKS {
        if *stix_type != object_type {
            continue;
        }

        let Some(obj) = patched.as_object_mut() else {
            continue;
        };
        if obj.get(*field).is_some() {
            continue;
        }

        obj.insert(
            (*field).to_owned(),
            Value::String(processing_utc.to_owned()),
        );
        append_structured_correction(
            obj,
            StructuredCorrection {
                field: (*field).to_owned(),
                strategy: "processing_utc_default".to_owned(),
                value: Value::String(processing_utc.to_owned()),
                reason: format!("missing required temporal field '{}'", field),
                playbook_id: (*playbook_id).to_owned(),
            },
        );
        append_temporal_autofix_note(obj, field, processing_utc);

        all_playbooks.push(AppliedPlaybook {
            id: (*playbook_id).to_owned(),
            description: format!(
                "set {}.{} to processing UTC and document substitution in description",
                stix_type, field
            ),
            node_id: object_id.to_owned(),
        });
    }
}

fn append_structured_correction(
    object: &mut serde_json::Map<String, Value>,
    correction: StructuredCorrection,
) {
    let correction_value = serde_json::to_value(&correction).unwrap_or(Value::Null);
    let correction_field = correction.field.clone();

    let array = object
        .entry("x_corrobore_corrections".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));

    if let Value::Array(items) = array {
        let already_present = items.iter().any(|item| {
            item.get("field")
                .and_then(Value::as_str)
                .map(|field| field == correction_field)
                .unwrap_or(false)
        });
        if !already_present {
            items.push(correction_value);
        }
    }
}

fn summarize_corrections(objects: &[Value]) -> CorrectionsSummary {
    let mut by_field = BTreeMap::new();
    let mut by_strategy = BTreeMap::new();
    let mut by_playbook_id = BTreeMap::new();
    let mut total_corrections = 0u64;

    for object in objects {
        let Some(corrections) = object
            .get("x_corrobore_corrections")
            .and_then(Value::as_array)
        else {
            continue;
        };

        for correction in corrections {
            total_corrections = total_corrections.saturating_add(1);

            if let Some(field) = correction.get("field").and_then(Value::as_str) {
                *by_field.entry(field.to_owned()).or_insert(0) += 1;
            }
            if let Some(strategy) = correction.get("strategy").and_then(Value::as_str) {
                *by_strategy.entry(strategy.to_owned()).or_insert(0) += 1;
            }
            if let Some(playbook_id) = correction.get("playbook_id").and_then(Value::as_str) {
                *by_playbook_id.entry(playbook_id.to_owned()).or_insert(0) += 1;
            }
        }
    }

    CorrectionsSummary {
        total_corrections,
        by_field,
        by_strategy,
        by_playbook_id,
    }
}

fn append_temporal_autofix_note(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    processing_utc: &str,
) {
    let note = format!(
        "Auto-correction: '{}' was missing in source and was defaulted to processing UTC {}.",
        field, processing_utc
    );

    match object.get_mut("description") {
        Some(Value::String(existing)) => {
            if existing.trim().is_empty() {
                *existing = note;
            } else if !existing.contains(&format!("'{}'", field)) {
                existing.push_str("\n\n");
                existing.push_str(&note);
            }
        }
        _ => {
            object.insert("description".to_owned(), Value::String(note));
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_source_mode(source: Option<&str>) -> Result<String, ApiError> {
    match source.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) if value == "bundle" => Ok("bundle".to_owned()),
        Some(value) if value == "graph" => Ok("graph".to_owned()),
        Some(value) => Err(ApiError::bad_request(
            "INVALID_SOURCE_MODE",
            format!("unsupported stix validation source: {value}"),
        )),
        None => Ok("bundle".to_owned()),
    }
}

/// Build a STIX bundle from the current graph (used by export endpoint, kept here
/// for completeness — the validate endpoint uses graph-native validation directly).
pub async fn build_bundle_from_graph(
    state: &AppState,
    snapshot_id: Option<String>,
) -> Result<Value, ApiError> {
    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    let engine = state.engine.clone();

    tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let engine = engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;

            let options = StixExportOptions {
                snapshot_id: snapshot_id.unwrap_or_else(|| "snapshot--current".to_owned()),
                transaction_id: "transaction--stix-validate-graph".to_owned(),
                exporter_version: "corrobore-http-server-validate-v0".to_owned(),
                profile: ExportProfile::StixMvp,
                mode: ExportMode::Strict,
                force: false,
            };

            let bundle = engine
                .export_stix_bundle(&options)
                .map_err(map_engine_export_error)?;
            serde_json::to_value(bundle)
                .map_err(|error| ApiError::internal("SERIALIZATION_FAILED", error.to_string()))
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "graph to stix conversion timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))?
}

fn map_engine_export_error(error: EngineError) -> ApiError {
    match error {
        EngineError::InvalidConfiguration {
            field: "transaction_id",
            reason,
        } => ApiError::bad_request("INVALID_TRANSACTION_ID", reason),
        EngineError::InvalidConfiguration { reason, .. } => {
            ApiError::bad_request("INVALID_EXPORT_METADATA", reason)
        }
        EngineError::Export(reason) => ApiError::bad_request("EXPORT_PLAN_FAILED", reason),
        other => ApiError::internal("EXPORT_FAILED", other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::validate_and_fix_bundle_objects;

    #[test]
    fn temporal_playbook_sets_processing_utc_and_description_note() {
        let bundle = json!({
            "type": "bundle",
            "objects": [
                {
                    "type": "indicator",
                    "id": "indicator--temporal-playbook-1",
                    "pattern": "[ipv4-addr:value = '198.51.100.23']",
                    "pattern_type": "stix"
                }
            ]
        });

        let (issues, playbooks, corrected) = validate_and_fix_bundle_objects(&bundle);

        assert!(
            issues
                .iter()
                .any(|i| i.code == "STIX_INDICATOR_VALID_FROM_REQUIRED")
        );
        assert!(
            playbooks
                .iter()
                .any(|p| p.id == "PLAYBOOK_FIX_INDICATOR_VALID_FROM_PROCESSING_UTC")
        );

        let corrected_indicator = corrected[0]
            .as_object()
            .expect("corrected indicator should be object");
        let valid_from = corrected_indicator
            .get("valid_from")
            .and_then(serde_json::Value::as_str)
            .expect("valid_from should be set by temporal playbook");
        assert!(valid_from.contains('T'));
        assert!(valid_from.ends_with('Z'));

        let description = corrected_indicator
            .get("description")
            .and_then(serde_json::Value::as_str)
            .expect("description should document temporal substitution");
        assert!(description.contains("Auto-correction:"));
        assert!(description.contains("valid_from"));

        let corrections = corrected_indicator
            .get("x_corrobore_corrections")
            .and_then(serde_json::Value::as_array)
            .expect("x_corrobore_corrections should be present");
        assert!(corrections.iter().any(|item| {
            item.get("field") == Some(&serde_json::Value::String("valid_from".to_owned()))
                && item.get("strategy")
                    == Some(&serde_json::Value::String(
                        "processing_utc_default".to_owned(),
                    ))
        }));
    }

    #[test]
    fn temporal_playbook_sets_report_published_and_description_note() {
        let bundle = json!({
            "type": "bundle",
            "objects": [
                {
                    "type": "report",
                    "id": "report--temporal-playbook-1",
                    "name": "Threat report"
                }
            ]
        });

        let (issues, playbooks, corrected) = validate_and_fix_bundle_objects(&bundle);

        assert!(issues.iter().all(|i| i.code != "STIX_TYPE_REQUIRED"));
        assert!(
            playbooks
                .iter()
                .any(|p| p.id == "PLAYBOOK_FIX_REPORT_PUBLISHED_PROCESSING_UTC")
        );

        let corrected_report = corrected[0]
            .as_object()
            .expect("corrected report should be object");
        let published = corrected_report
            .get("published")
            .and_then(serde_json::Value::as_str)
            .expect("published should be set by temporal playbook");
        assert!(published.contains('T'));
        assert!(published.ends_with('Z'));

        let description = corrected_report
            .get("description")
            .and_then(serde_json::Value::as_str)
            .expect("description should document temporal substitution");
        assert!(description.contains("published"));

        let corrections = corrected_report
            .get("x_corrobore_corrections")
            .and_then(serde_json::Value::as_array)
            .expect("x_corrobore_corrections should be present");
        assert!(corrections.iter().any(|item| {
            item.get("field") == Some(&serde_json::Value::String("published".to_owned()))
                && item.get("strategy")
                    == Some(&serde_json::Value::String(
                        "processing_utc_default".to_owned(),
                    ))
        }));
    }

    #[test]
    fn temporal_playbook_sets_observed_data_window_when_missing() {
        let bundle = json!({
            "type": "bundle",
            "objects": [
                {
                    "type": "observed-data",
                    "id": "observed-data--temporal-playbook-1",
                    "number_observed": 1,
                    "objects": {
                        "0": {
                            "type": "ipv4-addr",
                            "value": "198.51.100.44"
                        }
                    }
                }
            ]
        });

        let (_issues, playbooks, corrected) = validate_and_fix_bundle_objects(&bundle);

        assert!(
            playbooks
                .iter()
                .any(|p| { p.id == "PLAYBOOK_FIX_OBSERVED_DATA_FIRST_OBSERVED_PROCESSING_UTC" })
        );
        assert!(
            playbooks
                .iter()
                .any(|p| { p.id == "PLAYBOOK_FIX_OBSERVED_DATA_LAST_OBSERVED_PROCESSING_UTC" })
        );

        let corrected_observed_data = corrected[0]
            .as_object()
            .expect("corrected observed-data should be object");
        let first_observed = corrected_observed_data
            .get("first_observed")
            .and_then(serde_json::Value::as_str)
            .expect("first_observed should be set by temporal playbook");
        let last_observed = corrected_observed_data
            .get("last_observed")
            .and_then(serde_json::Value::as_str)
            .expect("last_observed should be set by temporal playbook");
        assert!(first_observed.contains('T'));
        assert!(first_observed.ends_with('Z'));
        assert!(last_observed.contains('T'));
        assert!(last_observed.ends_with('Z'));

        let description = corrected_observed_data
            .get("description")
            .and_then(serde_json::Value::as_str)
            .expect("description should document temporal substitutions");
        assert!(description.contains("first_observed"));
        assert!(description.contains("last_observed"));

        let corrections = corrected_observed_data
            .get("x_corrobore_corrections")
            .and_then(serde_json::Value::as_array)
            .expect("x_corrobore_corrections should be present");
        assert!(corrections.iter().any(|item| {
            item.get("field") == Some(&serde_json::Value::String("first_observed".to_owned()))
        }));
        assert!(corrections.iter().any(|item| {
            item.get("field") == Some(&serde_json::Value::String("last_observed".to_owned()))
        }));
    }
}
