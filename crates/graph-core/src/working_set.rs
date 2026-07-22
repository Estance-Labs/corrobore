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
//! Bounded graph working set model.
//!
//! Design boundary:
//!
//! A `GraphWorkingSet` is not the full graph. It is the bounded in-memory view
//! an agent or query engine will operate on while the persistent graph remains
//! outside this model. The working set may reference cold, indexed, warm, and hot
//! records, but it must not require any storage backend to exist.
//!
//! Runtime boundary:
//!
//! This module owns only the structural model and basic in-memory bookkeeping for
//! working sets. Graph loading, eviction, semantic search, Cypher execution,
//! persistent storage, graph paging, and prefetch behavior remain outside this
//! module.
//!
//! Warm adjacency boundary for issue 40:
//!
//! - Model warm frontier entries without loading full node payloads.
//! - Model relationship identity, type, direction, endpoint IDs, target labels,
//!   target loading state, relevance placeholder, and future storage references.
//! - Support construction, read access, and working-set attachment for warm
//!   adjacency metadata only.
//! - Do not implement prefetch scheduling, page-in, traversal, eviction, or
//!   relationship/node payload loading in this module.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::graph_pager::{AdjacencyDirection, StorageRef};
use crate::{GraphError, LabelSet, NodeId, RelationshipId, RelationshipType};

/// Typed identifier for a bounded graph working set.
///
///
/// - Keep working set IDs separate from graph record IDs and workspace IDs.
/// - Reuse the same string-backed identifier policy as other graph-core IDs.
/// - Avoid tying the ID to any storage backend, cache key, semantic index, or
///   query execution session.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkingSetId {
    value: String,
}

impl WorkingSetId {
    /// Build a typed working set identifier from a string-like value.
    ///
    /// The identifier is validated with `trim().is_empty()` but stored exactly as
    /// provided. This keeps validation separate from normalization, matching the
    /// policy used by the other graph-core typed identifiers.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err(GraphError::InvalidIdentifier("WorkingSetId".to_owned()));
        }

        Ok(Self { value })
    }

    /// Return the identifier as a borrowed string slice without allocation.
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

/// Loading state of a graph record from the point of view of a working set.
///
/// The state is explicit on purpose. Callers should not infer loading behavior
/// from boolean flags such as `loaded: true`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadingState {
    /// The record exists outside the active working set and no useful record data
    /// or adjacency has been loaded into memory.
    Cold,

    /// The working set knows the record identity and enough indexed metadata to
    /// find or reason about it, but it does not own the record payload.
    Indexed,

    /// The working set has partial information, such as adjacency summaries,
    /// lightweight metadata, or frontier hints, but not the full hot payload.
    Warm,

    /// The working set has the record information required for active traversal,
    /// mutation planning, validation, or explanation.
    Hot,
}

/// Placeholder relevance score carried by a warm adjacency entry.
///
///
/// - Keep relevance scoring explicit without committing to a final scoring model.
/// - Give later profile-based expansion logic a typed place to store edge or
///   neighbor priority.
/// - Avoid mixing score semantics with the loading state itself.
///
/// The score uses the same bounded numeric contract as graph confidence values:
/// finite values in the inclusive `0.0..=1.0` range are accepted.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarmAdjacencyRelevanceScore {
    value: f64,
}

impl WarmAdjacencyRelevanceScore {
    /// Build a warm-adjacency relevance score.
    ///
    ///
    /// keep score validation at the primitive boundary before entries reach a
    /// working set.
    ///
    ///
    /// accept finite values in the inclusive `0.0..=1.0` range and preserve the
    /// original value without normalization.
    ///
    /// # Errors
    ///
    /// reject NaN, infinity, negative scores, or values above `1.0` with
    /// `GraphError::InvalidConfidence` to reuse the existing bounded-score error
    /// boundary.
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidConfidence(value));
        }

        Ok(Self { value })
    }

    /// Return the raw relevance score value.
    ///
    ///
    /// expose the score without leaking any future profile or traversal policy.
    ///
    ///
    /// return the validated stored score.
    ///
    /// # Errors
    ///
    /// none expected because invalid values are rejected by the constructor.
    pub fn value(&self) -> f64 {
        self.value
    }
}

