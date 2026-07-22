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
//! Graph pager trait and storage-reference contracts.
//!
//! Design boundary:
//!
//! The graph pager is the seam between a bounded working set and whichever
//! storage implementation can provide graph records. It lets a caller request
//! node payloads, relationship payloads, adjacency, and indexed metadata without
//! assuming the full graph is already resident in memory.
//!
//! Implementation boundary:
//!
//! This module provides the pager contract and a lightweight adapter for the
//! current in-memory `Graph`. It does not implement persistent storage,
//! lazy page-in, prefetching, eviction, query execution, or semantic search.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    Graph, GraphError, LabelSet, LoadingState, Node, NodeId, PropertyMap, Relationship,
    RelationshipId, RelationshipType,
};

/// Reference to a future storage location for a graph record or page.
///
///
/// - Represent where a node, relationship, adjacency list, or metadata fragment
///   can be loaded from without exposing a final storage format.
/// - Keep the reference portable across an in-memory mock, append-only files,
///   paged storage, catalog records, or external object locations.
/// - Avoid embedding loading policy, cache state, or traversal semantics in the
///   storage reference itself.
///
///
/// Storage implementations should be able to attach one of these references to
/// pager results so the working set can reason about loaded records without
/// knowing how the backend is physically organized.
///
/// # Errors
///
///
/// Invalid or unavailable references are not validated by this enum. Pager
/// implementations report lookup failures through `GraphPagerError`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StorageRef {
    /// A logical record key in a named backend collection.
    Record {
        /// Collection.
        collection: String,
        /// Key.
        key: String,
    },

    /// A logical page identifier in a named storage segment.
    Page {
        /// Segment.
        segment: String,
        /// Page id.
        page_id: u64,
    },

    /// A byte offset inside a named storage segment.
    Offset {
        /// Segment.
        segment: String,
        /// Byte offset.
        byte_offset: u64,
    },

    /// An opaque location owned by a backend outside graph-core.
    External {
        /// Uri.
        uri: String,
    },
}

/// Logical graph record reference used by metadata-level pager calls.
///
///
/// - Let callers request lightweight metadata by stable graph record identity.
/// - Keep metadata lookup separate from full node or relationship payload loading.
/// - Give future catalog-backed implementations a stable public input type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphRecordRef {
    /// Metadata request for a node record.
    Node(NodeId),

    /// Metadata request for a relationship record.
    Relationship(RelationshipId),
}

/// Direction of an adjacency request from the point of view of one node.
///
///
/// - Keep incoming and outgoing adjacency explicit in pager results.
/// - Avoid representing direction as a raw string in the working-set boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AdjacencyDirection {
    /// Relationships where the requested node is the source endpoint.
    Outgoing,

    /// Relationships where the requested node is the target endpoint.
    Incoming,
}

/// Lightweight metadata for a graph record without its full payload.
///
///
/// - Represent the indexed information a working set can use before loading the
///   full node or relationship payload.
/// - Support cold/indexed/warm decisions with labels, relationship type hints,
///   indexed properties, and storage references.
/// - Avoid requiring evidence payloads, full text, full property maps, or full
///   relationship objects for early relevance decisions.
///
///
/// A pager implementation may return this type from a catalog, in-memory index,
/// or persistent metadata page. The returned metadata should describe the record
/// referenced by `record_ref` and may include only the indexed subset known to
/// that backend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphRecordMetadata {
    /// Stable logical record identity described by this metadata.
    pub record_ref: GraphRecordRef,

    /// Optional backend location from which the full record can later be loaded.
    pub storage_ref: Option<StorageRef>,

    /// Loading state suggested by the source that produced this metadata.
    pub loading_state: LoadingState,

    /// Indexed node labels when the metadata describes a node.
    pub labels: LabelSet,

    /// Indexed relationship type when the metadata describes a relationship.
    pub relationship_type: Option<RelationshipType>,

    /// Lightweight indexed properties available without full payload loading.
    pub indexed_properties: PropertyMap,
}

/// Node payload returned by a graph pager.
///
///
/// - Wrap a loaded node with the storage reference that produced it.
/// - Make room for pager metadata without changing the core `Node` model.
/// - Keep the result compatible with both an in-memory mock and future paged
///   persistent storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagedNode {
    /// Full node payload made hot by the pager call.
    pub node: Node,

    /// Optional backend location used to load the node payload.
    pub storage_ref: Option<StorageRef>,
}

/// Relationship payload returned by a graph pager.
///
///
/// - Wrap a loaded relationship with the storage reference that produced it.
/// - Keep relationship loading separate from adjacency loading.
/// - Keep the result compatible with both an in-memory mock and future paged
///   persistent storage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagedRelationship {
    /// Full relationship payload made hot by the pager call.
    pub relationship: Relationship,

    /// Optional backend location used to load the relationship payload.
    pub storage_ref: Option<StorageRef>,
}

