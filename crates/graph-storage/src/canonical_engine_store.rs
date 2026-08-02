// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Canonical WAL-backed graph store used by standalone engine hosts.
//!
//! This boundary composes the append-only record logs, atomic mutation WAL,
//! recovered catalog, persistent adjacency and file-backed pager. Opening it is
//! metadata-only. Payloads enter a bounded operational projection only through
//! [`CanonicalEngineStore::load_projection`].

use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use graph_core::{
    AdjacencyDirection, EvidenceRecordStore, Graph, GraphPager, GraphPersistenceSnapshot,
    GraphSequenceFloor, Node, NodeId, PropertyValue, RecordStatus, Relationship, RelationshipId,
};
use opencti_access::AccessContext;
use opencti_access::{AccessMetadata, OpenCtiAccessPolicy};
use opencti_file_search::{
    EnqueueOutcome, ExtractionArtifact, FileContentError, FileContentIndex,
    FileContentIndexSettings, FileContentQuery, FileDescriptor, FileJobMetrics, FileJobStore,
};
use opencti_search::{
    FullTextDocument, FullTextIndex, FullTextIndexSettings, FullTextQuery, FullTextRebuildOutcome,
    FullTextSearchPage, FullTextSearchReadiness, document_from_node, document_from_relationship,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    AtomicPersistentMutationAdjacencyRecord, AtomicPersistentMutationBatch,
    AtomicPersistentMutationNodeRecord, AtomicPersistentMutationOutcome,
    AtomicPersistentMutationRelationshipRecord, AtomicPersistentRecoveryPath,
    AtomicPersistentRuntimeState, DurableTransactionId, EncodedRecord, FileBackedGraphPager,
    GraphCatalog, GraphCatalogIndexes, GraphStorageError, GraphStorageResult, JsonLinesRecordCodec,
    LabelIndexNodeMetadata, MutationCrashStage, NodeReadIndexDocument, NodeReadIndexValue,
    PersistedAdjacencyEntry, PersistedRecordEnvelope, PersistedRecordKind, RecordCodec,
    RecordFormat, RelationshipTypeIndexRelationshipMetadata, StorageRef, StorageRoot,
    StorageSegment, StorageVersion, apply_atomic_persistent_mutation_batch,
    create_file_backed_graph_pager, create_file_backed_graph_store, create_node_record_envelope,
    create_relationship_record_envelope, index_relationship_type, persist_graph_catalog_metadata,
    read_incoming_adjacency_by_node_id, read_incoming_adjacency_log_for_catalog_rebuild,
    read_outgoing_adjacency_by_node_id, read_outgoing_adjacency_log_for_catalog_rebuild,
    recover_atomic_persistent_runtime_state_with_report, replace_node_read_indexes,
    replace_relationship_access_index, resolve_identifier_index_entries, resolve_node_ids_by_label,
    resolve_property_index_entries, resolve_property_presence_entries,
    resolve_relationship_ids_by_type,
};

const LEGACY_SNAPSHOT: &str = "engine-graph.json";
const LEGACY_ROLLBACK_SNAPSHOT: &str = "engine-graph.rollback.json";
const MIGRATION_RECORD: &str = "engine-graph-migration.json";
const MIGRATION_RECORD_TEMP: &str = "engine-graph-migration.next.json";
const FULL_TEXT_SCHEMA_VERSION: &str = "opencti-full-text-v1";
const FULL_TEXT_CURSOR_KEY: &str = "cursor.key";
const FULL_TEXT_WRITER_MEMORY_BYTES: usize = 50_000_000;
const FULL_TEXT_MAX_CANDIDATES: usize = 100_000;
const FILE_CONTENT_SCHEMA_VERSION: &str = "opencti-file-content-v1";
const FILE_CONTENT_WRITER_MEMORY_BYTES: usize = 50_000_000;
const FILE_CONTENT_MAX_CANDIDATES: usize = 100_000;
const FILE_CONTENT_SNIPPET_CHARS: usize = 240;
const FILE_CONTENT_MAX_ATTEMPTS: u32 = 3;
const FILE_CONTENT_LEASE_MS: u64 = 60_000;
const EVIDENCE_SIDECAR: &str = "evidence-records-v1.json";
const EVIDENCE_SIDECAR_NEXT: &str = "evidence-records-v1.next.json";

/// Bounded resident-record budgets for the standalone canonical store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalStoreOptions {
    /// Maximum node payloads admitted into one operational projection.
    pub max_hot_nodes: u64,
    /// Maximum relationship payloads admitted into one operational projection.
    pub max_hot_relationships: u64,
    /// Maximum lightweight adjacency entries retained for one projection.
    pub max_warm_adjacency_entries: u64,
}

impl Default for CanonicalStoreOptions {
    fn default() -> Self {
        Self {
            max_hot_nodes: 16_384,
            max_hot_relationships: 32_768,
            max_warm_adjacency_entries: 65_536,
        }
    }
}

/// Scalar operators supported by compact canonical property indexes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalPropertyOperator {
    /// Exact typed equality.
    Equal,
    /// Typed inequality.
    NotEqual,
    /// Property presence.
    Exists,
    /// Property absence.
    NotExists,
    /// Exact membership in a non-empty typed set.
    In,
    /// Exclusion from a non-empty typed set.
    NotIn,
    /// Strict lower bound.
    GreaterThan,
    /// Inclusive lower bound.
    GreaterThanOrEqual,
    /// Strict upper bound.
    LessThan,
    /// Inclusive upper bound.
    LessThanOrEqual,
}

/// One storage-neutral property index predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPropertyFilter {
    /// Canonical graph property name.
    pub field: String,
    /// Comparison operator.
    pub operator: CanonicalPropertyOperator,
    /// Typed comparison value.
    pub value: Option<Value>,
}

/// Bounded persistent adjacency projection requested by a graph read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalAdjacencyProjection {
    /// Include incoming adjacency.
    pub incoming: bool,
    /// Include outgoing adjacency.
    pub outgoing: bool,
    /// Optional relationship-type allow-list.
    pub relationship_types: Vec<String>,
    /// Maximum expansion depth.
    pub max_depth: u32,
    /// Hard relationship expansion budget.
    pub max_relationships: u32,
    /// Degree threshold for explicit supernode refusal.
    pub supernode_threshold: u32,
}

/// Catalog selection used to build a bounded operational graph projection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalProjectionRequest {
    /// Whether current node records should be selected directly.
    pub include_nodes: bool,
    /// Optional node-label index key.
    pub node_label: Option<String>,
    /// Stable graph or OpenCTI identifiers resolved through compact metadata.
    pub identifiers: Vec<String>,
    /// Compact property and temporal index predicates.
    pub property_filters: Vec<CanonicalPropertyFilter>,
    /// Optional relationship-type index key.
    pub relationship_type: Option<String>,
    /// Whether relationships and their endpoint nodes are required.
    pub include_relationships: bool,
    /// Optional bounded adjacency expansion rooted at selected identifiers.
    pub adjacency: Option<CanonicalAdjacencyProjection>,
    /// Request-scoped authorization facts evaluated before payload page-in.
    pub access_context: Option<AccessContext>,
}

impl CanonicalProjectionRequest {
    /// Select current nodes carrying one label without loading relationships.
    pub fn for_label(label: impl Into<String>) -> Self {
        Self {
            include_nodes: true,
            node_label: Some(label.into()),
            identifiers: Vec::new(),
            property_filters: Vec::new(),
            relationship_type: None,
            include_relationships: false,
            adjacency: None,
            access_context: None,
        }
    }

    /// Select one current node through its graph or OpenCTI identifier index.
    pub fn for_identifier(identifier: impl Into<String>) -> Self {
        Self {
            include_nodes: true,
            node_label: None,
            identifiers: vec![identifier.into()],
            property_filters: Vec::new(),
            relationship_type: None,
            include_relationships: false,
            adjacency: None,
            access_context: None,
        }
    }

    /// Select every current node without loading relationships.
    pub fn all_nodes() -> Self {
        Self {
            include_nodes: true,
            node_label: None,
            identifiers: Vec::new(),
            property_filters: Vec::new(),
            relationship_type: None,
            include_relationships: false,
            adjacency: None,
            access_context: None,
        }
    }

    /// Select the full small-profile graph through the pager.
    pub fn all() -> Self {
        Self {
            include_nodes: true,
            node_label: None,
            identifiers: Vec::new(),
            property_filters: Vec::new(),
            relationship_type: None,
            include_relationships: true,
            adjacency: None,
            access_context: None,
        }
    }

    /// Include relationships, optionally constrained by type.
    pub fn with_relationships(mut self, relationship_type: Option<String>) -> Self {
        self.include_relationships = true;
        self.relationship_type = relationship_type;
        self
    }

    /// Add conjunctive compact property predicates.
    pub fn with_property_filters(
        mut self,
        filters: impl IntoIterator<Item = CanonicalPropertyFilter>,
    ) -> Self {
        self.property_filters = filters.into_iter().collect();
        self
    }

    /// Replace the identifier seed set used by point or graph reads.
    pub fn with_identifiers(mut self, identifiers: impl IntoIterator<Item = String>) -> Self {
        self.identifiers = identifiers.into_iter().collect();
        self
    }

    /// Add one bounded persistent adjacency expansion.
    pub fn with_adjacency(mut self, adjacency: CanonicalAdjacencyProjection) -> Self {
        self.include_relationships = true;
        self.adjacency = Some(adjacency);
        self
    }

    /// Scope index selection and payload residency to one access policy.
    pub fn with_access_context(mut self, access_context: AccessContext) -> Self {
        self.access_context = Some(access_context);
        self
    }
}

/// Metadata-only startup diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalStartupReport {
    /// Number of graph payloads hydrated while opening the store. This remains
    /// zero; migration uses a temporary graph before readiness.
    pub payloads_hydrated: u64,
    /// Whether derived indexes were rebuilt from committed transaction records.
    pub derived_indexes_rebuilt: bool,
    /// Whether this open performed the one-time legacy snapshot migration.
    pub legacy_snapshot_migrated: bool,
    /// Recovery path selected for committed WAL transactions.
    pub recovery_path: Option<AtomicPersistentRecoveryPath>,
    /// Number of committed transactions replayed after the selected checkpoint.
    pub replayed_transaction_count: usize,
    /// Non-fatal recovery diagnostics.
    pub warnings: Vec<String>,
}