/// Public builder input for a warm adjacency entry.
///
///
/// - Keep warm adjacency construction readable without exposing a long positional
///   constructor.
/// - Make required relationship/source/target metadata explicit.
/// - Keep optional ranking, loading-state, and storage-reference metadata as
///   named builder steps so callers cannot accidentally swap positional values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarmAdjacencyEntryInput {
    relationship_id: RelationshipId,
    relationship_type: RelationshipType,
    source_node_id: NodeId,
    target_node_id: NodeId,
    target_labels: LabelSet,
    direction: AdjacencyDirection,
    relevance_score: Option<WarmAdjacencyRelevanceScore>,
    target_loading_state: LoadingState,
    relationship_storage_ref: Option<StorageRef>,
    target_storage_ref: Option<StorageRef>,
}

impl WarmAdjacencyEntryInput {
    /// Build the required warm adjacency metadata.
    ///
    ///
    /// require relationship identity, relationship type, frontier source, target,
    /// labels, and direction before optional loading hints are added.
    ///
    ///
    /// create an input object with no relevance score, `LoadingState::Warm`, and
    /// no storage references by default.
    ///
    /// # Errors
    ///
    /// none expected because required values are already typed and validated.
    pub fn new(
        relationship_id: RelationshipId,
        relationship_type: RelationshipType,
        source_node_id: NodeId,
        target_node_id: NodeId,
        target_labels: LabelSet,
        direction: AdjacencyDirection,
    ) -> Self {
        Self {
            relationship_id,
            relationship_type,
            source_node_id,
            target_node_id,
            target_labels,
            direction,
            // Relevance score.
            relevance_score: None,
            // Target loading state.
            target_loading_state: LoadingState::Warm,
            // Relationship storage ref.
            relationship_storage_ref: None,
            // Target storage ref.
            target_storage_ref: None,
        }
    }

    /// Attach an optional ranking hint to the warm adjacency input.
    pub fn with_relevance_score(mut self, relevance_score: WarmAdjacencyRelevanceScore) -> Self {
        self.relevance_score = Some(relevance_score);
        self
    }

    /// Override the target loading state represented by the warm entry.
    pub fn with_target_loading_state(mut self, target_loading_state: LoadingState) -> Self {
        self.target_loading_state = target_loading_state;
        self
    }

    /// Attach a future page-in reference for the relationship payload.
    pub fn with_relationship_storage_ref(mut self, storage_ref: StorageRef) -> Self {
        self.relationship_storage_ref = Some(storage_ref);
        self
    }

    /// Attach a future page-in reference for the target node payload.
    pub fn with_target_storage_ref(mut self, storage_ref: StorageRef) -> Self {
        self.target_storage_ref = Some(storage_ref);
        self
    }

    /// Attach optional future page-in references when they are already optional.
    pub fn with_storage_refs(
        mut self,
        relationship_storage_ref: Option<StorageRef>,
        target_storage_ref: Option<StorageRef>,
    ) -> Self {
        self.relationship_storage_ref = relationship_storage_ref;
        self.target_storage_ref = target_storage_ref;
        self
    }
}

/// Lightweight warm frontier edge attached to a working set.
///
///
/// - Represent a neighbor that can be loaded next without requiring the full
///   target node payload.
/// - Preserve relationship ID, relationship type, direction, and endpoint IDs so
///   the query or loading layer can reason about the edge without loading the full
///   relationship payload.
/// - Preserve target labels and loading state so profile-based expansion can make
///   decisions before the target becomes hot.
/// - Preserve backend-neutral storage references for future lazy page-in.
///
/// Important boundary:
///
/// `source_node_id` is the working-set frontier node for this entry and
/// `target_node_id` is the neighboring node candidate to load next. The explicit
/// `direction` records how the underlying relationship is oriented relative to the
/// frontier node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarmAdjacencyEntry {
    relationship_id: RelationshipId,
    relationship_type: RelationshipType,
    source_node_id: NodeId,
    target_node_id: NodeId,
    target_labels: LabelSet,
    direction: AdjacencyDirection,
    relevance_score: Option<WarmAdjacencyRelevanceScore>,
    target_loading_state: LoadingState,
    relationship_storage_ref: Option<StorageRef>,
    target_storage_ref: Option<StorageRef>,
}

