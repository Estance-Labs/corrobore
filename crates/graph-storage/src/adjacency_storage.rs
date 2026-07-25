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
use std::collections::{HashMap, HashSet};

use graph_core::{AdjacencyDirection, NodeId, RelationshipId, RelationshipType};
use serde::{Deserialize, Serialize};

use crate::{
    GraphCatalog, GraphStorageError, GraphStorageResult, HistoricalRecordCatalogEntry,
    PersistedRecordId, PersistedRecordKind, StorageRef, StorageSegment, validate_storage_ref,
};

/// Lookup behavior for persisted adjacency when the owner node or adjacency page
/// is not known to the catalog.
///
///
/// - Let strict callers distinguish an unknown adjacency page from a known node
///   that simply has no edges in the requested direction.
/// - Let exploratory callers request deterministic empty adjacency once the
///   implementation can verify that the owner node is known.
/// - Keep missing adjacency separate from missing node payloads, missing
///   relationship payloads, and corrupted adjacency pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdjacencyStorageLookupMode {
    /// Missing owner adjacency should become an explicit storage error.
    Strict,

    /// Known owner nodes with no adjacency should resolve to an empty record once
    /// implemented. Unknown owner nodes must still remain distinguishable by the
    /// catalog lookup path for future implementations.
    EmptyWhenKnownNodeHasNoEdges,
}

/// Persistent adjacency storage handle.
///
///
/// - Represent the future file-backed adjacency segment boundary independently
///   from node and relationship record logs.
/// - Give write/read function stubs a concrete storage handle without committing
///   to page layout, sharding, buffering, or flush behavior in the current implementation.
/// - Keep this type outside `graph-core`; persistent adjacency is a storage-layer
///   concern consumed by future pagers and working-set loaders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphAdjacencyStorage {
    outgoing_records: HashMap<NodeId, PersistedAdjacencyRecord>,
    incoming_records: HashMap<NodeId, PersistedAdjacencyRecord>,
    next_offset: u64,
}

/// Catalog entry pointing from one owner node and direction to a persisted
/// adjacency page.
///
///
/// - Integrate adjacency storage references with the catalog without embedding
///   full adjacency payloads in the catalog itself.
/// - Preserve the owner node, direction, storage reference, relationship count,
///   and indexed relationship-type hints needed by warm-frontier loading.
/// - Keep outgoing and incoming adjacency references independent so callers can
///   read only the direction they need.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjacencyStorageCatalogEntry {
    /// Owner node id.
    pub owner_node_id: NodeId,
    /// Direction.
    pub direction: AdjacencyDirection,
    /// Storage ref.
    pub storage_ref: StorageRef,
    /// Relationship count.
    pub relationship_count: usize,
    /// Relationship types.
    pub relationship_types: Vec<RelationshipType>,
}

/// Persisted adjacency record for one owner node and one direction.
///
///
/// - Store graph neighborhood information separately from node and relationship
///   payload records.
/// - Provide enough metadata for a warm working-set boundary: relationship ID,
///   source ID, target ID, type, direction, and optional storage references.
/// - Allow future read paths to answer adjacency requests without loading full
///   node payloads or full relationship payloads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAdjacencyRecord {
    /// Owner node id.
    pub owner_node_id: NodeId,
    /// Direction.
    pub direction: AdjacencyDirection,
    /// Entries.
    pub entries: Vec<PersistedAdjacencyEntry>,
    /// Storage ref.
    pub storage_ref: Option<StorageRef>,
}

/// One lightweight persisted edge entry inside an adjacency record.
///
///
/// - Preserve the relationship identity and endpoints required for traversal.
/// - Preserve the relationship type required for type-filtered expansion and
///   loading profiles.
/// - Carry optional storage references so future lazy page-in can load the
///   relationship, source node, or target node payload only when needed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAdjacencyEntry {
    /// Relationship id.
    pub relationship_id: RelationshipId,
    /// Source node id.
    pub source_node_id: NodeId,
    /// Target node id.
    pub target_node_id: NodeId,
    /// Relationship type.
    pub relationship_type: RelationshipType,
    /// Direction.
    pub direction: AdjacencyDirection,
    /// Relationship storage ref.
    pub relationship_storage_ref: Option<StorageRef>,
    /// Source node storage ref.
    pub source_node_storage_ref: Option<StorageRef>,
    /// Target node storage ref.
    pub target_node_storage_ref: Option<StorageRef>,
}