/// One lightweight entry in an incoming or outgoing adjacency page.
///
///
/// - Describe a relationship edge discovered through adjacency without forcing
///   the full relationship payload to be loaded immediately.
/// - Give the working set enough information to decide whether the neighboring
///   record should remain indexed, become warm, or be loaded as hot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagedAdjacencyEntry {
    /// Relationship ID present in the adjacency list.
    pub relationship_id: RelationshipId,

    /// Neighbor node reached through this adjacency entry.
    pub neighbor_node_id: NodeId,

    /// Optional relationship type hint when it is indexed with adjacency.
    pub relationship_type: Option<RelationshipType>,

    /// Optional backend location for the relationship payload.
    pub relationship_storage_ref: Option<StorageRef>,

    /// Optional backend location for the neighbor node payload.
    pub neighbor_storage_ref: Option<StorageRef>,
}

/// Adjacency page returned by a graph pager.
///
///
/// - Represent incoming or outgoing adjacency for a single owner node.
/// - Keep adjacency loading cheaper than loading every related payload.
/// - Preserve enough storage information for later lazy page-in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagedAdjacency {
    /// Node whose adjacency was requested.
    pub owner_node_id: NodeId,

    /// Direction represented by this adjacency result.
    pub direction: AdjacencyDirection,

    /// Lightweight adjacency entries loaded for the owner node.
    pub entries: Vec<PagedAdjacencyEntry>,

    /// Optional backend location used to load this adjacency page.
    pub storage_ref: Option<StorageRef>,
}

/// Stable page identity used by advanced page-in requests.
///
///
/// - Identify a page-like loading unit without exposing backend internals.
/// - Keep query execution and working-set management independent from storage
///   layout details.
/// - Support node, relationship, adjacency, and metadata page categories.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PageIdentity {
    /// Logical kind of page represented by this identifier.
    pub kind: PageIdentityKind,

    /// Logical storage segment or namespace that owns this page.
    pub segment: String,

    /// Stable page identifier interpreted by the backing pager implementation.
    pub page_id: String,

    /// Optional backend reference tied to the page identity.
    pub storage_ref: Option<StorageRef>,
}

/// Typed page categories used by advanced page-in calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PageIdentityKind {
    /// Page targeting a node payload.
    NodePayload,

    /// Page targeting a relationship payload.
    RelationshipPayload,

    /// Page targeting adjacency entries.
    Adjacency,

    /// Page targeting indexed metadata.
    IndexedMetadata,
}

/// Request contract for advanced page-in operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageInRequest {
    /// Page to page-in from storage.
    pub page: PageIdentity,

    /// Logical graph records expected from this page-in request.
    pub record_refs: Vec<GraphRecordRef>,
}

/// Typed result status for page-in requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PageInStatus {
    /// Requested records were already available in memory.
    Hit,

    /// Requested page did not contain records needed by the request.
    Miss,

    /// Requested records were loaded from the target page.
    Loaded,

    /// Request was rejected by pager policy or validation.
    Rejected,
}

/// Result contract returned by advanced page-in operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageInResult {
    /// Original page-in request.
    pub request: PageInRequest,

    /// Typed page-in outcome status.
    pub status: PageInStatus,

    /// Record references loaded as part of the page-in request.
    pub loaded_record_refs: Vec<GraphRecordRef>,
}

/// Result alias for graph pager calls.
pub type GraphPagerResult<T> = Result<T, GraphPagerError>;

/// Typed failures returned by graph pager implementations.
///
///
/// - Keep pager errors matchable by variant without parsing display strings.
/// - Distinguish missing pages, corrupted pages, and unavailable graph records.
/// - Stay independent from any final persistent storage format.
///
///
/// Pager implementations should use these errors for expected page and record
/// loading failures instead of panicking or collapsing everything into a generic
/// string error.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum GraphPagerError {
    /// The backend could not find the page or location needed for a load request.
    #[error("missing storage page: {storage_ref:?}")]
    MissingPage {
        /// Storage ref.
        storage_ref: StorageRef,
    },

    /// The backend found a page or location but could not safely decode it.
    #[error("corrupted storage page: {storage_ref:?}: {reason}")]
    CorruptedPage {
        /// Storage ref.
        storage_ref: StorageRef,
        /// Reason.
        reason: String,
    },

    /// The backend could not return a requested logical graph record.
    #[error("unavailable graph record: {record_ref:?}")]
    UnavailableRecord {
        /// Record ref.
        record_ref: GraphRecordRef,
    },

    /// The pager implementation does not support advanced page-in yet.
    #[error("page-in is not supported for page: {page:?}")]
    PageInNotSupported {
        /// Page.
        page: PageIdentity,
    },
}

/// Pager contract used by working-set loading code.
///
///
/// - Provide a narrow boundary between working-set management and graph storage.
/// - Let callers load node payloads, relationship payloads, incoming adjacency,
///   outgoing adjacency, and indexed metadata through one trait.
/// - Keep the contract implementable by both an in-memory mock and a future
///   persistent paged storage backend.
///
///
/// Implementations should return full payload wrappers only for explicit payload
/// calls. Adjacency and metadata calls may return lightweight information without
/// loading every related node or relationship as hot.
///
/// # Errors
///
///
/// Implementations should return `GraphPagerError::MissingPage` when the physical
/// page or storage reference is absent, `GraphPagerError::CorruptedPage` when a
/// page cannot be decoded safely, and `GraphPagerError::UnavailableRecord` when a
/// logical node or relationship cannot be provided.
pub trait GraphPager {
    /// Load the current node payload identified by a stable node ID.
    fn load_node_payload(&self, node_id: &NodeId) -> GraphPagerResult<PagedNode>;

