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

use graph_core::{NodeId, RelationshipId};
use serde::{Deserialize, Serialize};

use crate::{
    GraphRecordVersion, GraphStorageError, GraphStorageResult, PersistedRecordEnvelope,
    PersistedRecordId, PersistedRecordKind, StorageRef, catalog_indexes::GraphCatalogIndexes,
    validate_persisted_record_envelope, validate_storage_ref,
};

/// Persistent storage catalog model owned by and extended by issue 56.
///
///
/// - Represent the minimum catalog state needed to resolve graph-core record IDs
///   into byte-addressable storage references.
/// - Keep latest node and relationship record lookups separate from label,
///   relationship-type, property, and adjacency indexes.
/// - Preserve enough version metadata to distinguish the latest persisted record
///   from historical records without loading full node or relationship payloads.
/// - Carry lightweight metadata indexes for labels and relationship types without
///   introducing property, semantic, vector, or adjacency indexes.
/// - Provide a future file-backed pager with a compact lookup structure that can
///   be loaded before any graph payloads are paged in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphCatalog {
    /// Latest node record references keyed by stable graph-core `NodeId`.
    pub latest_node_records: HashMap<NodeId, LatestRecordCatalogEntry>,

    /// Latest relationship record references keyed by stable graph-core
    /// `RelationshipId`.
    pub latest_relationship_records: HashMap<RelationshipId, LatestRecordCatalogEntry>,

    /// Historical record references retained as metadata only.
    ///
    ///
    /// - Keep older versions visible to the catalog model.
    /// - Make it explicit that historical records are not part of latest lookups.
    /// - Avoid requiring full payload loading to know that a previous record exists.
    pub historical_records: Vec<HistoricalRecordCatalogEntry>,

    /// Catalog-level metadata indexes for labels and relationship types.
    ///
    ///
    /// - Keep label and relationship-type lookup state close to the catalog that
    ///   owns latest ID-to-storage-reference mappings.
    /// - Allow working-set loaders and future planners to resolve IDs or
    ///   lightweight metadata without loading full payloads.
    /// - Keep property, semantic, vector, and adjacency indexes out of this
    ///   structure until separate issues own them.
    pub metadata_indexes: GraphCatalogIndexes,
}

/// Catalog entry for a record considered latest for its stable graph ID.
///
///
/// - Store the persisted record identity, kind, version metadata, and storage
///   reference needed for latest-record lookup.
/// - Avoid carrying the full graph-core node or relationship payload.
/// - Leave duplicate latest-record conflict handling to later implementation
///   phases while reserving the shape they will compare.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestRecordCatalogEntry {
    /// Record id.
    pub record_id: PersistedRecordId,
    /// Kind.
    pub kind: PersistedRecordKind,
    /// Graph record version.
    pub graph_record_version: Option<GraphRecordVersion>,
    /// Storage ref.
    pub storage_ref: StorageRef,
}

/// Catalog entry for a persisted graph record that is no longer latest.
///
///
/// - Keep historical record references separate from latest lookup maps.
/// - Preserve the metadata required for future version-history reads and catalog
///   rebuild checks.
/// - Ensure append-only retention does not make older records look current.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalRecordCatalogEntry {
    /// Record id.
    pub record_id: PersistedRecordId,
    /// Kind.
    pub kind: PersistedRecordKind,
    /// Graph record version.
    pub graph_record_version: Option<GraphRecordVersion>,
    /// Storage ref.
    pub storage_ref: StorageRef,
    /// Superseded by.
    pub superseded_by: Option<StorageRef>,
}

/// Create an empty graph catalog.
///
///
/// - Declare the construction boundary for a catalog that starts with no latest
///   node or relationship entries.
/// - Keep creation independent from storage-root opening, record-log opening,
///   catalog file loading, and catalog rebuild.
pub fn create_empty_graph_catalog() -> GraphStorageResult<GraphCatalog> {
    Ok(GraphCatalog::default())
}