impl WarmAdjacencyEntry {
    /// Build a warm adjacency entry from lightweight frontier metadata.
    ///
    ///
    /// create the only public constructor for warm adjacency entries so callers
    /// attach consistent edge metadata to a working set.
    ///
    ///
    /// build an entry that carries relationship identity, explicit relationship
    /// type, source and target IDs, target labels, direction, relevance score,
    /// target loading state, and future storage references without loading full
    /// node or relationship payloads.
    ///
    /// # Errors
    ///
    /// none expected here because identifiers, relationship types, relevance
    /// scores, direction, and loading state are already typed or validated before
    /// this constructor is called. Cross-record orientation is validated when the
    /// entry is attached to a working set.
    pub fn new(input: WarmAdjacencyEntryInput) -> Result<Self, GraphError> {
        Ok(Self {
            relationship_id: input.relationship_id,
            relationship_type: input.relationship_type,
            source_node_id: input.source_node_id,
            target_node_id: input.target_node_id,
            target_labels: input.target_labels,
            direction: input.direction,
            relevance_score: input.relevance_score,
            target_loading_state: input.target_loading_state,
            relationship_storage_ref: input.relationship_storage_ref,
            target_storage_ref: input.target_storage_ref,
        })
    }

    /// Return the relationship ID represented by this warm adjacency entry.
    ///
    ///
    /// let callers correlate the warm edge with relationship metadata or a future
    /// relationship payload load.
    ///
    ///
    /// return the stable relationship ID without loading the relationship payload.
    ///
    /// # Errors
    ///
    /// none expected because entry construction guarantees this field exists.
    pub fn relationship_id(&self) -> &RelationshipId {
        &self.relationship_id
    }

    /// Return the explicit relationship type for expansion decisions.
    ///
    ///
    /// allow loading profiles to inspect relationship type without loading the full
    /// relationship payload.
    ///
    ///
    /// return the relationship type supplied at construction time.
    ///
    /// # Errors
    ///
    /// none expected because warm adjacency entries require an explicit type.
    pub fn relationship_type(&self) -> &RelationshipType {
        &self.relationship_type
    }

    /// Return the frontier source node ID for this warm entry.
    ///
    ///
    /// identify the node from which this warm edge was attached to the working set.
    ///
    ///
    /// return the source/frontier node ID without requiring its full payload.
    ///
    /// # Errors
    ///
    /// none expected because entry construction guarantees this field exists.
    pub fn source_node_id(&self) -> &NodeId {
        &self.source_node_id
    }

    /// Return the neighboring target node ID for this warm entry.
    ///
    ///
    /// expose the candidate node that can later be made hot by a page-in operation.
    ///
    ///
    /// return the target node ID without loading the target payload.
    ///
    /// # Errors
    ///
    /// none expected because entry construction guarantees this field exists.
    pub fn target_node_id(&self) -> &NodeId {
        &self.target_node_id
    }

    /// Return target labels known from indexed or adjacency-level metadata.
    ///
    ///
    /// let loading profiles make label-based expansion decisions without loading
    /// the target node payload.
    ///
    ///
    /// return the lightweight target labels stored with the warm entry.
    ///
    /// # Errors
    ///
    /// none expected because an empty label set can represent unknown labels.
    pub fn target_labels(&self) -> &LabelSet {
        &self.target_labels
    }

    /// Return the relationship direction relative to the frontier node.
    ///
    ///
    /// keep incoming and outgoing warm frontiers explicit for traversal planning.
    ///
    ///
    /// return the direction supplied at construction time.
    ///
    /// # Errors
    ///
    /// none expected because direction is a required typed field.
    pub fn direction(&self) -> AdjacencyDirection {
        self.direction
    }

    /// Return the optional relevance score placeholder.
    ///
    ///
    /// expose ranking metadata without defining final scoring policy in this model.
    ///
    ///
    /// return `Some` when the source has a score, otherwise `None`.
    ///
    /// # Errors
    ///
    /// none expected because score validation belongs to the score constructor.
    pub fn relevance_score(&self) -> Option<WarmAdjacencyRelevanceScore> {
        self.relevance_score
    }