    /// Load the current relationship payload identified by a stable relationship ID.
    fn load_relationship_payload(
        &self,
        relationship_id: &RelationshipId,
    ) -> GraphPagerResult<PagedRelationship>;

    /// Load lightweight outgoing adjacency for a node without requiring all payloads.
    fn load_outgoing_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency>;

    /// Load lightweight incoming adjacency for a node without requiring all payloads.
    fn load_incoming_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency>;

    /// Load indexed metadata for a node or relationship without its full payload.
    fn load_indexed_metadata(
        &self,
        record_ref: &GraphRecordRef,
    ) -> GraphPagerResult<GraphRecordMetadata>;

    /// Request page-in for one logical storage page.
    ///
    /// Implementations may override this method when they can expose explicit
    /// page-in behavior. The default implementation preserves backward
    /// compatibility for existing pagers while returning a typed error.
    fn page_in(&self, request: &PageInRequest) -> GraphPagerResult<PageInResult> {
        Err(GraphPagerError::PageInNotSupported {
            page: request.page.clone(),
        })
    }
}

impl GraphPager for Graph {
    /// Load a node payload from the current in-memory graph.
    ///
    ///
    /// provide a real pager implementation for the current in-memory graph so
    /// working-set loading code can depend on `GraphPager` before a persistent
    /// storage backend exists.
    ///
    ///
    /// return the current visible node as a `PagedNode` and attach a backend-neutral
    /// in-memory storage reference.
    ///
    /// # Errors
    ///
    /// return `UnavailableRecord` when the node is absent or tombstoned, and map
    /// graph invariant failures to `CorruptedPage` with the in-memory storage ref.
    fn load_node_payload(&self, node_id: &NodeId) -> GraphPagerResult<PagedNode> {
        match self.get_node(node_id) {
            Ok(Some(node)) => Ok(PagedNode {
                storage_ref: Some(node_storage_ref(node_id)),
                node,
            }),
            Ok(None) => Err(unavailable_node(node_id)),
            Err(error) => Err(corrupted_in_memory_page(node_storage_ref(node_id), error)),
        }
    }

    /// Load a relationship payload from the current in-memory graph.
    ///
    ///
    /// expose relationship payload loading through the same pager boundary used by
    /// future persistent storage implementations.
    ///
    ///
    /// return the current visible relationship as a `PagedRelationship` and attach
    /// a backend-neutral in-memory storage reference.
    ///
    /// # Errors
    ///
    /// return `UnavailableRecord` when the relationship is absent or tombstoned,
    /// and map graph invariant failures to `CorruptedPage` with the in-memory ref.
    fn load_relationship_payload(
        &self,
        relationship_id: &RelationshipId,
    ) -> GraphPagerResult<PagedRelationship> {
        match self.get_relationship(relationship_id) {
            Ok(Some(relationship)) => Ok(PagedRelationship {
                storage_ref: Some(relationship_storage_ref(relationship_id)),
                relationship,
            }),
            Ok(None) => Err(unavailable_relationship(relationship_id)),
            Err(error) => Err(corrupted_in_memory_page(
                relationship_storage_ref(relationship_id),
                error,
            )),
        }
    }

    /// Load outgoing adjacency from the current in-memory graph.
    ///
    ///
    /// let working-set code request a lightweight outgoing frontier without
    /// binding itself to the graph's private adjacency index representation.
    ///
    ///
    /// validate that the owner node is visible, then return one adjacency entry per
    /// visible outgoing relationship with relationship and neighbor storage refs.
    ///
    /// # Errors
    ///
    /// return `UnavailableRecord` when the owner node is absent or tombstoned, and
    /// map graph invariant failures to `CorruptedPage`.
    fn load_outgoing_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        ensure_node_available(self, node_id)?;

        match self.outgoing(node_id) {
            Ok(relationships) => Ok(adjacency_page(
                node_id,
                AdjacencyDirection::Outgoing,
                relationships,
            )),
            Err(error) => Err(corrupted_in_memory_page(
                adjacency_storage_ref(node_id, AdjacencyDirection::Outgoing),
                error,
            )),
        }
    }

    /// Load incoming adjacency from the current in-memory graph.
    ///
    ///
    /// let working-set code request a lightweight incoming frontier without
    /// binding itself to the graph's private adjacency index representation.
    ///
    ///
    /// validate that the owner node is visible, then return one adjacency entry per
    /// visible incoming relationship with relationship and neighbor storage refs.
    ///
    /// # Errors
    ///
    /// return `UnavailableRecord` when the owner node is absent or tombstoned, and
    /// map graph invariant failures to `CorruptedPage`.
    fn load_incoming_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        ensure_node_available(self, node_id)?;

        match self.incoming(node_id) {
            Ok(relationships) => Ok(adjacency_page(
                node_id,
                AdjacencyDirection::Incoming,
                relationships,
            )),
            Err(error) => Err(corrupted_in_memory_page(
                adjacency_storage_ref(node_id, AdjacencyDirection::Incoming),
                error,
            )),
        }
    }

    /// Load indexed metadata from the current in-memory graph.
    ///
    ///
    /// provide the same metadata-level contract expected from a future catalog even
    /// though the current graph keeps records in memory rather than in pages.
    ///
    ///
    /// derive lightweight metadata from the current visible node or relationship
    /// and mark it as indexed with a storage ref that can later be used for payload
    /// loading through this same pager.
    ///
    /// # Errors
    ///
    /// return `UnavailableRecord` when the requested graph record is absent or
    /// tombstoned, and map graph invariant failures to `CorruptedPage`.
    fn load_indexed_metadata(
        &self,
        record_ref: &GraphRecordRef,
    ) -> GraphPagerResult<GraphRecordMetadata> {
        match record_ref {
            GraphRecordRef::Node(node_id) => match self.get_node(node_id) {
                Ok(Some(node)) => Ok(node_metadata(node)),
                Ok(None) => Err(unavailable_node(node_id)),
                Err(error) => Err(corrupted_in_memory_page(node_storage_ref(node_id), error)),
            },
            GraphRecordRef::Relationship(relationship_id) => {
                match self.get_relationship(relationship_id) {
                    Ok(Some(relationship)) => Ok(relationship_metadata(relationship)),
                    Ok(None) => Err(unavailable_relationship(relationship_id)),
                    Err(error) => Err(corrupted_in_memory_page(
                        relationship_storage_ref(relationship_id),
                        error,
                    )),
                }
            }
        }
    }
}

