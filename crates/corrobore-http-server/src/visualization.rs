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
//! Stable, bounded read contracts for temporal graph visualization clients.
//!
//! This module deliberately separates browser-facing data transfer objects from
//! graph-core storage models. Session lookup and temporal materialization stay
//! in the service layer; this projection only receives an already resolved graph.

use std::collections::{BTreeMap, BTreeSet};

use chrono::DateTime;
use graph_core::{Graph, PropertyMap, PropertyValue, RecordStatus};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hard server limit for nodes returned by one visualization projection.
pub const MAX_VISUALIZATION_NODES: usize = 50_000;
/// Hard server limit for relationships returned by one visualization projection.
pub const MAX_VISUALIZATION_RELATIONSHIPS: usize = 100_000;
/// Hard server limit for properties returned on one graph record.
pub const MAX_VISUALIZATION_PROPERTIES_PER_RECORD: usize = 256;
/// Hard server limit for a serialized visualization response.
pub const MAX_VISUALIZATION_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
/// Hard server limit for deterministic projection work units.
pub const MAX_VISUALIZATION_COMPUTATION_UNITS: usize = 1_000_000;

const MIN_VISUALIZATION_PAYLOAD_BYTES: usize = 1_024;

/// Failures exposed by the bounded visualization projection contract.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VisualizationProjectionError {
    /// A caller-supplied projection budget is outside supported limits.
    #[error(
        "invalid visualization budget {field}={requested}; expected {minimum}..={maximum}: {fix_hint}"
    )]
    InvalidBudget {
        /// Name of the invalid budget field.
        field: String,
        /// Value supplied by the caller.
        requested: usize,
        /// Inclusive supported minimum.
        minimum: usize,
        /// Inclusive supported maximum.
        maximum: usize,
        /// Actionable remediation for the caller.
        fix_hint: String,
    },
    /// A temporal boundary contains incomplete or malformed identity data.
    #[error("invalid temporal boundary field {field}: {fix_hint}")]
    InvalidTemporalBoundary {
        /// Name of the invalid boundary field.
        field: String,
        /// Actionable remediation for the caller.
        fix_hint: String,
    },
    /// The graph could not provide a consistent resolved view.
    #[error("graph projection failed: {message}")]
    GraphProjection {
        /// Stable diagnostic suitable for logs and API error responses.
        message: String,
    },
    /// Even an empty bounded response cannot fit the payload budget.
    #[error("visualization metadata exceeds the payload budget of {max_payload_bytes} bytes")]
    PayloadBudgetTooSmall {
        /// Requested serialized payload ceiling.
        max_payload_bytes: usize,
    },
}

/// Caller-controlled limits applied by the server before returning graph data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualizationProjectionBudget {
    /// Maximum returned nodes.
    pub max_nodes: usize,
    /// Maximum returned relationships.
    pub max_relationships: usize,
    /// Maximum returned properties per node or relationship.
    pub max_properties_per_record: usize,
    /// Maximum serialized response size in bytes.
    pub max_payload_bytes: usize,
    /// Maximum deterministic projection work units.
    pub max_computation_units: usize,
}

impl VisualizationProjectionBudget {
    /// Build and validate a projection budget against server hard limits.
    pub fn new(
        max_nodes: usize,
        max_relationships: usize,
        max_properties_per_record: usize,
        max_payload_bytes: usize,
        max_computation_units: usize,
    ) -> Result<Self, VisualizationProjectionError> {
        let budget = Self {
            max_nodes,
            max_relationships,
            max_properties_per_record,
            max_payload_bytes,
            max_computation_units,
        };
        budget.validate()?;
        Ok(budget)
    }

    /// Revalidate a deserialized or directly constructed budget at execution time.
    pub fn validate(&self) -> Result<(), VisualizationProjectionError> {
        validate_budget("max_nodes", self.max_nodes, 1, MAX_VISUALIZATION_NODES)?;
        validate_budget(
            "max_relationships",
            self.max_relationships,
            1,
            MAX_VISUALIZATION_RELATIONSHIPS,
        )?;
        validate_budget(
            "max_properties_per_record",
            self.max_properties_per_record,
            1,
            MAX_VISUALIZATION_PROPERTIES_PER_RECORD,
        )?;
        validate_budget(
            "max_payload_bytes",
            self.max_payload_bytes,
            MIN_VISUALIZATION_PAYLOAD_BYTES,
            MAX_VISUALIZATION_PAYLOAD_BYTES,
        )?;
        validate_budget(
            "max_computation_units",
            self.max_computation_units,
            1,
            MAX_VISUALIZATION_COMPUTATION_UNITS,
        )?;
        Ok(())
    }
}