/// Reserve the catalog boundary for an outgoing adjacency storage reference.
///
///
/// - Record where outgoing adjacency for `owner_node_id` can be loaded from.
/// - Keep the reference in catalog-level metadata rather than in the node payload.
/// - Defer actual mutation, duplicate handling, and consistency checks to later
///   phases.
pub fn index_outgoing_adjacency_storage_ref(
    catalog: &mut GraphCatalog,
    entry: AdjacencyStorageCatalogEntry,
) -> GraphStorageResult<()> {
    index_adjacency_storage_ref(catalog, entry, AdjacencyDirection::Outgoing)
}

/// Reserve the catalog boundary for an incoming adjacency storage reference.
///
///
/// - Record where incoming adjacency for `owner_node_id` can be loaded from.
/// - Keep incoming references independent from outgoing references.
/// - Defer actual mutation, duplicate handling, and consistency checks to later
///   phases.
pub fn index_incoming_adjacency_storage_ref(
    catalog: &mut GraphCatalog,
    entry: AdjacencyStorageCatalogEntry,
) -> GraphStorageResult<()> {
    index_adjacency_storage_ref(catalog, entry, AdjacencyDirection::Incoming)
}

/// Reserve lookup of an outgoing adjacency storage reference by owner node ID.
///
///
/// - Let future pagers find the outgoing adjacency page without loading the owner
///   node payload or any relationship payloads.
/// - Make strict unknown-adjacency behavior explicit through `lookup_mode`.
/// - Preserve deterministic empty-adjacency behavior for known nodes with no
///   outgoing edges in the later implementation phase.
pub fn resolve_outgoing_adjacency_storage_ref(
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    lookup_mode: AdjacencyStorageLookupMode,
) -> GraphStorageResult<Option<StorageRef>> {
    resolve_adjacency_storage_ref(
        catalog,
        owner_node_id,
        AdjacencyDirection::Outgoing,
        lookup_mode,
    )
}

/// Reserve lookup of an incoming adjacency storage reference by owner node ID.
///
///
/// - Let future pagers find the incoming adjacency page without loading the owner
///   node payload or any relationship payloads.
/// - Keep incoming unknown-adjacency behavior distinct from outgoing behavior.
/// - Preserve deterministic empty-adjacency behavior for known nodes with no
///   incoming edges in the later implementation phase.
pub fn resolve_incoming_adjacency_storage_ref(
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    lookup_mode: AdjacencyStorageLookupMode,
) -> GraphStorageResult<Option<StorageRef>> {
    resolve_adjacency_storage_ref(
        catalog,
        owner_node_id,
        AdjacencyDirection::Incoming,
        lookup_mode,
    )
}

/// Reserve the write boundary for outgoing adjacency by owner node ID.
///
///
/// - Persist outgoing neighborhood metadata independently from node and
///   relationship payload records.
/// - Return the future adjacency `StorageRef` that can be registered in the
///   catalog.
/// - Defer page layout, append/overwrite policy, checksums, and catalog mutation
///   for future implementations.
pub fn write_outgoing_adjacency_by_node_id(
    storage: &mut GraphAdjacencyStorage,
    catalog: &mut GraphCatalog,
    owner_node_id: &NodeId,
    entries: Vec<PersistedAdjacencyEntry>,
) -> GraphStorageResult<StorageRef> {
    write_adjacency_by_node_id(
        storage,
        catalog,
        owner_node_id,
        AdjacencyDirection::Outgoing,
        entries,
    )
}

