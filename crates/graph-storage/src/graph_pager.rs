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
//! File-backed graph pager boundary for persistent graph storage.
//!
//! Design note:
//!
//! - implement the storage-layer adapter for the `GraphPager` contract;
//! - load node and relationship payloads from cataloged file offsets;
//! - load outgoing and incoming adjacency as lightweight warm-frontier records;
//! - expose indexed metadata without full payload hydration;
//! - map storage errors into deterministic pager errors;
//! - keep working-set management, semantic seed selection, prefetching, and
//!   eviction policy outside this storage adapter.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use graph_core::{
    AdjacencyDirection, GraphPager, GraphPagerError, GraphPagerResult, GraphRecordMetadata,
    GraphRecordRef, LoadingState, Node, NodeId, PagedAdjacency, PagedAdjacencyEntry, PagedNode,
    PagedRelationship, PropertyMap, Relationship, RelationshipId, StorageRef as PagerStorageRef,
};

use crate::{
    AdjacencyStorageLookupMode, GraphAdjacencyStorage, GraphCatalog, GraphStorageError,
    GraphStorageResult, JsonLinesRecordCodec, PersistedAdjacencyRecord, StorageRef, StorageRoot,
    StorageSegment, read_incoming_adjacency_by_node_id, read_outgoing_adjacency_by_node_id,
    resolve_latest_node_storage_ref, resolve_latest_relationship_storage_ref,
    validate_encoded_record_checksum, validate_storage_ref,
};

/// File-backed persistent graph store handle consumed by `FileBackedGraphPager`.
///
///
/// - Keep the pager construction boundary centered on one store handle.
/// - Represent the storage root, catalog, and adjacency storage already introduced
///   by .
/// - Avoid loading the full graph during construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileBackedGraphStore {
    root: StorageRoot,
    catalog: GraphCatalog,
    adjacency_storage: GraphAdjacencyStorage,
}

impl FileBackedGraphStore {
    /// Construct a file-backed store handle from already-opened storage parts.
    pub fn from_parts(
        root: StorageRoot,
        catalog: GraphCatalog,
        adjacency_storage: GraphAdjacencyStorage,
    ) -> GraphStorageResult<Self> {
        Ok(Self {
            root,
            catalog,
            adjacency_storage,
        })
    }

    /// Return the storage root backing this file-backed graph store.
    pub fn root(&self) -> &StorageRoot {
        &self.root
    }

    /// Return the catalog used to resolve graph IDs to persisted storage refs.
    pub fn catalog(&self) -> &GraphCatalog {
        &self.catalog
    }

    /// Return the adjacency storage handle used by adjacency page reads.
    pub fn adjacency_storage(&self) -> &GraphAdjacencyStorage {
        &self.adjacency_storage
    }
}

/// Construct a file-backed graph store handle.
pub fn create_file_backed_graph_store(
    root: StorageRoot,
    catalog: GraphCatalog,
    adjacency_storage: GraphAdjacencyStorage,
) -> GraphStorageResult<FileBackedGraphStore> {
    FileBackedGraphStore::from_parts(root, catalog, adjacency_storage)
}

/// File-backed implementation of the `GraphPager` trait.
///
///
/// Hide catalog lookup, file offsets, adjacency storage, and metadata indexes
/// behind the graph-core pager API so working-set callers do not need to know the
/// persistent layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileBackedGraphPager {
    store: FileBackedGraphStore,
}

impl FileBackedGraphPager {
    /// Construct a file-backed pager from a file-backed graph store.
    pub fn new(store: FileBackedGraphStore) -> Self {
        Self { store }
    }

    /// Return the underlying file-backed store handle.
    pub fn store(&self) -> &FileBackedGraphStore {
        &self.store
    }
}

/// Construct a file-backed graph pager.
pub fn create_file_backed_graph_pager(
    store: FileBackedGraphStore,
) -> GraphStorageResult<FileBackedGraphPager> {
    Ok(FileBackedGraphPager::new(store))
}

impl GraphPager for FileBackedGraphPager {
    /// Load a node payload from persisted storage by stable node ID.
    fn load_node_payload(&self, node_id: &NodeId) -> GraphPagerResult<PagedNode> {
        let record_ref = GraphRecordRef::Node(node_id.clone());
        let storage_ref =
            resolve_latest_node_storage_ref(self.store.catalog(), node_id).map_err(|error| {
                map_storage_error_to_graph_pager_error(error, record_ref.clone(), None)
            })?;
        let bytes = read_storage_ref_bytes(self.store.root(), &storage_ref, &record_ref)?;
        let node = decode_node_payload(&bytes, &record_ref, &storage_ref)?;

        if node.id() != node_id {
            return Err(corrupted_page(
                &record_ref,
                Some(&storage_ref),
                format!(
                    "decoded node id {} does not match requested {}",
                    node.id().as_str(),
                    node_id.as_str()
                ),
            ));
        }

        Ok(PagedNode {
            node,
            storage_ref: Some(pager_storage_ref_from_storage_ref(&storage_ref)),
        })
    }

    /// Load a relationship payload from persisted storage by stable relationship ID.
    fn load_relationship_payload(
        &self,
        relationship_id: &RelationshipId,
    ) -> GraphPagerResult<PagedRelationship> {
        let record_ref = GraphRecordRef::Relationship(relationship_id.clone());
        let storage_ref =
            resolve_latest_relationship_storage_ref(self.store.catalog(), relationship_id)
                .map_err(|error| {
                    map_storage_error_to_graph_pager_error(error, record_ref.clone(), None)
                })?;
        let bytes = read_storage_ref_bytes(self.store.root(), &storage_ref, &record_ref)?;
        let relationship = decode_relationship_payload(&bytes, &record_ref, &storage_ref)?;

        if relationship.id() != relationship_id {
            return Err(corrupted_page(
                &record_ref,
                Some(&storage_ref),
                format!(
                    "decoded relationship id {} does not match requested {}",
                    relationship.id().as_str(),
                    relationship_id.as_str()
                ),
            ));
        }

        Ok(PagedRelationship {
            relationship,
            storage_ref: Some(pager_storage_ref_from_storage_ref(&storage_ref)),
        })
    }