fn validate_budget(
    field: &str,
    requested: usize,
    minimum: usize,
    maximum: usize,
) -> Result<(), VisualizationProjectionError> {
    if !(minimum..=maximum).contains(&requested) {
        return Err(VisualizationProjectionError::InvalidBudget {
            field: field.to_owned(),
            requested,
            minimum,
            maximum,
            fix_hint: format!("choose {field} between {minimum} and {maximum}"),
        });
    }
    Ok(())
}

/// Identity of the already-resolved temporal graph supplied to the projector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisualizationTemporalBoundary {
    /// Current graph state.
    Current,
    /// Named persisted snapshot at a transaction and timestamp.
    Snapshot {
        /// Stable snapshot identifier.
        boundary_id: String,
        /// Transaction that produced the snapshot.
        transaction_id: String,
        /// RFC 3339 timestamp associated with the snapshot.
        at: String,
    },
    /// Analytical point-in-time view, optionally anchored to a transaction.
    Timeshot {
        /// Stable timeshot identifier.
        boundary_id: String,
        /// Optional transaction anchor.
        transaction_id: Option<String>,
        /// RFC 3339 timestamp resolved by the service.
        at: String,
    },
}

impl VisualizationTemporalBoundary {
    /// Construct a current-state boundary.
    pub fn current() -> Self {
        Self::Current
    }

    /// Construct a persisted snapshot boundary.
    pub fn snapshot(
        boundary_id: impl Into<String>,
        transaction_id: impl Into<String>,
        at: impl Into<String>,
    ) -> Result<Self, VisualizationProjectionError> {
        let boundary_id = required_boundary_value("boundary_id", boundary_id.into())?;
        let transaction_id = required_boundary_value("transaction_id", transaction_id.into())?;
        let at = required_timestamp(at.into())?;
        Ok(Self::Snapshot {
            boundary_id,
            transaction_id,
            at,
        })
    }

    /// Construct an analytical point-in-time boundary.
    pub fn timeshot(
        boundary_id: impl Into<String>,
        transaction_id: Option<impl Into<String>>,
        at: impl Into<String>,
    ) -> Result<Self, VisualizationProjectionError> {
        let boundary_id = required_boundary_value("boundary_id", boundary_id.into())?;
        let transaction_id = transaction_id
            .map(Into::into)
            .map(|value| required_boundary_value("transaction_id", value))
            .transpose()?;
        let at = required_timestamp(at.into())?;
        Ok(Self::Timeshot {
            boundary_id,
            transaction_id,
            at,
        })
    }

    /// Return the stable boundary discriminator used by API clients.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Snapshot { .. } => "snapshot",
            Self::Timeshot { .. } => "timeshot",
        }
    }

    /// Return the snapshot or timeshot identifier, if applicable.
    pub fn boundary_id(&self) -> Option<&str> {
        match self {
            Self::Current => None,
            Self::Snapshot { boundary_id, .. } | Self::Timeshot { boundary_id, .. } => {
                Some(boundary_id)
            }
        }
    }

    /// Return the transaction anchor, if one exists.
    pub fn transaction_id(&self) -> Option<&str> {
        match self {
            Self::Current => None,
            Self::Snapshot { transaction_id, .. } => Some(transaction_id),
            Self::Timeshot { transaction_id, .. } => transaction_id.as_deref(),
        }
    }

    /// Return the resolved timestamp, if applicable.
    pub fn at(&self) -> Option<&str> {
        match self {
            Self::Current => None,
            Self::Snapshot { at, .. } | Self::Timeshot { at, .. } => Some(at),
        }
    }
}