/// Reserve the write boundary for incoming adjacency by owner node ID.
///
///
/// - Persist incoming neighborhood metadata independently from node and
///   relationship payload records.
/// - Return the future adjacency `StorageRef` that can be registered in the
///   catalog.
/// - Defer page layout, append/overwrite policy, checksums, and catalog mutation
///   for future implementations.
pub fn write_incoming_adjacency_by_node_id(
    storage: &mut GraphAdjacencyStorage,
    catalog: &mut GraphCatalog,
    owner_node_id: &NodeId,
    entries: Vec<PersistedAdjacencyEntry>,
) -> GraphStorageResult<StorageRef> {
    write_adjacency_by_node_id(
        storage,
        catalog,
        owner_node_id,
        AdjacencyDirection::Incoming,
        entries,
    )
}

/// Reserve the read boundary for outgoing adjacency by owner node ID.
///
///
/// - Read outgoing adjacency without loading the owner node payload.
/// - Read outgoing adjacency without loading full relationship payloads.
/// - Return lightweight entries that are compatible with warm working-set
///   boundaries and future lazy page-in.
pub fn read_outgoing_adjacency_by_node_id(
    storage: &GraphAdjacencyStorage,
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    lookup_mode: AdjacencyStorageLookupMode,
) -> GraphStorageResult<PersistedAdjacencyRecord> {
    read_adjacency_by_node_id(
        storage,
        catalog,
        owner_node_id,
        AdjacencyDirection::Outgoing,
        lookup_mode,
    )
}

/// Reserve the read boundary for incoming adjacency by owner node ID.
///
///
/// - Read incoming adjacency without loading the owner node payload.
/// - Read incoming adjacency without loading full relationship payloads.
/// - Return lightweight entries that are compatible with warm working-set
///   boundaries and future lazy page-in.
pub fn read_incoming_adjacency_by_node_id(
    storage: &GraphAdjacencyStorage,
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    lookup_mode: AdjacencyStorageLookupMode,
) -> GraphStorageResult<PersistedAdjacencyRecord> {
    read_adjacency_by_node_id(
        storage,
        catalog,
        owner_node_id,
        AdjacencyDirection::Incoming,
        lookup_mode,
    )
}

/// Snapshot all persisted adjacency records for checkpoint materialization.
pub fn snapshot_persisted_adjacency_records(
    storage: &GraphAdjacencyStorage,
) -> Vec<PersistedAdjacencyRecord> {
    let mut records: Vec<PersistedAdjacencyRecord> = storage
        .outgoing_records
        .values()
        .chain(storage.incoming_records.values())
        .cloned()
        .collect();
    records.sort_by(|left, right| {
        left.owner_node_id
            .as_str()
            .cmp(right.owner_node_id.as_str())
            .then_with(|| {
                adjacency_direction_sort_key(left.direction)
                    .cmp(&adjacency_direction_sort_key(right.direction))
            })
    });
    records
}

/// Restore persisted adjacency records from a checkpoint snapshot.
pub fn restore_persisted_adjacency_records(
    records: &[PersistedAdjacencyRecord],
) -> GraphStorageResult<GraphAdjacencyStorage> {
    let mut storage = GraphAdjacencyStorage::default();
    let mut next_offset = 0_u64;
    for record in records {
        if record.direction == AdjacencyDirection::Outgoing {
            storage
                .outgoing_records
                .insert(record.owner_node_id.clone(), record.clone());
        } else {
            storage
                .incoming_records
                .insert(record.owner_node_id.clone(), record.clone());
        }
        if let Some(storage_ref) = &record.storage_ref {
            validate_adjacency_storage_ref(storage_ref, record.direction)?;
            let candidate = storage_ref.offset.saturating_add(storage_ref.length);
            if candidate > next_offset {
                next_offset = candidate;
            }
        }
    }
    storage.next_offset = next_offset;
    Ok(storage)
}