/// Runtime metrics for page-in, cache residency, indexes and recovery.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalStoreStats {
    /// Payload page-ins since store open.
    pub page_ins: u64,
    /// Reuses of payloads resident in the immediately preceding projection.
    pub cache_hits: u64,
    /// Resident hot node payloads.
    pub resident_hot_nodes: u64,
    /// Resident hot relationship payloads.
    pub resident_hot_relationships: u64,
    /// Resident lightweight adjacency entries.
    pub resident_warm_adjacency_entries: u64,
    /// Canonical node records not resident in the current projection.
    pub resident_cold_nodes: u64,
    /// Canonical relationship records not resident in the current projection.
    pub resident_cold_relationships: u64,
    /// Latest-node catalog size.
    pub node_index_entries: u64,
    /// Latest-relationship catalog size.
    pub relationship_index_entries: u64,
    /// Label-index bucket count.
    pub label_index_entries: u64,
    /// Relationship-type-index bucket count.
    pub relationship_type_index_entries: u64,
    /// Distinct identifier-to-node mappings.
    pub identifier_index_entries: u64,
    /// Scalar property index values.
    pub property_index_entries: u64,
    /// Temporal scalar index values.
    pub temporal_index_entries: u64,
    /// Current node access-policy documents.
    pub node_access_index_entries: u64,
    /// Current relationship access-policy documents.
    pub relationship_access_index_entries: u64,
}

/// Diagnostics for the most recent bounded projection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalProjectionStats {
    /// Compact access paths used before any payload page-in.
    pub access_paths: Vec<&'static str>,
    /// Payload page-ins for this projection.
    pub page_ins: u64,
    /// Resident payload cache hits for this projection.
    pub cache_hits: u64,
    /// Candidates rejected through compact access metadata before page-in.
    pub authorization_denials: u64,
    /// Whether a policy-scope change invalidated the resident payload cache.
    pub policy_cache_invalidated: bool,
    /// Low-cardinality timing class that does not reveal record existence.
    pub timing_class: &'static str,
}

/// WAL-backed, append-only canonical graph store.
#[derive(Clone, Debug)]
pub struct CanonicalEngineStore {
    root: StorageRoot,
    state: AtomicPersistentRuntimeState,
    options: CanonicalStoreOptions,
    startup_report: CanonicalStartupReport,
    stats: CanonicalStoreStats,
    resident_nodes: HashMap<NodeId, Node>,
    resident_relationships: HashMap<RelationshipId, Relationship>,
    resident_policy_fingerprint: Option<String>,
    last_projection_stats: CanonicalProjectionStats,
    file_content_index: Option<FileContentIndex>,
    evidence: EvidenceRecordStore,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyMigrationRecord {
    schema_version: u32,
    source: String,
    rollback_boundary: String,
    source_bytes: u64,
    node_count: u64,
    relationship_count: u64,
    node_version_count: u64,
    relationship_version_count: u64,
}

#[derive(Debug, Deserialize)]
struct LegacyEngineGraphSnapshot {
    storage_version: StorageVersion,
    record_format: RecordFormat,
    graph: GraphPersistenceSnapshot,
}

impl CanonicalEngineStore {
    /// Open the canonical store by restoring manifest-adjacent WAL/checkpoint
    /// state and compact indexes without paging graph payloads.
    pub fn open(root: StorageRoot, options: CanonicalStoreOptions) -> GraphStorageResult<Self> {
        Self::open_with_strict_recovery(root, options, false)
    }

    /// Open the canonical store and validate append logs when strict recovery
    /// must rebuild missing derived metadata.
    pub fn open_with_strict_recovery(
        root: StorageRoot,
        options: CanonicalStoreOptions,
        strict_recovery: bool,
    ) -> GraphStorageResult<Self> {
        let catalog_metadata_existed = root
            .path()
            .join("catalog")
            .join("catalog_metadata.json")
            .is_file();
        if strict_recovery && !catalog_metadata_existed {
            validate_canonical_append_logs(&root)?;
        }
        let recovered = recover_atomic_persistent_runtime_state_with_report(&root)?;
        recover_evidence_sidecar(&root)?;
        let evidence = load_evidence_sidecar(&root)?;
        let mut store = Self {
            root,
            state: recovered.state,
            options,
            startup_report: CanonicalStartupReport {
                payloads_hydrated: 0,
                derived_indexes_rebuilt: !catalog_metadata_existed
                    && recovered.report.replayed_transaction_count > 0,
                legacy_snapshot_migrated: false,
                recovery_path: Some(recovered.report.recovery_path),
                replayed_transaction_count: recovered.report.replayed_transaction_count,
                warnings: recovered.report.warnings,
            },
            stats: CanonicalStoreStats::default(),
            resident_nodes: HashMap::new(),
            resident_relationships: HashMap::new(),
            resident_policy_fingerprint: None,
            last_projection_stats: CanonicalProjectionStats::default(),
            file_content_index: None,
            evidence,
        };
        store.migrate_legacy_snapshot_if_needed()?;
        store.refresh_index_stats();
        Ok(store)
    }

    /// Return the storage root.
    pub fn root(&self) -> &StorageRoot {
        &self.root
    }

    /// Return the recovered compact catalog.
    pub fn catalog(&self) -> &GraphCatalog {
        &self.state.catalog
    }

    /// Return configured hot/warm budgets.
    pub fn options(&self) -> &CanonicalStoreOptions {
        &self.options
    }

    /// Return metadata-only startup diagnostics.
    pub fn startup_report(&self) -> &CanonicalStartupReport {
        &self.startup_report
    }

    /// Return current storage and working-set counters.
    pub fn stats(&self) -> &CanonicalStoreStats {
        &self.stats
    }

    /// Return diagnostics for the latest bounded projection.
    pub fn last_projection_stats(&self) -> &CanonicalProjectionStats {
        &self.last_projection_stats
    }

    /// Build a lightweight file-backed handle from the currently recovered
    /// catalog and adjacency metadata without paging payloads.
    pub fn file_backed_store(&self) -> GraphStorageResult<crate::FileBackedGraphStore> {
        create_file_backed_graph_store(
            self.root.clone(),
            self.state.catalog.clone(),
            self.state.adjacency_storage.clone(),
        )
    }

    /// Build a bounded graph projection by resolving compact indexes first and
    /// paging only selected payload records.
    pub fn load_projection(
        &mut self,
        request: CanonicalProjectionRequest,
    ) -> GraphStorageResult<Graph> {
        let policy = request
            .access_context
            .as_ref()
            .map(OpenCtiAccessPolicy::compile)
            .transpose()
            .map_err(|error| GraphStorageError::OperationFailed {
                operation: "load_canonical_graph_projection",
                message: error.to_string(),
            })?;
        let policy_fingerprint = policy.as_ref().map(OpenCtiAccessPolicy::fingerprint);
        let policy_cache_invalidated = self.resident_policy_fingerprint != policy_fingerprint
            && (!self.resident_nodes.is_empty() || !self.resident_relationships.is_empty());
        if self.resident_policy_fingerprint != policy_fingerprint {
            self.resident_nodes.clear();
            self.resident_relationships.clear();
        }
        self.resident_policy_fingerprint = policy_fingerprint;
        let mut access_paths = Vec::new();
        if !request.identifiers.is_empty() {
            access_paths.push("identifier_index");
        }
        if request.node_label.is_some() {
            access_paths.push("label_index");
        }
        if request
            .property_filters
            .iter()
            .any(|filter| !is_temporal_field(&filter.field))
        {
            access_paths.push("property_index");
        }
        if request
            .property_filters
            .iter()
            .any(|filter| is_temporal_field(&filter.field))
        {
            access_paths.push("temporal_index");
        }
        if request.adjacency.is_some() {
            access_paths.push("persistent_adjacency");
        }
        if request.access_context.is_some() {
            // Candidate IDs are authorized from compact catalog and adjacency
            // metadata before the payload pager is invoked.
            access_paths.push("access_policy_index");
        }
        let (mut node_ids, mut authorization_denials) = if request.include_nodes {
            self.select_indexed_node_ids(&request, policy.as_ref())?
        } else {
            (Vec::new(), 0)
        };
        let (relationship_ids, relationship_denials) = if let Some(adjacency) = &request.adjacency {
            self.select_adjacency_relationship_ids(&mut node_ids, adjacency, policy.as_ref())?
        } else if request.include_relationships {
            self.select_relationship_ids(request.relationship_type.as_deref(), policy.as_ref())?
        } else {
            (Vec::new(), 0)
        };
        authorization_denials = authorization_denials.saturating_add(relationship_denials);
        self.enforce_relationship_budget(relationship_ids.len())?;
        self.enforce_adjacency_budget(relationship_ids.len().saturating_mul(2))?;

        let pager = self.pager()?;
        let mut page_ins = 0_u64;
        let mut cache_hits = 0_u64;
        let mut relationships = Vec::with_capacity(relationship_ids.len());
        for relationship_id in &relationship_ids {
            let relationship =
                if let Some(relationship) = self.resident_relationships.get(relationship_id) {
                    cache_hits = cache_hits.saturating_add(1);
                    relationship.clone()
                } else {
                    page_ins = page_ins.saturating_add(1);
                    pager
                        .load_relationship_payload(relationship_id)
                        .map_err(pager_error)?
                        .relationship
                };
            node_ids.push(relationship.source().clone());
            node_ids.push(relationship.target().clone());
            relationships.push(relationship);
        }
        node_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        node_ids.dedup();
        self.enforce_node_budget(node_ids.len())?;

        let mut nodes = Vec::with_capacity(node_ids.len());
        for node_id in &node_ids {
            if let Some(node) = self.resident_nodes.get(node_id) {
                cache_hits = cache_hits.saturating_add(1);
                nodes.push(node.clone());
            } else {
                page_ins = page_ins.saturating_add(1);
                nodes.push(pager.load_node_payload(node_id).map_err(pager_error)?.node);
            }
        }

        self.record_projection_stats(&nodes, &relationships, page_ins, cache_hits);
        self.last_projection_stats = CanonicalProjectionStats {
            access_paths,
            page_ins,
            cache_hits,
            authorization_denials,
            policy_cache_invalidated,
            timing_class: if !request.identifiers.is_empty() && nodes.is_empty() {
                "visibility_miss"
            } else {
                "selected"
            },
        };
        let mut graph = Graph::from_current_records(nodes, relationships, self.sequence_floor())
            .map_err(graph_error)?;
        graph.replace_evidence_store(self.evidence.clone());
        Ok(graph)
    }