    /// Load lightweight outgoing adjacency for a node without loading payloads.
    fn load_outgoing_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        let record_ref = GraphRecordRef::Node(node_id.clone());
        let record = read_outgoing_adjacency_by_node_id(
            self.store.adjacency_storage(),
            self.store.catalog(),
            node_id,
            AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
        )
        .map_err(|error| map_storage_error_to_graph_pager_error(error, record_ref, None))?;
        Ok(paged_adjacency_from_persisted(record))
    }

    /// Load lightweight incoming adjacency for a node without loading payloads.
    fn load_incoming_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        let record_ref = GraphRecordRef::Node(node_id.clone());
        let record = read_incoming_adjacency_by_node_id(
            self.store.adjacency_storage(),
            self.store.catalog(),
            node_id,
            AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
        )
        .map_err(|error| map_storage_error_to_graph_pager_error(error, record_ref, None))?;
        Ok(paged_adjacency_from_persisted(record))
    }

    /// Load selected indexed metadata without loading the full graph payload.
    fn load_indexed_metadata(
        &self,
        record_ref: &GraphRecordRef,
    ) -> GraphPagerResult<GraphRecordMetadata> {
        match record_ref {
            GraphRecordRef::Node(node_id) => self.load_node_indexed_metadata(node_id),
            GraphRecordRef::Relationship(relationship_id) => {
                self.load_relationship_indexed_metadata(relationship_id)
            }
        }
    }
}

impl FileBackedGraphPager {
    fn load_node_indexed_metadata(
        &self,
        node_id: &NodeId,
    ) -> GraphPagerResult<GraphRecordMetadata> {
        let record_ref = GraphRecordRef::Node(node_id.clone());
        let storage_ref =
            resolve_latest_node_storage_ref(self.store.catalog(), node_id).map_err(|error| {
                map_storage_error_to_graph_pager_error(error, record_ref.clone(), None)
            })?;

        Ok(GraphRecordMetadata {
            record_ref,
            storage_ref: Some(pager_storage_ref_from_storage_ref(&storage_ref)),
            loading_state: LoadingState::Indexed,
            labels: indexed_labels_for_node(self.store.catalog(), node_id),
            relationship_type: None,
            indexed_properties: PropertyMap::new(),
        })
    }

    fn load_relationship_indexed_metadata(
        &self,
        relationship_id: &RelationshipId,
    ) -> GraphPagerResult<GraphRecordMetadata> {
        let record_ref = GraphRecordRef::Relationship(relationship_id.clone());
        let storage_ref =
            resolve_latest_relationship_storage_ref(self.store.catalog(), relationship_id)
                .map_err(|error| {
                    map_storage_error_to_graph_pager_error(error, record_ref.clone(), None)
                })?;

        Ok(GraphRecordMetadata {
            record_ref,
            storage_ref: Some(pager_storage_ref_from_storage_ref(&storage_ref)),
            loading_state: LoadingState::Indexed,
            labels: Vec::new(),
            relationship_type: indexed_type_for_relationship(self.store.catalog(), relationship_id),
            indexed_properties: PropertyMap::new(),
        })
    }
}

/// Map a storage-layer failure into the graph-core pager error model.
pub fn map_storage_error_to_graph_pager_error(
    error: GraphStorageError,
    record_ref: GraphRecordRef,
    storage_ref: Option<&StorageRef>,
) -> GraphPagerError {
    match &error {
        GraphStorageError::MissingNodeCatalogEntry { node_id } => {
            GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Node(node_id.clone()),
            }
        }
        GraphStorageError::MissingRelationshipCatalogEntry { relationship_id } => {
            GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Relationship(relationship_id.clone()),
            }
        }
        GraphStorageError::UnknownNodeAdjacencyCatalogEntry { node_id, .. } => {
            GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Node(node_id.clone()),
            }
        }
        GraphStorageError::StorageRootNotFound { .. }
        | GraphStorageError::ManifestNotFound { .. }
        | GraphStorageError::CatalogRebuildSourceMissing { .. } => GraphPagerError::MissingPage {
            storage_ref: pager_storage_ref_or_record_ref(&record_ref, storage_ref),
        },
        GraphStorageError::ChecksumMismatch { .. }
        | GraphStorageError::DecodeFailed { .. }
        | GraphStorageError::InvalidEnvelope { .. }
        | GraphStorageError::InvalidStorageRef { .. }
        | GraphStorageError::UnexpectedRecordKind { .. }
        | GraphStorageError::CatalogRebuildCorruptedRecord { .. }
        | GraphStorageError::ManifestCorrupted { .. } => GraphPagerError::CorruptedPage {
            storage_ref: pager_storage_ref_or_record_ref(&record_ref, storage_ref),
            reason: error.to_string(),
        },
        _ => GraphPagerError::CorruptedPage {
            storage_ref: pager_storage_ref_or_record_ref(&record_ref, storage_ref),
            reason: error.to_string(),
        },
    }
}

/// Convert a persistent storage reference into the graph-core pager storage-ref shape.
pub fn pager_storage_ref_from_storage_ref(storage_ref: &StorageRef) -> PagerStorageRef {
    PagerStorageRef::Offset {
        segment: format!("{:?}", storage_ref.segment),
        byte_offset: storage_ref.offset,
    }
}

fn read_storage_ref_bytes(
    root: &StorageRoot,
    storage_ref: &StorageRef,
    record_ref: &GraphRecordRef,
) -> GraphPagerResult<Vec<u8>> {
    validate_storage_ref(storage_ref).map_err(|error| {
        map_storage_error_to_graph_pager_error(error, record_ref.clone(), Some(storage_ref))
    })?;

    let path = storage_segment_path(root, storage_ref, record_ref)?;
    let pager_ref = pager_storage_ref_from_storage_ref(storage_ref);
    if !path.is_file() {
        return Err(GraphPagerError::MissingPage {
            storage_ref: pager_ref,
        });
    }

    let file_len = fs::metadata(&path)
        .map_err(|error| corrupted_page(record_ref, Some(storage_ref), error.to_string()))?
        .len();
    let end = storage_ref
        .offset
        .checked_add(storage_ref.length)
        .ok_or_else(|| {
            corrupted_page(
                record_ref,
                Some(storage_ref),
                "storage reference offset plus length overflows".to_owned(),
            )
        })?;
    if end > file_len {
        return Err(GraphPagerError::MissingPage {
            storage_ref: pager_ref,
        });
    }

    let length = usize::try_from(storage_ref.length)
        .map_err(|error| corrupted_page(record_ref, Some(storage_ref), error.to_string()))?;
    let mut bytes = vec![0; length];
    let mut file = File::open(&path)
        .map_err(|error| corrupted_page(record_ref, Some(storage_ref), error.to_string()))?;
    file.seek(SeekFrom::Start(storage_ref.offset))
        .map_err(|error| corrupted_page(record_ref, Some(storage_ref), error.to_string()))?;
    file.read_exact(&mut bytes)
        .map_err(|error| corrupted_page(record_ref, Some(storage_ref), error.to_string()))?;

    if let Some(checksum) = &storage_ref.checksum {
        validate_encoded_record_checksum(&JsonLinesRecordCodec, &bytes, checksum).map_err(
            |error| {
                map_storage_error_to_graph_pager_error(error, record_ref.clone(), Some(storage_ref))
            },
        )?;
    }

    Ok(bytes)
}