    /// Return the loading state of the neighboring target node.
    ///
    ///
    /// allow callers to distinguish indexed, warm, and hot targets without loading
    /// the target payload.
    ///
    ///
    /// return the target loading state stored with this warm edge.
    ///
    /// # Errors
    ///
    /// none expected because loading state is a required typed field.
    pub fn target_loading_state(&self) -> LoadingState {
        self.target_loading_state
    }

    /// Return the optional storage reference for future relationship page-in.
    ///
    ///
    /// preserve where the relationship payload can be loaded from later without
    /// requiring the payload now.
    ///
    ///
    /// return the relationship storage reference when the pager or catalog knows it.
    ///
    /// # Errors
    ///
    /// none expected because absence is represented as `None`.
    pub fn relationship_storage_ref(&self) -> Option<&StorageRef> {
        self.relationship_storage_ref.as_ref()
    }

    /// Return the optional storage reference for future target node page-in.
    ///
    ///
    /// preserve where the target node payload can be loaded from later without
    /// requiring the payload now.
    ///
    ///
    /// return the target storage reference when the pager or catalog knows it.
    ///
    /// # Errors
    ///
    /// none expected because absence is represented as `None`.
    pub fn target_storage_ref(&self) -> Option<&StorageRef> {
        self.target_storage_ref.as_ref()
    }

    /// Report whether the target is already loaded enough for hot traversal.
    ///
    ///
    /// give callers a stable helper instead of comparing raw loading-state values
    /// throughout traversal code.
    ///
    ///
    /// return true only when the target loading state is `LoadingState::Hot`.
    ///
    /// # Errors
    ///
    /// none expected because loading state is a required typed field.
    pub fn is_target_loaded(&self) -> bool {
        self.target_loading_state == LoadingState::Hot
    }

    /// Report whether the target still needs payload loading before hot traversal.
    ///
    ///
    /// give callers a stable helper for lazy page-in decisions without implementing
    /// page-in in the warm adjacency model.
    ///
    ///
    /// return true for cold, indexed, or warm targets.
    ///
    /// # Errors
    ///
    /// none expected because loading state is a required typed field.
    pub fn is_target_unloaded(&self) -> bool {
        !self.is_target_loaded()
    }
}

/// Placeholder counters for loaded records inside a working set.
///
///
/// - Give future implementations a stable place to report bounded loading statistics.
/// - Keep stats descriptive only in this model.
/// - Avoid committing to a memory accounting or pager implementation yet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphWorkingSetStats {
    hot_node_count: u64,
    hot_relationship_count: u64,
    warm_node_count: u64,
    warm_relationship_count: u64,
}

impl GraphWorkingSetStats {
    /// Create an empty stats placeholder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the number of hot node records represented by this working set.
    pub fn hot_node_count(&self) -> u64 {
        self.hot_node_count
    }

    /// Return the number of hot relationship records represented by this working set.
    pub fn hot_relationship_count(&self) -> u64 {
        self.hot_relationship_count
    }

    /// Return the number of warm node records represented by this working set.
    pub fn warm_node_count(&self) -> u64 {
        self.warm_node_count
    }

    /// Return the number of warm relationship records represented by this working set.
    pub fn warm_relationship_count(&self) -> u64 {
        self.warm_relationship_count
    }
}

/// Initial bounded working set structure for graph loading.
///
///
/// - Represent the active operational subgraph separately from the full graph.
/// - Track seed nodes without forcing unrelated graph records to load.
/// - Track hot nodes and hot relationships independently.
/// - Represent pinned and dirty records explicitly.
/// - Keep node and relationship loading states visible to the model.
/// - Track warm adjacency entries as frontier metadata without loading full node
///   or relationship payloads.
/// - Stay independent from semantic search, Cypher execution, graph paging,
///   prefetch policies, and persistent storage implementation details.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphWorkingSet {
    id: WorkingSetId,
    seed_node_ids: HashSet<NodeId>,
    hot_node_ids: HashSet<NodeId>,
    hot_relationship_ids: HashSet<RelationshipId>,
    pinned_node_ids: HashSet<NodeId>,
    pinned_relationship_ids: HashSet<RelationshipId>,
    dirty_node_ids: HashSet<NodeId>,
    dirty_relationship_ids: HashSet<RelationshipId>,
    node_loading_states: HashMap<NodeId, LoadingState>,
    relationship_loading_states: HashMap<RelationshipId, LoadingState>,
    warm_adjacency_by_source: HashMap<NodeId, Vec<WarmAdjacencyEntry>>,
    stats: GraphWorkingSetStats,
}