fn required_boundary_value(
    field: &str,
    value: String,
) -> Result<String, VisualizationProjectionError> {
    if value.trim().is_empty() {
        return Err(VisualizationProjectionError::InvalidTemporalBoundary {
            field: field.to_owned(),
            fix_hint: format!("provide a non-empty {field}"),
        });
    }
    Ok(value)
}

fn required_timestamp(value: String) -> Result<String, VisualizationProjectionError> {
    let value = required_boundary_value("at", value)?;
    DateTime::parse_from_rfc3339(&value).map_err(|_| {
        VisualizationProjectionError::InvalidTemporalBoundary {
            field: "at".to_owned(),
            fix_hint: "provide an RFC 3339 timestamp such as 2026-07-17T00:00:00Z".to_owned(),
        }
    })?;
    Ok(value)
}

/// Complete request for one deterministic graph projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualizationProjectionRequest {
    /// Temporal identity already resolved by the service layer.
    pub boundary: VisualizationTemporalBoundary,
    /// Resource and response limits for this projection.
    pub budget: VisualizationProjectionBudget,
}

impl VisualizationProjectionRequest {
    /// Construct a projection request from a boundary and validated budget.
    pub fn new(
        boundary: VisualizationTemporalBoundary,
        budget: VisualizationProjectionBudget,
    ) -> Self {
        Self { boundary, budget }
    }
}

/// JSON-safe property value owned by the visualization read model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum VisualizationPropertyValue {
    /// Explicit null value.
    Null,
    /// Boolean scalar.
    Bool(bool),
    /// Signed integer scalar.
    Integer(i64),
    /// Floating-point scalar.
    Float(f64),
    /// Text scalar.
    String(String),
    /// Ordered text list.
    StringList(Vec<String>),
    /// Ordered integer list.
    IntegerList(Vec<i64>),
    /// Ordered floating-point list.
    FloatList(Vec<f64>),
    /// Ordered boolean list.
    BoolList(Vec<bool>),
}

/// Positive pheromone dimensions transported independently from graph-core.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VisualizationPheromoneVector {
    /// Access-frequency contribution.
    pub access_frequency: f64,
    /// Recency contribution.
    pub recency: f64,
    /// Downstream-success contribution.
    pub downstream_success: f64,
    /// Novelty contribution.
    pub novelty: f64,
    /// Information-gain contribution.
    pub information_gain: f64,
    /// Confidence-improvement contribution.
    pub confidence_improvement: f64,
    /// Contradiction-resolution contribution.
    pub contradiction_resolution: f64,
    /// Coverage contribution.
    pub coverage: f64,
    /// Analyst-feedback contribution.
    pub analyst_feedback: f64,
    /// Task-alignment contribution.
    pub task_alignment: f64,
}

/// Negative pheromone dimensions transported independently from graph-core.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VisualizationAntiPheromoneVector {
    /// Dead-end contribution.
    pub dead_end: f64,
    /// Redundancy contribution.
    pub redundancy: f64,
    /// Contradictory-path contribution.
    pub contradictory_path: f64,
    /// Low-value contribution.
    pub low_value: f64,
    /// Stale-path contribution.
    pub stale_path: f64,
    /// Budget-overrun contribution.
    pub budget_overrun: f64,
}

/// Raw navigation field attached to a projected relationship.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualizationNavigationField {
    /// Loading or investigation scope that owns the field.
    pub scope: String,
    /// Monotonic field tick used for decay and replay.
    pub tick: u64,
    /// Positive pheromone vector, when observed.
    pub positive: Option<VisualizationPheromoneVector>,
    /// Negative pheromone vector, when observed.
    pub negative: Option<VisualizationAntiPheromoneVector>,
}

/// Stable browser-facing node record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualizationNode {
    /// Stable graph identifier.
    pub id: String,
    /// Stable graph version identifier.
    pub version_id: String,
    /// Monotonic graph version.
    pub version: u64,
    /// Sorted semantic labels.
    pub labels: Vec<String>,
    /// Deterministically ordered bounded properties.
    pub properties: BTreeMap<String, VisualizationPropertyValue>,
    /// Stable lifecycle status string.
    pub status: String,
    /// Optional validated confidence value.
    pub confidence: Option<f64>,
    /// Optional first-observed timestamp.
    pub first_seen: Option<String>,
    /// Optional last-observed timestamp.
    pub last_seen: Option<String>,
}