fn storage_segment_path(
    root: &StorageRoot,
    storage_ref: &StorageRef,
    record_ref: &GraphRecordRef,
) -> GraphPagerResult<PathBuf> {
    match storage_ref.segment {
        StorageSegment::NodeRecords => Ok(root.path().join("nodes").join("node_records.log")),
        StorageSegment::RelationshipRecords => Ok(root
            .path()
            .join("relationships")
            .join("relationship_records.log")),
        _ => Err(corrupted_page(
            record_ref,
            Some(storage_ref),
            format!(
                "file-backed pager cannot load payloads from {:?}",
                storage_ref.segment
            ),
        )),
    }
}

fn decode_node_payload(
    bytes: &[u8],
    record_ref: &GraphRecordRef,
    storage_ref: &StorageRef,
) -> GraphPagerResult<Node> {
    serde_json::from_slice(bytes).map_err(|error| {
        let storage_error = GraphStorageError::DecodeFailed {
            format: "JsonLinesV1".to_owned(),
            reason: error.to_string(),
        };
        map_storage_error_to_graph_pager_error(storage_error, record_ref.clone(), Some(storage_ref))
    })
}

fn decode_relationship_payload(
    bytes: &[u8],
    record_ref: &GraphRecordRef,
    storage_ref: &StorageRef,
) -> GraphPagerResult<Relationship> {
    serde_json::from_slice(bytes).map_err(|error| {
        let storage_error = GraphStorageError::DecodeFailed {
            format: "JsonLinesV1".to_owned(),
            reason: error.to_string(),
        };
        map_storage_error_to_graph_pager_error(storage_error, record_ref.clone(), Some(storage_ref))
    })
}

fn paged_adjacency_from_persisted(record: PersistedAdjacencyRecord) -> PagedAdjacency {
    let direction = record.direction;
    let entries = record
        .entries
        .into_iter()
        .map(|entry| {
            let neighbor_node_id = match direction {
                AdjacencyDirection::Outgoing => entry.target_node_id.clone(),
                AdjacencyDirection::Incoming => entry.source_node_id.clone(),
            };
            let neighbor_storage_ref = match direction {
                AdjacencyDirection::Outgoing => entry.target_node_storage_ref.as_ref(),
                AdjacencyDirection::Incoming => entry.source_node_storage_ref.as_ref(),
            }
            .map(pager_storage_ref_from_storage_ref);

            PagedAdjacencyEntry {
                // Relationship id.
                relationship_id: entry.relationship_id,
                neighbor_node_id,
                // Relationship type.
                relationship_type: Some(entry.relationship_type),
                // Relationship storage ref.
                relationship_storage_ref: entry
                    .relationship_storage_ref
                    .as_ref()
                    .map(pager_storage_ref_from_storage_ref),
                neighbor_storage_ref,
            }
        })
        .collect();

    PagedAdjacency {
        // Owner node id.
        owner_node_id: record.owner_node_id,
        direction,
        entries,
        // Storage ref.
        storage_ref: record
            .storage_ref
            .as_ref()
            .map(pager_storage_ref_from_storage_ref),
    }
}

fn indexed_labels_for_node(catalog: &GraphCatalog, node_id: &NodeId) -> Vec<String> {
    let mut labels: Vec<String> = catalog
        .metadata_indexes
        .labels
        .iter()
        .filter_map(|(label, entry)| {
            entry
                .nodes
                .iter()
                .any(|metadata| &metadata.node_id == node_id)
                .then(|| label.clone())
        })
        .collect();
    labels.sort();
    labels
}

fn indexed_type_for_relationship(
    catalog: &GraphCatalog,
    relationship_id: &RelationshipId,
) -> Option<graph_core::RelationshipType> {
    catalog
        .metadata_indexes
        .relationship_types
        .iter()
        .find_map(|(relationship_type, entry)| {
            entry
                .relationships
                .iter()
                .any(|metadata| &metadata.relationship_id == relationship_id)
                .then(|| relationship_type.clone())
        })
}

fn pager_storage_ref_or_record_ref(
    record_ref: &GraphRecordRef,
    storage_ref: Option<&StorageRef>,
) -> PagerStorageRef {
    storage_ref
        .map(pager_storage_ref_from_storage_ref)
        .unwrap_or_else(|| fallback_pager_storage_ref(record_ref))
}

fn fallback_pager_storage_ref(record_ref: &GraphRecordRef) -> PagerStorageRef {
    match record_ref {
        GraphRecordRef::Node(node_id) => PagerStorageRef::Record {
            collection: "nodes".to_owned(),
            key: node_id.as_str().to_owned(),
        },
        GraphRecordRef::Relationship(relationship_id) => PagerStorageRef::Record {
            collection: "relationships".to_owned(),
            key: relationship_id.as_str().to_owned(),
        },
    }
}