/// Index one appended node record in the catalog.
///
///
/// - Declare the boundary called after a node envelope has been appended to the
///   node record log.
/// - Register the latest `NodeId -> StorageRef` mapping without scanning the
///   append-only log.
/// - Move or preserve any previous latest entry as historical metadata instead of
///   overwriting it silently.
pub fn index_appended_node_record(
    catalog: &mut GraphCatalog,
    envelope: &PersistedRecordEnvelope,
    storage_ref: StorageRef,
) -> GraphStorageResult<()> {
    validate_index_candidate(envelope, &storage_ref, PersistedRecordKind::Node)?;
    let node_id = match &envelope.record_id {
        PersistedRecordId::Node(node_id) => node_id.clone(),
        _ => {
            return Err(GraphStorageError::InvalidEnvelope {
                reason: format!(
                    "node catalog entry requires node record id, got {:?}",
                    envelope.record_id
                ),
            });
        }
    };

    let candidate = latest_entry_from_envelope(envelope, storage_ref);
    if catalog_entry_is_current(&candidate)? {
        if let Some(existing) = catalog.latest_node_records.get(&node_id).cloned() {
            if existing.storage_ref == candidate.storage_ref {
                catalog.latest_node_records.insert(node_id, candidate);
                return Ok(());
            }
            if !is_normal_successor(&existing, &candidate) {
                check_duplicate_latest_record_conflict(
                    &candidate.record_id,
                    &existing,
                    &candidate,
                )?;
            }
            catalog
                .historical_records
                .push(historical_entry_from_latest(
                    existing,
                    Some(candidate.storage_ref.clone()),
                ));
        }
        catalog.latest_node_records.insert(node_id, candidate);
    } else {
        catalog
            .historical_records
            .push(historical_entry_from_latest(candidate, None));
    }

    Ok(())
}

/// Index one appended relationship record in the catalog.
///
///
/// - Declare the boundary called after a relationship envelope has been appended
///   to the relationship record log.
/// - Register the latest `RelationshipId -> StorageRef` mapping without scanning
///   the append-only log.
/// - Keep relationship catalog state separate from node, label, type, and
///   adjacency indexes.
pub fn index_appended_relationship_record(
    catalog: &mut GraphCatalog,
    envelope: &PersistedRecordEnvelope,
    storage_ref: StorageRef,
) -> GraphStorageResult<()> {
    validate_index_candidate(envelope, &storage_ref, PersistedRecordKind::Relationship)?;
    let relationship_id = match &envelope.record_id {
        PersistedRecordId::Relationship(relationship_id) => relationship_id.clone(),
        _ => {
            return Err(GraphStorageError::InvalidEnvelope {
                reason: format!(
                    "relationship catalog entry requires relationship record id, got {:?}",
                    envelope.record_id
                ),
            });
        }
    };

    let candidate = latest_entry_from_envelope(envelope, storage_ref);
    if catalog_entry_is_current(&candidate)? {
        if let Some(existing) = catalog
            .latest_relationship_records
            .get(&relationship_id)
            .cloned()
        {
            if existing.storage_ref == candidate.storage_ref {
                catalog
                    .latest_relationship_records
                    .insert(relationship_id, candidate);
                return Ok(());
            }
            if !is_normal_successor(&existing, &candidate) {
                check_duplicate_latest_record_conflict(
                    &candidate.record_id,
                    &existing,
                    &candidate,
                )?;
            }
            catalog
                .historical_records
                .push(historical_entry_from_latest(
                    existing,
                    Some(candidate.storage_ref.clone()),
                ));
        }
        catalog
            .latest_relationship_records
            .insert(relationship_id, candidate);
    } else {
        catalog
            .historical_records
            .push(historical_entry_from_latest(candidate, None));
    }

    Ok(())
}

/// Resolve the latest persisted node storage reference.
///
///
/// - Provide the lookup boundary future pagers will use before loading a node
///   payload.
/// - Make latest-record lookup explicit rather than requiring callers to infer it
///   from append-only log order.
/// - Keep missing node catalog entries distinguishable from corrupted storage,
///   missing payload bytes, and unsupported record formats.
pub fn resolve_latest_node_storage_ref(
    catalog: &GraphCatalog,
    node_id: &NodeId,
) -> GraphStorageResult<StorageRef> {
    catalog
        .latest_node_records
        .get(node_id)
        .map(|entry| entry.storage_ref.clone())
        .ok_or_else(|| GraphStorageError::MissingNodeCatalogEntry {
            node_id: node_id.clone(),
        })
}

/// Resolve the latest persisted relationship storage reference.
///
///
/// - Provide the lookup boundary future pagers will use before loading a
///   relationship payload.
/// - Make latest-record lookup explicit rather than requiring callers to infer it
///   from append-only log order.
/// - Keep missing relationship catalog entries distinguishable from corrupted
///   storage, missing payload bytes, and unsupported record formats.
pub fn resolve_latest_relationship_storage_ref(
    catalog: &GraphCatalog,
    relationship_id: &RelationshipId,
) -> GraphStorageResult<StorageRef> {
    catalog
        .latest_relationship_records
        .get(relationship_id)
        .map(|entry| entry.storage_ref.clone())
        .ok_or_else(|| GraphStorageError::MissingRelationshipCatalogEntry {
            relationship_id: relationship_id.clone(),
        })
}