/// Stable browser-facing relationship record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualizationRelationship {
    /// Stable graph identifier.
    pub id: String,
    /// Stable graph version identifier.
    pub version_id: String,
    /// Monotonic graph version.
    pub version: u64,
    /// Source node identifier.
    pub source: String,
    /// Target node identifier.
    pub target: String,
    /// Semantic relationship type.
    pub relationship_type: String,
    /// Deterministically ordered bounded properties.
    pub properties: BTreeMap<String, VisualizationPropertyValue>,
    /// Stable lifecycle status string.
    pub status: String,
    /// Optional validated confidence value.
    pub confidence: Option<f64>,
    /// Optional first-observed timestamp.
    pub first_seen: Option<String>,
    /// Optional last-observed timestamp.
    pub last_seen: Option<String>,
    /// Optional raw navigation field for later hot/cold classification.
    pub navigation: Option<VisualizationNavigationField>,
}

/// Exact accounting for records omitted by a bounded projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualizationProjectionMetadata {
    /// Limits requested by the caller after validation.
    pub requested_budget: VisualizationProjectionBudget,
    /// Limits actually applied by this server version.
    pub applied_budget: VisualizationProjectionBudget,
    /// Whether any record or property was omitted.
    pub partial: bool,
    /// Number of returned nodes.
    pub returned_nodes: usize,
    /// Number of omitted nodes.
    pub omitted_nodes: usize,
    /// Number of returned relationships.
    pub returned_relationships: usize,
    /// Number of omitted relationships.
    pub omitted_relationships: usize,
    /// Number of omitted properties across returned and payload-trimmed records.
    pub omitted_properties: usize,
    /// Deterministic work units consumed by the projection.
    pub computation_units: usize,
}

/// Serializable graph payload consumed by the temporal 3D explorer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualizationProjectionResponse {
    /// Resolved temporal identity represented by this payload.
    pub boundary: VisualizationTemporalBoundary,
    /// Deterministically ordered nodes.
    pub nodes: Vec<VisualizationNode>,
    /// Deterministically ordered relationships with no dangling endpoints.
    pub relationships: Vec<VisualizationRelationship>,
    /// Bounded-projection accounting.
    pub metadata: VisualizationProjectionMetadata,
}