fn corrupted_page(
    record_ref: &GraphRecordRef,
    storage_ref: Option<&StorageRef>,
    reason: String,
) -> GraphPagerError {
    GraphPagerError::CorruptedPage {
        storage_ref: pager_storage_ref_or_record_ref(record_ref, storage_ref),
        reason,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::{
        GraphId, GraphRecordVersion, LabelIndexNodeMetadata, LatestRecordCatalogEntry,
        PersistedAdjacencyEntry, PersistedRecordId, PersistedRecordKind, RecordChecksum,
        RecordFormat, RelationshipTypeIndexRelationshipMetadata, StorageManifest, StorageTimestamp,
        StorageVersion, calculate_encoded_record_checksum, create_node_record_envelope,
        create_relationship_record_envelope, create_storage_root, index_appended_node_record,
        index_appended_relationship_record, index_node_labels, index_relationship_type,
        write_incoming_adjacency_by_node_id, write_outgoing_adjacency_by_node_id,
    };
    use graph_core::{Graph, NodeInput, PropertyValue, RelationshipInput, RelationshipType};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "intelligence_graph_engine_issue_59_{test_name}_{}_{}",
            std::process::id(),
            unique
        ))
    }

    fn manifest() -> StorageManifest {
        StorageManifest {
            // Storage version.
            storage_version: StorageVersion::V1,
            // Graph id.
            graph_id: GraphId {
                // Value.
                value: "graph--issue-59".to_owned(),
            },
            // Created at.
            created_at: StorageTimestamp {
                // Value.
                value: "2026-07-05T00:00:00Z".to_owned(),
            },
            // Updated at.
            updated_at: StorageTimestamp {
                // Value.
                value: "2026-07-05T00:00:00Z".to_owned(),
            },
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
        }
    }

    fn storage_root(test_name: &str) -> StorageRoot {
        let path = unique_temp_path(test_name);
        let _ = fs::remove_dir_all(&path);
        create_storage_root(path, manifest()).unwrap()
    }

    fn graph_fixture() -> (Graph, NodeId, NodeId, RelationshipId) {
        let mut graph = Graph::new();
        let source = graph
            .create_node(
                NodeInput::new(["Campaign", "FIMI"])
                    .with_property("name", PropertyValue::String("campaign-alpha".to_owned())),
            )
            .unwrap();
        let target = graph
            .create_node(NodeInput::new(["Infrastructure"]))
            .unwrap();
        let relationship = graph
            .create_relationship(
                RelationshipInput::new(source.clone(), "USES", target.clone())
                    .unwrap()
                    .with_property("confidence", PropertyValue::Integer(80)),
            )
            .unwrap();
        (graph, source, target, relationship)
    }

    fn payload_path(root: &StorageRoot, segment: StorageSegment) -> PathBuf {
        match segment {
            StorageSegment::NodeRecords => root.path().join("nodes").join("node_records.log"),
            StorageSegment::RelationshipRecords => root
                .path()
                .join("relationships")
                .join("relationship_records.log"),
            _ => panic!("test payload segment must be node or relationship records"),
        }
    }

    fn write_payload(
        root: &StorageRoot,
        segment: StorageSegment,
        bytes: &[u8],
        checksum: Option<RecordChecksum>,
    ) -> StorageRef {
        let path = payload_path(root, segment.clone());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let offset = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        let checksum = checksum.or_else(|| {
            Some(calculate_encoded_record_checksum(&JsonLinesRecordCodec, bytes).unwrap())
        });
        StorageRef {
            segment,
            offset,
            // Length.
            length: bytes.len() as u64,
            checksum,
        }
    }

    fn index_node(
        catalog: &mut GraphCatalog,
        node: &Node,
        storage_ref: StorageRef,
        labels: Vec<String>,
    ) {
        let envelope = create_node_record_envelope(
            node,
            storage_ref.clone(),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            storage_ref.checksum.clone(),
        )
        .unwrap();
        index_appended_node_record(catalog, &envelope, storage_ref.clone()).unwrap();
        index_node_labels(
            catalog,
            &labels,
            LabelIndexNodeMetadata {
                // Node id.
                node_id: node.id().clone(),
                // Latest storage ref.
                latest_storage_ref: Some(storage_ref),
                // Graph record version.
                graph_record_version: Some(GraphRecordVersion::Node {
                    // Version id.
                    version_id: node.version_id().clone(),
                    // Version.
                    version: node.version(),
                    // Current.
                    current: node.is_current(),
                    // Previous version id.
                    previous_version_id: node.previous_version_id().cloned(),
                }),
            },
        )
        .unwrap();
    }

    fn index_relationship(
        catalog: &mut GraphCatalog,
        relationship: &Relationship,
        storage_ref: StorageRef,
    ) {
        let envelope = create_relationship_record_envelope(
            relationship,
            storage_ref.clone(),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            storage_ref.checksum.clone(),
        )
        .unwrap();
        index_appended_relationship_record(catalog, &envelope, storage_ref.clone()).unwrap();
        index_relationship_type(
            catalog,
            relationship.rel_type(),
            RelationshipTypeIndexRelationshipMetadata {
                // Relationship id.
                relationship_id: relationship.id().clone(),
                // Latest storage ref.
                latest_storage_ref: Some(storage_ref),
                // Graph record version.
                graph_record_version: Some(GraphRecordVersion::Relationship {
                    // Version id.
                    version_id: relationship.version_id().clone(),
                    // Version.
                    version: relationship.version(),
                    // Current.
                    current: relationship.is_current(),
                    // Previous version id.
                    previous_version_id: relationship.previous_version_id().cloned(),
                }),
            },
        )
        .unwrap();
    }

    fn pager_fixture(
        test_name: &str,
    ) -> (
        StorageRoot,
        FileBackedGraphPager,
        NodeId,
        NodeId,
        RelationshipId,
    ) {
        let root = storage_root(test_name);
        let (graph, source_id, target_id, relationship_id) = graph_fixture();
        let source = graph.get_node(&source_id).unwrap().unwrap();
        let target = graph.get_node(&target_id).unwrap().unwrap();
        let relationship = graph.get_relationship(&relationship_id).unwrap().unwrap();

        let mut catalog = GraphCatalog::default();
        let source_ref = write_payload(
            &root,
            StorageSegment::NodeRecords,
            &serde_json::to_vec(&source).unwrap(),
            None,
        );
        let target_ref = write_payload(
            &root,
            StorageSegment::NodeRecords,
            &serde_json::to_vec(&target).unwrap(),
            None,
        );
        let relationship_ref = write_payload(
            &root,
            StorageSegment::RelationshipRecords,
            &serde_json::to_vec(&relationship).unwrap(),
            None,
        );

        index_node(
            &mut catalog,
            &source,
            source_ref.clone(),
            vec!["Campaign".to_owned(), "FIMI".to_owned()],
        );
        index_node(
            &mut catalog,
            &target,
            target_ref.clone(),
            vec!["Infrastructure".to_owned()],
        );
        index_relationship(&mut catalog, &relationship, relationship_ref.clone());

        let mut adjacency_storage = GraphAdjacencyStorage::default();
        write_outgoing_adjacency_by_node_id(
            &mut adjacency_storage,
            &mut catalog,
            &source_id,
            vec![PersistedAdjacencyEntry {
                relationship_id: relationship_id.clone(),
                source_node_id: source_id.clone(),
                target_node_id: target_id.clone(),
                relationship_type: RelationshipType::new("USES").unwrap(),
                direction: AdjacencyDirection::Outgoing,
                relationship_storage_ref: Some(relationship_ref.clone()),
                source_node_storage_ref: Some(source_ref.clone()),
                target_node_storage_ref: Some(target_ref.clone()),
            }],
        )
        .unwrap();
        write_incoming_adjacency_by_node_id(
            &mut adjacency_storage,
            &mut catalog,
            &target_id,
            vec![PersistedAdjacencyEntry {
                relationship_id: relationship_id.clone(),
                source_node_id: source_id.clone(),
                target_node_id: target_id.clone(),
                relationship_type: RelationshipType::new("USES").unwrap(),
                direction: AdjacencyDirection::Incoming,
                relationship_storage_ref: Some(relationship_ref),
                source_node_storage_ref: Some(source_ref),
                target_node_storage_ref: Some(target_ref),
            }],
        )
        .unwrap();

        let store =
            create_file_backed_graph_store(root.clone(), catalog, adjacency_storage).unwrap();
        let pager = create_file_backed_graph_pager(store).unwrap();
        (root, pager, source_id, target_id, relationship_id)
    }

    /// Construction should not require full graph payload loading.
    #[test]
    fn file_backed_pager_can_be_constructed_from_store() {
        let root = storage_root("construct_pager");
        let store = create_file_backed_graph_store(
            root.clone(),
            GraphCatalog::default(),
            GraphAdjacencyStorage::default(),
        )
        .unwrap();
        let pager = create_file_backed_graph_pager(store).unwrap();
        assert_eq!(pager.store().root().path(), root.path());
        let _ = fs::remove_dir_all(root.path());
    }

    /// Node payload loading should page in the cataloged node record.
    #[test]
    fn load_node_payload_reads_cataloged_file_backed_node_record() {
        let (root, pager, source_id, _, _) = pager_fixture("load_node_payload");
        let paged = pager.load_node_payload(&source_id).unwrap();
        assert_eq!(paged.node.id(), &source_id);
        assert!(paged.node.has_label("Campaign"));
        assert!(matches!(
            paged.storage_ref,
            Some(PagerStorageRef::Offset { .. })
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    /// Relationship payload loading should page in the cataloged relationship record.
    #[test]
    fn load_relationship_payload_reads_cataloged_file_backed_relationship_record() {
        let (root, pager, source_id, target_id, relationship_id) =
            pager_fixture("load_relationship_payload");
        let paged = pager.load_relationship_payload(&relationship_id).unwrap();
        assert_eq!(paged.relationship.id(), &relationship_id);
        assert_eq!(paged.relationship.source(), &source_id);
        assert_eq!(paged.relationship.target(), &target_id);
        assert_eq!(paged.relationship.rel_type().as_str(), "USES");
        assert!(matches!(
            paged.storage_ref,
            Some(PagerStorageRef::Offset { .. })
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    /// Outgoing adjacency should expose a warm frontier entry.
    #[test]
    fn load_outgoing_adjacency_returns_lightweight_warm_entries() {
        let (root, pager, source_id, target_id, relationship_id) =
            pager_fixture("load_outgoing_adjacency");
        let adjacency = pager.load_outgoing_adjacency(&source_id).unwrap();
        assert_eq!(adjacency.owner_node_id, source_id);
        assert_eq!(adjacency.direction, AdjacencyDirection::Outgoing);
        assert_eq!(adjacency.entries.len(), 1);
        assert_eq!(adjacency.entries[0].relationship_id, relationship_id);
        assert_eq!(adjacency.entries[0].neighbor_node_id, target_id);
        assert_eq!(
            adjacency.entries[0]
                .relationship_type
                .as_ref()
                .map(|rel_type| rel_type.as_str()),
            Some("USES")
        );
        assert!(adjacency.entries[0].relationship_storage_ref.is_some());
        assert!(adjacency.entries[0].neighbor_storage_ref.is_some());
        let _ = fs::remove_dir_all(root.path());
    }

    /// Incoming adjacency should expose the source node as neighbor.
    #[test]
    fn load_incoming_adjacency_returns_source_neighbor_for_target_node() {
        let (root, pager, source_id, target_id, relationship_id) =
            pager_fixture("load_incoming_adjacency");
        let adjacency = pager.load_incoming_adjacency(&target_id).unwrap();
        assert_eq!(adjacency.owner_node_id, target_id);
        assert_eq!(adjacency.direction, AdjacencyDirection::Incoming);
        assert_eq!(adjacency.entries.len(), 1);
        assert_eq!(adjacency.entries[0].relationship_id, relationship_id);
        assert_eq!(adjacency.entries[0].neighbor_node_id, source_id);
        let _ = fs::remove_dir_all(root.path());
    }

    /// Indexed metadata should come from catalog metadata.
    #[test]
    fn load_indexed_metadata_uses_catalog_without_full_payload_loading() {
        let (root, pager, source_id, _, relationship_id) = pager_fixture("load_indexed_metadata");
        let node_metadata = pager
            .load_indexed_metadata(&GraphRecordRef::Node(source_id.clone()))
            .unwrap();
        let relationship_metadata = pager
            .load_indexed_metadata(&GraphRecordRef::Relationship(relationship_id.clone()))
            .unwrap();
        assert_eq!(node_metadata.loading_state, LoadingState::Indexed);
        assert_eq!(
            node_metadata.labels,
            vec!["Campaign".to_owned(), "FIMI".to_owned()]
        );
        assert!(node_metadata.indexed_properties.is_empty());
        assert_eq!(
            relationship_metadata
                .relationship_type
                .as_ref()
                .map(|rel_type| rel_type.as_str()),
            Some("USES")
        );
        assert!(relationship_metadata.indexed_properties.is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    /// Missing catalog records should become unavailable-record errors.
    #[test]
    fn missing_node_catalog_entry_maps_to_unavailable_record() {
        let root = storage_root("missing_node_catalog_entry");
        let store = create_file_backed_graph_store(
            root.clone(),
            GraphCatalog::default(),
            GraphAdjacencyStorage::default(),
        )
        .unwrap();
        let pager = create_file_backed_graph_pager(store).unwrap();
        let missing = NodeId::new("node--missing").unwrap();
        let error = pager.load_node_payload(&missing).unwrap_err();
        assert!(matches!(
        error,
        GraphPagerError::UnavailableRecord { record_ref }
        if record_ref == GraphRecordRef::Node(missing)
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    /// Checksum mismatches should become corrupted-page errors.
    #[test]
    fn checksum_mismatch_maps_to_corrupted_page() {
        let root = storage_root("checksum_mismatch");
        let (graph, source_id, _, _) = graph_fixture();
        let source = graph.get_node(&source_id).unwrap().unwrap();
        let source_ref = write_payload(
            &root,
            StorageSegment::NodeRecords,
            &serde_json::to_vec(&source).unwrap(),
            Some(RecordChecksum {
                algorithm: "sha256".to_owned(),
                value: "not-the-real-checksum".to_owned(),
            }),
        );
        let mut catalog = GraphCatalog::default();
        index_node(
            &mut catalog,
            &source,
            source_ref,
            vec!["Campaign".to_owned(), "FIMI".to_owned()],
        );
        let store =
            create_file_backed_graph_store(root.clone(), catalog, GraphAdjacencyStorage::default())
                .unwrap();
        let pager = create_file_backed_graph_pager(store).unwrap();
        let error = pager.load_node_payload(&source_id).unwrap_err();
        assert!(matches!(error, GraphPagerError::CorruptedPage { .. }));
        let _ = fs::remove_dir_all(root.path());
    }

    /// Missing payload bytes should become missing-page errors.
    #[test]
    fn missing_payload_bytes_map_to_missing_page() {
        let root = storage_root("missing_payload_bytes");
        let (graph, source_id, _, _) = graph_fixture();
        let source = graph.get_node(&source_id).unwrap().unwrap();
        let source_ref = StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 128,
            length: 32,
            checksum: None,
        };
        let mut catalog = GraphCatalog::default();
        index_node(
            &mut catalog,
            &source,
            source_ref,
            vec!["Campaign".to_owned(), "FIMI".to_owned()],
        );
        let store =
            create_file_backed_graph_store(root.clone(), catalog, GraphAdjacencyStorage::default())
                .unwrap();
        let pager = create_file_backed_graph_pager(store).unwrap();
        let error = pager.load_node_payload(&source_id).unwrap_err();
        assert!(matches!(error, GraphPagerError::MissingPage { .. }));
        let _ = fs::remove_dir_all(root.path());
    }

    /// Unknown relationship type metadata should remain optional.
    #[test]
    fn load_indexed_metadata_returns_none_when_relationship_type_not_indexed() {
        let (root, pager, _, _, relationship_id) = pager_fixture("metadata_without_rel_type");
        let mut store = pager.store().clone();
        store.catalog.metadata_indexes.relationship_types.clear();
        let pager = create_file_backed_graph_pager(store).unwrap();

        let metadata = pager
            .load_indexed_metadata(&GraphRecordRef::Relationship(relationship_id))
            .expect("metadata read should succeed even when relationship type index is empty");

        assert!(metadata.relationship_type.is_none());
        let _ = fs::remove_dir_all(root.path());
    }

    /// Storage-root failures should map to typed missing-page results.
    #[test]
    fn storage_root_not_found_maps_to_missing_page_with_record_fallback() {
        let missing_path = unique_temp_path("missing-root-mapping");
        let error = map_storage_error_to_graph_pager_error(
            GraphStorageError::StorageRootNotFound {
                path: missing_path.clone(),
            },
            GraphRecordRef::Node(NodeId::new("node--fallback").unwrap()),
            None,
        );

        assert!(matches!(
        error,
        GraphPagerError::MissingPage {
        storage_ref: PagerStorageRef::Record { collection, key }
        } if collection == "nodes" && key == "node--fallback"
        ));
    }

    /// Decode failures should preserve actionable corrupted-page context.
    #[test]
    fn decode_failed_maps_to_corrupted_page_with_reason() {
        let error = map_storage_error_to_graph_pager_error(
            GraphStorageError::DecodeFailed {
                format: "JsonLinesV1".to_owned(),
                reason: "unexpected token".to_owned(),
            },
            GraphRecordRef::Relationship(RelationshipId::new("relationship--decode").unwrap()),
            None,
        );

        assert!(matches!(
        error,
        GraphPagerError::CorruptedPage { reason, .. }
        if reason.contains("decode failed") && reason.contains("unexpected token")
        ));
    }

    /// Incoming persisted adjacency should resolve neighbor storage from source node refs.
    #[test]
    fn paged_adjacency_from_incoming_uses_source_neighbor_refs() {
        let owner = NodeId::new("node--owner").unwrap();
        let source = NodeId::new("node--source").unwrap();
        let target = owner.clone();
        let relationship_id = RelationshipId::new("relationship--incoming").unwrap();
        let source_ref = StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 10,
            length: 5,
            checksum: None,
        };
        let target_ref = StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 20,
            length: 5,
            checksum: None,
        };
        let relationship_ref = StorageRef {
            segment: StorageSegment::RelationshipRecords,
            offset: 30,
            length: 7,
            checksum: None,
        };

        let persisted = PersistedAdjacencyRecord {
            owner_node_id: owner,
            direction: AdjacencyDirection::Incoming,
            entries: vec![PersistedAdjacencyEntry {
                relationship_id,
                source_node_id: source.clone(),
                target_node_id: target,
                relationship_type: RelationshipType::new("USES").unwrap(),
                direction: AdjacencyDirection::Incoming,
                relationship_storage_ref: Some(relationship_ref.clone()),
                source_node_storage_ref: Some(source_ref.clone()),
                target_node_storage_ref: Some(target_ref),
            }],
            storage_ref: Some(source_ref),
        };

        let page = paged_adjacency_from_persisted(persisted);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].neighbor_node_id, source);
        assert_eq!(
            page.entries[0].neighbor_storage_ref,
            Some(PagerStorageRef::Offset {
                segment: "NodeRecords".to_owned(),
                byte_offset: 10,
            })
        );
        assert_eq!(
            page.entries[0].relationship_storage_ref,
            Some(PagerStorageRef::Offset {
                segment: "RelationshipRecords".to_owned(),
                byte_offset: 30,
            })
        );
    }

    #[test]
    fn load_node_payload_rejects_decoded_node_id_mismatch() {
        let root = storage_root("node_id_mismatch");
        let (graph, source_id, _, _) = graph_fixture();
        let source = graph.get_node(&source_id).unwrap().unwrap();
        let source_ref = write_payload(
            &root,
            StorageSegment::NodeRecords,
            &serde_json::to_vec(&source).unwrap(),
            None,
        );

        let requested = NodeId::new("node--requested-mismatch").unwrap();
        let mut catalog = GraphCatalog::default();
        catalog.latest_node_records.insert(
            requested.clone(),
            LatestRecordCatalogEntry {
                // Record id.
                record_id: PersistedRecordId::Node(requested.clone()),
                // Kind.
                kind: PersistedRecordKind::Node,
                // Graph record version.
                graph_record_version: None,
                // Storage ref.
                storage_ref: source_ref,
            },
        );

        let pager = create_file_backed_graph_pager(
            create_file_backed_graph_store(root.clone(), catalog, GraphAdjacencyStorage::default())
                .unwrap(),
        )
        .unwrap();

        let error = pager.load_node_payload(&requested).unwrap_err();
        assert!(matches!(
        error,
        GraphPagerError::CorruptedPage { reason, .. }
        if reason.contains("does not match requested")
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn load_relationship_payload_rejects_decoded_relationship_id_mismatch() {
        let root = storage_root("relationship_id_mismatch");
        let (graph, _, _, relationship_id) = graph_fixture();
        let relationship = graph.get_relationship(&relationship_id).unwrap().unwrap();
        let relationship_ref = write_payload(
            &root,
            StorageSegment::RelationshipRecords,
            &serde_json::to_vec(&relationship).unwrap(),
            None,
        );

        let requested = RelationshipId::new("relationship--requested-mismatch").unwrap();
        let mut catalog = GraphCatalog::default();
        catalog.latest_relationship_records.insert(
            requested.clone(),
            LatestRecordCatalogEntry {
                // Record id.
                record_id: PersistedRecordId::Relationship(requested.clone()),
                // Kind.
                kind: PersistedRecordKind::Relationship,
                // Graph record version.
                graph_record_version: None,
                // Storage ref.
                storage_ref: relationship_ref,
            },
        );

        let pager = create_file_backed_graph_pager(
            create_file_backed_graph_store(root.clone(), catalog, GraphAdjacencyStorage::default())
                .unwrap(),
        )
        .unwrap();

        let error = pager.load_relationship_payload(&requested).unwrap_err();
        assert!(matches!(
        error,
        GraphPagerError::CorruptedPage { reason, .. }
        if reason.contains("does not match requested")
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn load_node_payload_rejects_non_payload_storage_segment() {
        let root = storage_root("unsupported_payload_segment");
        let requested = NodeId::new("node--unsupported-segment").unwrap();
        let mut catalog = GraphCatalog::default();
        catalog.latest_node_records.insert(
            requested.clone(),
            LatestRecordCatalogEntry {
                // Record id.
                record_id: PersistedRecordId::Node(requested.clone()),
                // Kind.
                kind: PersistedRecordKind::Node,
                // Graph record version.
                graph_record_version: None,
                // Storage ref.
                storage_ref: StorageRef {
                    // Segment.
                    segment: StorageSegment::OutgoingAdjacency,
                    // Offset.
                    offset: 0,
                    // Length.
                    length: 8,
                    // Checksum.
                    checksum: None,
                },
            },
        );

        let pager = create_file_backed_graph_pager(
            create_file_backed_graph_store(root.clone(), catalog, GraphAdjacencyStorage::default())
                .unwrap(),
        )
        .unwrap();

        let error = pager.load_node_payload(&requested).unwrap_err();
        assert!(matches!(
        error,
        GraphPagerError::CorruptedPage { reason, .. }
        if reason.contains("cannot load payloads from OutgoingAdjacency")
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn manifest_not_found_maps_to_missing_page_with_offset_storage_ref() {
        let storage_ref = StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 77,
            length: 10,
            checksum: None,
        };

        let error = map_storage_error_to_graph_pager_error(
            GraphStorageError::ManifestNotFound {
                path: PathBuf::from("/tmp/missing-manifest.json"),
            },
            GraphRecordRef::Node(NodeId::new("node--manifest-missing").unwrap()),
            Some(&storage_ref),
        );

        assert!(matches!(
        error,
        GraphPagerError::MissingPage {
        storage_ref: PagerStorageRef::Offset { segment, byte_offset }
        } if segment == "NodeRecords" && byte_offset == 77
        ));
    }

    #[test]
    fn catalog_rebuild_source_missing_maps_to_missing_page_with_fallback_record_ref() {
        let relationship_id = RelationshipId::new("relationship--rebuild-missing").unwrap();
        let error = map_storage_error_to_graph_pager_error(
            GraphStorageError::CatalogRebuildSourceMissing {
                segment: StorageSegment::RelationshipRecords,
                path: PathBuf::from("/tmp/missing-relationships.log"),
            },
            GraphRecordRef::Relationship(relationship_id.clone()),
            None,
        );

        assert!(matches!(
        error,
        GraphPagerError::MissingPage {
        storage_ref: PagerStorageRef::Record { collection, key }
        } if collection == "relationships" && key == relationship_id.as_str()
        ));
    }

    #[test]
    fn operation_failed_maps_to_corrupted_page_in_default_branch() {
        let storage_ref = StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 19,
            length: 5,
            checksum: None,
        };

        let error = map_storage_error_to_graph_pager_error(
            GraphStorageError::OperationFailed {
                operation: "read-node-page",
                message: "os-level transient failure".to_owned(),
            },
            GraphRecordRef::Node(NodeId::new("node--operation-failed").unwrap()),
            Some(&storage_ref),
        );

        assert!(matches!(
        error,
        GraphPagerError::CorruptedPage {
        storage_ref: PagerStorageRef::Offset { segment, byte_offset },
        reason,
        } if segment == "NodeRecords"
        && byte_offset == 19
        && reason.contains("storage operation failed")
        && reason.contains("read-node-page")
        ));
    }

    #[test]
    fn pager_storage_ref_or_record_ref_prefers_explicit_storage_ref_when_present() {
        let storage_ref = StorageRef {
            segment: StorageSegment::RelationshipRecords,
            offset: 41,
            length: 3,
            checksum: None,
        };

        let resolved = pager_storage_ref_or_record_ref(
            &GraphRecordRef::Node(NodeId::new("node--unused-fallback").unwrap()),
            Some(&storage_ref),
        );

        assert_eq!(
            resolved,
            PagerStorageRef::Offset {
                segment: "RelationshipRecords".to_owned(),
                byte_offset: 41,
            }
        );
    }

    #[test]
    fn missing_relationship_catalog_entry_maps_to_unavailable_relationship_record() {
        let relationship_id = RelationshipId::new("relationship--missing-catalog").unwrap();

        let error = map_storage_error_to_graph_pager_error(
            GraphStorageError::MissingRelationshipCatalogEntry {
                relationship_id: relationship_id.clone(),
            },
            GraphRecordRef::Relationship(relationship_id.clone()),
            None,
        );

        assert!(matches!(
        error,
        GraphPagerError::UnavailableRecord { record_ref }
        if record_ref == GraphRecordRef::Relationship(relationship_id)
        ));
    }

    #[test]
    fn unknown_node_adjacency_catalog_entry_maps_to_unavailable_node_record() {
        let node_id = NodeId::new("node--unknown-adjacency").unwrap();

        let error = map_storage_error_to_graph_pager_error(
            GraphStorageError::UnknownNodeAdjacencyCatalogEntry {
                node_id: node_id.clone(),
                direction: AdjacencyDirection::Incoming,
            },
            GraphRecordRef::Node(node_id.clone()),
            None,
        );

        assert!(matches!(
        error,
        GraphPagerError::UnavailableRecord { record_ref }
        if record_ref == GraphRecordRef::Node(node_id)
        ));
    }

    #[test]
    fn fallback_pager_storage_ref_uses_relationship_collection() {
        let relationship_id = RelationshipId::new("relationship--fallback").unwrap();

        let fallback =
            fallback_pager_storage_ref(&GraphRecordRef::Relationship(relationship_id.clone()));

        assert_eq!(
            fallback,
            PagerStorageRef::Record {
                collection: "relationships".to_owned(),
                key: relationship_id.as_str().to_owned(),
            }
        );
    }

    #[test]
    fn indexed_labels_for_node_returns_sorted_labels_only_for_target_node() {
        let node_a = NodeId::new("node--a").unwrap();
        let node_b = NodeId::new("node--b").unwrap();
        let mut catalog = GraphCatalog::default();
        let labels_a = vec!["Zulu".to_owned(), "Alpha".to_owned()];
        let labels_b = vec!["Beta".to_owned()];

        index_node_labels(
            &mut catalog,
            &labels_a,
            LabelIndexNodeMetadata {
                // Node id.
                node_id: node_a.clone(),
                // Latest storage ref.
                latest_storage_ref: None,
                // Graph record version.
                graph_record_version: None,
            },
        )
        .unwrap();
        index_node_labels(
            &mut catalog,
            &labels_b,
            LabelIndexNodeMetadata {
                // Node id.
                node_id: node_b,
                // Latest storage ref.
                latest_storage_ref: None,
                // Graph record version.
                graph_record_version: None,
            },
        )
        .unwrap();

        let labels = indexed_labels_for_node(&catalog, &node_a);

        assert_eq!(labels, vec!["Alpha".to_owned(), "Zulu".to_owned()]);
    }

    #[test]
    fn storage_segment_path_supports_payload_segments_and_rejects_others() {
        let root = storage_root("storage_segment_path");
        let node_ref = GraphRecordRef::Node(NodeId::new("node--segment").unwrap());

        let node_path = storage_segment_path(
            &root,
            &StorageRef {
                segment: StorageSegment::NodeRecords,
                offset: 0,
                length: 1,
                checksum: None,
            },
            &node_ref,
        )
        .expect("node segment should resolve");
        let relationship_path = storage_segment_path(
            &root,
            &StorageRef {
                segment: StorageSegment::RelationshipRecords,
                offset: 0,
                length: 1,
                checksum: None,
            },
            &node_ref,
        )
        .expect("relationship segment should resolve");

        assert!(node_path.ends_with("nodes/node_records.log"));
        assert!(relationship_path.ends_with("relationships/relationship_records.log"));

        let invalid = storage_segment_path(
            &root,
            &StorageRef {
                segment: StorageSegment::OutgoingAdjacency,
                offset: 0,
                length: 1,
                checksum: None,
            },
            &node_ref,
        )
        .expect_err("non-payload segments should be rejected");
        assert!(matches!(
        invalid,
        GraphPagerError::CorruptedPage { reason, .. }
        if reason.contains("cannot load payloads from OutgoingAdjacency")
        ));

        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn paged_adjacency_from_outgoing_uses_target_neighbor_refs() {
        let owner = NodeId::new("node--owner-outgoing").unwrap();
        let target = NodeId::new("node--target-outgoing").unwrap();
        let relationship_id = RelationshipId::new("relationship--outgoing").unwrap();
        let target_ref = StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 21,
            length: 5,
            checksum: None,
        };

        let persisted = PersistedAdjacencyRecord {
            owner_node_id: owner,
            direction: AdjacencyDirection::Outgoing,
            entries: vec![PersistedAdjacencyEntry {
                relationship_id,
                source_node_id: NodeId::new("node--source-outgoing").unwrap(),
                target_node_id: target.clone(),
                relationship_type: RelationshipType::new("USES").unwrap(),
                direction: AdjacencyDirection::Outgoing,
                relationship_storage_ref: None,
                source_node_storage_ref: None,
                target_node_storage_ref: Some(target_ref),
            }],
            storage_ref: None,
        };

        let page = paged_adjacency_from_persisted(persisted);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].neighbor_node_id, target);
        assert_eq!(
            page.entries[0].neighbor_storage_ref,
            Some(PagerStorageRef::Offset {
                segment: "NodeRecords".to_owned(),
                byte_offset: 21,
            })
        );
    }

    #[test]
    fn load_indexed_metadata_for_missing_node_maps_to_unavailable_record() {
        let root = storage_root("missing_node_metadata");
        let pager = create_file_backed_graph_pager(
            create_file_backed_graph_store(
                root.clone(),
                GraphCatalog::default(),
                GraphAdjacencyStorage::default(),
            )
            .unwrap(),
        )
        .unwrap();
        let missing = NodeId::new("node--missing-metadata").unwrap();

        let error = pager
            .load_indexed_metadata(&GraphRecordRef::Node(missing.clone()))
            .expect_err("missing node metadata should map to unavailable record");

        assert!(matches!(
        error,
        GraphPagerError::UnavailableRecord { record_ref }
        if record_ref == GraphRecordRef::Node(missing)
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn decode_payload_helpers_map_invalid_json_to_corrupted_page() {
        let record_ref = GraphRecordRef::Node(NodeId::new("node--decode").unwrap());
        let storage_ref = StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 0,
            length: 7,
            checksum: None,
        };

        let node_error = decode_node_payload(b"not-json", &record_ref, &storage_ref)
            .expect_err("invalid json node payload should map to corrupted page");
        assert!(matches!(
        node_error,
        GraphPagerError::CorruptedPage { reason, .. } if reason.contains("decode failed")
        ));

        let relationship_ref =
            GraphRecordRef::Relationship(RelationshipId::new("relationship--decode").unwrap());
        let relationship_error = decode_relationship_payload(
            b"not-json",
            &relationship_ref,
            &StorageRef {
                segment: StorageSegment::RelationshipRecords,
                offset: 0,
                length: 7,
                checksum: None,
            },
        )
        .expect_err("invalid json relationship payload should map to corrupted page");
        assert!(matches!(
        relationship_error,
        GraphPagerError::CorruptedPage { reason, .. } if reason.contains("decode failed")
        ));
    }

    #[test]
    fn corrupted_page_without_storage_ref_falls_back_to_record_reference() {
        let node_ref = GraphRecordRef::Node(NodeId::new("node--fallback-corrupted").unwrap());

        let error = corrupted_page(&node_ref, None, "simulated corruption".to_owned());
        assert!(matches!(
        error,
        GraphPagerError::CorruptedPage {
        storage_ref: PagerStorageRef::Record { collection, key },
        reason,
        } if collection == "nodes" && key == "node--fallback-corrupted" && reason == "simulated corruption"
        ));
    }
}