impl GraphWorkingSet {
    /// Create a bounded working set separate from the persistent full graph.
    ///
    /// This constructor only initializes the model. It does not open storage,
    /// execute semantic search, run a query, or load graph records.
    pub fn new(id: WorkingSetId) -> Self {
        Self {
            id,
            // Seed node ids.
            seed_node_ids: HashSet::new(),
            // Hot node ids.
            hot_node_ids: HashSet::new(),
            // Hot relationship ids.
            hot_relationship_ids: HashSet::new(),
            // Pinned node ids.
            pinned_node_ids: HashSet::new(),
            // Pinned relationship ids.
            pinned_relationship_ids: HashSet::new(),
            // Dirty node ids.
            dirty_node_ids: HashSet::new(),
            // Dirty relationship ids.
            dirty_relationship_ids: HashSet::new(),
            // Node loading states.
            node_loading_states: HashMap::new(),
            // Relationship loading states.
            relationship_loading_states: HashMap::new(),
            // Warm adjacency by source.
            warm_adjacency_by_source: HashMap::new(),
            // Stats.
            stats: GraphWorkingSetStats::new(),
        }
    }

    /// Return the working set identifier.
    pub fn id(&self) -> &WorkingSetId {
        &self.id
    }

    /// Record a seed node for this working set without loading unrelated graph records.
    ///
    /// Recording a seed means the working set knows the node identity. It does not
    /// mean the node payload, adjacency, or relationships are hot.
    pub fn record_seed_node(&mut self, node_id: NodeId) {
        self.seed_node_ids.insert(node_id.clone());
        self.node_loading_states
            .entry(node_id)
            .or_insert(LoadingState::Indexed);
        self.recalculate_stats();
    }

    /// Return all seed node IDs currently associated with the working set.
    pub fn seed_node_ids(&self) -> &HashSet<NodeId> {
        &self.seed_node_ids
    }

    /// Track a node as hot inside the working set.
    pub fn track_hot_node(&mut self, node_id: NodeId) {
        self.hot_node_ids.insert(node_id.clone());
        self.node_loading_states.insert(node_id, LoadingState::Hot);
        self.recalculate_stats();
    }

    /// Return all hot node IDs represented by the working set.
    pub fn hot_node_ids(&self) -> &HashSet<NodeId> {
        &self.hot_node_ids
    }

    /// Track a relationship as hot inside the working set.
    pub fn track_hot_relationship(&mut self, relationship_id: RelationshipId) {
        self.hot_relationship_ids.insert(relationship_id.clone());
        self.relationship_loading_states
            .insert(relationship_id, LoadingState::Hot);
        self.recalculate_stats();
    }

    /// Return all hot relationship IDs represented by the working set.
    pub fn hot_relationship_ids(&self) -> &HashSet<RelationshipId> {
        &self.hot_relationship_ids
    }

    /// Pin a node so later eviction logic cannot remove it from the working set.
    pub fn pin_node(&mut self, node_id: NodeId) {
        self.pinned_node_ids.insert(node_id);
    }

    /// Pin a relationship so later eviction logic cannot remove it from the working set.
    pub fn pin_relationship(&mut self, relationship_id: RelationshipId) {
        self.pinned_relationship_ids.insert(relationship_id);
    }

    /// Return pinned node IDs.
    pub fn pinned_node_ids(&self) -> &HashSet<NodeId> {
        &self.pinned_node_ids
    }

    /// Return pinned relationship IDs.
    pub fn pinned_relationship_ids(&self) -> &HashSet<RelationshipId> {
        &self.pinned_relationship_ids
    }

    /// Mark a node record as dirty relative to the clean loaded state.
    pub fn mark_dirty_node(&mut self, node_id: NodeId) {
        self.dirty_node_ids.insert(node_id);
    }

