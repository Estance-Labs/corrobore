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
use std::collections::HashMap;

use graph_core::{LabelSet, NodeId, RelationshipId, RelationshipType};
use serde::{Deserialize, Serialize};

use crate::{
    GraphCatalog, GraphRecordVersion, GraphStorageError, GraphStorageResult, StorageRef,
    StorageSegment, validate_storage_ref,
};

/// Storage-catalog node label key.
///
///
/// - Keep node label indexes keyed by the same string shape used by graph-core
///   `LabelSet`.
/// - Avoid introducing a storage-specific label wrapper before validation and
///   normalization rules are owned by a later issue.
pub type NodeLabel = String;

/// Lookup behavior for catalog metadata indexes when a key is not known.
///
///
/// - Make unknown-key behavior deterministic at the API boundary.
/// - Let strict callers receive explicit storage errors when an expected label or
///   relationship type is absent.
/// - Let exploratory callers request an empty result once the implementation is
///   added, without conflating absence with corruption or IO failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogIndexLookupMode {
    /// Missing labels or relationship types should become explicit catalog
    /// errors.
    Strict,

    /// Missing labels or relationship types should resolve to an empty result.
    EmptyWhenUnknown,
}

/// Catalog-level metadata indexes.
///
///
/// - Represent label to node ID mappings and relationship-type to relationship ID
///   mappings without loading full node or relationship payloads.
/// - Keep property indexes, semantic indexes, vector indexes, and adjacency
///   indexes out of this contract.
/// - Provide a compact shape that can later be embedded in `GraphCatalog` or
///   persisted alongside it during catalog rebuild.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCatalogIndexes {
    /// Node label index keyed by graph-core label value.
    pub labels: HashMap<NodeLabel, LabelIndexCatalogEntry>,

    /// Relationship type index keyed by graph-core relationship type value.
    pub relationship_types: HashMap<RelationshipType, RelationshipTypeIndexCatalogEntry>,
}

/// One label-index bucket in the storage catalog.
///
///
/// - Store every latest node known to carry a given label.
/// - Preserve enough lightweight metadata for future working-set expansion and
///   catalog-aware planning.
/// - Avoid copying node properties, evidence payloads, or full serialized node
///   records into the index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelIndexCatalogEntry {
    /// Label.
    pub label: NodeLabel,
    /// Nodes.
    pub nodes: Vec<LabelIndexNodeMetadata>,
}

/// Lightweight node metadata stored in a label index.
///
///
/// - Preserve the stable `NodeId` needed by graph traversal and working-set
///   loading.
/// - Preserve the latest storage reference when it is known so callers can page in
///   the node without scanning the append-only record log.
/// - Preserve version metadata only as catalog metadata, not as a full node
///   payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelIndexNodeMetadata {
    /// Node id.
    pub node_id: NodeId,
    /// Latest storage ref.
    pub latest_storage_ref: Option<StorageRef>,
    /// Graph record version.
    pub graph_record_version: Option<GraphRecordVersion>,
}

/// One relationship-type-index bucket in the storage catalog.
///
///
/// - Store every latest relationship known to carry a given relationship type.
/// - Preserve enough lightweight metadata for future type-filtered traversal,
///   loading profiles, and catalog-aware query planning.
/// - Avoid copying relationship properties, evidence payloads, or full serialized
///   relationship records into the index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipTypeIndexCatalogEntry {
    /// Relationship type.
    pub relationship_type: RelationshipType,
    /// Relationships.
    pub relationships: Vec<RelationshipTypeIndexRelationshipMetadata>,
}

/// Lightweight relationship metadata stored in a relationship-type index.
///
///
/// - Preserve the stable `RelationshipId` needed by traversal and storage lookup.
/// - Preserve the latest storage reference when it is known so callers can page in
///   the relationship without scanning the append-only record log.
/// - Preserve version metadata only as catalog metadata, not as a full
///   relationship payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipTypeIndexRelationshipMetadata {
    /// Relationship id.
    pub relationship_id: RelationshipId,
    /// Latest storage ref.
    pub latest_storage_ref: Option<StorageRef>,
    /// Graph record version.
    pub graph_record_version: Option<GraphRecordVersion>,
}