/// Reserve duplicate latest-record conflict detection for later implementation.
///
///
/// - Give tests and implementation phases a named boundary for comparing a
///   current latest catalog entry with a newly appended candidate.
/// - Ensure duplicate latest-record conflicts are treated as catalog consistency
///   errors, not as silent replacement.
/// - Keep conflict detection focused on node and relationship latest maps only;
///   label, relationship-type, and adjacency index conflicts belong to later
///   issues.
pub fn check_duplicate_latest_record_conflict(
    record_id: &PersistedRecordId,
    existing: &LatestRecordCatalogEntry,
    candidate: &LatestRecordCatalogEntry,
) -> GraphStorageResult<()> {
    if &existing.record_id != record_id || &candidate.record_id != record_id {
        return Err(GraphStorageError::InvalidEnvelope {
            reason: format!(
                "duplicate latest check requires matching record ids: expected {:?}, existing {:?}, candidate {:?}",
                record_id, existing.record_id, candidate.record_id
            ),
        });
    }

    if existing.storage_ref == candidate.storage_ref || is_normal_successor(existing, candidate) {
        return Ok(());
    }

    Err(GraphStorageError::DuplicateLatestRecordConflict {
        record_id: record_id.clone(),
        existing_ref: Box::new(existing.storage_ref.clone()),
        conflicting_ref: Box::new(candidate.storage_ref.clone()),
    })
}

fn validate_index_candidate(
    envelope: &PersistedRecordEnvelope,
    storage_ref: &StorageRef,
    expected_kind: PersistedRecordKind,
) -> GraphStorageResult<()> {
    validate_persisted_record_envelope(envelope)?;
    validate_storage_ref(storage_ref)?;

    if envelope.kind != expected_kind {
        return Err(GraphStorageError::UnexpectedRecordKind {
            expected: expected_kind,
            actual: envelope.kind,
        });
    }

    match (&envelope.kind, &envelope.graph_record_version) {
        (PersistedRecordKind::Node, Some(GraphRecordVersion::Node { .. })) => Ok(()),
        (PersistedRecordKind::Relationship, Some(GraphRecordVersion::Relationship { .. })) => {
            Ok(())
        }
        _ => Err(GraphStorageError::InvalidEnvelope {
            reason: format!(
                "catalog entry kind/version mismatch: {:?} with {:?}",
                envelope.kind, envelope.graph_record_version
            ),
        }),
    }
}

fn latest_entry_from_envelope(
    envelope: &PersistedRecordEnvelope,
    storage_ref: StorageRef,
) -> LatestRecordCatalogEntry {
    LatestRecordCatalogEntry {
        // Record id.
        record_id: envelope.record_id.clone(),
        // Kind.
        kind: envelope.kind,
        // Graph record version.
        graph_record_version: envelope.graph_record_version.clone(),
        storage_ref,
    }
}

fn historical_entry_from_latest(
    latest: LatestRecordCatalogEntry,
    superseded_by: Option<StorageRef>,
) -> HistoricalRecordCatalogEntry {
    HistoricalRecordCatalogEntry {
        // Record id.
        record_id: latest.record_id,
        // Kind.
        kind: latest.kind,
        // Graph record version.
        graph_record_version: latest.graph_record_version,
        // Storage ref.
        storage_ref: latest.storage_ref,
        superseded_by,
    }
}

fn catalog_entry_is_current(entry: &LatestRecordCatalogEntry) -> GraphStorageResult<bool> {
    match &entry.graph_record_version {
        Some(GraphRecordVersion::Node { current, .. })
        | Some(GraphRecordVersion::Relationship { current, .. }) => Ok(*current),
        None => Err(GraphStorageError::InvalidEnvelope {
            reason: "catalog entry requires graph record version metadata".to_owned(),
        }),
    }
}