/// Verify that a node is visible before adjacency is loaded.
///
///
/// prevent pager adjacency calls from silently returning an empty frontier for an
/// owner node that does not exist or has been tombstoned.
///
///
/// return `Ok(())` only when `Graph::get_node` returns a visible current node.
///
/// # Errors
///
/// return `UnavailableRecord` for absent or tombstoned nodes and `CorruptedPage`
/// for graph invariant failures surfaced by the graph read API.
fn ensure_node_available(graph: &Graph, node_id: &NodeId) -> GraphPagerResult<()> {
    match graph.get_node(node_id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(unavailable_node(node_id)),
        Err(error) => Err(corrupted_in_memory_page(node_storage_ref(node_id), error)),
    }
}

/// Convert a visible node into lightweight pager metadata.
///
///
/// keep metadata construction in one place so the in-memory pager presents the
/// same result shape a future catalog-backed pager should expose.
///
///
/// preserve the node identity, labels, indexed properties, and storage reference,
/// while omitting any relationship type.
///
/// # Errors
///
/// none expected because the caller provides an already visible node payload.
fn node_metadata(node: Node) -> GraphRecordMetadata {
    let node_id = node.id().clone();

    GraphRecordMetadata {
        // Record ref.
        record_ref: GraphRecordRef::Node(node_id.clone()),
        // Storage ref.
        storage_ref: Some(node_storage_ref(&node_id)),
        // Loading state.
        loading_state: LoadingState::Indexed,
        // Labels.
        labels: node.labels,
        // Relationship type.
        relationship_type: None,
        // Indexed properties.
        indexed_properties: node.properties,
    }
}

/// Convert a visible relationship into lightweight pager metadata.
///
///
/// keep relationship metadata construction separate from full relationship
/// loading while preserving the type hint used by relevance and expansion logic.
///
///
/// preserve the relationship identity, relationship type, indexed properties, and
/// storage reference, while leaving node labels empty.
///
/// # Errors
///
/// none expected because the caller provides an already visible relationship payload.
fn relationship_metadata(relationship: Relationship) -> GraphRecordMetadata {
    let relationship_id = relationship.id().clone();

    GraphRecordMetadata {
        // Record ref.
        record_ref: GraphRecordRef::Relationship(relationship_id.clone()),
        // Storage ref.
        storage_ref: Some(relationship_storage_ref(&relationship_id)),
        // Loading state.
        loading_state: LoadingState::Indexed,
        // Labels.
        labels: Vec::new(),
        // Relationship type.
        relationship_type: Some(relationship.rel_type),
        // Indexed properties.
        indexed_properties: relationship.properties,
    }
}

/// Convert visible relationships into a lightweight adjacency page.
///
///
/// represent the current in-memory traversal frontier through pager result types
/// without exposing the graph's private adjacency index.
///
///
/// create one entry per relationship, choose the neighbor node according to the
/// requested direction, and attach in-memory storage references for future payload
/// loading.
///
/// # Errors
///
/// none expected because relationship visibility has already been resolved by the
/// graph traversal API before this helper is called.
fn adjacency_page(
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
    relationships: Vec<Relationship>,
) -> PagedAdjacency {
    let entries = relationships
        .into_iter()
        .map(|relationship| {
            let relationship_id = relationship.id().clone();
            let neighbor_node_id = match direction {
                AdjacencyDirection::Outgoing => relationship.target().clone(),
                AdjacencyDirection::Incoming => relationship.source().clone(),
            };

            PagedAdjacencyEntry {
                // Relationship id.
                relationship_id: relationship_id.clone(),
                // Neighbor node id.
                neighbor_node_id: neighbor_node_id.clone(),
                // Relationship type.
                relationship_type: Some(relationship.rel_type().clone()),
                // Relationship storage ref.
                relationship_storage_ref: Some(relationship_storage_ref(&relationship_id)),
                // Neighbor storage ref.
                neighbor_storage_ref: Some(node_storage_ref(&neighbor_node_id)),
            }
        })
        .collect();

    PagedAdjacency {
        // Owner node id.
        owner_node_id: owner_node_id.clone(),
        direction,
        entries,
        // Storage ref.
        storage_ref: Some(adjacency_storage_ref(owner_node_id, direction)),
    }
}