/// Index the labels attached to one latest node record.
///
///
/// - Declare the boundary that will be called after the latest node ID catalog is
///   updated.
/// - Register each label to the stable node ID and lightweight storage metadata.
/// - Keep label indexing separate from property indexing, semantic indexing, and
///   adjacency indexing.
///
///
/// - For every label in `labels`, the catalog should make `metadata.node_id`
///   discoverable by label lookup.
/// - Duplicate indexing of the same latest node for the same label should be
///   deterministic and should not create duplicate result IDs.
/// - Empty label sets should be accepted as a no-op.
///
/// # Errors
///
///
/// - Invalid or internally inconsistent metadata should become a typed storage
///   error.
/// - Duplicate conflicting metadata for the same label and node should become a
///   catalog consistency error in a later implementation phase.
pub fn index_node_labels(
    catalog: &mut GraphCatalog,
    labels: &LabelSet,
    metadata: LabelIndexNodeMetadata,
) -> GraphStorageResult<()> {
    validate_label_index_metadata(&metadata)?;

    for label in labels {
        validate_node_label(label)?;
        let entry = catalog
            .metadata_indexes
            .labels
            .entry(label.clone())
            .or_insert_with(|| LabelIndexCatalogEntry {
                label: label.clone(),
                nodes: Vec::new(),
            });
        upsert_label_node_metadata(&mut entry.nodes, metadata.clone());
    }

    Ok(())
}

/// Index the type attached to one latest relationship record.
///
///
/// - Declare the boundary that will be called after the latest relationship ID
///   catalog is updated.
/// - Register the relationship type to the stable relationship ID and lightweight
///   storage metadata.
/// - Keep relationship-type indexing separate from property indexing, semantic
///   indexing, and adjacency indexing.
///
///
/// - The catalog should make `metadata.relationship_id` discoverable by
///   relationship-type lookup.
/// - Duplicate indexing of the same latest relationship for the same type should
///   be deterministic and should not create duplicate result IDs.
///
/// # Errors
///
///
/// - Invalid or internally inconsistent metadata should become a typed storage
///   error.
/// - Duplicate conflicting metadata for the same relationship type and
///   relationship should become a catalog consistency error in a later
///   implementation phase.
pub fn index_relationship_type(
    catalog: &mut GraphCatalog,
    relationship_type: &RelationshipType,
    metadata: RelationshipTypeIndexRelationshipMetadata,
) -> GraphStorageResult<()> {
    validate_relationship_type_index_metadata(&metadata)?;

    let entry = catalog
        .metadata_indexes
        .relationship_types
        .entry(relationship_type.clone())
        .or_insert_with(|| RelationshipTypeIndexCatalogEntry {
            relationship_type: relationship_type.clone(),
            relationships: Vec::new(),
        });
    upsert_relationship_type_metadata(&mut entry.relationships, metadata);

    Ok(())
}

/// Resolve stable node IDs by label.
///
///
/// - Provide the lookup boundary future working-set loaders and planners will use
///   before loading node payloads.
/// - Return stable graph-core IDs, not node records.
/// - Make unknown-label behavior explicit through `lookup_mode`.
///
///
/// - `Strict` mode should return `GraphStorageError::UnknownLabelCatalogEntry`
///   when the label is not indexed.
/// - `EmptyWhenUnknown` mode should return an empty vector when the label is not
///   indexed.
/// - Known labels should return deterministic, duplicate-free node IDs.
///
/// # Errors
///
///
/// - Corrupted or inconsistent label index state should become a typed storage
///   error in a later implementation phase.
pub fn resolve_node_ids_by_label(
    catalog: &GraphCatalog,
    label: &str,
    lookup_mode: CatalogIndexLookupMode,
) -> GraphStorageResult<Vec<NodeId>> {
    Ok(resolve_label_index_entries(catalog, label, lookup_mode)?
        .into_iter()
        .map(|metadata| metadata.node_id)
        .collect())
}