    /// Mark a relationship record as dirty relative to the clean loaded state.
    pub fn mark_dirty_relationship(&mut self, relationship_id: RelationshipId) {
        self.dirty_relationship_ids.insert(relationship_id);
    }

    /// Return dirty node IDs tracked separately from clean loaded records.
    pub fn dirty_node_ids(&self) -> &HashSet<NodeId> {
        &self.dirty_node_ids
    }

    /// Return dirty relationship IDs tracked separately from clean loaded records.
    pub fn dirty_relationship_ids(&self) -> &HashSet<RelationshipId> {
        &self.dirty_relationship_ids
    }

    /// Attach a warm adjacency entry to this working set.
    ///
    ///
    /// let future loading code keep a nearby graph boundary available without
    /// forcing target node or relationship payloads into the hot set.
    ///
    ///
    /// add the entry under its source/frontier node, mark the target node according
    /// to the entry metadata, mark the relationship as warm metadata, and update
    /// working-set statistics without loading any full payloads. Existing hotter
    /// states are not downgraded.
    ///
    /// # Errors
    ///
    /// reject an entry whose source does not match the provided source node because
    /// it would make the grouped warm frontier inconsistent.
    pub fn attach_warm_adjacency(
        &mut self,
        source_node_id: NodeId,
        entry: WarmAdjacencyEntry,
    ) -> Result<(), GraphError> {
        if source_node_id != entry.source_node_id {
            return Err(GraphError::InternalInvariantViolation(format!(
                "warm adjacency source mismatch: provided {}, entry {}",
                source_node_id.as_str(),
                entry.source_node_id.as_str()
            )));
        }

        let relationship_id = entry.relationship_id.clone();
        let target_node_id = entry.target_node_id.clone();
        let target_loading_state = entry.target_loading_state;

        self.merge_node_loading_state(target_node_id, target_loading_state);
        self.merge_relationship_loading_state(relationship_id, LoadingState::Warm);
        self.warm_adjacency_by_source
            .entry(source_node_id)
            .or_default()
            .push(entry);
        self.recalculate_stats();

        Ok(())
    }

    /// Return warm adjacency entries attached to a source/frontier node.
    ///
    ///
    /// expose the currently known warm frontier for a node without loading any full
    /// neighboring payloads.
    ///
    ///
    /// return entries in the order they were attached for deterministic tests.
    ///
    /// # Errors
    ///
    /// none expected; absence is represented as `None`.
    pub fn warm_adjacency_for_source(
        &self,
        source_node_id: &NodeId,
    ) -> Option<&Vec<WarmAdjacencyEntry>> {
        self.warm_adjacency_by_source.get(source_node_id)
    }

    /// Return all warm adjacency entries grouped by source/frontier node.
    ///
    ///
    /// give diagnostics and acceptance tests a stable read-only view of the warm
    /// frontier while preserving ownership inside the working set.
    ///
    ///
    /// return the grouped warm adjacency map without loading additional records.
    ///
    /// # Errors
    ///
    /// none expected because this is a read-only accessor.
    pub fn warm_adjacency_by_source(&self) -> &HashMap<NodeId, Vec<WarmAdjacencyEntry>> {
        &self.warm_adjacency_by_source
    }

    /// Set the explicit loading state for a node record.
    pub fn set_node_loading_state(&mut self, node_id: NodeId, state: LoadingState) {
        if state == LoadingState::Hot {
            self.hot_node_ids.insert(node_id.clone());
        } else {
            self.hot_node_ids.remove(&node_id);
        }

        self.node_loading_states.insert(node_id, state);
        self.recalculate_stats();
    }

    /// Set the explicit loading state for a relationship record.
    pub fn set_relationship_loading_state(
        &mut self,
        relationship_id: RelationshipId,
        state: LoadingState,
    ) {
        if state == LoadingState::Hot {
            self.hot_relationship_ids.insert(relationship_id.clone());
        } else {
            self.hot_relationship_ids.remove(&relationship_id);
        }

        self.relationship_loading_states
            .insert(relationship_id, state);
        self.recalculate_stats();
    }

    /// Return the explicit loading state for a node record when known.
    pub fn node_loading_state(&self, node_id: &NodeId) -> Option<LoadingState> {
        self.node_loading_states.get(node_id).copied()
    }