    /// Persist only current record versions changed by an engine transition,
    /// preceded by durable WAL intent and covered by periodic checkpoints.
    pub fn commit_transition(
        &mut self,
        previous: &Graph,
        current: &Graph,
        transaction_id: DurableTransactionId,
        crash_stage: Option<MutationCrashStage>,
    ) -> GraphStorageResult<AtomicPersistentMutationOutcome> {
        self.commit_transition_with_audit(
            previous,
            current,
            transaction_id,
            vec!["canonical engine graph transition committed".to_owned()],
            crash_stage,
        )
    }

    /// Persist a graph transition and caller-supplied payload-free receipts in
    /// one WAL transaction. Receipt replay visibility will match the canonical
    /// and projection visibility selected by recovery.
    pub fn commit_transition_with_audit(
        &mut self,
        previous: &Graph,
        current: &Graph,
        transaction_id: DurableTransactionId,
        audit_events: Vec<String>,
        crash_stage: Option<MutationCrashStage>,
    ) -> GraphStorageResult<AtomicPersistentMutationOutcome> {
        let batch = self.build_mutation_batch(previous, current, transaction_id)?;
        let evidence_changed = previous.evidence_store() != current.evidence_store();
        if batch.node_records.is_empty()
            && batch.relationship_records.is_empty()
            && batch.outgoing_adjacency.is_empty()
            && batch.incoming_adjacency.is_empty()
        {
            if evidence_changed {
                stage_evidence_sidecar(&self.root, current.evidence_store())?;
                promote_evidence_sidecar(&self.root)?;
                self.evidence = current.evidence_store().clone();
            }
            return Ok(AtomicPersistentMutationOutcome {
                applied: evidence_changed,
                mutation_sequence_number: None,
            });
        }
        let full_text_index = self.full_text_index()?;
        full_text_index.invalidate().map_err(full_text_error)?;
        if evidence_changed {
            stage_evidence_sidecar(&self.root, current.evidence_store())?;
        }
        let mut batch = batch;
        batch.audit_events = audit_events;
        let outcome =
            apply_atomic_persistent_mutation_batch(&self.root, &mut self.state, batch, crash_stage);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                discard_staged_evidence_sidecar(&self.root);
                return Err(error);
            }
        };
        if evidence_changed {
            promote_evidence_sidecar(&self.root)?;
            self.evidence = current.evidence_store().clone();
        }
        // A mutation may replace any payload in the operational projection.
        // Drop the request cache so the next read observes cataloged versions.
        self.resident_nodes.clear();
        self.resident_relationships.clear();
        self.resident_policy_fingerprint = None;
        self.stats.resident_hot_nodes = 0;
        self.stats.resident_hot_relationships = 0;
        self.stats.resident_warm_adjacency_entries = 0;
        self.refresh_index_stats();
        // Full-text search is a rebuildable projection. The durable invalidation
        // marker above prevents stale reads; `search_full_text` rebuilds the
        // complete canonical generation before serving the next query.
        Ok(outcome)
    }

    /// Search the complete canonical graph through the durable access-aware
    /// inverted index, rebuilding a missing or corrupt generation on demand.
    pub fn search_full_text(
        &mut self,
        query: &FullTextQuery,
        access: &AccessContext,
    ) -> GraphStorageResult<FullTextSearchPage> {
        let index = self.full_text_index()?;
        if index.inspect().readiness != FullTextSearchReadiness::Ready {
            self.rebuild_full_text_index()?;
        }
        index.search(query, access).map_err(full_text_error)
    }

    /// Reconstruct and atomically publish the full-text index entirely from
    /// canonical current node and relationship versions.
    pub fn rebuild_full_text_index(&self) -> GraphStorageResult<FullTextRebuildOutcome> {
        let documents = self.canonical_full_text_documents()?;
        self.full_text_index()?
            .rebuild_with_checkpoint(&documents, 5_000, None)
            .map_err(full_text_error)
    }

    /// Inspect whether the access-aware full-text projection is current without
    /// triggering its lazy rebuild. Reconciliation uses this read-only signal to
    /// report stale derived state before deciding whether repair is authorized.
    pub fn full_text_readiness(&self) -> GraphStorageResult<FullTextSearchReadiness> {
        Ok(self.full_text_index()?.inspect().readiness)
    }

    /// Report whether the derived full-text projection is ready for queries.
    pub fn full_text_projection_is_ready(&self) -> GraphStorageResult<bool> {
        Ok(self.full_text_readiness()? == FullTextSearchReadiness::Ready)
    }

    /// Persist one completed extraction artifact and atomically publish the
    /// resulting file-content search generation.
    pub fn publish_file_content(
        &mut self,
        artifact: ExtractionArtifact,
        now_ms: u64,
    ) -> GraphStorageResult<FullTextRebuildOutcome> {
        let mut metadata = self.file_content_store()?;
        metadata
            .publish_artifact(artifact, now_ms)
            .map_err(file_content_error)?;
        self.file_content_index()?
            .rebuild_from_store(&metadata)
            .map_err(file_content_error)
    }

    /// Enqueue one canonical file version for processing by the isolated
    /// extraction worker. Replays of the same file, hash and version are
    /// idempotent.
    pub fn enqueue_file_extraction(
        &self,
        descriptor: FileDescriptor,
        now_ms: u64,
    ) -> GraphStorageResult<EnqueueOutcome> {
        self.file_content_store()?
            .enqueue(descriptor, now_ms)
            .map_err(file_content_error)
    }

    /// Read bounded queue, retry, quarantine, byte and lag measurements.
    pub fn file_extraction_metrics(&self, now_ms: u64) -> GraphStorageResult<FileJobMetrics> {
        Ok(self.file_content_store()?.metrics(now_ms))
    }

    /// Delete one file and synchronously publish visibility without its stale
    /// extracted chunks.
    pub fn delete_file_content(
        &mut self,
        file_id: &str,
    ) -> GraphStorageResult<FullTextRebuildOutcome> {
        let mut metadata = self.file_content_store()?;
        metadata.delete_file(file_id).map_err(file_content_error)?;
        self.file_content_index()?
            .rebuild_from_store(&metadata)
            .map_err(file_content_error)
    }

    /// Search only authorized extracted file content. Rebuild is generation
    /// aware, so newly published metadata cannot remain hidden behind an older
    /// otherwise healthy derived index.
    pub fn search_file_content(
        &mut self,
        query: &FileContentQuery,
        access: &AccessContext,
    ) -> GraphStorageResult<FullTextSearchPage> {
        let metadata = self.file_content_store()?;
        let index = self.file_content_index()?;
        index
            .rebuild_from_store(&metadata)
            .map_err(file_content_error)?;
        index.search(query, access).map_err(file_content_error)
    }

    /// Rebuild file-content search entirely from Corrobore-owned extraction
    /// metadata; object storage remains authoritative for re-extraction.
    pub fn rebuild_file_content_index(&mut self) -> GraphStorageResult<FullTextRebuildOutcome> {
        let metadata = self.file_content_store()?;
        self.file_content_index()?
            .rebuild_from_store(&metadata)
            .map_err(file_content_error)
    }

    /// Reconstruct every compact canonical lookup from current canonical
    /// payloads and atomically publish the resulting catalog metadata.
    pub fn rebuild_compact_indexes(&mut self) -> GraphStorageResult<u64> {
        let graph = self.load_projection(CanonicalProjectionRequest::all())?;
        let mut nodes = graph.current_node_records().map_err(graph_error)?;
        let mut relationships = graph.current_relationship_records().map_err(graph_error)?;
        nodes.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        relationships.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));

        // Adjacency is derived solely from current canonical relationships. A
        // content-addressed transaction makes retry idempotent while allowing a
        // later canonical generation to produce a fresh rebuild transaction.
        if !nodes.is_empty() {
            let mut digest = Sha256::new();
            for node in &nodes {
                digest.update(node.id().as_str().as_bytes());
                digest.update(node.version_id().as_str().as_bytes());
            }
            for relationship in &relationships {
                digest.update(relationship.id().as_str().as_bytes());
                digest.update(relationship.version_id().as_str().as_bytes());
            }
            let transaction_id = DurableTransactionId::new(format!(
                "tx--derived-adjacency-rebuild-{:x}",
                digest.finalize()
            ))
            .map_err(|error| GraphStorageError::OperationFailed {
                operation: "rebuild_compact_indexes",
                message: error.to_string(),
            })?;
            let adjacency_records = |direction| {
                nodes
                    .iter()
                    .map(|node| AtomicPersistentMutationAdjacencyRecord {
                        owner_node_id: node.id().clone(),
                        direction,
                        entries: relationships
                            .iter()
                            .filter(|relationship| {
                                relationship.status() != RecordStatus::Tombstoned
                                    && match direction {
                                        AdjacencyDirection::Outgoing => {
                                            relationship.source() == node.id()
                                        }
                                        AdjacencyDirection::Incoming => {
                                            relationship.target() == node.id()
                                        }
                                    }
                            })
                            .map(|relationship| adjacency_entry(relationship, direction))
                            .collect(),
                    })
                    .collect()
            };
            let batch = AtomicPersistentMutationBatch {
                transaction_id,
                node_records: Vec::new(),
                relationship_records: Vec::new(),
                outgoing_adjacency: adjacency_records(AdjacencyDirection::Outgoing),
                incoming_adjacency: adjacency_records(AdjacencyDirection::Incoming),
                audit_events: vec!["derived adjacency rebuilt from canonical graph".to_owned()],
            };
            apply_atomic_persistent_mutation_batch(&self.root, &mut self.state, batch, None)?;
        }
        let mut rebuilt = self.state.catalog.clone();
        rebuilt.metadata_indexes = GraphCatalogIndexes::default();

        for node in &nodes {
            let latest = rebuilt
                .latest_node_records
                .get(node.id())
                .cloned()
                .ok_or_else(|| GraphStorageError::OperationFailed {
                    operation: "rebuild_compact_indexes",
                    message: format!(
                        "canonical node {:?} is absent from latest catalog",
                        node.id()
                    ),
                })?;
            let labels = node.labels().to_vec();
            replace_node_read_indexes(
                &mut rebuilt,
                &labels,
                &node_read_index_document(node)?,
                LabelIndexNodeMetadata {
                    node_id: node.id().clone(),
                    latest_storage_ref: Some(latest.storage_ref.clone()),
                    graph_record_version: latest.graph_record_version.clone(),
                },
            )?;
        }
        for relationship in &relationships {
            let latest = rebuilt
                .latest_relationship_records
                .get(relationship.id())
                .cloned()
                .ok_or_else(|| GraphStorageError::OperationFailed {
                    operation: "rebuild_compact_indexes",
                    message: format!(
                        "canonical relationship {:?} is absent from latest catalog",
                        relationship.id()
                    ),
                })?;
            index_relationship_type(
                &mut rebuilt,
                relationship.rel_type(),
                RelationshipTypeIndexRelationshipMetadata {
                    relationship_id: relationship.id().clone(),
                    latest_storage_ref: Some(latest.storage_ref.clone()),
                    graph_record_version: latest.graph_record_version.clone(),
                },
            )?;
            replace_relationship_access_index(
                &mut rebuilt,
                relationship.id(),
                relationship.status() != RecordStatus::Tombstoned,
                &access_document(relationship.property("opencti.access"))?,
            );
        }
        persist_graph_catalog_metadata(&self.root, &rebuilt)?;
        self.state.catalog = rebuilt;
        self.refresh_index_stats();
        Ok((nodes.len() + relationships.len()) as u64)
    }

    fn file_content_store(&self) -> GraphStorageResult<FileJobStore> {
        FileJobStore::open(
            self.root.path().join("file-content").join("metadata"),
            FILE_CONTENT_MAX_ATTEMPTS,
            FILE_CONTENT_LEASE_MS,
        )
        .map_err(file_content_error)
    }

    fn file_content_index(&mut self) -> GraphStorageResult<&FileContentIndex> {
        if self.file_content_index.is_none() {
            let root = self.root.path().join("search").join("file-content-v1");
            self.file_content_index = Some(
                FileContentIndex::open(
                    root,
                    FileContentIndexSettings {
                        schema_version: FILE_CONTENT_SCHEMA_VERSION.to_owned(),
                        cursor_key: self.file_content_cursor_key()?,
                        writer_memory_bytes: FILE_CONTENT_WRITER_MEMORY_BYTES,
                        max_candidates: FILE_CONTENT_MAX_CANDIDATES,
                        snippet_chars: FILE_CONTENT_SNIPPET_CHARS,
                    },
                )
                .map_err(file_content_error)?,
            );
        }
        self.file_content_index
            .as_ref()
            .ok_or_else(|| GraphStorageError::OperationFailed {
                operation: "initialize_file_content_index",
                message: "index initialization did not retain its handle".to_owned(),
            })
    }

    fn file_content_cursor_key(&self) -> GraphStorageResult<Vec<u8>> {
        let search_root = self.root.path().join("search").join("file-content-v1");
        fs::create_dir_all(&search_root)
            .map_err(|error| io_error("create_file_content_index_root", &search_root, error))?;
        let key_path = search_root.join(FULL_TEXT_CURSOR_KEY);
        match fs::read(&key_path) {
            Ok(key) if key.len() >= 32 => return Ok(key),
            Ok(_) => {
                return Err(GraphStorageError::OperationFailed {
                    operation: "read_file_content_cursor_key",
                    message: "persistent file-content cursor key is shorter than 32 bytes"
                        .to_owned(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io_error("read_file_content_cursor_key", &key_path, error));
            }
        }
        let mut key = vec![0_u8; 32];
        getrandom::fill(&mut key).map_err(|error| GraphStorageError::OperationFailed {
            operation: "generate_file_content_cursor_key",
            message: error.to_string(),
        })?;
        let temporary = search_root.join(format!("{FULL_TEXT_CURSOR_KEY}.next"));
        fs::write(&temporary, &key)
            .map_err(|error| io_error("write_file_content_cursor_key", &temporary, error))?;
        set_private_file_permissions(&temporary)?;
        fs::rename(&temporary, &key_path)
            .map_err(|error| io_error("publish_file_content_cursor_key", &key_path, error))?;
        Ok(key)
    }

    fn full_text_index(&self) -> GraphStorageResult<FullTextIndex> {
        let search_root = self.root.path().join("search").join("full-text-v1");
        FullTextIndex::open(
            search_root,
            FullTextIndexSettings {
                schema_version: FULL_TEXT_SCHEMA_VERSION.to_owned(),
                cursor_key: self.full_text_cursor_key()?,
                writer_memory_bytes: FULL_TEXT_WRITER_MEMORY_BYTES,
                max_candidates: FULL_TEXT_MAX_CANDIDATES,
            },
        )
        .map_err(full_text_error)
    }

    fn full_text_cursor_key(&self) -> GraphStorageResult<Vec<u8>> {
        let search_root = self.root.path().join("search").join("full-text-v1");
        fs::create_dir_all(&search_root)
            .map_err(|error| io_error("create_full_text_index_root", &search_root, error))?;
        let key_path = search_root.join(FULL_TEXT_CURSOR_KEY);
        match fs::read(&key_path) {
            Ok(key) if key.len() >= 32 => return Ok(key),
            Ok(_) => {
                return Err(GraphStorageError::OperationFailed {
                    operation: "read_full_text_cursor_key",
                    message: "persistent full-text cursor key is shorter than 32 bytes".to_owned(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io_error("read_full_text_cursor_key", &key_path, error));
            }
        }

        let mut key = vec![0_u8; 32];
        getrandom::fill(&mut key).map_err(|error| GraphStorageError::OperationFailed {
            operation: "generate_full_text_cursor_key",
            message: error.to_string(),
        })?;
        let temporary = search_root.join(format!("{FULL_TEXT_CURSOR_KEY}.next"));
        fs::write(&temporary, &key)
            .map_err(|error| io_error("write_full_text_cursor_key", &temporary, error))?;
        set_private_file_permissions(&temporary)?;
        fs::rename(&temporary, &key_path)
            .map_err(|error| io_error("publish_full_text_cursor_key", &key_path, error))?;
        Ok(key)
    }

    fn canonical_full_text_documents(&self) -> GraphStorageResult<Vec<FullTextDocument>> {
        let pager = self.pager()?;
        let mut documents = Vec::with_capacity(
            self.state
                .catalog
                .latest_node_records
                .len()
                .saturating_add(self.state.catalog.latest_relationship_records.len()),
        );
        let mut node_ids = self
            .state
            .catalog
            .latest_node_records
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        node_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for node_id in node_ids {
            let node = pager.load_node_payload(&node_id).map_err(pager_error)?.node;
            if node.status() != RecordStatus::Tombstoned {
                documents.push(document_from_node(&node).map_err(full_text_error)?);
            }
        }
        let mut relationship_ids = self
            .state
            .catalog
            .latest_relationship_records
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        relationship_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for relationship_id in relationship_ids {
            let relationship = pager
                .load_relationship_payload(&relationship_id)
                .map_err(pager_error)?
                .relationship;
            if relationship.status() != RecordStatus::Tombstoned {
                documents.push(document_from_relationship(&relationship).map_err(full_text_error)?);
            }
        }
        Ok(documents)
    }

    fn select_indexed_node_ids(
        &self,
        request: &CanonicalProjectionRequest,
        policy: Option<&OpenCtiAccessPolicy>,
    ) -> GraphStorageResult<(Vec<NodeId>, u64)> {
        let mut selected: Option<HashSet<NodeId>> = (!request.identifiers.is_empty()).then(|| {
            request
                .identifiers
                .iter()
                .flat_map(|identifier| {
                    resolve_identifier_index_entries(&self.state.catalog, identifier)
                })
                .map(|entry| entry.node_id)
                .collect()
        });
        if let Some(label) = request.node_label.as_deref() {
            let label_ids = resolve_node_ids_by_label(
                &self.state.catalog,
                label,
                crate::CatalogIndexLookupMode::EmptyWhenUnknown,
            )?
            .into_iter()
            .collect::<HashSet<_>>();
            intersect_selection(&mut selected, label_ids);
        }
        for filter in &request.property_filters {
            let temporal = is_temporal_field(&filter.field);
            let matches = self.property_filter_node_ids(filter, temporal)?;
            intersect_selection(&mut selected, matches);
        }
        let mut ids: Vec<NodeId> = selected.map_or_else(
            || {
                self.state
                    .catalog
                    .latest_node_records
                    .keys()
                    .cloned()
                    .collect()
            },
            |ids| ids.into_iter().collect(),
        );
        ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        ids.dedup();
        let before = ids.len();
        if let Some(policy) = policy {
            ids.retain(|node_id| self.node_is_authorized(node_id, policy));
        }
        let authorization_denials = before.saturating_sub(ids.len()) as u64;
        Ok((ids, authorization_denials))
    }

    fn property_filter_node_ids(
        &self,
        filter: &CanonicalPropertyFilter,
        temporal: bool,
    ) -> GraphStorageResult<HashSet<NodeId>> {
        let present = || {
            resolve_property_presence_entries(&self.state.catalog, &filter.field, temporal)
                .into_iter()
                .map(|entry| entry.node_id)
                .collect::<HashSet<_>>()
        };
        match filter.operator {
            CanonicalPropertyOperator::Exists => Ok(present()),
            CanonicalPropertyOperator::NotExists => {
                let all = self
                    .state
                    .catalog
                    .latest_node_records
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>();
                Ok(all.difference(&present()).cloned().collect())
            }
            CanonicalPropertyOperator::Equal | CanonicalPropertyOperator::NotEqual => {
                let encoded = encode_filter_value(filter)?;
                let equal = resolve_property_index_entries(
                    &self.state.catalog,
                    &filter.field,
                    &encoded,
                    temporal,
                )
                .into_iter()
                .map(|entry| entry.node_id)
                .collect::<HashSet<_>>();
                if filter.operator == CanonicalPropertyOperator::Equal {
                    Ok(equal)
                } else {
                    Ok(present().difference(&equal).cloned().collect())
                }
            }
            CanonicalPropertyOperator::In | CanonicalPropertyOperator::NotIn => {
                let values = filter
                    .value
                    .as_ref()
                    .and_then(Value::as_array)
                    .ok_or_else(|| GraphStorageError::OperationFailed {
                        operation: "load_canonical_graph_projection",
                        message: format!("filter {} requires a value array", filter.field),
                    })?;
                let mut included = HashSet::new();
                for value in values {
                    let encoded = serde_json::to_string(value).map_err(|error| {
                        GraphStorageError::OperationFailed {
                            operation: "load_canonical_graph_projection",
                            message: format!("failed to encode filter value: {error}"),
                        }
                    })?;
                    included.extend(
                        resolve_property_index_entries(
                            &self.state.catalog,
                            &filter.field,
                            &encoded,
                            temporal,
                        )
                        .into_iter()
                        .map(|entry| entry.node_id),
                    );
                }
                if filter.operator == CanonicalPropertyOperator::In {
                    Ok(included)
                } else {
                    Ok(present().difference(&included).cloned().collect())
                }
            }
            operator => {
                let expected =
                    filter
                        .value
                        .as_ref()
                        .ok_or_else(|| GraphStorageError::OperationFailed {
                            operation: "load_canonical_graph_projection",
                            message: format!("filter {} requires a comparison value", filter.field),
                        })?;
                let index = if temporal {
                    &self.state.catalog.metadata_indexes.temporal
                } else {
                    &self.state.catalog.metadata_indexes.properties
                };
                let mut matches = HashSet::new();
                for (encoded, entries) in index.get(&filter.field).into_iter().flatten() {
                    let actual: Value = serde_json::from_str(encoded).map_err(|error| {
                        GraphStorageError::OperationFailed {
                            operation: "load_canonical_graph_projection",
                            message: format!("invalid compact index value: {error}"),
                        }
                    })?;
                    if indexed_value_matches_range(&actual, expected, operator) {
                        matches.extend(entries.iter().map(|entry| entry.node_id.clone()));
                    }
                }
                Ok(matches)
            }
        }
    }

    fn select_adjacency_relationship_ids(
        &self,
        node_ids: &mut Vec<NodeId>,
        request: &CanonicalAdjacencyProjection,
        policy: Option<&OpenCtiAccessPolicy>,
    ) -> GraphStorageResult<(Vec<RelationshipId>, u64)> {
        if request.max_depth == 0 || request.max_relationships == 0 {
            return Err(GraphStorageError::OperationFailed {
                operation: "load_canonical_graph_projection",
                message: "adjacency expansion must declare non-zero depth and relationship budget"
                    .to_owned(),
            });
        }
        let allowed_types = request
            .relationship_types
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut visited_nodes = node_ids.iter().cloned().collect::<HashSet<_>>();
        let mut frontier = node_ids.clone();
        let mut relationship_ids = HashSet::new();
        let mut authorization_denials = 0_u64;
        for _ in 0..request.max_depth {
            let mut next = Vec::new();
            for owner in frontier {
                let mut entries = Vec::new();
                if request.outgoing {
                    entries.extend(
                        read_outgoing_adjacency_by_node_id(
                            &self.state.adjacency_storage,
                            &self.state.catalog,
                            &owner,
                            crate::AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
                        )?
                        .entries,
                    );
                }
                if request.incoming {
                    entries.extend(
                        read_incoming_adjacency_by_node_id(
                            &self.state.adjacency_storage,
                            &self.state.catalog,
                            &owner,
                            crate::AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
                        )?
                        .entries,
                    );
                }
                entries.sort_by(|left, right| {
                    left.relationship_id
                        .as_str()
                        .cmp(right.relationship_id.as_str())
                });
                entries.dedup_by(|left, right| left.relationship_id == right.relationship_id);
                if let Some(policy) = policy {
                    entries.retain(|entry| {
                        let neighbor = if entry.source_node_id == owner {
                            &entry.target_node_id
                        } else {
                            &entry.source_node_id
                        };
                        let allowed = self
                            .relationship_is_authorized(&entry.relationship_id, policy)
                            && self.node_is_authorized(neighbor, policy);
                        if !allowed {
                            authorization_denials = authorization_denials.saturating_add(1);
                        }
                        allowed
                    });
                }
                if request.supernode_threshold > 0
                    && entries.len() as u32 > request.supernode_threshold
                    && allowed_types.is_empty()
                {
                    return Err(GraphStorageError::OperationFailed {
                        operation: "load_canonical_graph_projection",
                        message: format!(
                            "supernode expansion blocked at {} with degree {}",
                            owner.as_str(),
                            entries.len()
                        ),
                    });
                }
                for entry in entries {
                    if !allowed_types.is_empty()
                        && !allowed_types.contains(entry.relationship_type.as_str())
                    {
                        continue;
                    }
                    relationship_ids.insert(entry.relationship_id.clone());
                    if relationship_ids.len() as u32 > request.max_relationships {
                        return Err(GraphStorageError::OperationFailed {
                            operation: "load_canonical_graph_projection",
                            message: format!(
                                "adjacency query budget exceeded: more than {} relationships",
                                request.max_relationships
                            ),
                        });
                    }
                    let neighbor = if entry.source_node_id == owner {
                        entry.target_node_id
                    } else {
                        entry.source_node_id
                    };
                    if visited_nodes.insert(neighbor.clone()) {
                        next.push(neighbor.clone());
                        node_ids.push(neighbor);
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        let mut ids = relationship_ids.into_iter().collect::<Vec<_>>();
        ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok((ids, authorization_denials))
    }

    fn select_relationship_ids(
        &self,
        relationship_type: Option<&str>,
        policy: Option<&OpenCtiAccessPolicy>,
    ) -> GraphStorageResult<(Vec<RelationshipId>, u64)> {
        let mut ids = match relationship_type {
            Some(value) => {
                let relationship_type =
                    graph_core::RelationshipType::new(value).map_err(graph_error)?;
                resolve_relationship_ids_by_type(
                    &self.state.catalog,
                    &relationship_type,
                    crate::CatalogIndexLookupMode::EmptyWhenUnknown,
                )?
            }
            None => self
                .state
                .catalog
                .latest_relationship_records
                .keys()
                .cloned()
                .collect(),
        };
        ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        ids.dedup();
        let mut authorization_denials = 0_u64;
        if let Some(policy) = policy {
            let endpoints = self.relationship_endpoints();
            ids.retain(|relationship_id| {
                let allowed = self.relationship_is_authorized(relationship_id, policy)
                    && endpoints
                        .get(relationship_id)
                        .is_some_and(|(source, target)| {
                            self.node_is_authorized(source, policy)
                                && self.node_is_authorized(target, policy)
                        });
                if !allowed {
                    authorization_denials = authorization_denials.saturating_add(1);
                }
                allowed
            });
        }
        Ok((ids, authorization_denials))
    }

    fn relationship_endpoints(&self) -> HashMap<RelationshipId, (NodeId, NodeId)> {
        let mut endpoints = HashMap::new();
        for record in crate::snapshot_persisted_adjacency_records(&self.state.adjacency_storage) {
            for entry in record.entries {
                endpoints
                    .entry(entry.relationship_id)
                    .or_insert((entry.source_node_id, entry.target_node_id));
            }
        }
        endpoints
    }

    fn node_is_authorized(&self, node_id: &NodeId, policy: &OpenCtiAccessPolicy) -> bool {
        self.state
            .catalog
            .metadata_indexes
            .node_access
            .get(node_id)
            .map_or_else(
                || policy.is_system(),
                |metadata| policy.evaluate(metadata).allowed(),
            )
    }

    fn relationship_is_authorized(
        &self,
        relationship_id: &RelationshipId,
        policy: &OpenCtiAccessPolicy,
    ) -> bool {
        self.state
            .catalog
            .metadata_indexes
            .relationship_access
            .get(relationship_id)
            .map_or_else(
                || policy.is_system(),
                |metadata| policy.evaluate(metadata).allowed(),
            )
    }

    fn pager(&self) -> GraphStorageResult<FileBackedGraphPager> {
        create_file_backed_graph_pager(self.file_backed_store()?)
    }

    fn enforce_node_budget(&self, count: usize) -> GraphStorageResult<()> {
        if count as u64 <= self.options.max_hot_nodes {
            return Ok(());
        }
        Err(GraphStorageError::OperationFailed {
            operation: "load_canonical_graph_projection",
            message: format!(
                "node projection requires {count} hot records, budget is {}",
                self.options.max_hot_nodes
            ),
        })
    }

    fn enforce_relationship_budget(&self, count: usize) -> GraphStorageResult<()> {
        if count as u64 <= self.options.max_hot_relationships {
            return Ok(());
        }
        Err(GraphStorageError::OperationFailed {
            operation: "load_canonical_graph_projection",
            message: format!(
                "relationship projection requires {count} hot records, budget is {}",
                self.options.max_hot_relationships
            ),
        })
    }

    fn enforce_adjacency_budget(&self, count: usize) -> GraphStorageResult<()> {
        if count as u64 <= self.options.max_warm_adjacency_entries {
            return Ok(());
        }
        Err(GraphStorageError::OperationFailed {
            operation: "load_canonical_graph_projection",
            message: format!(
                "relationship projection requires {count} warm adjacency entries, budget is {}",
                self.options.max_warm_adjacency_entries
            ),
        })
    }

    fn record_projection_stats(
        &mut self,
        nodes: &[Node],
        relationships: &[Relationship],
        page_ins: u64,
        cache_hits: u64,
    ) {
        self.stats.page_ins = self.stats.page_ins.saturating_add(page_ins);
        self.stats.cache_hits = self.stats.cache_hits.saturating_add(cache_hits);
        self.resident_nodes = nodes
            .iter()
            .map(|node| (node.id().clone(), node.clone()))
            .collect();
        self.resident_relationships = relationships
            .iter()
            .map(|relationship| (relationship.id().clone(), relationship.clone()))
            .collect();
        self.stats.resident_hot_nodes = nodes.len() as u64;
        self.stats.resident_hot_relationships = relationships.len() as u64;
        self.stats.resident_warm_adjacency_entries = (relationships.len() as u64).saturating_mul(2);
        self.refresh_index_stats();
    }

    fn refresh_index_stats(&mut self) {
        self.stats.node_index_entries = self.state.catalog.latest_node_records.len() as u64;
        self.stats.relationship_index_entries =
            self.state.catalog.latest_relationship_records.len() as u64;
        self.stats.label_index_entries = self.state.catalog.metadata_indexes.labels.len() as u64;
        self.stats.relationship_type_index_entries =
            self.state.catalog.metadata_indexes.relationship_types.len() as u64;
        self.stats.identifier_index_entries = self
            .state
            .catalog
            .metadata_indexes
            .identifiers
            .values()
            .map(|entries| entries.len() as u64)
            .sum();
        self.stats.property_index_entries = self
            .state
            .catalog
            .metadata_indexes
            .properties
            .values()
            .map(|values| {
                values
                    .values()
                    .map(|entries| entries.len() as u64)
                    .sum::<u64>()
            })
            .sum();
        self.stats.temporal_index_entries = self
            .state
            .catalog
            .metadata_indexes
            .temporal
            .values()
            .map(|values| {
                values
                    .values()
                    .map(|entries| entries.len() as u64)
                    .sum::<u64>()
            })
            .sum();
        self.stats.node_access_index_entries =
            self.state.catalog.metadata_indexes.node_access.len() as u64;
        self.stats.relationship_access_index_entries = self
            .state
            .catalog
            .metadata_indexes
            .relationship_access
            .len() as u64;
        self.stats.resident_cold_nodes = self
            .stats
            .node_index_entries
            .saturating_sub(self.stats.resident_hot_nodes);
        self.stats.resident_cold_relationships = self
            .stats
            .relationship_index_entries
            .saturating_sub(self.stats.resident_hot_relationships);
    }

    fn build_mutation_batch(
        &self,
        previous: &Graph,
        current: &Graph,
        transaction_id: DurableTransactionId,
    ) -> GraphStorageResult<AtomicPersistentMutationBatch> {
        let previous_nodes: HashMap<NodeId, Node> = previous
            .current_node_records()
            .map_err(graph_error)?
            .into_iter()
            .map(|node| (node.id().clone(), node))
            .collect();
        let previous_relationships: HashMap<RelationshipId, Relationship> = previous
            .current_relationship_records()
            .map_err(graph_error)?
            .into_iter()
            .map(|relationship| (relationship.id().clone(), relationship))
            .collect();
        let current_nodes = current.current_node_records().map_err(graph_error)?;
        let current_relationships = current
            .current_relationship_records()
            .map_err(graph_error)?;

        let changed_nodes: Vec<Node> = current_nodes
            .iter()
            .filter(|node| {
                previous_nodes
                    .get(node.id())
                    .is_none_or(|previous| previous.version_id() != node.version_id())
            })
            .cloned()
            .collect();
        let changed_relationships: Vec<Relationship> = current_relationships
            .iter()
            .filter(|relationship| {
                previous_relationships
                    .get(relationship.id())
                    .is_none_or(|previous| previous.version_id() != relationship.version_id())
            })
            .cloned()
            .collect();

        let node_records = changed_nodes
            .iter()
            .map(encode_node_mutation)
            .collect::<GraphStorageResult<Vec<_>>>()?;
        let relationship_records = changed_relationships
            .iter()
            .map(encode_relationship_mutation)
            .collect::<GraphStorageResult<Vec<_>>>()?;

        // Adjacency changes only when a relationship changes. Including every
        // changed node here would replace canonical adjacency with the partial
        // request projection during a node-only update.
        let changed_ids: HashSet<RelationshipId> = changed_relationships
            .iter()
            .map(|relationship| relationship.id().clone())
            .collect();
        let mut affected_nodes: HashSet<NodeId> = HashSet::new();
        let mut outgoing_by_owner: HashMap<NodeId, Vec<PersistedAdjacencyEntry>> = HashMap::new();
        let mut incoming_by_owner: HashMap<NodeId, Vec<PersistedAdjacencyEntry>> = HashMap::new();
        for relationship in &changed_relationships {
            affected_nodes.insert(relationship.source().clone());
            affected_nodes.insert(relationship.target().clone());
            if relationship.status() != RecordStatus::Tombstoned {
                outgoing_by_owner
                    .entry(relationship.source().clone())
                    .or_default()
                    .push(adjacency_entry(relationship, AdjacencyDirection::Outgoing));
                incoming_by_owner
                    .entry(relationship.target().clone())
                    .or_default()
                    .push(adjacency_entry(relationship, AdjacencyDirection::Incoming));
            }
        }
        let mut sorted_affected: Vec<NodeId> = affected_nodes.into_iter().collect();
        sorted_affected.sort_by(|left, right| left.as_str().cmp(right.as_str()));

        let outgoing_adjacency = sorted_affected
            .iter()
            .map(|owner| {
                self.adjacency_update(
                    owner,
                    AdjacencyDirection::Outgoing,
                    &changed_ids,
                    outgoing_by_owner
                        .get(owner)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                )
            })
            .collect::<GraphStorageResult<Vec<_>>>()?;
        let incoming_adjacency = sorted_affected
            .iter()
            .map(|owner| {
                self.adjacency_update(
                    owner,
                    AdjacencyDirection::Incoming,
                    &changed_ids,
                    incoming_by_owner
                        .get(owner)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                )
            })
            .collect::<GraphStorageResult<Vec<_>>>()?;

        Ok(AtomicPersistentMutationBatch {
            transaction_id,
            node_records,
            relationship_records,
            outgoing_adjacency,
            incoming_adjacency,
            audit_events: vec!["canonical engine graph transition committed".to_owned()],
        })
    }

    fn adjacency_update(
        &self,
        owner: &NodeId,
        direction: AdjacencyDirection,
        changed_ids: &HashSet<RelationshipId>,
        replacement_entries: &[PersistedAdjacencyEntry],
    ) -> GraphStorageResult<AtomicPersistentMutationAdjacencyRecord> {
        let mut entries = if self.state.catalog.latest_node_records.contains_key(owner) {
            match direction {
                AdjacencyDirection::Outgoing => read_outgoing_adjacency_by_node_id(
                    &self.state.adjacency_storage,
                    &self.state.catalog,
                    owner,
                    crate::AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
                )?,
                AdjacencyDirection::Incoming => read_incoming_adjacency_by_node_id(
                    &self.state.adjacency_storage,
                    &self.state.catalog,
                    owner,
                    crate::AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
                )?,
            }
            .entries
        } else {
            Vec::new()
        };
        entries.retain(|entry| !changed_ids.contains(&entry.relationship_id));
        entries.extend(replacement_entries.iter().cloned());
        entries.sort_by(|left, right| {
            left.relationship_id
                .as_str()
                .cmp(right.relationship_id.as_str())
        });
        Ok(AtomicPersistentMutationAdjacencyRecord {
            owner_node_id: owner.clone(),
            direction,
            entries,
        })
    }

    fn sequence_floor(&self) -> GraphSequenceFloor {
        let mut floor = GraphSequenceFloor::default();
        for (id, entry) in &self.state.catalog.latest_node_records {
            floor.node = floor.node.max(sequence_suffix(id.as_str()));
            if let Some(crate::GraphRecordVersion::Node { version_id, .. }) =
                &entry.graph_record_version
            {
                floor.node_version = floor.node_version.max(sequence_suffix(version_id.as_str()));
            }
        }
        for (id, entry) in &self.state.catalog.latest_relationship_records {
            floor.relationship = floor.relationship.max(sequence_suffix(id.as_str()));
            if let Some(crate::GraphRecordVersion::Relationship { version_id, .. }) =
                &entry.graph_record_version
            {
                floor.relationship_version = floor
                    .relationship_version
                    .max(sequence_suffix(version_id.as_str()));
            }
        }
        floor
    }

    fn migrate_legacy_snapshot_if_needed(&mut self) -> GraphStorageResult<()> {
        let runtime = self.root.path().join("runtime");
        let current = runtime.join(LEGACY_SNAPSHOT);
        let rollback = runtime.join(LEGACY_ROLLBACK_SNAPSHOT);
        let migration_record = runtime.join(MIGRATION_RECORD);
        if migration_record.is_file() {
            return validate_migration_record(&migration_record, &self.state.catalog);
        }
        let source = if current.is_file() {
            Some(current.clone())
        } else if rollback.is_file() {
            Some(rollback.clone())
        } else {
            None
        };
        let Some(source) = source else {
            return Ok(());
        };

        let source_bytes = fs::metadata(&source)
            .map_err(|error| io_error("migrate_legacy_engine_snapshot", &source, error))?
            .len();
        let legacy = load_legacy_engine_graph_snapshot(&source)?;
        let legacy_nodes = legacy.current_node_records().map_err(graph_error)?;
        let legacy_relationships = legacy.current_relationship_records().map_err(graph_error)?;
        let legacy_node_versions = legacy.all_node_records();
        let legacy_relationship_versions = legacy.all_relationship_records();
        let node_count = legacy_nodes.len() as u64;
        let relationship_count = legacy_relationships.len() as u64;
        if self.state.catalog.latest_node_records.is_empty()
            && self.state.catalog.latest_relationship_records.is_empty()
            && (node_count > 0 || relationship_count > 0)
        {
            self.commit_legacy_snapshot(
                &legacy,
                DurableTransactionId::new("tx--legacy-engine-graph-migration").map_err(
                    |error| GraphStorageError::OperationFailed {
                        operation: "migrate_legacy_engine_snapshot",
                        message: error.to_string(),
                    },
                )?,
            )?;
        }
        if !legacy_catalog_matches(
            &self.root,
            &self.state,
            &legacy_nodes,
            &legacy_relationships,
            legacy_node_versions.len(),
            legacy_relationship_versions.len(),
        )? {
            return Err(GraphStorageError::OperationFailed {
                operation: "migrate_legacy_engine_snapshot",
                message: "canonical record identities, current versions, and payloads do not match legacy snapshot"
                    .to_owned(),
            });
        }

        if current.is_file() {
            fs::rename(&current, &rollback)
                .map_err(|error| io_error("migrate_legacy_engine_snapshot", &current, error))?;
            sync_directory(&runtime)?;
        }
        let record = LegacyMigrationRecord {
            schema_version: 1,
            source: LEGACY_SNAPSHOT.to_owned(),
            rollback_boundary: LEGACY_ROLLBACK_SNAPSHOT.to_owned(),
            source_bytes,
            node_count,
            relationship_count,
            node_version_count: legacy_node_versions.len() as u64,
            relationship_version_count: legacy_relationship_versions.len() as u64,
        };
        write_migration_record(&runtime, &migration_record, &record)?;
        self.startup_report.legacy_snapshot_migrated = true;
        Ok(())
    }

    fn commit_legacy_snapshot(
        &mut self,
        graph: &Graph,
        transaction_id: DurableTransactionId,
    ) -> GraphStorageResult<AtomicPersistentMutationOutcome> {
        let node_records = graph
            .all_node_records()
            .iter()
            .map(encode_node_mutation)
            .collect::<GraphStorageResult<Vec<_>>>()?;
        let relationship_records = graph
            .all_relationship_records()
            .iter()
            .map(encode_relationship_mutation)
            .collect::<GraphStorageResult<Vec<_>>>()?;
        let current_relationships = graph
            .current_relationship_records()
            .map_err(graph_error)?
            .into_iter()
            .filter(|relationship| relationship.status() != RecordStatus::Tombstoned)
            .collect::<Vec<_>>();
        let mut affected_nodes = HashSet::new();
        for relationship in &current_relationships {
            affected_nodes.insert(relationship.source().clone());
            affected_nodes.insert(relationship.target().clone());
        }
        let mut affected_nodes = affected_nodes.into_iter().collect::<Vec<_>>();
        affected_nodes.sort_by(|left: &NodeId, right| left.as_str().cmp(right.as_str()));
        let adjacency_records = |direction| {
            affected_nodes
                .iter()
                .map(|owner| AtomicPersistentMutationAdjacencyRecord {
                    owner_node_id: owner.clone(),
                    direction,
                    entries: current_relationships
                        .iter()
                        .filter(|relationship| match direction {
                            AdjacencyDirection::Outgoing => relationship.source() == owner,
                            AdjacencyDirection::Incoming => relationship.target() == owner,
                        })
                        .map(|relationship| adjacency_entry(relationship, direction))
                        .collect(),
                })
                .collect::<Vec<_>>()
        };
        let batch = AtomicPersistentMutationBatch {
            transaction_id,
            node_records,
            relationship_records,
            outgoing_adjacency: adjacency_records(AdjacencyDirection::Outgoing),
            incoming_adjacency: adjacency_records(AdjacencyDirection::Incoming),
            audit_events: vec!["legacy engine graph migrated to canonical storage".to_owned()],
        };
        let outcome =
            apply_atomic_persistent_mutation_batch(&self.root, &mut self.state, batch, None)?;
        self.refresh_index_stats();
        Ok(outcome)
    }
}

fn validate_migration_record(path: &Path, catalog: &GraphCatalog) -> GraphStorageResult<()> {
    let bytes = fs::read(path)
        .map_err(|error| io_error("validate_legacy_migration_record", path, error))?;
    let record: LegacyMigrationRecord =
        serde_json::from_slice(&bytes).map_err(|error| GraphStorageError::DecodeFailed {
            format: "engine-graph-migration-json-v1".to_owned(),
            reason: error.to_string(),
        })?;
    if record.schema_version != 1
        || record.source != LEGACY_SNAPSHOT
        || record.rollback_boundary != LEGACY_ROLLBACK_SNAPSHOT
        || record.node_count != catalog.latest_node_records.len() as u64
        || record.relationship_count != catalog.latest_relationship_records.len() as u64
        || record
            .node_version_count
            .saturating_sub(record.node_count)
            .saturating_add(
                record
                    .relationship_version_count
                    .saturating_sub(record.relationship_count),
            )
            != catalog.historical_records.len() as u64
    {
        return Err(GraphStorageError::OperationFailed {
            operation: "validate_legacy_migration_record",
            message: "legacy migration marker does not match canonical catalog metadata".to_owned(),
        });
    }
    Ok(())
}

fn write_migration_record(
    directory: &Path,
    target: &Path,
    record: &LegacyMigrationRecord,
) -> GraphStorageResult<()> {
    let temporary = directory.join(MIGRATION_RECORD_TEMP);
    if temporary.exists() {
        fs::remove_file(&temporary)
            .map_err(|error| io_error("migrate_legacy_engine_snapshot", &temporary, error))?;
    }
    let mut bytes =
        serde_json::to_vec_pretty(record).map_err(|error| GraphStorageError::OperationFailed {
            operation: "migrate_legacy_engine_snapshot",
            message: error.to_string(),
        })?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| io_error("migrate_legacy_engine_snapshot", &temporary, error))?;
    file.write_all(&bytes)
        .map_err(|error| io_error("migrate_legacy_engine_snapshot", &temporary, error))?;
    file.sync_all()
        .map_err(|error| io_error("migrate_legacy_engine_snapshot", &temporary, error))?;
    fs::rename(&temporary, target)
        .map_err(|error| io_error("migrate_legacy_engine_snapshot", target, error))?;
    sync_directory(directory)
}

fn sync_directory(path: &Path) -> GraphStorageResult<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error("migrate_legacy_engine_snapshot", path, error))
}

fn legacy_catalog_matches(
    root: &StorageRoot,
    state: &AtomicPersistentRuntimeState,
    nodes: &[Node],
    relationships: &[Relationship],
    node_version_count: usize,
    relationship_version_count: usize,
) -> GraphStorageResult<bool> {
    let catalog = &state.catalog;
    if catalog.latest_node_records.len() != nodes.len()
        || catalog.latest_relationship_records.len() != relationships.len()
        || catalog.historical_records.len()
            != node_version_count
                .saturating_sub(nodes.len())
                .saturating_add(relationship_version_count.saturating_sub(relationships.len()))
    {
        return Ok(false);
    }
    let pager = create_file_backed_graph_pager(create_file_backed_graph_store(
        root.clone(),
        catalog.clone(),
        state.adjacency_storage.clone(),
    )?)?;
    for node in nodes {
        let Some(entry) = catalog.latest_node_records.get(node.id()) else {
            return Ok(false);
        };
        if !matches!(
            entry.graph_record_version.as_ref(),
            Some(crate::GraphRecordVersion::Node { version_id, .. })
                if version_id == node.version_id()
        ) || pager
            .load_node_payload(node.id())
            .map_err(pager_error)?
            .node
            != *node
        {
            return Ok(false);
        }
    }
    for relationship in relationships {
        let Some(entry) = catalog.latest_relationship_records.get(relationship.id()) else {
            return Ok(false);
        };
        if !matches!(
            entry.graph_record_version.as_ref(),
            Some(crate::GraphRecordVersion::Relationship { version_id, .. })
                if version_id == relationship.version_id()
        ) || pager
            .load_relationship_payload(relationship.id())
            .map_err(pager_error)?
            .relationship
            != *relationship
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_canonical_append_logs(root: &StorageRoot) -> GraphStorageResult<()> {
    validate_payload_log::<Node>(
        root.path().join("nodes").join("node_records.log"),
        PersistedRecordKind::Node,
    )?;
    validate_payload_log::<Relationship>(
        root.path()
            .join("relationships")
            .join("relationship_records.log"),
        PersistedRecordKind::Relationship,
    )?;
    read_outgoing_adjacency_log_for_catalog_rebuild(root)?;
    read_incoming_adjacency_log_for_catalog_rebuild(root)?;
    Ok(())
}

fn validate_payload_log<T: DeserializeOwned>(
    path: PathBuf,
    expected_kind: PersistedRecordKind,
) -> GraphStorageResult<()> {
    let file = fs::File::open(&path)
        .map_err(|error| io_error("validate_canonical_append_log", &path, error))?;
    for line in BufReader::new(file).split(b'\n') {
        let line = line.map_err(|error| io_error("validate_canonical_append_log", &path, error))?;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if serde_json::from_slice::<T>(&line).is_ok() {
            continue;
        }
        let envelope: PersistedRecordEnvelope =
            serde_json::from_slice(&line).map_err(|error| GraphStorageError::DecodeFailed {
                format: "canonical-json-lines-v1".to_owned(),
                reason: format!("{}: {error}", path.display()),
            })?;
        if envelope.kind != expected_kind {
            return Err(GraphStorageError::DecodeFailed {
                format: "canonical-json-lines-v1".to_owned(),
                reason: format!(
                    "{} contains {:?}, expected {expected_kind:?}",
                    path.display(),
                    envelope.kind
                ),
            });
        }
    }
    Ok(())
}

fn load_legacy_engine_graph_snapshot(path: &Path) -> GraphStorageResult<Graph> {
    let bytes =
        fs::read(path).map_err(|error| io_error("migrate_legacy_engine_snapshot", path, error))?;
    let snapshot: LegacyEngineGraphSnapshot =
        serde_json::from_slice(&bytes).map_err(|error| GraphStorageError::DecodeFailed {
            format: "engine-graph-json-v1".to_owned(),
            reason: error.to_string(),
        })?;
    if snapshot.storage_version != StorageVersion::V1
        || snapshot.record_format != RecordFormat::JsonLinesV1
    {
        return Err(GraphStorageError::DecodeFailed {
            format: "engine-graph-json-v1".to_owned(),
            reason: format!(
                "unsupported engine snapshot compatibility: {:?}/{:?}",
                snapshot.storage_version, snapshot.record_format
            ),
        });
    }
    Graph::from_persistence_snapshot(snapshot.graph).map_err(|error| {
        GraphStorageError::DecodeFailed {
            format: "engine-graph-json-v1".to_owned(),
            reason: error.to_string(),
        }
    })
}

fn encode_node_mutation(node: &Node) -> GraphStorageResult<AtomicPersistentMutationNodeRecord> {
    let envelope = create_node_record_envelope(
        node,
        placeholder_ref(StorageSegment::NodeRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )?;
    Ok(AtomicPersistentMutationNodeRecord {
        encoded_record: encode_payload(PersistedRecordKind::Node, node)?,
        envelope,
        labels: node.labels().to_vec(),
        read_index: node_read_index_document(node)?,
    })
}

fn node_read_index_document(node: &Node) -> GraphStorageResult<NodeReadIndexDocument> {
    let active = node.status() != RecordStatus::Tombstoned;
    let mut identifiers = vec![node.id().as_str().to_owned()];
    for key in [
        "opencti.canonical_id",
        "opencti.identifiers",
        "opencti.field.id",
        "opencti.field.internal_id",
        "opencti.field.standard_id",
        "opencti.field.x_opencti_stix_ids",
        "opencti.field.aliases",
        "opencti.field.x_opencti_aliases",
        "opencti.field.x_opencti_deduplication_id",
        "opencti.field.external_references",
    ] {
        if let Some(value) = node.property(key) {
            collect_identifier_values(value, &mut identifiers);
        }
    }
    identifiers.sort();
    identifiers.dedup();

    let mut values = Vec::new();
    let mut properties = node.properties().iter().collect::<Vec<_>>();
    properties.sort_by(|left, right| left.0.cmp(right.0));
    for (field, value) in properties {
        if field == "opencti.raw" || field == "opencti.access" {
            continue;
        }
        let temporal = is_temporal_field(field);
        for scalar in property_scalars(value) {
            values.push(NodeReadIndexValue {
                field: field.clone(),
                encoded_value: serde_json::to_string(&scalar).map_err(|error| {
                    GraphStorageError::OperationFailed {
                        operation: "encode_node_read_index",
                        message: error.to_string(),
                    }
                })?,
                temporal,
            });
        }
    }
    values.sort_by(|left, right| {
        left.field
            .cmp(&right.field)
            .then_with(|| left.encoded_value.cmp(&right.encoded_value))
    });
    values.dedup();
    Ok(NodeReadIndexDocument {
        active,
        identifiers,
        values,
        access: access_document(node.property("opencti.access"))?,
    })
}

fn collect_identifier_values(value: &PropertyValue, identifiers: &mut Vec<String>) {
    match value {
        PropertyValue::String(value) => identifiers.push(value.clone()),
        PropertyValue::StringList(values) => identifiers.extend(values.iter().cloned()),
        PropertyValue::Json(value) => collect_identifier_json(value, identifiers),
        _ => {}
    }
}

fn collect_identifier_json(value: &Value, identifiers: &mut Vec<String>) {
    match value {
        Value::String(value) => identifiers.push(value.clone()),
        Value::Array(values) => {
            for value in values {
                collect_identifier_json(value, identifiers);
            }
        }
        Value::Object(values) => {
            for key in ["value", "id", "external_id"] {
                if let Some(value) = values.get(key) {
                    collect_identifier_json(value, identifiers);
                }
            }
        }
        _ => {}
    }
}

fn property_scalars(value: &PropertyValue) -> Vec<Value> {
    match value {
        PropertyValue::Null => vec![Value::Null],
        PropertyValue::Bool(value) => vec![Value::Bool(*value)],
        PropertyValue::Integer(value) => vec![Value::from(*value)],
        PropertyValue::Float(value) => vec![Value::from(*value)],
        PropertyValue::String(value) => vec![Value::String(value.clone())],
        PropertyValue::StringList(values) => values.iter().cloned().map(Value::String).collect(),
        PropertyValue::IntegerList(values) => values.iter().copied().map(Value::from).collect(),
        PropertyValue::FloatList(values) => values.iter().copied().map(Value::from).collect(),
        PropertyValue::BoolList(values) => values.iter().copied().map(Value::Bool).collect(),
        PropertyValue::Json(Value::Array(values)) => values
            .iter()
            .filter(|value| !value.is_array() && !value.is_object())
            .cloned()
            .collect(),
        PropertyValue::Json(value) if !value.is_array() && !value.is_object() => {
            vec![value.clone()]
        }
        PropertyValue::Json(_) => Vec::new(),
    }
}

fn is_temporal_field(field: &str) -> bool {
    [
        "created",
        "modified",
        "created_at",
        "updated_at",
        "refreshed_at",
        "valid_from",
        "valid_until",
        "first_seen",
        "last_seen",
    ]
    .iter()
    .any(|suffix| field == *suffix || field.ends_with(&format!(".{suffix}")))
}

fn intersect_selection(selected: &mut Option<HashSet<NodeId>>, matches: HashSet<NodeId>) {
    match selected {
        Some(current) => current.retain(|node_id| matches.contains(node_id)),
        None => *selected = Some(matches),
    }
}

fn encode_filter_value(filter: &CanonicalPropertyFilter) -> GraphStorageResult<String> {
    let value = filter
        .value
        .as_ref()
        .ok_or_else(|| GraphStorageError::OperationFailed {
            operation: "load_canonical_graph_projection",
            message: format!("filter {} requires a comparison value", filter.field),
        })?;
    serde_json::to_string(value).map_err(|error| GraphStorageError::OperationFailed {
        operation: "load_canonical_graph_projection",
        message: error.to_string(),
    })
}

fn indexed_value_matches_range(
    actual: &Value,
    expected: &Value,
    operator: CanonicalPropertyOperator,
) -> bool {
    let ordering = match (actual, expected) {
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .zip(right.as_f64())
            .and_then(|(left, right)| left.partial_cmp(&right)),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        _ => None,
    };
    ordering.is_some_and(|ordering| match operator {
        CanonicalPropertyOperator::GreaterThan => ordering.is_gt(),
        CanonicalPropertyOperator::GreaterThanOrEqual => ordering.is_ge(),
        CanonicalPropertyOperator::LessThan => ordering.is_lt(),
        CanonicalPropertyOperator::LessThanOrEqual => ordering.is_le(),
        CanonicalPropertyOperator::Equal => ordering.is_eq(),
        CanonicalPropertyOperator::NotEqual => !ordering.is_eq(),
        CanonicalPropertyOperator::Exists => true,
        CanonicalPropertyOperator::NotExists => false,
        CanonicalPropertyOperator::In | CanonicalPropertyOperator::NotIn => false,
    })
}

fn encode_relationship_mutation(
    relationship: &Relationship,
) -> GraphStorageResult<AtomicPersistentMutationRelationshipRecord> {
    let envelope = create_relationship_record_envelope(
        relationship,
        placeholder_ref(StorageSegment::RelationshipRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )?;
    Ok(AtomicPersistentMutationRelationshipRecord {
        encoded_record: encode_payload(PersistedRecordKind::Relationship, relationship)?,
        envelope,
        relationship_type: relationship.rel_type().clone(),
        active: relationship.status() != RecordStatus::Tombstoned,
        access: access_document(relationship.property("opencti.access"))?,
    })
}

fn access_document(value: Option<&PropertyValue>) -> GraphStorageResult<AccessMetadata> {
    match value {
        Some(PropertyValue::Json(value)) => {
            AccessMetadata::from_value(value).map_err(|error| GraphStorageError::OperationFailed {
                operation: "encode_opencti_access_index",
                message: error.to_string(),
            })
        }
        None => Ok(AccessMetadata::default()),
        Some(_) => Err(GraphStorageError::OperationFailed {
            operation: "encode_opencti_access_index",
            message: "opencti.access must be canonical JSON".to_owned(),
        }),
    }
}

fn encode_payload(
    kind: PersistedRecordKind,
    value: &impl Serialize,
) -> GraphStorageResult<EncodedRecord> {
    let mut bytes =
        serde_json::to_vec(value).map_err(|error| GraphStorageError::OperationFailed {
            operation: "encode_canonical_graph_payload",
            message: error.to_string(),
        })?;
    bytes.push(b'\n');
    let checksum = JsonLinesRecordCodec.calculate_checksum(&bytes)?;
    Ok(EncodedRecord {
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        kind,
        bytes,
        checksum,
    })
}

fn evidence_sidecar_paths(root: &StorageRoot) -> (PathBuf, PathBuf) {
    let runtime = root.path().join("runtime");
    (
        runtime.join(EVIDENCE_SIDECAR),
        runtime.join(EVIDENCE_SIDECAR_NEXT),
    )
}

fn load_evidence_sidecar(root: &StorageRoot) -> GraphStorageResult<EvidenceRecordStore> {
    let (current, _) = evidence_sidecar_paths(root);
    if !current.is_file() {
        return Ok(EvidenceRecordStore::new());
    }
    let bytes =
        fs::read(&current).map_err(|error| io_error("load_evidence_sidecar", &current, error))?;
    serde_json::from_slice(&bytes).map_err(|error| GraphStorageError::DecodeFailed {
        format: "evidence-records-json-v1".to_owned(),
        reason: error.to_string(),
    })
}

fn stage_evidence_sidecar(
    root: &StorageRoot,
    evidence: &EvidenceRecordStore,
) -> GraphStorageResult<()> {
    let (current, next) = evidence_sidecar_paths(root);
    let directory = current.parent().expect("evidence sidecar has a parent");
    fs::create_dir_all(directory)
        .map_err(|error| io_error("stage_evidence_sidecar", directory, error))?;
    let mut bytes =
        serde_json::to_vec(evidence).map_err(|error| GraphStorageError::OperationFailed {
            operation: "stage_evidence_sidecar",
            message: error.to_string(),
        })?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&next)
        .map_err(|error| io_error("stage_evidence_sidecar", &next, error))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| io_error("stage_evidence_sidecar", &next, error))
}

fn promote_evidence_sidecar(root: &StorageRoot) -> GraphStorageResult<()> {
    let (current, next) = evidence_sidecar_paths(root);
    fs::rename(&next, &current).map_err(|error| io_error("promote_evidence_sidecar", &next, error))
}

fn recover_evidence_sidecar(root: &StorageRoot) -> GraphStorageResult<()> {
    let (_, next) = evidence_sidecar_paths(root);
    if next.is_file() {
        promote_evidence_sidecar(root)?;
    }
    Ok(())
}

fn discard_staged_evidence_sidecar(root: &StorageRoot) {
    let (_, next) = evidence_sidecar_paths(root);
    if next.is_file() {
        let _ = fs::remove_file(next);
    }
}

fn placeholder_ref(segment: StorageSegment) -> StorageRef {
    StorageRef {
        segment,
        offset: 0,
        length: 1,
        checksum: None,
    }
}

fn adjacency_entry(
    relationship: &Relationship,
    direction: AdjacencyDirection,
) -> PersistedAdjacencyEntry {
    PersistedAdjacencyEntry {
        relationship_id: relationship.id().clone(),
        source_node_id: relationship.source().clone(),
        target_node_id: relationship.target().clone(),
        relationship_type: relationship.rel_type().clone(),
        direction,
        relationship_storage_ref: None,
        source_node_storage_ref: None,
        target_node_storage_ref: None,
    }
}

fn sequence_suffix(value: &str) -> u64 {
    value
        .rsplit_once("--")
        .and_then(|(_, suffix)| suffix.parse::<u64>().ok())
        .unwrap_or(0)
}

fn graph_error(error: impl std::fmt::Display) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "canonical_engine_store",
        message: error.to_string(),
    }
}

fn full_text_error(error: opencti_search::FullTextSearchError) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "canonical_full_text_index",
        message: error.to_string(),
    }
}

fn file_content_error(error: FileContentError) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "canonical_file_content_index",
        message: error.to_string(),
    }
}

fn pager_error(error: impl std::fmt::Display) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "page_canonical_graph_record",
        message: error.to_string(),
    }
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> GraphStorageResult<()> {
    use std::os::unix::fs::PermissionsExt as _;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| io_error("secure_full_text_cursor_key", path, error))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> GraphStorageResult<()> {
    Ok(())
}

fn io_error(operation: &'static str, path: &Path, error: std::io::Error) -> GraphStorageError {
    GraphStorageError::IoOperationFailed {
        operation,
        path: Some(path.to_path_buf()),
        message: error.to_string(),
    }
}