/// Resolve lightweight node metadata by label.
///
///
/// - Let future pagers and query planners inspect label matches without loading
///   full node payloads.
/// - Preserve the option to page in only the records selected by a loading profile
///   or traversal budget.
/// - Keep this metadata lookup separate from property and semantic indexes.
///
///
/// - Known labels should return deterministic, duplicate-free node metadata.
/// - Unknown labels should follow `lookup_mode`.
///
/// # Errors
///
///
/// - Corrupted or inconsistent label index state should become a typed storage
///   error in a later implementation phase.
pub fn resolve_label_index_entries(
    catalog: &GraphCatalog,
    label: &str,
    lookup_mode: CatalogIndexLookupMode,
) -> GraphStorageResult<Vec<LabelIndexNodeMetadata>> {
    match catalog.metadata_indexes.labels.get(label) {
        Some(entry) => Ok(entry.nodes.clone()),
        None => resolve_unknown_label(label, lookup_mode),
    }
}

/// Resolve stable relationship IDs by relationship type.
///
///
/// - Provide the lookup boundary future traversal, loading-profile, and query
///   planner code will use before loading relationship payloads.
/// - Return stable graph-core IDs, not relationship records.
/// - Make unknown-relationship-type behavior explicit through `lookup_mode`.
///
///
/// - `Strict` mode should return
///   `GraphStorageError::UnknownRelationshipTypeCatalogEntry` when the type is
///   not indexed.
/// - `EmptyWhenUnknown` mode should return an empty vector when the type is not
///   indexed.
/// - Known relationship types should return deterministic, duplicate-free
///   relationship IDs.
///
/// # Errors
///
///
/// - Corrupted or inconsistent relationship-type index state should become a
///   typed storage error in a later implementation phase.
pub fn resolve_relationship_ids_by_type(
    catalog: &GraphCatalog,
    relationship_type: &RelationshipType,
    lookup_mode: CatalogIndexLookupMode,
) -> GraphStorageResult<Vec<RelationshipId>> {
    Ok(
        resolve_relationship_type_index_entries(catalog, relationship_type, lookup_mode)?
            .into_iter()
            .map(|metadata| metadata.relationship_id)
            .collect(),
    )
}

/// Resolve lightweight relationship metadata by relationship type.
///
///
/// - Let future pagers and query planners inspect type-filtered relationship
///   matches without loading full relationship payloads.
/// - Preserve the option to page in only the records selected by a loading profile
///   or traversal budget.
/// - Keep this metadata lookup separate from property, semantic, and adjacency
///   indexes.
///
///
/// - Known relationship types should return deterministic, duplicate-free
///   relationship metadata.
/// - Unknown relationship types should follow `lookup_mode`.
///
/// # Errors
///
///
/// - Corrupted or inconsistent relationship-type index state should become a
///   typed storage error in a later implementation phase.
pub fn resolve_relationship_type_index_entries(
    catalog: &GraphCatalog,
    relationship_type: &RelationshipType,
    lookup_mode: CatalogIndexLookupMode,
) -> GraphStorageResult<Vec<RelationshipTypeIndexRelationshipMetadata>> {
    match catalog
        .metadata_indexes
        .relationship_types
        .get(relationship_type)
    {
        Some(entry) => Ok(entry.relationships.clone()),
        None => resolve_unknown_relationship_type(relationship_type, lookup_mode),
    }
}

fn validate_node_label(label: &str) -> GraphStorageResult<()> {
    if label.trim().is_empty() {
        return Err(GraphStorageError::InvalidEnvelope {
            reason: "catalog label index cannot store an empty node label".to_owned(),
        });
    }
    Ok(())
}