/// Build a backend-neutral in-memory storage reference for a node.
///
///
/// identify the source of an in-memory pager node result without committing to a
/// future disk, page, or catalog layout.
///
///
/// return a deterministic opaque URI that can be compared in tests and diagnostics.
///
/// # Errors
///
/// none expected because node identifiers are already validated typed IDs.
fn node_storage_ref(node_id: &NodeId) -> StorageRef {
    StorageRef::External {
        uri: format!("memory://graph/nodes/{}", node_id.as_str()),
    }
}

/// Build a backend-neutral in-memory storage reference for a relationship.
///
///
/// identify the source of an in-memory pager relationship result without exposing
/// graph internals or a final persistent format.
///
///
/// return a deterministic opaque URI that can be compared in tests and diagnostics.
///
/// # Errors
///
/// none expected because relationship identifiers are already validated typed IDs.
fn relationship_storage_ref(relationship_id: &RelationshipId) -> StorageRef {
    StorageRef::External {
        uri: format!("memory://graph/relationships/{}", relationship_id.as_str()),
    }
}

/// Build a backend-neutral in-memory storage reference for adjacency.
///
///
/// identify the source of an in-memory adjacency result without exposing the
/// private adjacency index shape.
///
///
/// include the owner node ID and direction in a deterministic opaque URI.
///
/// # Errors
///
/// none expected because node identifiers and directions are already typed.
fn adjacency_storage_ref(node_id: &NodeId, direction: AdjacencyDirection) -> StorageRef {
    let direction_segment = match direction {
        AdjacencyDirection::Outgoing => "outgoing",
        AdjacencyDirection::Incoming => "incoming",
    };

    StorageRef::External {
        uri: format!(
            "memory://graph/adjacency/{}/{}",
            direction_segment,
            node_id.as_str()
        ),
    }
}

/// Convert a graph read invariant failure into a pager corruption error.
///
///
/// preserve the pager error boundary even when the current in-memory graph reports
/// an internal invariant failure instead of a storage page failure.
///
///
/// map the graph diagnostic into `CorruptedPage` using the logical in-memory
/// storage reference that was being read.
///
/// # Errors
///
/// none expected because this helper itself only constructs an error value.
fn corrupted_in_memory_page(storage_ref: StorageRef, error: GraphError) -> GraphPagerError {
    GraphPagerError::CorruptedPage {
        storage_ref,
        reason: error.to_string(),
    }
}

/// Build an unavailable-record error for a node.
///
///
/// keep missing or tombstoned node handling consistent across payload, adjacency,
/// and metadata pager calls.
///
///
/// return a typed `UnavailableRecord` carrying the requested node identity.
///
/// # Errors
///
/// none expected because this helper only constructs an error value.
fn unavailable_node(node_id: &NodeId) -> GraphPagerError {
    GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(node_id.clone()),
    }
}