fn index_adjacency_storage_ref(
    catalog: &mut GraphCatalog,
    entry: AdjacencyStorageCatalogEntry,
    expected_direction: AdjacencyDirection,
) -> GraphStorageResult<()> {
    validate_adjacency_catalog_entry(catalog, &entry, expected_direction)?;
    catalog
        .historical_records
        .push(HistoricalRecordCatalogEntry {
            record_id: PersistedRecordId::Adjacency {
                owner_node_id: entry.owner_node_id,
                direction: expected_direction,
            },
            kind: persisted_record_kind_for_direction(expected_direction),
            graph_record_version: None,
            storage_ref: entry.storage_ref,
            superseded_by: None,
        });
    Ok(())
}

fn resolve_adjacency_storage_ref(
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
    lookup_mode: AdjacencyStorageLookupMode,
) -> GraphStorageResult<Option<StorageRef>> {
    if let Some(storage_ref) = find_cataloged_adjacency_ref(catalog, owner_node_id, direction) {
        validate_adjacency_storage_ref(&storage_ref, direction)?;
        return Ok(Some(storage_ref));
    }

    match lookup_mode {
        AdjacencyStorageLookupMode::Strict => Err(unknown_adjacency(owner_node_id, direction)),
        AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges => {
            if catalog.latest_node_records.contains_key(owner_node_id) {
                Ok(None)
            } else {
                Err(unknown_adjacency(owner_node_id, direction))
            }
        }
    }
}

fn write_adjacency_by_node_id(
    storage: &mut GraphAdjacencyStorage,
    catalog: &mut GraphCatalog,
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
    entries: Vec<PersistedAdjacencyEntry>,
) -> GraphStorageResult<StorageRef> {
    ensure_known_owner_node(catalog, owner_node_id, direction)?;
    validate_adjacency_entries(owner_node_id, direction, &entries)?;

    let storage_ref = storage.allocate_storage_ref(direction, entries.len());
    let record = PersistedAdjacencyRecord {
        owner_node_id: owner_node_id.clone(),
        direction,
        entries: entries.clone(),
        storage_ref: Some(storage_ref.clone()),
    };
    storage.store_record(record.clone());

    let catalog_entry = AdjacencyStorageCatalogEntry {
        owner_node_id: owner_node_id.clone(),
        direction,
        storage_ref: storage_ref.clone(),
        relationship_count: entries.len(),
        relationship_types: relationship_types_for_entries(&entries),
    };
    index_adjacency_storage_ref(catalog, catalog_entry, direction)?;

    Ok(storage_ref)
}

fn read_adjacency_by_node_id(
    storage: &GraphAdjacencyStorage,
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
    lookup_mode: AdjacencyStorageLookupMode,
) -> GraphStorageResult<PersistedAdjacencyRecord> {
    match resolve_adjacency_storage_ref(catalog, owner_node_id, direction, lookup_mode)? {
        Some(storage_ref) => storage
            .record(owner_node_id, direction)
            .filter(|record| record.storage_ref.as_ref() == Some(&storage_ref))
            .cloned()
            .ok_or_else(|| GraphStorageError::OperationFailed {
                operation: "read_adjacency_by_node_id",
                message: format!(
                    "cataloged {:?} adjacency for {:?} is missing from adjacency storage",
                    direction, owner_node_id
                ),
            }),
        None => Ok(PersistedAdjacencyRecord {
            owner_node_id: owner_node_id.clone(),
            direction,
            entries: Vec::new(),
            storage_ref: None,
        }),
    }
}

impl GraphAdjacencyStorage {
    fn allocate_storage_ref(
        &mut self,
        direction: AdjacencyDirection,
        entry_count: usize,
    ) -> StorageRef {
        let length = entry_count as u64 + 1;
        let storage_ref = StorageRef {
            segment: storage_segment_for_direction(direction),
            offset: self.next_offset,
            length,
            checksum: None,
        };
        self.next_offset += length;
        storage_ref
    }