fn validate_label_index_metadata(metadata: &LabelIndexNodeMetadata) -> GraphStorageResult<()> {
    if let Some(storage_ref) = &metadata.latest_storage_ref {
        validate_storage_ref(storage_ref)?;
        validate_storage_segment(storage_ref, StorageSegment::NodeRecords)?;
    }

    if let Some(graph_record_version) = &metadata.graph_record_version {
        match graph_record_version {
            GraphRecordVersion::Node { current: true, .. } => Ok(()),
            GraphRecordVersion::Node { current: false, .. } => {
                Err(GraphStorageError::InvalidEnvelope {
                    reason: "label index metadata must reference a current node version".to_owned(),
                })
            }
            GraphRecordVersion::Relationship { .. } => Err(GraphStorageError::InvalidEnvelope {
                reason: "label index metadata cannot reference a relationship version".to_owned(),
            }),
        }?;
    }

    Ok(())
}

fn validate_relationship_type_index_metadata(
    metadata: &RelationshipTypeIndexRelationshipMetadata,
) -> GraphStorageResult<()> {
    if let Some(storage_ref) = &metadata.latest_storage_ref {
        validate_storage_ref(storage_ref)?;
        validate_storage_segment(storage_ref, StorageSegment::RelationshipRecords)?;
    }

    if let Some(graph_record_version) = &metadata.graph_record_version {
        match graph_record_version {
 GraphRecordVersion::Relationship { current: true, .. } => Ok(()),
 GraphRecordVersion::Relationship { current: false, .. } => {
 Err(GraphStorageError::InvalidEnvelope {
 reason: "relationship-type index metadata must reference a current relationship version"
 .to_owned(),
 })
 }
 GraphRecordVersion::Node { .. } => Err(GraphStorageError::InvalidEnvelope {
 reason: "relationship-type index metadata cannot reference a node version".to_owned(),
 }),
 }?;
    }

    Ok(())
}

fn validate_storage_segment(
    storage_ref: &StorageRef,
    expected_segment: StorageSegment,
) -> GraphStorageResult<()> {
    if storage_ref.segment != expected_segment {
        return Err(GraphStorageError::InvalidStorageRef {
            storage_ref: storage_ref.clone(),
            reason: format!(
                "catalog metadata expected {:?}, got {:?}",
                expected_segment, storage_ref.segment
            ),
        });
    }
    Ok(())
}

fn upsert_label_node_metadata(
    nodes: &mut Vec<LabelIndexNodeMetadata>,
    metadata: LabelIndexNodeMetadata,
) {
    if let Some(existing) = nodes
        .iter_mut()
        .find(|existing| existing.node_id == metadata.node_id)
    {
        *existing = metadata;
    } else {
        nodes.push(metadata);
    }
}

fn upsert_relationship_type_metadata(
    relationships: &mut Vec<RelationshipTypeIndexRelationshipMetadata>,
    metadata: RelationshipTypeIndexRelationshipMetadata,
) {
    if let Some(existing) = relationships
        .iter_mut()
        .find(|existing| existing.relationship_id == metadata.relationship_id)
    {
        *existing = metadata;
    } else {
        relationships.push(metadata);
    }
}

fn resolve_unknown_label(
    label: &str,
    lookup_mode: CatalogIndexLookupMode,
) -> GraphStorageResult<Vec<LabelIndexNodeMetadata>> {
    match lookup_mode {
        CatalogIndexLookupMode::Strict => Err(GraphStorageError::UnknownLabelCatalogEntry {
            label: label.to_owned(),
        }),
        CatalogIndexLookupMode::EmptyWhenUnknown => Ok(Vec::new()),
    }
}