/// Project an already-resolved graph into the stable visualization read model.
///
pub fn project_resolved_graph(
    graph: &Graph,
    request: &VisualizationProjectionRequest,
    navigation: &BTreeMap<String, VisualizationNavigationField>,
) -> Result<VisualizationProjectionResponse, VisualizationProjectionError> {
    request.budget.validate()?;
    let graph_nodes = graph.list_nodes().map_err(graph_projection_error)?;
    let graph_relationships = graph.list_relationships().map_err(graph_projection_error)?;
    let total_nodes = graph_nodes.len();
    let total_relationships = graph_relationships.len();
    let mut computation_units = 0usize;
    let mut omitted_properties = 0usize;
    let mut nodes = Vec::new();

    for node in graph_nodes {
        if nodes.len() >= request.budget.max_nodes
            || computation_units >= request.budget.max_computation_units
        {
            omitted_properties = omitted_properties.saturating_add(node.properties().len());
            continue;
        }

        computation_units += 1;
        let (properties, omitted, property_units) = project_properties(
            node.properties(),
            request.budget.max_properties_per_record,
            request
                .budget
                .max_computation_units
                .saturating_sub(computation_units),
        );
        computation_units += property_units;
        omitted_properties += omitted;
        let mut labels = node.labels().to_vec();
        labels.sort();
        nodes.push(VisualizationNode {
            id: node.id().as_str().to_owned(),
            version_id: node.version_id().as_str().to_owned(),
            version: node.version(),
            labels,
            properties,
            status: status_name(node.status()).to_owned(),
            confidence: node.confidence().map(|value| value.value()),
            first_seen: node.first_seen().map(str::to_owned),
            last_seen: node.last_seen().map(str::to_owned),
        });
    }

    let returned_node_ids: BTreeSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let mut relationships = Vec::new();
    for relationship in graph_relationships {
        let endpoints_returned = returned_node_ids.contains(relationship.source().as_str())
            && returned_node_ids.contains(relationship.target().as_str());
        let has_record_capacity = relationships.len() < request.budget.max_relationships;
        let has_computation_capacity = computation_units < request.budget.max_computation_units;
        if !endpoints_returned || !has_record_capacity || !has_computation_capacity {
            omitted_properties = omitted_properties.saturating_add(relationship.properties().len());
            continue;
        }

        computation_units += 1;
        let (properties, omitted, property_units) = project_properties(
            relationship.properties(),
            request.budget.max_properties_per_record,
            request
                .budget
                .max_computation_units
                .saturating_sub(computation_units),
        );
        computation_units += property_units;
        omitted_properties += omitted;
        relationships.push(VisualizationRelationship {
            id: relationship.id().as_str().to_owned(),
            version_id: relationship.version_id().as_str().to_owned(),
            version: relationship.version(),
            source: relationship.source().as_str().to_owned(),
            target: relationship.target().as_str().to_owned(),
            relationship_type: relationship.rel_type().as_str().to_owned(),
            properties,
            status: status_name(relationship.status()).to_owned(),
            confidence: relationship.confidence().map(|value| value.value()),
            first_seen: relationship.first_seen().map(str::to_owned),
            last_seen: relationship.last_seen().map(str::to_owned),
            navigation: navigation.get(relationship.id().as_str()).cloned(),
        });
    }

    let mut response = VisualizationProjectionResponse {
        boundary: request.boundary.clone(),
        metadata: VisualizationProjectionMetadata {
            requested_budget: request.budget.clone(),
            applied_budget: request.budget.clone(),
            partial: false,
            returned_nodes: nodes.len(),
            omitted_nodes: total_nodes.saturating_sub(nodes.len()),
            returned_relationships: relationships.len(),
            omitted_relationships: total_relationships.saturating_sub(relationships.len()),
            omitted_properties,
            computation_units,
        },
        nodes,
        relationships,
    };
    refresh_partial_flag(&mut response.metadata);
    fit_payload_budget(&mut response, request.budget.max_payload_bytes)?;
    Ok(response)
}

fn graph_projection_error(error: graph_core::GraphError) -> VisualizationProjectionError {
    VisualizationProjectionError::GraphProjection {
        message: error.to_string(),
    }
}

fn project_properties(
    properties: &PropertyMap,
    max_properties: usize,
    remaining_computation_units: usize,
) -> (BTreeMap<String, VisualizationPropertyValue>, usize, usize) {
    let returned_count = properties
        .len()
        .min(max_properties)
        .min(remaining_computation_units);
    let projected = properties
        .iter()
        .map(|(key, value)| (key.clone(), visualization_property(value)))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .take(returned_count)
        .collect();
    (
        projected,
        properties.len().saturating_sub(returned_count),
        returned_count,
    )
}

fn visualization_property(value: &PropertyValue) -> VisualizationPropertyValue {
    match value {
        PropertyValue::Null => VisualizationPropertyValue::Null,
        PropertyValue::Bool(value) => VisualizationPropertyValue::Bool(*value),
        PropertyValue::Integer(value) => VisualizationPropertyValue::Integer(*value),
        PropertyValue::Float(value) => VisualizationPropertyValue::Float(*value),
        PropertyValue::String(value) => VisualizationPropertyValue::String(value.clone()),
        PropertyValue::StringList(value) => VisualizationPropertyValue::StringList(value.clone()),
        PropertyValue::IntegerList(value) => VisualizationPropertyValue::IntegerList(value.clone()),
        PropertyValue::FloatList(value) => VisualizationPropertyValue::FloatList(value.clone()),
        PropertyValue::BoolList(value) => VisualizationPropertyValue::BoolList(value.clone()),
    }
}

fn status_name(status: RecordStatus) -> &'static str {
    match status {
        RecordStatus::Candidate => "candidate",
        RecordStatus::NeedsEvidence => "needs_evidence",
        RecordStatus::NeedsReview => "needs_review",
        RecordStatus::Validated => "validated",
        RecordStatus::Rejected => "rejected",
        RecordStatus::Exportable => "exportable",
        RecordStatus::Exported => "exported",
        RecordStatus::Tombstoned => "tombstoned",
    }
}