    fn store_record(&mut self, record: PersistedAdjacencyRecord) {
        match record.direction {
            AdjacencyDirection::Outgoing => {
                self.outgoing_records
                    .insert(record.owner_node_id.clone(), record);
            }
            AdjacencyDirection::Incoming => {
                self.incoming_records
                    .insert(record.owner_node_id.clone(), record);
            }
        }
    }

    fn record(
        &self,
        owner_node_id: &NodeId,
        direction: AdjacencyDirection,
    ) -> Option<&PersistedAdjacencyRecord> {
        match direction {
            AdjacencyDirection::Outgoing => self.outgoing_records.get(owner_node_id),
            AdjacencyDirection::Incoming => self.incoming_records.get(owner_node_id),
        }
    }
}

fn validate_adjacency_catalog_entry(
    catalog: &GraphCatalog,
    entry: &AdjacencyStorageCatalogEntry,
    expected_direction: AdjacencyDirection,
) -> GraphStorageResult<()> {
    if entry.direction != expected_direction {
        return Err(GraphStorageError::InvalidEnvelope {
            reason: format!(
                "adjacency catalog entry direction mismatch: expected {:?}, got {:?}",
                expected_direction, entry.direction
            ),
        });
    }
    ensure_known_owner_node(catalog, &entry.owner_node_id, expected_direction)?;
    validate_adjacency_storage_ref(&entry.storage_ref, expected_direction)?;
    Ok(())
}

fn validate_adjacency_storage_ref(
    storage_ref: &StorageRef,
    direction: AdjacencyDirection,
) -> GraphStorageResult<()> {
    validate_storage_ref(storage_ref)?;
    let expected_segment = storage_segment_for_direction(direction);
    if storage_ref.segment != expected_segment {
        return Err(GraphStorageError::InvalidStorageRef {
            storage_ref: storage_ref.clone(),
            reason: format!(
                "{:?} adjacency storage requires {:?}, got {:?}",
                direction, expected_segment, storage_ref.segment
            ),
        });
    }
    Ok(())
}

fn validate_adjacency_entries(
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
    entries: &[PersistedAdjacencyEntry],
) -> GraphStorageResult<()> {
    for entry in entries {
        if entry.direction != direction {
            return Err(GraphStorageError::InvalidEnvelope {
                reason: format!(
                    "adjacency entry direction mismatch: expected {:?}, got {:?}",
                    direction, entry.direction
                ),
            });
        }

        let owner_matches = match direction {
            AdjacencyDirection::Outgoing => &entry.source_node_id == owner_node_id,
            AdjacencyDirection::Incoming => &entry.target_node_id == owner_node_id,
        };
        if !owner_matches {
            return Err(GraphStorageError::InvalidEnvelope {
                reason: format!(
                    "{:?} adjacency entry endpoints do not match owner node {:?}",
                    direction, owner_node_id
                ),
            });
        }

        if let Some(storage_ref) = &entry.relationship_storage_ref {
            validate_storage_ref_segment(storage_ref, StorageSegment::RelationshipRecords)?;
        }
        if let Some(storage_ref) = &entry.source_node_storage_ref {
            validate_storage_ref_segment(storage_ref, StorageSegment::NodeRecords)?;
        }
        if let Some(storage_ref) = &entry.target_node_storage_ref {
            validate_storage_ref_segment(storage_ref, StorageSegment::NodeRecords)?;
        }
    }
    Ok(())
}

fn validate_storage_ref_segment(
    storage_ref: &StorageRef,
    expected_segment: StorageSegment,
) -> GraphStorageResult<()> {
    validate_storage_ref(storage_ref)?;
    if storage_ref.segment != expected_segment {
        return Err(GraphStorageError::InvalidStorageRef {
            storage_ref: storage_ref.clone(),
            reason: format!(
                "storage reference requires {:?}, got {:?}",
                expected_segment, storage_ref.segment
            ),
        });
    }
    Ok(())
}