/// Build an unavailable-record error for a relationship.
///
///
/// keep missing or tombstoned relationship handling consistent across payload and
/// metadata pager calls.
///
///
/// return a typed `UnavailableRecord` carrying the requested relationship identity.
///
/// # Errors
///
/// none expected because this helper only constructs an error value.
fn unavailable_relationship(relationship_id: &RelationshipId) -> GraphPagerError {
    GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Relationship(relationship_id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeInput, PropertyValue, RelationshipInput};

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node ID should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship ID should be valid")
    }

    fn relationship_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
    }

    fn graph_with_one_relationship() -> (Graph, NodeId, NodeId, RelationshipId) {
        let mut graph = Graph::new();

        let source = graph
            .create_node(NodeInput::new(["Campaign"]))
            .expect("source node should be created");
        let target = graph
            .create_node(NodeInput::new(["Narrative"]))
            .expect("target node should be created");
        let relationship = graph
            .create_relationship(
                RelationshipInput::new(source.clone(), "AMPLIFIES", target.clone())
                    .expect("relationship input should be valid"),
            )
            .expect("relationship should be created");

        (graph, source, target, relationship)
    }

    struct UnavailablePager;

    impl GraphPager for UnavailablePager {
        fn load_node_payload(&self, node_id: &NodeId) -> GraphPagerResult<PagedNode> {
            Err(GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Node(node_id.clone()),
            })
        }

        fn load_relationship_payload(
            &self,
            relationship_id: &RelationshipId,
        ) -> GraphPagerResult<PagedRelationship> {
            Err(GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Relationship(relationship_id.clone()),
            })
        }

        fn load_outgoing_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
            Err(GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Node(node_id.clone()),
            })
        }

        fn load_incoming_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
            Err(GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Node(node_id.clone()),
            })
        }

        fn load_indexed_metadata(
            &self,
            record_ref: &GraphRecordRef,
        ) -> GraphPagerResult<GraphRecordMetadata> {
            Err(GraphPagerError::UnavailableRecord {
                record_ref: record_ref.clone(),
            })
        }
    }

    //
    // Verify that storage references can describe future backend-neutral locations
    // without exposing a final file, page, catalog, or external object format.
    //
    // Given representative record, page, offset, and external storage references,
    // when they are constructed as `StorageRef` values,
    // then each location should preserve its identifying fields and remain directly matchable.
    #[test]
    fn storage_ref_represents_backend_neutral_locations() {
        let record_ref = StorageRef::Record {
            collection: "nodes".to_owned(),
            key: "node--1".to_owned(),
        };
        let page_ref = StorageRef::Page {
            segment: "adjacency/outgoing".to_owned(),
            page_id: 42,
        };
        let offset_ref = StorageRef::Offset {
            segment: "relationships".to_owned(),
            byte_offset: 2048,
        };
        let external_ref = StorageRef::External {
            uri: "memory://graph/page/1".to_owned(),
        };

        assert!(matches!(
        record_ref,
        StorageRef::Record { collection, key }
        if collection == "nodes" && key == "node--1"
        ));
        assert!(matches!(
        page_ref,
        StorageRef::Page { segment, page_id }
        if segment == "adjacency/outgoing" && page_id == 42
        ));
        assert!(matches!(
        offset_ref,
        StorageRef::Offset { segment, byte_offset }
        if segment == "relationships" && byte_offset == 2048
        ));
        assert!(matches!(
        external_ref,
        StorageRef::External { uri } if uri == "memory://graph/page/1"
        ));
    }

    //
    // Verify that indexed metadata can describe a node without loading its full
    // payload. This is the key cold/indexed/warm boundary required by the pager.
    //
    // Given a node metadata record with labels, indexed properties, and a storage ref,
    // when the metadata is inspected,
    // then it should expose lightweight graph information without requiring `Node` construction.
    #[test]
    fn graph_record_metadata_represents_node_without_full_payload() {
        let id = node_id("node--campaign-1");
        let storage_ref = StorageRef::Record {
            collection: "nodes".to_owned(),
            key: id.as_str().to_owned(),
        };
        let mut indexed_properties = PropertyMap::new();
        indexed_properties.insert(
            "source_reliability".to_owned(),
            PropertyValue::String("high".to_owned()),
        );

        let metadata = GraphRecordMetadata {
            record_ref: GraphRecordRef::Node(id.clone()),
            storage_ref: Some(storage_ref.clone()),
            loading_state: LoadingState::Indexed,
            labels: vec!["Campaign".to_owned(), "FIMI".to_owned()],
            relationship_type: None,
            indexed_properties,
        };

        assert_eq!(metadata.record_ref, GraphRecordRef::Node(id));
        assert_eq!(metadata.storage_ref, Some(storage_ref));
        assert_eq!(metadata.loading_state, LoadingState::Indexed);
        assert_eq!(
            metadata.labels,
            vec!["Campaign".to_owned(), "FIMI".to_owned()]
        );
        assert_eq!(metadata.relationship_type, None);
        assert!(matches!(
        metadata.indexed_properties.get("source_reliability"),
        Some(PropertyValue::String(value)) if value == "high"
        ));
    }

    //
    // Verify that indexed metadata can also describe a relationship without
    // loading its full payload. Relationship metadata must support type hints for
    // relevance decisions before page-in.
    //
    // Given a relationship metadata record with a relationship type and storage ref,
    // when the metadata is inspected,
    // then it should preserve the relationship identity and lightweight type hint.
    #[test]
    fn graph_record_metadata_represents_relationship_without_full_payload() {
        let id = relationship_id("relationship--amplifies-1");
        let rel_type = relationship_type("AMPLIFIES");
        let storage_ref = StorageRef::Record {
            collection: "relationships".to_owned(),
            key: id.as_str().to_owned(),
        };

        let metadata = GraphRecordMetadata {
            record_ref: GraphRecordRef::Relationship(id.clone()),
            storage_ref: Some(storage_ref.clone()),
            loading_state: LoadingState::Warm,
            labels: Vec::new(),
            relationship_type: Some(rel_type.clone()),
            indexed_properties: PropertyMap::new(),
        };

        assert_eq!(metadata.record_ref, GraphRecordRef::Relationship(id));
        assert_eq!(metadata.storage_ref, Some(storage_ref));
        assert_eq!(metadata.loading_state, LoadingState::Warm);
        assert_eq!(metadata.relationship_type, Some(rel_type));
        assert!(metadata.labels.is_empty());
    }

    //
    // Verify that adjacency pages can represent a lightweight outgoing frontier
    // without forcing the neighbor node or relationship payloads to be hot.
    //
    // Given an outgoing adjacency page with one entry and storage references,
    // when the page is inspected,
    // then it should preserve owner, direction, relationship ID, neighbor ID, and lazy-load refs.
    #[test]
    fn paged_adjacency_represents_lightweight_outgoing_frontier() {
        let owner_node_id = node_id("node--campaign-1");
        let neighbor_node_id = node_id("node--narrative-1");
        let relationship_id = relationship_id("relationship--promotes-1");
        let relationship_storage_ref = StorageRef::Record {
            collection: "relationships".to_owned(),
            key: relationship_id.as_str().to_owned(),
        };
        let neighbor_storage_ref = StorageRef::Record {
            collection: "nodes".to_owned(),
            key: neighbor_node_id.as_str().to_owned(),
        };

        let page = PagedAdjacency {
            owner_node_id: owner_node_id.clone(),
            direction: AdjacencyDirection::Outgoing,
            entries: vec![PagedAdjacencyEntry {
                relationship_id: relationship_id.clone(),
                neighbor_node_id: neighbor_node_id.clone(),
                relationship_type: Some(relationship_type("PROMOTES")),
                relationship_storage_ref: Some(relationship_storage_ref.clone()),
                neighbor_storage_ref: Some(neighbor_storage_ref.clone()),
            }],
            storage_ref: Some(StorageRef::Page {
                segment: "adjacency/outgoing".to_owned(),
                page_id: 7,
            }),
        };

        assert_eq!(page.owner_node_id, owner_node_id);
        assert_eq!(page.direction, AdjacencyDirection::Outgoing);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].relationship_id, relationship_id);
        assert_eq!(page.entries[0].neighbor_node_id, neighbor_node_id);
        assert_eq!(
            page.entries[0].relationship_storage_ref,
            Some(relationship_storage_ref)
        );
        assert_eq!(
            page.entries[0].neighbor_storage_ref,
            Some(neighbor_storage_ref)
        );
        assert!(matches!(
        page.storage_ref,
        Some(StorageRef::Page { segment, page_id })
        if segment == "adjacency/outgoing" && page_id == 7
        ));
    }

    //
    // Verify that pager errors remain typed and matchable. Working-set code and
    // later tests should be able to distinguish missing pages, corrupted pages,
    // and unavailable records without parsing display text.
    //
    // Given representative `GraphPagerError` values,
    // when callers pattern-match on them,
    // then each expected failure category should be identifiable by variant and payload.
    #[test]
    fn graph_pager_errors_are_matchable_by_variant() {
        let page_ref = StorageRef::Page {
            segment: "nodes".to_owned(),
            page_id: 404,
        };
        let node_ref = GraphRecordRef::Node(node_id("node--missing"));

        let missing_page = GraphPagerError::MissingPage {
            storage_ref: page_ref.clone(),
        };
        let corrupted_page = GraphPagerError::CorruptedPage {
            storage_ref: page_ref.clone(),
            reason: "checksum mismatch".to_owned(),
        };
        let unavailable_record = GraphPagerError::UnavailableRecord {
            record_ref: node_ref.clone(),
        };

        assert!(matches!(
        missing_page,
        GraphPagerError::MissingPage { storage_ref } if storage_ref == page_ref
        ));
        assert!(matches!(
        corrupted_page,
        GraphPagerError::CorruptedPage { storage_ref, reason }
        if storage_ref == page_ref && reason == "checksum mismatch"
        ));
        assert!(matches!(
        unavailable_record,
        GraphPagerError::UnavailableRecord { record_ref } if record_ref == node_ref
        ));
    }

    //
    // Verify that the `GraphPager` trait can be implemented by a test double
    // without requiring persistent storage or real payload loading. This protects
    // compatibility with an in-memory mock for future implementations.
    //
    // Given a pager test double that reports every requested record as unavailable,
    // when each pager method is called,
    // then the calls should compile through the trait boundary and return typed pager errors.
    #[test]
    fn graph_pager_trait_accepts_test_double_without_real_storage() {
        let pager = UnavailablePager;
        let node_id = node_id("node--unavailable");
        let relationship_id = relationship_id("relationship--unavailable");
        let node_record_ref = GraphRecordRef::Node(node_id.clone());

        assert!(matches!(
        pager.load_node_payload(&node_id),
        Err(GraphPagerError::UnavailableRecord { record_ref })
        if record_ref == GraphRecordRef::Node(node_id.clone())
        ));
        assert!(matches!(
        pager.load_relationship_payload(&relationship_id),
        Err(GraphPagerError::UnavailableRecord { record_ref })
        if record_ref == GraphRecordRef::Relationship(relationship_id.clone())
        ));
        assert!(matches!(
        pager.load_outgoing_adjacency(&node_id),
        Err(GraphPagerError::UnavailableRecord { record_ref })
        if record_ref == GraphRecordRef::Node(node_id.clone())
        ));
        assert!(matches!(
        pager.load_incoming_adjacency(&node_id),
        Err(GraphPagerError::UnavailableRecord { record_ref })
        if record_ref == GraphRecordRef::Node(node_id.clone())
        ));
        assert!(matches!(
        pager.load_indexed_metadata(&node_record_ref),
        Err(GraphPagerError::UnavailableRecord { record_ref }) if record_ref == node_record_ref
        ));
    }

    #[test]
    fn graph_pager_default_page_in_returns_not_supported() {
        let pager = UnavailablePager;
        let node_record_ref = GraphRecordRef::Node(node_id("node--page-in"));
        let request = PageInRequest {
            page: PageIdentity {
                kind: PageIdentityKind::IndexedMetadata,
                segment: "memory/metadata".to_owned(),
                page_id: "page-1".to_owned(),
                storage_ref: None,
            },
            record_refs: vec![node_record_ref],
        };

        let error = pager
            .page_in(&request)
            .expect_err("default page-in should return not-supported");

        assert!(matches!(
        error,
        GraphPagerError::PageInNotSupported { page }
        if page.kind == PageIdentityKind::IndexedMetadata
        && page.segment == "memory/metadata"
        && page.page_id == "page-1"
        ));
    }

    #[test]
    fn graph_implements_pager_payload_and_metadata_loading() {
        let (graph, source, _target, relationship_id) = graph_with_one_relationship();

        let paged_node = graph
            .load_node_payload(&source)
            .expect("node payload should be available");
        assert_eq!(paged_node.node.id(), &source);
        assert!(matches!(
        paged_node.storage_ref,
        Some(StorageRef::External { uri })
        if uri == format!("memory://graph/nodes/{}", source.as_str())
        ));

        let paged_relationship = graph
            .load_relationship_payload(&relationship_id)
            .expect("relationship payload should be available");
        assert_eq!(paged_relationship.relationship.id(), &relationship_id);
        assert!(matches!(
        paged_relationship.storage_ref,
        Some(StorageRef::External { uri })
        if uri == format!("memory://graph/relationships/{}", relationship_id.as_str())
        ));

        let node_metadata = graph
            .load_indexed_metadata(&GraphRecordRef::Node(source.clone()))
            .expect("node metadata should be available");
        assert_eq!(node_metadata.loading_state, LoadingState::Indexed);
        assert_eq!(node_metadata.record_ref, GraphRecordRef::Node(source));

        let relationship_metadata = graph
            .load_indexed_metadata(&GraphRecordRef::Relationship(relationship_id.clone()))
            .expect("relationship metadata should be available");
        assert_eq!(
            relationship_metadata.record_ref,
            GraphRecordRef::Relationship(relationship_id)
        );
        assert!(relationship_metadata.relationship_type.is_some());
    }

    #[test]
    fn graph_implements_pager_adjacency_loading() {
        let (graph, source, target, relationship_id) = graph_with_one_relationship();

        let outgoing = graph
            .load_outgoing_adjacency(&source)
            .expect("outgoing adjacency should be available");
        assert_eq!(outgoing.owner_node_id, source);
        assert_eq!(outgoing.direction, AdjacencyDirection::Outgoing);
        assert_eq!(outgoing.entries.len(), 1);
        assert_eq!(outgoing.entries[0].relationship_id, relationship_id);
        assert_eq!(outgoing.entries[0].neighbor_node_id, target.clone());

        let incoming = graph
            .load_incoming_adjacency(&target)
            .expect("incoming adjacency should be available");
        assert_eq!(incoming.owner_node_id, target);
        assert_eq!(incoming.direction, AdjacencyDirection::Incoming);
        assert_eq!(incoming.entries.len(), 1);
    }

    #[test]
    fn graph_pager_returns_unavailable_for_missing_records() {
        let graph = Graph::new();
        let missing_node = node_id("node--missing-for-pager");
        let missing_relationship = relationship_id("relationship--missing-for-pager");

        assert!(matches!(
        graph.load_node_payload(&missing_node),
        Err(GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(id)
        }) if id == missing_node
        ));
        assert!(matches!(
        graph.load_outgoing_adjacency(&missing_node),
        Err(GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(id)
        }) if id == missing_node
        ));
        assert!(matches!(
        graph.load_incoming_adjacency(&missing_node),
        Err(GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(id)
        }) if id == missing_node
        ));
        assert!(matches!(
        graph.load_relationship_payload(&missing_relationship),
        Err(GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Relationship(id)
        }) if id == missing_relationship
        ));
        assert!(matches!(
        graph.load_indexed_metadata(&GraphRecordRef::Relationship(missing_relationship.clone())),
        Err(GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Relationship(id)
        }) if id == missing_relationship
        ));
    }

    #[test]
    fn helper_builders_and_corruption_mapping_are_deterministic() {
        let node = node_id("node--helper-ref");
        let relationship = relationship_id("relationship--helper-ref");

        assert_eq!(
            node_storage_ref(&node),
            StorageRef::External {
                uri: format!("memory://graph/nodes/{}", node.as_str())
            }
        );
        assert_eq!(
            relationship_storage_ref(&relationship),
            StorageRef::External {
                uri: format!("memory://graph/relationships/{}", relationship.as_str())
            }
        );
        assert_eq!(
            adjacency_storage_ref(&node, AdjacencyDirection::Outgoing),
            StorageRef::External {
                uri: format!("memory://graph/adjacency/outgoing/{}", node.as_str())
            }
        );
        assert_eq!(
            adjacency_storage_ref(&node, AdjacencyDirection::Incoming),
            StorageRef::External {
                uri: format!("memory://graph/adjacency/incoming/{}", node.as_str())
            }
        );

        assert!(matches!(
        unavailable_node(&node),
        GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(id)
        } if id == node
        ));
        assert!(matches!(
        unavailable_relationship(&relationship),
        GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Relationship(id)
        } if id == relationship
        ));

        let corruption = corrupted_in_memory_page(
            node_storage_ref(&node),
            GraphError::InvalidVersionState("broken-invariant".to_owned()),
        );
        assert!(matches!(
        corruption,
        GraphPagerError::CorruptedPage { storage_ref, reason }
        if storage_ref == node_storage_ref(&node)
        && reason.contains("broken-invariant")
        ));
    }
}