fn is_normal_successor(
    existing: &LatestRecordCatalogEntry,
    candidate: &LatestRecordCatalogEntry,
) -> bool {
    match (
        &existing.graph_record_version,
        &candidate.graph_record_version,
    ) {
        (
            Some(GraphRecordVersion::Node {
                version_id: existing_version_id,
                version: existing_version,
                ..
            }),
            Some(GraphRecordVersion::Node {
                version: candidate_version,
                current: true,
                previous_version_id: Some(previous_version_id),
                ..
            }),
        ) => previous_version_id == existing_version_id && candidate_version > existing_version,
        (
            Some(GraphRecordVersion::Relationship {
                version_id: existing_version_id,
                version: existing_version,
                ..
            }),
            Some(GraphRecordVersion::Relationship {
                version: candidate_version,
                current: true,
                previous_version_id: Some(previous_version_id),
                ..
            }),
        ) => previous_version_id == existing_version_id && candidate_version > existing_version,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{NodeVersionId, RelationshipVersionId};

    use crate::{RecordChecksum, RecordFormat, StorageSegment, StorageVersion};

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

    fn node_version(
        version_id: &str,
        version: u64,
        current: bool,
        previous_version_id: Option<&str>,
    ) -> GraphRecordVersion {
        GraphRecordVersion::Node {
            version_id: NodeVersionId::new(version_id)
                .expect("test node version id should be valid"),
            version,
            current,
            previous_version_id: previous_version_id.map(|value| {
                NodeVersionId::new(value).expect("test previous node version id should be valid")
            }),
        }
    }

    fn relationship_version(
        version_id: &str,
        version: u64,
        current: bool,
        previous_version_id: Option<&str>,
    ) -> GraphRecordVersion {
        GraphRecordVersion::Relationship {
            version_id: RelationshipVersionId::new(version_id)
                .expect("test relationship version id should be valid"),
            version,
            current,
            previous_version_id: previous_version_id.map(|value| {
                RelationshipVersionId::new(value)
                    .expect("test previous relationship version id should be valid")
            }),
        }
    }

    fn node_envelope(
        node_id: &NodeId,
        graph_record_version: GraphRecordVersion,
        storage_ref: StorageRef,
    ) -> PersistedRecordEnvelope {
        PersistedRecordEnvelope {
            // Record id.
            record_id: PersistedRecordId::Node(node_id.clone()),
            // Kind.
            kind: PersistedRecordKind::Node,
            // Storage version.
            storage_version: StorageVersion::V1,
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
            // Graph record version.
            graph_record_version: Some(graph_record_version),
            storage_ref,
            // Record checksum.
            record_checksum: Some(checksum("node-record")),
        }
    }

    fn relationship_envelope(
        relationship_id: &RelationshipId,
        graph_record_version: GraphRecordVersion,
        storage_ref: StorageRef,
    ) -> PersistedRecordEnvelope {
        PersistedRecordEnvelope {
            // Record id.
            record_id: PersistedRecordId::Relationship(relationship_id.clone()),
            // Kind.
            kind: PersistedRecordKind::Relationship,
            // Storage version.
            storage_version: StorageVersion::V1,
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
            // Graph record version.
            graph_record_version: Some(graph_record_version),
            storage_ref,
            // Record checksum.
            record_checksum: Some(checksum("relationship-record")),
        }
    }

    fn latest_node_entry(node_id: &NodeId, storage_ref: StorageRef) -> LatestRecordCatalogEntry {
        LatestRecordCatalogEntry {
            // Record id.
            record_id: PersistedRecordId::Node(node_id.clone()),
            // Kind.
            kind: PersistedRecordKind::Node,
            // Graph record version.
            graph_record_version: Some(node_version("node-version--1", 1, true, None)),
            storage_ref,
        }
    }

    //
    // Verify that catalog construction creates a lookup structure with no latest
    // node records, no latest relationship records, no historical references, and
    // no metadata indexes. This documents that the catalog can be opened before
    // loading any graph payloads.
    #[test]
    fn create_empty_graph_catalog_returns_empty_indexes() {
        let catalog = create_empty_graph_catalog().expect("empty catalog creation should succeed");

        assert!(catalog.latest_node_records.is_empty());
        assert!(catalog.latest_relationship_records.is_empty());
        assert!(catalog.historical_records.is_empty());
        assert!(catalog.metadata_indexes.labels.is_empty());
        assert!(catalog.metadata_indexes.relationship_types.is_empty());
    }

    //
    // Verify that indexing an appended node envelope records a latest
    // `NodeId -> StorageRef` mapping and that resolving the same ID returns the
    // appended reference without requiring payload loading.
    #[test]
    fn index_appended_node_record_registers_latest_node_storage_ref() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--campaign-1");
        let reference = storage_ref(StorageSegment::NodeRecords, 128);
        let envelope = node_envelope(
            &id,
            node_version("node-version--1", 1, true, None),
            reference.clone(),
        );

        index_appended_node_record(&mut catalog, &envelope, reference.clone())
            .expect("node catalog indexing should succeed");

        assert_eq!(
            resolve_latest_node_storage_ref(&catalog, &id),
            Ok(reference)
        );
        assert!(catalog.historical_records.is_empty());
    }

    //
    // Verify that indexing an appended relationship envelope records a latest
    // `RelationshipId -> StorageRef` mapping and keeps relationship lookup
    // independent from node lookup.
    #[test]
    fn index_appended_relationship_record_registers_latest_relationship_storage_ref() {
        let mut catalog = GraphCatalog::default();
        let id = relationship_id("relationship--uses-1");
        let reference = storage_ref(StorageSegment::RelationshipRecords, 256);
        let envelope = relationship_envelope(
            &id,
            relationship_version("relationship-version--1", 1, true, None),
            reference.clone(),
        );

        index_appended_relationship_record(&mut catalog, &envelope, reference.clone())
            .expect("relationship catalog indexing should succeed");

        assert_eq!(
            resolve_latest_relationship_storage_ref(&catalog, &id),
            Ok(reference)
        );
        assert!(catalog.latest_node_records.is_empty());
        assert!(catalog.historical_records.is_empty());
    }

    //
    // Verify that missing node lookups return the explicit catalog error rather
    // than a generic operation failure or a storage payload error.
    #[test]
    fn resolve_latest_node_storage_ref_reports_missing_node_catalog_entry() {
        let catalog = GraphCatalog::default();
        let missing = node_id("node--missing");

        let error = resolve_latest_node_storage_ref(&catalog, &missing)
            .expect_err("missing node catalog entry should be explicit");

        assert!(matches!(
        error,
        GraphStorageError::MissingNodeCatalogEntry { node_id } if node_id == missing
        ));
    }

    //
    // Verify that missing relationship lookups return the explicit catalog error
    // rather than a generic operation failure or a storage payload error.
    #[test]
    fn resolve_latest_relationship_storage_ref_reports_missing_relationship_catalog_entry() {
        let catalog = GraphCatalog::default();
        let missing = relationship_id("relationship--missing");

        let error = resolve_latest_relationship_storage_ref(&catalog, &missing)
            .expect_err("missing relationship catalog entry should be explicit");

        assert!(matches!(
        error,
        GraphStorageError::MissingRelationshipCatalogEntry { relationship_id }
        if relationship_id == missing
        ));
    }

    //
    // Verify that appending a newer current node version updates the latest map
    // while preserving the previous latest record as historical metadata. This
    // prevents append-only retention from being confused with current lookup.
    #[test]
    fn index_appended_node_record_preserves_superseded_latest_as_history() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--versioned");
        let first_ref = storage_ref(StorageSegment::NodeRecords, 100);
        let second_ref = storage_ref(StorageSegment::NodeRecords, 200);
        let first_envelope = node_envelope(
            &id,
            node_version("node-version--1", 1, true, None),
            first_ref.clone(),
        );
        let second_envelope = node_envelope(
            &id,
            node_version("node-version--2", 2, true, Some("node-version--1")),
            second_ref.clone(),
        );

        index_appended_node_record(&mut catalog, &first_envelope, first_ref.clone())
            .expect("first node version should index successfully");
        index_appended_node_record(&mut catalog, &second_envelope, second_ref.clone())
            .expect("second node version should index successfully");

        assert_eq!(
            resolve_latest_node_storage_ref(&catalog, &id),
            Ok(second_ref.clone())
        );
        assert_eq!(catalog.historical_records.len(), 1);
        assert_eq!(
            catalog.historical_records[0].record_id,
            PersistedRecordId::Node(id)
        );
        assert_eq!(catalog.historical_records[0].storage_ref, first_ref);
        assert_eq!(
            catalog.historical_records[0].superseded_by,
            Some(second_ref)
        );
    }

    //
    // Verify that duplicate latest-record conflicts are modeled as explicit
    // catalog consistency errors that include both competing storage references.
    #[test]
    fn check_duplicate_latest_record_conflict_reports_explicit_error() {
        let id = node_id("node--duplicate-latest");
        let record_id = PersistedRecordId::Node(id.clone());
        let existing_ref = storage_ref(StorageSegment::NodeRecords, 300);
        let conflicting_ref = storage_ref(StorageSegment::NodeRecords, 400);
        let existing = latest_node_entry(&id, existing_ref.clone());
        let candidate = latest_node_entry(&id, conflicting_ref.clone());

        let error = check_duplicate_latest_record_conflict(&record_id, &existing, &candidate)
            .expect_err("duplicate latest records should be rejected explicitly");

        assert!(matches!(
        error,
        GraphStorageError::DuplicateLatestRecordConflict {
        record_id: actual_record_id,
        existing_ref: actual_existing_ref,
        conflicting_ref: actual_conflicting_ref,
        } if actual_record_id == record_id
        && actual_existing_ref.as_ref() == &existing_ref
        && actual_conflicting_ref.as_ref() == &conflicting_ref
        ));
    }

    #[test]
    fn index_appended_node_record_non_current_version_is_historical_only() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--historical-only");
        let reference = storage_ref(StorageSegment::NodeRecords, 777);
        let envelope = node_envelope(
            &id,
            node_version("node-version--old", 1, false, None),
            reference.clone(),
        );

        index_appended_node_record(&mut catalog, &envelope, reference.clone())
            .expect("non-current node records should still be indexed as history");

        assert!(catalog.latest_node_records.is_empty());
        assert_eq!(catalog.historical_records.len(), 1);
        assert_eq!(
            catalog.historical_records[0].record_id,
            PersistedRecordId::Node(id)
        );
        assert_eq!(catalog.historical_records[0].storage_ref, reference);
        assert!(catalog.historical_records[0].superseded_by.is_none());
    }

    #[test]
    fn index_appended_relationship_record_preserves_superseded_latest_as_history() {
        let mut catalog = GraphCatalog::default();
        let id = relationship_id("relationship--versioned");
        let first_ref = storage_ref(StorageSegment::RelationshipRecords, 901);
        let second_ref = storage_ref(StorageSegment::RelationshipRecords, 902);
        let first_envelope = relationship_envelope(
            &id,
            relationship_version("relationship-version--1", 1, true, None),
            first_ref.clone(),
        );
        let second_envelope = relationship_envelope(
            &id,
            relationship_version(
                "relationship-version--2",
                2,
                true,
                Some("relationship-version--1"),
            ),
            second_ref.clone(),
        );

        index_appended_relationship_record(&mut catalog, &first_envelope, first_ref.clone())
            .expect("first relationship version should index");
        index_appended_relationship_record(&mut catalog, &second_envelope, second_ref.clone())
            .expect("second relationship version should index");

        assert_eq!(
            resolve_latest_relationship_storage_ref(&catalog, &id),
            Ok(second_ref.clone())
        );
        assert_eq!(catalog.historical_records.len(), 1);
        assert_eq!(
            catalog.historical_records[0].record_id,
            PersistedRecordId::Relationship(id)
        );
        assert_eq!(catalog.historical_records[0].storage_ref, first_ref);
        assert_eq!(
            catalog.historical_records[0].superseded_by,
            Some(second_ref)
        );
    }

    #[test]
    fn index_appended_node_record_rejects_wrong_record_kind() {
        let mut catalog = GraphCatalog::default();
        let id = relationship_id("relationship--wrong-kind");
        let reference = storage_ref(StorageSegment::RelationshipRecords, 333);
        let envelope = relationship_envelope(
            &id,
            relationship_version("relationship-version--1", 1, true, None),
            reference.clone(),
        );

        let error = index_appended_node_record(&mut catalog, &envelope, reference)
            .expect_err("node indexer should reject relationship envelopes");

        assert!(matches!(
            error,
            GraphStorageError::UnexpectedRecordKind {
                expected: PersistedRecordKind::Node,
                actual: PersistedRecordKind::Relationship,
            }
        ));
    }

    #[test]
    fn check_duplicate_latest_record_conflict_accepts_identical_storage_ref_and_rejects_mismatched_record_id()
     {
        let id = node_id("node--duplicate-check");
        let shared_ref = storage_ref(StorageSegment::NodeRecords, 1200);
        let entry = latest_node_entry(&id, shared_ref);

        check_duplicate_latest_record_conflict(
            &PersistedRecordId::Node(id.clone()),
            &entry,
            &entry,
        )
        .expect("identical storage refs should not be treated as a conflict");

        let other_id = node_id("node--other");
        let mismatch_error = check_duplicate_latest_record_conflict(
            &PersistedRecordId::Node(other_id),
            &entry,
            &entry,
        )
        .expect_err("mismatched record IDs should fail conflict check validation");

        assert!(matches!(
            mismatch_error,
            GraphStorageError::InvalidEnvelope { .. }
        ));
    }
}