fn fit_payload_budget(
    response: &mut VisualizationProjectionResponse,
    max_payload_bytes: usize,
) -> Result<(), VisualizationProjectionError> {
    let mut current_size = serialized_size(response)?;
    while current_size > max_payload_bytes {
        let metadata_size_before = serialized_value_size(&response.metadata)?;
        if let Some(removed_size) = remove_last_property(response)? {
            response.metadata.omitted_properties += 1;
            refresh_partial_flag(&mut response.metadata);
            update_serialized_size_for_metadata(
                &mut current_size,
                removed_size,
                metadata_size_before,
                serialized_value_size(&response.metadata)?,
            );
            continue;
        }

        if let Some(relationship) = response.relationships.pop() {
            let removed_size = serialized_value_size(&relationship)?
                + usize::from(!response.relationships.is_empty());
            response.metadata.returned_relationships -= 1;
            response.metadata.omitted_relationships += 1;
            refresh_partial_flag(&mut response.metadata);
            update_serialized_size_for_metadata(
                &mut current_size,
                removed_size,
                metadata_size_before,
                serialized_value_size(&response.metadata)?,
            );
            continue;
        }

        if let Some(node) = response.nodes.pop() {
            let removed_size =
                serialized_value_size(&node)? + usize::from(!response.nodes.is_empty());
            response.metadata.returned_nodes -= 1;
            response.metadata.omitted_nodes += 1;
            refresh_partial_flag(&mut response.metadata);
            update_serialized_size_for_metadata(
                &mut current_size,
                removed_size,
                metadata_size_before,
                serialized_value_size(&response.metadata)?,
            );
            continue;
        }
        return Err(VisualizationProjectionError::PayloadBudgetTooSmall { max_payload_bytes });
    }
    debug_assert_eq!(current_size, serialized_size(response)?);
    Ok(())
}

fn serialized_size(
    response: &VisualizationProjectionResponse,
) -> Result<usize, VisualizationProjectionError> {
    serde_json::to_vec(response)
        .map(|payload| payload.len())
        .map_err(|error| VisualizationProjectionError::GraphProjection {
            message: format!("visualization response serialization failed: {error}"),
        })
}

fn serialized_value_size(value: &impl Serialize) -> Result<usize, VisualizationProjectionError> {
    serde_json::to_vec(value)
        .map(|payload| payload.len())
        .map_err(|error| VisualizationProjectionError::GraphProjection {
            message: format!("visualization value serialization failed: {error}"),
        })
}

fn remove_last_property(
    response: &mut VisualizationProjectionResponse,
) -> Result<Option<usize>, VisualizationProjectionError> {
    for relationship in response.relationships.iter_mut().rev() {
        if let Some(removed_size) = pop_last_property(&mut relationship.properties)? {
            return Ok(Some(removed_size));
        }
    }
    for node in response.nodes.iter_mut().rev() {
        if let Some(removed_size) = pop_last_property(&mut node.properties)? {
            return Ok(Some(removed_size));
        }
    }
    Ok(None)
}

fn pop_last_property(
    properties: &mut BTreeMap<String, VisualizationPropertyValue>,
) -> Result<Option<usize>, VisualizationProjectionError> {
    let had_multiple_properties = properties.len() > 1;
    let Some((key, value)) = properties.pop_last() else {
        return Ok(None);
    };
    let removed_size = serialized_value_size(&key)?
        + 1
        + serialized_value_size(&value)?
        + usize::from(had_multiple_properties);
    Ok(Some(removed_size))
}

fn update_serialized_size_for_metadata(
    current_size: &mut usize,
    removed_size: usize,
    metadata_size_before: usize,
    metadata_size_after: usize,
) {
    *current_size = current_size
        .saturating_sub(removed_size)
        .saturating_sub(metadata_size_before)
        .saturating_add(metadata_size_after);
}

fn refresh_partial_flag(metadata: &mut VisualizationProjectionMetadata) {
    metadata.partial = metadata.omitted_nodes > 0
        || metadata.omitted_relationships > 0
        || metadata.omitted_properties > 0;
}