    /// Return the explicit loading state for a relationship record when known.
    pub fn relationship_loading_state(
        &self,
        relationship_id: &RelationshipId,
    ) -> Option<LoadingState> {
        self.relationship_loading_states
            .get(relationship_id)
            .copied()
    }

    /// Return placeholder loaded-record statistics for this working set.
    pub fn stats(&self) -> &GraphWorkingSetStats {
        &self.stats
    }

    fn merge_node_loading_state(&mut self, node_id: NodeId, incoming: LoadingState) {
        let merged = merge_loading_state(self.node_loading_states.get(&node_id).copied(), incoming);

        if merged == LoadingState::Hot {
            self.hot_node_ids.insert(node_id.clone());
        } else {
            self.hot_node_ids.remove(&node_id);
        }

        self.node_loading_states.insert(node_id, merged);
    }

    fn merge_relationship_loading_state(
        &mut self,
        relationship_id: RelationshipId,
        incoming: LoadingState,
    ) {
        let merged = merge_loading_state(
            self.relationship_loading_states
                .get(&relationship_id)
                .copied(),
            incoming,
        );

        if merged == LoadingState::Hot {
            self.hot_relationship_ids.insert(relationship_id.clone());
        } else {
            self.hot_relationship_ids.remove(&relationship_id);
        }

        self.relationship_loading_states
            .insert(relationship_id, merged);
    }

    fn recalculate_stats(&mut self) {
        self.stats.hot_node_count =
            Self::count_states(&self.node_loading_states, LoadingState::Hot);
        self.stats.hot_relationship_count =
            Self::count_states(&self.relationship_loading_states, LoadingState::Hot);
        self.stats.warm_node_count =
            Self::count_states(&self.node_loading_states, LoadingState::Warm);
        self.stats.warm_relationship_count =
            Self::count_states(&self.relationship_loading_states, LoadingState::Warm);
    }

    fn count_states<T>(states: &HashMap<T, LoadingState>, expected: LoadingState) -> u64
    where
        T: Eq + std::hash::Hash,
    {
        states.values().filter(|state| **state == expected).count() as u64
    }
}

fn merge_loading_state(existing: Option<LoadingState>, incoming: LoadingState) -> LoadingState {
    match existing {
        Some(existing) if loading_state_rank(existing) >= loading_state_rank(incoming) => existing,
        _ => incoming,
    }
}