fn resolve_unknown_relationship_type(
    relationship_type: &RelationshipType,
    lookup_mode: CatalogIndexLookupMode,
) -> GraphStorageResult<Vec<RelationshipTypeIndexRelationshipMetadata>> {
    match lookup_mode {
        CatalogIndexLookupMode::Strict => {
            Err(GraphStorageError::UnknownRelationshipTypeCatalogEntry {
                relationship_type: relationship_type.clone(),
            })
        }
        CatalogIndexLookupMode::EmptyWhenUnknown => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{NodeVersionId, RelationshipVersionId};

    use crate::{RecordChecksum, StorageSegment};

    fn checksum(value: impl Into<String>) -> RecordChecksum {
        RecordChecksum {
            // Algorithm.
            algorithm: "sha256".to_owned(),
            // Value.
            value: value.into(),
        }
    }

    fn storage_ref(segment: StorageSegment, offset: u64) -> StorageRef {
        StorageRef {
            segment,
            offset,
            // Length.
            length: 64,
            // Checksum.
            checksum: Some(checksum(format!("checksum-{offset}"))),
        }
    }

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node id should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship id should be valid")
    }

    fn relationship_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
    }

    fn node_version(version_id: &str) -> GraphRecordVersion {
        GraphRecordVersion::Node {
            version_id: NodeVersionId::new(version_id)
                .expect("test node version id should be valid"),
            version: 1,
            current: true,
            previous_version_id: None,
        }
    }

    fn relationship_version(version_id: &str) -> GraphRecordVersion {
        GraphRecordVersion::Relationship {
            version_id: RelationshipVersionId::new(version_id)
                .expect("test relationship version id should be valid"),
            version: 1,
            current: true,
            previous_version_id: None,
        }
    }

    fn label_metadata(node_id: &NodeId, offset: u64) -> LabelIndexNodeMetadata {
        LabelIndexNodeMetadata {
            // Node id.
            node_id: node_id.clone(),
            // Latest storage ref.
            latest_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, offset)),
            // Graph record version.
            graph_record_version: Some(node_version("node-version--1")),
        }
    }

    fn relationship_type_metadata(
        relationship_id: &RelationshipId,
        offset: u64,
    ) -> RelationshipTypeIndexRelationshipMetadata {
        RelationshipTypeIndexRelationshipMetadata {
            // Relationship id.
            relationship_id: relationship_id.clone(),
            // Latest storage ref.
            latest_storage_ref: Some(storage_ref(StorageSegment::RelationshipRecords, offset)),
            // Graph record version.
            graph_record_version: Some(relationship_version("relationship-version--1")),
        }
    }

    //
    // Verify that the index container starts empty and keeps label and
    // relationship-type maps separate from each other.
    //
    // Given a default `GraphCatalogIndexes`,
    // when the metadata index maps are inspected,
    // then both the label map and relationship-type map should be empty.
    #[test]
    fn graph_catalog_indexes_default_to_empty_metadata_maps() {
        let indexes = GraphCatalogIndexes::default();

        assert!(indexes.labels.is_empty());
        assert!(indexes.relationship_types.is_empty());
    }

    //
    // Specify that indexing a node's labels makes the stable node ID resolvable by
    // label without loading the full node payload.
    //
    // Given a graph catalog and a latest node metadata entry carrying two labels,
    // when those labels are indexed and one label is resolved,
    // then lookup should return the stable node ID attached to that label.
    #[test]
    fn index_node_labels_makes_node_id_resolvable_by_label() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--campaign-1");
        let labels = vec!["Campaign".to_owned(), "FIMI".to_owned()];
        let metadata = label_metadata(&id, 128);

        index_node_labels(&mut catalog, &labels, metadata)
            .expect("label indexing should succeed for latest node metadata");
        let resolved =
            resolve_node_ids_by_label(&catalog, "Campaign", CatalogIndexLookupMode::Strict)
                .expect("known label should resolve to node ids");

        assert_eq!(resolved, vec![id]);
    }

    //
    // Specify that label metadata lookup returns lightweight catalog entries, not
    // full node payloads.
    //
    // Given a graph catalog with one indexed node label,
    // when lightweight entries are resolved for that label,
    // then the returned metadata should include the node ID, storage reference,
    // and graph record version only.
    #[test]
    fn resolve_label_index_entries_returns_lightweight_node_metadata() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--actor-1");
        let labels = vec!["Actor".to_owned()];
        let metadata = label_metadata(&id, 256);

        index_node_labels(&mut catalog, &labels, metadata.clone())
            .expect("label indexing should succeed for metadata lookup");
        let resolved =
            resolve_label_index_entries(&catalog, "Actor", CatalogIndexLookupMode::Strict)
                .expect("known label should resolve to metadata entries");

        assert_eq!(resolved, vec![metadata]);
    }

    //
    // Specify deterministic duplicate handling for repeated indexing of the same
    // node under the same label.
    //
    // Given the same node metadata indexed twice for the same label,
    // when node IDs are resolved by that label,
    // then the result should contain the node ID only once.
    #[test]
    fn index_node_labels_does_not_duplicate_same_node_for_same_label() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--claim-1");
        let labels = vec!["Claim".to_owned()];
        let metadata = label_metadata(&id, 384);

        index_node_labels(&mut catalog, &labels, metadata.clone())
            .expect("first label indexing should succeed");
        index_node_labels(&mut catalog, &labels, metadata)
            .expect("repeat label indexing should be idempotent");
        let resolved = resolve_node_ids_by_label(&catalog, "Claim", CatalogIndexLookupMode::Strict)
            .expect("known label should resolve to node ids");

        assert_eq!(resolved, vec![id]);
    }

    //
    // Specify that an empty label set is a no-op rather than a catalog error.
    //
    // Given a node metadata entry and no labels,
    // when label indexing is requested,
    // then the operation should succeed without creating a resolvable label.
    #[test]
    fn index_node_labels_accepts_empty_label_set_as_noop() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--unlabeled-1");
        let labels = Vec::new();
        let metadata = label_metadata(&id, 512);

        index_node_labels(&mut catalog, &labels, metadata)
            .expect("empty label indexing should be a no-op");
        let resolved = resolve_node_ids_by_label(
            &catalog,
            "Unlabeled",
            CatalogIndexLookupMode::EmptyWhenUnknown,
        )
        .expect("unknown exploratory label lookup should succeed");

        assert!(resolved.is_empty());
    }

    //
    // Specify strict unknown-label behavior for callers that expect a label to be
    // present.
    //
    // Given an empty catalog,
    // when a missing label is resolved in strict mode,
    // then lookup should return `UnknownLabelCatalogEntry` for that label.
    #[test]
    fn resolve_node_ids_by_label_reports_unknown_label_in_strict_mode() {
        let catalog = GraphCatalog::default();

        let error =
            resolve_node_ids_by_label(&catalog, "UnknownLabel", CatalogIndexLookupMode::Strict)
                .expect_err("strict unknown-label lookup should return explicit error");

        assert!(matches!(
        error,
        GraphStorageError::UnknownLabelCatalogEntry { label } if label == "UnknownLabel"
        ));
    }

    //
    // Specify exploratory unknown-label behavior for callers that do not require a
    // label to exist.
    //
    // Given an empty catalog,
    // when a missing label is resolved in empty-when-unknown mode,
    // then lookup should succeed with an empty result.
    #[test]
    fn resolve_node_ids_by_label_returns_empty_for_unknown_label_when_requested() {
        let catalog = GraphCatalog::default();

        let resolved = resolve_node_ids_by_label(
            &catalog,
            "UnknownLabel",
            CatalogIndexLookupMode::EmptyWhenUnknown,
        )
        .expect("exploratory unknown-label lookup should succeed");

        assert!(resolved.is_empty());
    }

    //
    // Specify that indexing a relationship type makes the stable relationship ID
    // resolvable by type without loading the full relationship payload.
    //
    // Given a graph catalog and a latest relationship metadata entry carrying a
    // relationship type,
    // when that type is indexed and resolved,
    // then lookup should return the stable relationship ID attached to that type.
    #[test]
    fn index_relationship_type_makes_relationship_id_resolvable_by_type() {
        let mut catalog = GraphCatalog::default();
        let id = relationship_id("relationship--promotes-1");
        let rel_type = relationship_type("PROMOTES");
        let metadata = relationship_type_metadata(&id, 640);

        index_relationship_type(&mut catalog, &rel_type, metadata)
            .expect("relationship-type indexing should succeed");
        let resolved =
            resolve_relationship_ids_by_type(&catalog, &rel_type, CatalogIndexLookupMode::Strict)
                .expect("known relationship type should resolve to relationship ids");

        assert_eq!(resolved, vec![id]);
    }

    //
    // Specify that relationship-type metadata lookup returns lightweight catalog
    // entries, not full relationship payloads.
    //
    // Given a graph catalog with one indexed relationship type,
    // when lightweight entries are resolved for that type,
    // then the returned metadata should include the relationship ID, storage
    // reference, and graph record version only.
    #[test]
    fn resolve_relationship_type_index_entries_returns_lightweight_relationship_metadata() {
        let mut catalog = GraphCatalog::default();
        let id = relationship_id("relationship--supports-1");
        let rel_type = relationship_type("SUPPORTS");
        let metadata = relationship_type_metadata(&id, 768);

        index_relationship_type(&mut catalog, &rel_type, metadata.clone())
            .expect("relationship-type indexing should succeed for metadata lookup");
        let resolved = resolve_relationship_type_index_entries(
            &catalog,
            &rel_type,
            CatalogIndexLookupMode::Strict,
        )
        .expect("known relationship type should resolve to metadata entries");

        assert_eq!(resolved, vec![metadata]);
    }

    //
    // Specify deterministic duplicate handling for repeated indexing of the same
    // relationship under the same relationship type.
    //
    // Given the same relationship metadata indexed twice for the same relationship
    // type,
    // when relationship IDs are resolved by that type,
    // then the result should contain the relationship ID only once.
    #[test]
    fn index_relationship_type_does_not_duplicate_same_relationship_for_same_type() {
        let mut catalog = GraphCatalog::default();
        let id = relationship_id("relationship--amplifies-1");
        let rel_type = relationship_type("AMPLIFIES");
        let metadata = relationship_type_metadata(&id, 896);

        index_relationship_type(&mut catalog, &rel_type, metadata.clone())
            .expect("first relationship-type indexing should succeed");
        index_relationship_type(&mut catalog, &rel_type, metadata)
            .expect("repeat relationship-type indexing should be idempotent");
        let resolved =
            resolve_relationship_ids_by_type(&catalog, &rel_type, CatalogIndexLookupMode::Strict)
                .expect("known relationship type should resolve to relationship ids");

        assert_eq!(resolved, vec![id]);
    }

    //
    // Specify strict unknown-relationship-type behavior for callers that expect a
    // type to be present.
    //
    // Given an empty catalog,
    // when a missing relationship type is resolved in strict mode,
    // then lookup should return `UnknownRelationshipTypeCatalogEntry` for that
    // relationship type.
    #[test]
    fn resolve_relationship_ids_by_type_reports_unknown_type_in_strict_mode() {
        let catalog = GraphCatalog::default();
        let missing_type = relationship_type("UNKNOWN_TYPE");

        let error = resolve_relationship_ids_by_type(
            &catalog,
            &missing_type,
            CatalogIndexLookupMode::Strict,
        )
        .expect_err("strict unknown-relationship-type lookup should return explicit error");

        assert!(matches!(
        error,
        GraphStorageError::UnknownRelationshipTypeCatalogEntry { relationship_type }
        if relationship_type == missing_type
        ));
    }

    //
    // Specify exploratory unknown-relationship-type behavior for callers that do
    // not require a type to exist.
    //
    // Given an empty catalog,
    // when a missing relationship type is resolved in empty-when-unknown mode,
    // then lookup should succeed with an empty result.
    #[test]
    fn resolve_relationship_ids_by_type_returns_empty_for_unknown_type_when_requested() {
        let catalog = GraphCatalog::default();
        let missing_type = relationship_type("UNKNOWN_TYPE");

        let resolved = resolve_relationship_ids_by_type(
            &catalog,
            &missing_type,
            CatalogIndexLookupMode::EmptyWhenUnknown,
        )
        .expect("exploratory unknown-relationship-type lookup should succeed");

        assert!(resolved.is_empty());
    }
}