fn ensure_known_owner_node(
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
) -> GraphStorageResult<()> {
    if catalog.latest_node_records.contains_key(owner_node_id) {
        Ok(())
    } else {
        Err(unknown_adjacency(owner_node_id, direction))
    }
}

fn relationship_types_for_entries(entries: &[PersistedAdjacencyEntry]) -> Vec<RelationshipType> {
    let mut seen = HashSet::new();
    let mut relationship_types = Vec::new();
    for entry in entries {
        if seen.insert(entry.relationship_type.clone()) {
            relationship_types.push(entry.relationship_type.clone());
        }
    }
    relationship_types
}

fn find_cataloged_adjacency_ref(
    catalog: &GraphCatalog,
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
) -> Option<StorageRef> {
    catalog
        .historical_records
        .iter()
        .rev()
        .find_map(|entry| match (&entry.record_id, entry.kind) {
            (
                PersistedRecordId::Adjacency {
                    owner_node_id: entry_owner_node_id,
                    direction: entry_direction,
                },
                kind,
            ) if entry_owner_node_id == owner_node_id
                && *entry_direction == direction
                && kind == persisted_record_kind_for_direction(direction) =>
            {
                Some(entry.storage_ref.clone())
            }
            _ => None,
        })
}

fn persisted_record_kind_for_direction(direction: AdjacencyDirection) -> PersistedRecordKind {
    match direction {
        AdjacencyDirection::Outgoing => PersistedRecordKind::OutgoingAdjacency,
        AdjacencyDirection::Incoming => PersistedRecordKind::IncomingAdjacency,
    }
}

fn storage_segment_for_direction(direction: AdjacencyDirection) -> StorageSegment {
    match direction {
        AdjacencyDirection::Outgoing => StorageSegment::OutgoingAdjacency,
        AdjacencyDirection::Incoming => StorageSegment::IncomingAdjacency,
    }
}

fn adjacency_direction_sort_key(direction: AdjacencyDirection) -> u8 {
    match direction {
        AdjacencyDirection::Outgoing => 0,
        AdjacencyDirection::Incoming => 1,
    }
}