fn loading_state_rank(state: LoadingState) -> u8 {
    match state {
        LoadingState::Cold => 0,
        LoadingState::Indexed => 1,
        LoadingState::Warm => 2,
        LoadingState::Hot => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn working_set_id(value: &str) -> WorkingSetId {
        WorkingSetId::new(value).expect("test working set ID should be valid")
    }

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node ID should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship ID should be valid")
    }

    fn relationship_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
    }

    #[test]
    fn warm_adjacency_relevance_score_validates_bounds_and_finiteness() {
        assert_eq!(
            WarmAdjacencyRelevanceScore::new(0.0)
                .expect("lower bound should be accepted")
                .value(),
            0.0
        );
        assert_eq!(
            WarmAdjacencyRelevanceScore::new(1.0)
                .expect("upper bound should be accepted")
                .value(),
            1.0
        );

        let nan_error =
            WarmAdjacencyRelevanceScore::new(f64::NAN).expect_err("NaN score should be rejected");
        assert!(matches!(nan_error, GraphError::InvalidConfidence(value) if value.is_nan()));

        let inf_error = WarmAdjacencyRelevanceScore::new(f64::INFINITY)
            .expect_err("infinite score should be rejected");
        assert!(matches!(inf_error, GraphError::InvalidConfidence(value) if value.is_infinite()));

        let negative_error =
            WarmAdjacencyRelevanceScore::new(-0.01).expect_err("negative score should be rejected");
        assert!(matches!(negative_error, GraphError::InvalidConfidence(value) if value == -0.01));

        let over_error =
            WarmAdjacencyRelevanceScore::new(1.01).expect_err("score above 1.0 should be rejected");
        assert!(matches!(over_error, GraphError::InvalidConfidence(value) if value == 1.01));
    }

    #[test]
    fn attach_warm_adjacency_rejects_source_mismatch() {
        let mut working_set = GraphWorkingSet::new(working_set_id("working-set--mismatch"));
        let provided_source = node_id("node--provided");
        let entry_source = node_id("node--entry");

        let entry = WarmAdjacencyEntry::new(WarmAdjacencyEntryInput::new(
            relationship_id("relationship--1"),
            relationship_type("SUPPORTS"),
            entry_source.clone(),
            node_id("node--target"),
            vec!["Campaign".to_owned()],
            AdjacencyDirection::Outgoing,
        ))
        .expect("warm adjacency entry should be built");

        let error = working_set
            .attach_warm_adjacency(provided_source.clone(), entry)
            .expect_err("source mismatch should return invariant violation");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains(provided_source.as_str()) && message.contains(entry_source.as_str())
        ));
    }

    #[test]
    fn set_loading_state_updates_hot_sets_and_stats() {
        let mut working_set = GraphWorkingSet::new(working_set_id("working-set--hot-state"));
        let node = node_id("node--1");
        let relationship = relationship_id("relationship--1");

        working_set.set_node_loading_state(node.clone(), LoadingState::Hot);
        working_set.set_relationship_loading_state(relationship.clone(), LoadingState::Hot);

        assert!(working_set.hot_node_ids().contains(&node));
        assert!(working_set.hot_relationship_ids().contains(&relationship));
        assert_eq!(working_set.stats().hot_node_count(), 1);
        assert_eq!(working_set.stats().hot_relationship_count(), 1);

        working_set.set_node_loading_state(node.clone(), LoadingState::Indexed);
        working_set.set_relationship_loading_state(relationship.clone(), LoadingState::Warm);

        assert!(!working_set.hot_node_ids().contains(&node));
        assert!(!working_set.hot_relationship_ids().contains(&relationship));
        assert_eq!(working_set.stats().hot_node_count(), 0);
        assert_eq!(working_set.stats().hot_relationship_count(), 0);
        assert_eq!(working_set.stats().warm_relationship_count(), 1);
    }

    #[test]
    fn attach_warm_adjacency_keeps_hot_target_state_when_incoming_is_warm() {
        let mut working_set = GraphWorkingSet::new(working_set_id("working-set--merge-state"));
        let source = node_id("node--source");
        let target = node_id("node--target");

        working_set.set_node_loading_state(target.clone(), LoadingState::Hot);

        let entry = WarmAdjacencyEntry::new(
            WarmAdjacencyEntryInput::new(
                relationship_id("relationship--2"),
                relationship_type("USES"),
                source.clone(),
                target.clone(),
                vec!["Indicator".to_owned()],
                AdjacencyDirection::Outgoing,
            )
            .with_target_loading_state(LoadingState::Warm)
            .with_storage_refs(
                Some(StorageRef::Offset {
                    segment: "relationships".to_owned(),
                    byte_offset: 22,
                }),
                Some(StorageRef::Offset {
                    segment: "nodes".to_owned(),
                    byte_offset: 11,
                }),
            ),
        )
        .expect("warm adjacency entry should be built");

        working_set
            .attach_warm_adjacency(source.clone(), entry.clone())
            .expect("warm adjacency should attach");

        assert_eq!(
            working_set.node_loading_state(&target),
            Some(LoadingState::Hot)
        );
        assert_eq!(
            working_set.relationship_loading_state(entry.relationship_id()),
            Some(LoadingState::Warm)
        );
        assert_eq!(working_set.stats().hot_node_count(), 1);
        assert_eq!(working_set.stats().warm_relationship_count(), 1);
        assert_eq!(
            working_set.warm_adjacency_for_source(&source).map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn merge_loading_state_prefers_highest_rank() {
        assert_eq!(
            merge_loading_state(Some(LoadingState::Hot), LoadingState::Warm),
            LoadingState::Hot
        );
        assert_eq!(
            merge_loading_state(Some(LoadingState::Indexed), LoadingState::Warm),
            LoadingState::Warm
        );
        assert_eq!(
            merge_loading_state(None, LoadingState::Cold),
            LoadingState::Cold
        );
    }
}