fn unknown_adjacency(owner_node_id: &NodeId, direction: AdjacencyDirection) -> GraphStorageError {
    GraphStorageError::UnknownNodeAdjacencyCatalogEntry {
        node_id: owner_node_id.clone(),
        direction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::NodeVersionId;

    use crate::{
        GraphRecordVersion, LatestRecordCatalogEntry, PersistedRecordId, PersistedRecordKind,
        RecordChecksum, StorageSegment,
    };

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node id should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship id should be valid")
    }

    fn relationship_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
    }

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

    fn latest_node_entry(node_id: &NodeId) -> LatestRecordCatalogEntry {
        LatestRecordCatalogEntry {
            // Record id.
            record_id: PersistedRecordId::Node(node_id.clone()),
            // Kind.
            kind: PersistedRecordKind::Node,
            // Graph record version.
            graph_record_version: Some(GraphRecordVersion::Node {
                // Version id.
                version_id: NodeVersionId::new(format!("{}--version-1", node_id.as_str()))
                    .expect("test node version id should be valid"),
                // Version.
                version: 1,
                // Current.
                current: true,
                // Previous version id.
                previous_version_id: None,
            }),
            // Storage ref.
            storage_ref: storage_ref(StorageSegment::NodeRecords, 100),
        }
    }

    fn catalog_with_known_node(node_id: &NodeId) -> GraphCatalog {
        let mut catalog = GraphCatalog::default();
        catalog
            .latest_node_records
            .insert(node_id.clone(), latest_node_entry(node_id));
        catalog
    }

    fn adjacency_entry(direction: AdjacencyDirection) -> PersistedAdjacencyEntry {
        PersistedAdjacencyEntry {
            // Relationship id.
            relationship_id: relationship_id("relationship--amplifies-1"),
            // Source node id.
            source_node_id: node_id("node--source-1"),
            // Target node id.
            target_node_id: node_id("node--target-1"),
            // Relationship type.
            relationship_type: relationship_type("AMPLIFIES"),
            direction,
            // Relationship storage ref.
            relationship_storage_ref: Some(storage_ref(StorageSegment::RelationshipRecords, 200)),
            // Source node storage ref.
            source_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 300)),
            // Target node storage ref.
            target_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 400)),
        }
    }

    //
    // Specify that outgoing adjacency is persisted independently from node and
    // relationship payloads, indexed through a catalog reference, and readable as
    // lightweight warm-frontier metadata.
    #[test]
    fn write_outgoing_adjacency_persists_reference_and_readable_record() {
        let owner = node_id("node--source-1");
        let mut storage = GraphAdjacencyStorage::default();
        let mut catalog = catalog_with_known_node(&owner);
        let entry = adjacency_entry(AdjacencyDirection::Outgoing);

        let adjacency_ref = write_outgoing_adjacency_by_node_id(
            &mut storage,
            &mut catalog,
            &owner,
            vec![entry.clone()],
        )
        .expect("outgoing adjacency write should succeed once implemented");

        assert_eq!(adjacency_ref.segment, StorageSegment::OutgoingAdjacency);
        assert_eq!(
            resolve_outgoing_adjacency_storage_ref(
                &catalog,
                &owner,
                AdjacencyStorageLookupMode::Strict,
            ),
            Ok(Some(adjacency_ref.clone()))
        );

        let record = read_outgoing_adjacency_by_node_id(
            &storage,
            &catalog,
            &owner,
            AdjacencyStorageLookupMode::Strict,
        )
        .expect("outgoing adjacency read should succeed once implemented");

        assert_eq!(record.owner_node_id, owner);
        assert_eq!(record.direction, AdjacencyDirection::Outgoing);
        assert_eq!(record.storage_ref, Some(adjacency_ref));
        assert_eq!(record.entries, vec![entry]);
    }

    //
    // Specify that incoming adjacency is persisted independently from outgoing
    // adjacency and can be read through the incoming node-owned lookup path.
    #[test]
    fn write_incoming_adjacency_persists_reference_and_readable_record() {
        let owner = node_id("node--target-1");
        let mut storage = GraphAdjacencyStorage::default();
        let mut catalog = catalog_with_known_node(&owner);
        let entry = adjacency_entry(AdjacencyDirection::Incoming);

        let adjacency_ref = write_incoming_adjacency_by_node_id(
            &mut storage,
            &mut catalog,
            &owner,
            vec![entry.clone()],
        )
        .expect("incoming adjacency write should succeed once implemented");

        assert_eq!(adjacency_ref.segment, StorageSegment::IncomingAdjacency);
        assert_eq!(
            resolve_incoming_adjacency_storage_ref(
                &catalog,
                &owner,
                AdjacencyStorageLookupMode::Strict,
            ),
            Ok(Some(adjacency_ref.clone()))
        );

        let record = read_incoming_adjacency_by_node_id(
            &storage,
            &catalog,
            &owner,
            AdjacencyStorageLookupMode::Strict,
        )
        .expect("incoming adjacency read should succeed once implemented");

        assert_eq!(record.owner_node_id, owner);
        assert_eq!(record.direction, AdjacencyDirection::Incoming);
        assert_eq!(record.storage_ref, Some(adjacency_ref));
        assert_eq!(record.entries, vec![entry]);
    }

    //
    // Specify deterministic empty adjacency behavior for a known node that has no
    // outgoing edges, without requiring the caller to load the full node payload.
    #[test]
    fn read_outgoing_adjacency_returns_empty_record_for_known_node_with_no_edges() {
        let owner = node_id("node--known-empty-outgoing");
        let storage = GraphAdjacencyStorage::default();
        let catalog = catalog_with_known_node(&owner);

        let record = read_outgoing_adjacency_by_node_id(
            &storage,
            &catalog,
            &owner,
            AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
        )
        .expect("known empty outgoing adjacency should be deterministic once implemented");

        assert_eq!(record.owner_node_id, owner);
        assert_eq!(record.direction, AdjacencyDirection::Outgoing);
        assert!(record.entries.is_empty());
        assert_eq!(record.storage_ref, None);
    }

    //
    // Specify deterministic empty adjacency behavior for a known node that has no
    // incoming edges, without requiring the caller to load full relationship
    // payloads to prove absence.
    #[test]
    fn read_incoming_adjacency_returns_empty_record_for_known_node_with_no_edges() {
        let owner = node_id("node--known-empty-incoming");
        let storage = GraphAdjacencyStorage::default();
        let catalog = catalog_with_known_node(&owner);

        let record = read_incoming_adjacency_by_node_id(
            &storage,
            &catalog,
            &owner,
            AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
        )
        .expect("known empty incoming adjacency should be deterministic once implemented");

        assert_eq!(record.owner_node_id, owner);
        assert_eq!(record.direction, AdjacencyDirection::Incoming);
        assert!(record.entries.is_empty());
        assert_eq!(record.storage_ref, None);
    }

    //
    // Specify that strict outgoing adjacency lookup for an unknown owner returns a
    // typed adjacency error instead of silently returning empty data or loading
    // payloads as a fallback.
    #[test]
    fn strict_outgoing_lookup_reports_unknown_node_adjacency() {
        let catalog = GraphCatalog::default();
        let missing = node_id("node--missing-outgoing");

        let error = resolve_outgoing_adjacency_storage_ref(
            &catalog,
            &missing,
            AdjacencyStorageLookupMode::Strict,
        )
        .expect_err("unknown outgoing adjacency should be explicit once implemented");

        assert!(matches!(
        error,
        GraphStorageError::UnknownNodeAdjacencyCatalogEntry { node_id, direction }
        if node_id == missing && direction == AdjacencyDirection::Outgoing
        ));
    }

    //
    // Specify that strict incoming adjacency lookup for an unknown owner returns a
    // typed adjacency error with incoming direction preserved.
    #[test]
    fn strict_incoming_lookup_reports_unknown_node_adjacency() {
        let catalog = GraphCatalog::default();
        let missing = node_id("node--missing-incoming");

        let error = resolve_incoming_adjacency_storage_ref(
            &catalog,
            &missing,
            AdjacencyStorageLookupMode::Strict,
        )
        .expect_err("unknown incoming adjacency should be explicit once implemented");

        assert!(matches!(
        error,
        GraphStorageError::UnknownNodeAdjacencyCatalogEntry { node_id, direction }
        if node_id == missing && direction == AdjacencyDirection::Incoming
        ));
    }

    //
    // Verify the persisted adjacency entry shape carries enough lightweight
    // metadata for warm working-set boundaries and future lazy page-in without
    // embedding full node or relationship payloads.
    #[test]
    fn persisted_adjacency_entry_carries_warm_frontier_metadata() {
        let entry = adjacency_entry(AdjacencyDirection::Outgoing);

        assert_eq!(
            entry.relationship_id,
            relationship_id("relationship--amplifies-1")
        );
        assert_eq!(entry.source_node_id, node_id("node--source-1"));
        assert_eq!(entry.target_node_id, node_id("node--target-1"));
        assert_eq!(entry.relationship_type, relationship_type("AMPLIFIES"));
        assert_eq!(entry.direction, AdjacencyDirection::Outgoing);
        assert_eq!(
            entry
                .relationship_storage_ref
                .as_ref()
                .map(|storage_ref| &storage_ref.segment),
            Some(&StorageSegment::RelationshipRecords)
        );
        assert_eq!(
            entry
                .source_node_storage_ref
                .as_ref()
                .map(|storage_ref| &storage_ref.segment),
            Some(&StorageSegment::NodeRecords)
        );
        assert_eq!(
            entry
                .target_node_storage_ref
                .as_ref()
                .map(|storage_ref| &storage_ref.segment),
            Some(&StorageSegment::NodeRecords)
        );
    }
}
