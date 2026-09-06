// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

//! Corrobore-owned OpenCTI full-text search abstraction.
//!
//! This crate keeps Tantivy behind provider-neutral request, result and
//! lifecycle contracts. It owns normalization, query compilation, stable
//! score ordering, access filtering, cursor integrity and rebuild publication.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use graph_core::{Graph, Node, PropertyValue, RecordStatus, Relationship};
use hmac::{Hmac, KeyInit, Mac};
use opencti_access::{AccessContext, AccessMetadata};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tantivy::{
    Index, IndexWriter, TantivyDocument, Term,
    collector::{Count, TopDocs},
    query::{BooleanQuery, Occur, Query, QueryParser, TermQuery},
    schema::{Field, IndexRecordOption, STORED, STRING, Schema, TEXT, Value as _},
};
use thiserror::Error;

const LIFECYCLE_METADATA: &str = "corrobore-full-text.json";
const PUBLISHED_DIRECTORY: &str = "published";
const STAGING_DIRECTORY: &str = "staging";
const BACKUP_DIRECTORY: &str = "published.previous";
const INVALIDATION_MARKER: &str = "rebuild-required";
const CURSOR_VERSION: u8 = 1;

/// Object or relationship document stored in the inverted index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullTextRecordClass {
    /// Canonical graph node.
    Object,
    /// Canonical graph relationship.
    Relationship,
    /// Extracted OpenCTI-managed file content.
    FileContent,
}

/// Canonical, payload-independent search document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextDocument {
    /// Canonical stable identifier.
    pub id: String,
    /// Object or relationship class.
    pub record_class: FullTextRecordClass,
    /// OpenCTI entity or relationship kind.
    pub kind: String,
    /// Canonical graph revision.
    pub revision: u64,
    /// Searchable OpenCTI fields and their ordered text values.
    pub fields: BTreeMap<String, Vec<String>>,
    /// Compact authorization document evaluated before a hit is exposed.
    pub access: AccessMetadata,
}

/// Full-text matching primitive supported by the pinned compatibility corpus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum FullTextMatchMode {
    /// Normalized terms, all of which must match.
    Term,
    /// Adjacent normalized terms in the declared order.
    Phrase,
    /// Levenshtein-distance matching, optionally over token prefixes.
    Fuzzy {
        /// Maximum edit distance.
        distance: u8,
        /// Whether the final token may match a prefix.
        prefix: bool,
    },
    /// Prefix match over the final normalized token.
    Prefix,
}

/// One exact structured predicate combined conjunctively with full text.
///
/// Repeating the same field expresses an any-of predicate for that field;
/// predicates on different fields remain conjunctive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextFieldFilter {
    /// OpenCTI field name.
    pub field: String,
    /// Exact textual value.
    pub value: String,
}

/// Backend-neutral full-text request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextQuery {
    /// User text.
    pub text: String,
    /// Match primitive.
    pub mode: FullTextMatchMode,
    /// Searchable fields; empty uses the documented default field set.
    pub fields: Vec<String>,
    /// Accepted OpenCTI kinds; empty accepts every kind.
    pub kinds: Vec<String>,
    /// Exact filters, conjunctive across fields and disjunctive within a field.
    pub filters: Vec<FullTextFieldFilter>,
    /// Maximum hits in this page.
    pub limit: u32,
    /// Opaque continuation cursor.
    pub cursor: Option<String>,
}

/// Published index state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullTextSearchReadiness {
    /// No compatible complete index is published.
    RebuildRequired,
    /// A resumable staging generation is incomplete.
    Building,
    /// A complete generation is safe to query.
    Ready,
}

/// One ranked authorized hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FullTextSearchHit {
    /// Canonical stable identifier.
    pub id: String,
    /// Object or relationship class.
    pub record_class: FullTextRecordClass,
    /// OpenCTI kind.
    pub kind: String,
    /// Canonical revision.
    pub revision: u64,
    /// Tantivy relevance score.
    pub score: f32,
    /// Bounded highlighted context for content-bearing records.
    #[serde(default)]
    pub snippet: Option<String>,
    /// Exact normalized terms highlighted in the snippet.
    #[serde(default)]
    pub highlights: Vec<String>,
    /// Payload-free provenance and filter metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Stable page of authorized full-text hits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FullTextSearchPage {
    /// Authorized total independent of the page limit.
    pub total: u64,
    /// Score-descending hits with canonical-ID tie-breaking.
    pub hits: Vec<FullTextSearchHit>,
    /// Opaque next-page cursor.
    pub next_cursor: Option<String>,
    /// Canonical generation bound into cursors.
    pub generation: String,
    /// Candidates rejected by compact authorization metadata.
    #[serde(default)]
    pub authorization_denials: u64,
}

/// Durable index settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullTextIndexSettings {
    /// Search schema identifier.
    pub schema_version: String,
    /// Deployment secret used to authenticate cursors.
    pub cursor_key: Vec<u8>,
    /// Tantivy indexing memory budget.
    pub writer_memory_bytes: usize,
    /// Hard bound on matches evaluated for authorization.
    pub max_candidates: usize,
}

/// Rebuild progress and publication outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextRebuildOutcome {
    /// Current readiness after this step.
    pub readiness: FullTextSearchReadiness,
    /// Documents durably staged or published.
    pub processed_documents: usize,
    /// Canonical documents in the requested generation.
    pub total_documents: usize,
    /// Whether the published canonical generation changed.
    pub generation_changed: bool,
    /// Deterministic canonical generation fingerprint.
    pub generation: String,
}

/// Low-cardinality resource measurements for the current index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextStorageStats {
    /// Recursive on-disk bytes.
    pub disk_bytes: u64,
    /// Configured writer memory bound.
    pub writer_memory_bytes: usize,
    /// Configured candidate bound.
    pub max_candidates: usize,
}

/// Stable search and lifecycle failures.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FullTextSearchError {
    /// Query validation failed.
    #[error("invalid full-text query: {0}")]
    InvalidQuery(String),
    /// No complete compatible generation may serve reads.
    #[error("full-text index is not ready")]
    IndexNotReady,
    /// Cursor integrity or generation/policy compatibility failed.
    #[error("full-text cursor is incompatible")]
    IncompatibleCursor,
    /// Persistent index I/O failed.
    #[error("full-text index I/O failed: {0}")]
    Io(String),
    /// Tantivy rejected an index or query operation.
    #[error("full-text backend failed: {0}")]
    Backend(String),
}

/// Durable full-text index handle.
#[derive(Clone, Debug)]
pub struct FullTextIndex {
    root: PathBuf,
    settings: FullTextIndexSettings,
}

impl FullTextIndex {
    /// Open a search root without hydrating graph payloads.
    ///
    /// Phase 3 validates metadata and creates missing lifecycle directories.
    pub fn open(
        root: PathBuf,
        settings: FullTextIndexSettings,
    ) -> Result<Self, FullTextSearchError> {
        if settings.cursor_key.len() < 32 {
            return Err(FullTextSearchError::InvalidQuery(
                "cursor key must contain at least 32 bytes".to_owned(),
            ));
        }
        fs::create_dir_all(&root).map_err(io_error)?;
        Ok(Self { root, settings })
    }

    /// Return the published Tantivy directory.
    pub fn index_path(&self) -> PathBuf {
        self.root.join(PUBLISHED_DIRECTORY)
    }

    /// Inspect metadata and Tantivy integrity without publishing staging data.
    pub fn inspect(&self) -> FullTextIndexStatus {
        if self.invalidation_path().exists() {
            return FullTextIndexStatus::rebuild_required();
        }
        if let Ok(metadata) = read_metadata(&self.staging_path())
            && metadata.schema_version == self.settings.schema_version
            && metadata.readiness == FullTextSearchReadiness::Building
        {
            return metadata.into();
        }
        let published = self.index_path();
        if let Ok(metadata) = read_metadata(&published)
            && metadata.schema_version == self.settings.schema_version
            && metadata.readiness == FullTextSearchReadiness::Ready
            && Index::open_in_dir(&published).is_ok()
        {
            return metadata.into();
        }
        FullTextIndexStatus::rebuild_required()
    }

    /// Atomically rebuild and publish one complete canonical generation.
    pub fn rebuild(
        &self,
        documents: &[FullTextDocument],
    ) -> Result<FullTextRebuildOutcome, FullTextSearchError> {
        self.rebuild_with_checkpoint(documents, documents.len().max(1), None)
    }

    /// Resume a checkpointed rebuild and optionally stop after a document count.
    ///
    /// Phase 3 writes only to staging, checkpoints commits and atomically
    /// publishes after every canonical document has been indexed.
    pub fn rebuild_with_checkpoint(
        &self,
        documents: &[FullTextDocument],
        checkpoint_every: usize,
        stop_after: Option<usize>,
    ) -> Result<FullTextRebuildOutcome, FullTextSearchError> {
        if checkpoint_every == 0 {
            return Err(FullTextSearchError::InvalidQuery(
                "rebuild checkpoint interval must be non-zero".to_owned(),
            ));
        }
        let canonical = canonical_documents(documents)?;
        let generation = document_generation(&canonical)?;
        let published_generation =
            ready_metadata(&self.index_path(), &self.settings.schema_version)
                .map(|metadata| metadata.generation);
        if published_generation.as_deref() == Some(generation.as_str())
            && self.staging_path().exists()
        {
            fs::remove_dir_all(self.staging_path()).map_err(io_error)?;
        }
        if published_generation.as_deref() == Some(generation.as_str())
            && Index::open_in_dir(self.index_path()).is_ok()
        {
            self.clear_invalidation()?;
            return Ok(FullTextRebuildOutcome {
                readiness: FullTextSearchReadiness::Ready,
                processed_documents: canonical.len(),
                total_documents: canonical.len(),
                generation_changed: false,
                generation,
            });
        }

        let staging = self.staging_path();
        let (index, schema_fields, mut metadata) =
            self.open_or_create_staging(&canonical, &generation)?;
        let stop_at = stop_after
            .map(|count| metadata.processed_documents.saturating_add(count))
            .unwrap_or(canonical.len())
            .min(canonical.len());
        let mut writer: IndexWriter = index
            .writer(self.settings.writer_memory_bytes)
            .map_err(backend_error)?;
        while metadata.processed_documents < stop_at {
            let document = &canonical[metadata.processed_documents];
            writer
                .add_document(tantivy_document(document, &schema_fields)?)
                .map_err(backend_error)?;
            metadata.processed_documents = metadata.processed_documents.saturating_add(1);
            if metadata.processed_documents % checkpoint_every == 0
                || metadata.processed_documents == stop_at
            {
                writer.commit().map_err(backend_error)?;
                write_metadata(&staging, &metadata)?;
            }
        }
        writer.wait_merging_threads().map_err(backend_error)?;

        if metadata.processed_documents < canonical.len() {
            return Ok(metadata.outcome(false));
        }
        metadata.readiness = FullTextSearchReadiness::Ready;
        write_metadata(&staging, &metadata)?;
        Index::open_in_dir(&staging).map_err(backend_error)?;
        self.publish_staging()?;
        Ok(metadata.outcome(published_generation.as_deref() != Some(generation.as_str())))
    }

    /// Prevent the currently published generation from serving reads until a
    /// canonical rebuild succeeds.
    pub fn invalidate(&self) -> Result<(), FullTextSearchError> {
        let temporary = self.root.join(format!("{INVALIDATION_MARKER}.next"));
        fs::write(&temporary, b"canonical-generation-changed\n").map_err(io_error)?;
        fs::rename(temporary, self.invalidation_path()).map_err(io_error)
    }

    /// Execute one access-aware query against the published generation.
    ///
    /// Phase 3 compiles only the supported primitives, filters compact access
    /// metadata before totals/page creation and binds cursors to query, policy
    /// and index generation.
    pub fn search(
        &self,
        query: &FullTextQuery,
        access: &AccessContext,
    ) -> Result<FullTextSearchPage, FullTextSearchError> {
        validate_query(query)?;
        if self.inspect().readiness != FullTextSearchReadiness::Ready {
            return Err(FullTextSearchError::IndexNotReady);
        }
        let published = self.index_path();
        let metadata = ready_metadata(&published, &self.settings.schema_version)
            .ok_or(FullTextSearchError::IndexNotReady)?;
        let index = Index::open_in_dir(&published).map_err(backend_error)?;
        let fields = SearchSchemaFields::from_schema(&index.schema())?;
        let selected_text_fields = select_text_fields(&index.schema(), &fields, &query.fields);
        if selected_text_fields.is_empty() {
            return Ok(FullTextSearchPage {
                total: 0,
                hits: Vec::new(),
                next_cursor: None,
                generation: metadata.generation,
                authorization_denials: 0,
            });
        }
        let compiled = compile_query(&index, query, &fields, selected_text_fields)?;
        let reader = index.reader().map_err(backend_error)?;
        let searcher = reader.searcher();
        let candidate_count = searcher
            .search(compiled.as_ref(), &Count)
            .map_err(backend_error)?;
        if candidate_count > self.settings.max_candidates {
            return Err(FullTextSearchError::InvalidQuery(format!(
                "query matched {candidate_count} documents, candidate budget is {}",
                self.settings.max_candidates
            )));
        }
        let ranked = searcher
            .search(
                compiled.as_ref(),
                &TopDocs::with_limit(candidate_count.max(1)).order_by_score(),
            )
            .map_err(backend_error)?;
        let policy = opencti_access::OpenCtiAccessPolicy::compile(access)
            .map_err(|error| FullTextSearchError::InvalidQuery(error.to_string()))?;
        let mut authorized = Vec::with_capacity(ranked.len());
        for (score, address) in ranked {
            let document = searcher
                .doc::<TantivyDocument>(address)
                .map_err(backend_error)?;
            let access_json = stored_text(&document, fields.access)?;
            let access_metadata: AccessMetadata =
                serde_json::from_str(access_json).map_err(|error| {
                    FullTextSearchError::Backend(format!(
                        "invalid indexed access metadata: {error}"
                    ))
                })?;
            if !policy.evaluate(&access_metadata).allowed() {
                continue;
            }
            authorized.push(FullTextSearchHit {
                id: stored_text(&document, fields.id)?.to_owned(),
                record_class: serde_json::from_str(stored_text(&document, fields.record_class)?)
                    .map_err(|error| {
                        FullTextSearchError::Backend(format!(
                            "invalid indexed record class: {error}"
                        ))
                    })?,
                kind: stored_text(&document, fields.kind)?.to_owned(),
                revision: stored_u64(&document, fields.revision)?,
                score,
                snippet: None,
                highlights: Vec::new(),
                metadata: BTreeMap::new(),
            });
        }
        authorized.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.id.cmp(&right.id))
        });
        let query_fingerprint = query_fingerprint(query)?;
        let policy_binding = policy_binding(&self.settings.cursor_key, &policy)?;
        let start = query
            .cursor
            .as_deref()
            .map(|cursor| {
                decode_cursor(
                    cursor,
                    &self.settings.cursor_key,
                    &query_fingerprint,
                    &metadata.generation,
                    policy.policy_version(),
                    &policy_binding,
                )
            })
            .transpose()?
            .map_or(0, |claims| {
                authorized
                    .iter()
                    .position(|hit| {
                        hit.id == claims.last_id && hit.score.to_bits() == claims.score_bits
                    })
                    .map(|position| position.saturating_add(1))
                    .unwrap_or(authorized.len())
            });
        if query.cursor.is_some() && start == authorized.len() && !authorized.is_empty() {
            let claims = decode_cursor(
                query.cursor.as_deref().unwrap_or_default(),
                &self.settings.cursor_key,
                &query_fingerprint,
                &metadata.generation,
                policy.policy_version(),
                &policy_binding,
            )?;
            if authorized.last().is_none_or(|hit| {
                hit.id != claims.last_id || hit.score.to_bits() != claims.score_bits
            }) {
                return Err(FullTextSearchError::IncompatibleCursor);
            }
        }
        let end = start
            .saturating_add(query.limit as usize)
            .min(authorized.len());
        let hits = authorized[start..end].to_vec();
        let next_cursor = if end < authorized.len() {
            hits.last()
                .map(|hit| {
                    encode_cursor(
                        &FullTextCursorClaims {
                            version: CURSOR_VERSION,
                            query_fingerprint: query_fingerprint.clone(),
                            generation: metadata.generation.clone(),
                            policy_version: policy.policy_version().to_owned(),
                            policy_binding: policy_binding.clone(),
                            score_bits: hit.score.to_bits(),
                            last_id: hit.id.clone(),
                        },
                        &self.settings.cursor_key,
                    )
                })
                .transpose()?
        } else {
            None
        };
        Ok(FullTextSearchPage {
            total: authorized.len() as u64,
            hits,
            next_cursor,
            generation: metadata.generation,
            authorization_denials: candidate_count.saturating_sub(authorized.len()) as u64,
        })
    }

    /// Measure bounded configuration and published on-disk size.
    pub fn storage_stats(&self) -> Result<FullTextStorageStats, FullTextSearchError> {
        Ok(FullTextStorageStats {
            disk_bytes: directory_bytes(&self.root)?,
            writer_memory_bytes: self.settings.writer_memory_bytes,
            max_candidates: self.settings.max_candidates,
        })
    }

    fn staging_path(&self) -> PathBuf {
        self.root.join(STAGING_DIRECTORY)
    }

    fn invalidation_path(&self) -> PathBuf {
        self.root.join(INVALIDATION_MARKER)
    }

    fn clear_invalidation(&self) -> Result<(), FullTextSearchError> {
        match fs::remove_file(self.invalidation_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(error)),
        }
    }

    fn open_or_create_staging(
        &self,
        documents: &[FullTextDocument],
        generation: &str,
    ) -> Result<(Index, SearchSchemaFields, LifecycleMetadata), FullTextSearchError> {
        let staging = self.staging_path();
        if let Ok(metadata) = read_metadata(&staging)
            && metadata.schema_version == self.settings.schema_version
            && metadata.generation == generation
            && metadata.total_documents == documents.len()
            && metadata.readiness == FullTextSearchReadiness::Building
            && let Ok(index) = Index::open_in_dir(&staging)
        {
            let fields = SearchSchemaFields::from_schema(&index.schema())?;
            return Ok((index, fields, metadata));
        }
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(io_error)?;
        }
        fs::create_dir_all(&staging).map_err(io_error)?;
        let (schema, fields) = build_schema(documents);
        let index = Index::create_in_dir(&staging, schema).map_err(backend_error)?;
        let metadata = LifecycleMetadata {
            schema_version: self.settings.schema_version.clone(),
            generation: generation.to_owned(),
            readiness: FullTextSearchReadiness::Building,
            processed_documents: 0,
            total_documents: documents.len(),
        };
        write_metadata(&staging, &metadata)?;
        Ok((index, fields, metadata))
    }

    fn publish_staging(&self) -> Result<(), FullTextSearchError> {
        let staging = self.staging_path();
        let published = self.index_path();
        let backup = self.root.join(BACKUP_DIRECTORY);
        if backup.exists() {
            fs::remove_dir_all(&backup).map_err(io_error)?;
        }
        if published.exists() {
            fs::rename(&published, &backup).map_err(io_error)?;
        }
        if let Err(error) = fs::rename(&staging, &published) {
            if backup.exists() {
                let _ = fs::rename(&backup, &published);
            }
            return Err(io_error(error));
        }
        if backup.exists() {
            fs::remove_dir_all(backup).map_err(io_error)?;
        }
        self.clear_invalidation()?;
        Ok(())
    }
}

/// Convert every current non-tombstoned graph record into canonical search
/// documents without indexing file-content payloads.
pub fn documents_from_graph(graph: &Graph) -> Result<Vec<FullTextDocument>, FullTextSearchError> {
    let nodes = graph
        .list_nodes()
        .map_err(|error| FullTextSearchError::Backend(error.to_string()))?;
    let relationships = graph
        .list_relationships()
        .map_err(|error| FullTextSearchError::Backend(error.to_string()))?;
    let mut documents = nodes
        .iter()
        .filter(|node| node.status() != RecordStatus::Tombstoned)
        .map(document_from_node)
        .collect::<Result<Vec<_>, _>>()?;
    documents.extend(
        relationships
            .iter()
            .filter(|relationship| relationship.status() != RecordStatus::Tombstoned)
            .map(document_from_relationship)
            .collect::<Result<Vec<_>, _>>()?,
    );
    canonical_documents(&documents)
}

/// Convert one canonical node into its payload-independent search document.
pub fn document_from_node(node: &Node) -> Result<FullTextDocument, FullTextSearchError> {
    record_document(
        node.id().as_str(),
        FullTextRecordClass::Object,
        node.version(),
        node.properties(),
        node.labels()
            .iter()
            .find_map(|label| label.strip_prefix("OpenCtiType_"))
            .unwrap_or("object"),
    )
}

/// Convert one canonical relationship into its search document.
pub fn document_from_relationship(
    relationship: &Relationship,
) -> Result<FullTextDocument, FullTextSearchError> {
    record_document(
        relationship.id().as_str(),
        FullTextRecordClass::Relationship,
        relationship.version(),
        relationship.properties(),
        relationship.rel_type().as_str(),
    )
}

/// Metadata-only lifecycle status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullTextIndexStatus {
    /// Published/staging readiness.
    pub readiness: FullTextSearchReadiness,
    /// Deterministic generation, when known.
    pub generation: Option<String>,
    /// Checkpointed staging progress.
    pub processed_documents: usize,
    /// Canonical documents expected by the staging generation.
    pub total_documents: usize,
}

impl FullTextIndexStatus {
    fn rebuild_required() -> Self {
        Self {
            readiness: FullTextSearchReadiness::RebuildRequired,
            generation: None,
            processed_documents: 0,
            total_documents: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LifecycleMetadata {
    schema_version: String,
    generation: String,
    readiness: FullTextSearchReadiness,
    processed_documents: usize,
    total_documents: usize,
}

impl LifecycleMetadata {
    fn outcome(&self, generation_changed: bool) -> FullTextRebuildOutcome {
        FullTextRebuildOutcome {
            readiness: self.readiness,
            processed_documents: self.processed_documents,
            total_documents: self.total_documents,
            generation_changed,
            generation: self.generation.clone(),
        }
    }
}

impl From<LifecycleMetadata> for FullTextIndexStatus {
    fn from(metadata: LifecycleMetadata) -> Self {
        Self {
            readiness: metadata.readiness,
            generation: Some(metadata.generation),
            processed_documents: metadata.processed_documents,
            total_documents: metadata.total_documents,
        }
    }
}

#[derive(Clone, Debug)]
struct SearchSchemaFields {
    id: Field,
    record_class: Field,
    kind: Field,
    revision: Field,
    access: Field,
    text_fields: BTreeMap<String, Field>,
    exact_fields: BTreeMap<String, Field>,
}

impl SearchSchemaFields {
    fn from_schema(schema: &Schema) -> Result<Self, FullTextSearchError> {
        let required = |name: &str| {
            schema.get_field(name).map_err(|_| {
                FullTextSearchError::Backend(format!("index schema is missing {name}"))
            })
        };
        let mut text_fields = BTreeMap::new();
        let mut exact_fields = BTreeMap::new();
        for (field, entry) in schema.fields() {
            if let Some(name) = entry.name().strip_prefix("text_") {
                text_fields.insert(name.to_owned(), field);
            } else if let Some(name) = entry.name().strip_prefix("exact_") {
                exact_fields.insert(name.to_owned(), field);
            }
        }
        Ok(Self {
            id: required("id")?,
            record_class: required("record_class")?,
            kind: required("kind")?,
            revision: required("revision")?,
            access: required("access")?,
            text_fields,
            exact_fields,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FullTextCursorClaims {
    version: u8,
    query_fingerprint: String,
    generation: String,
    policy_version: String,
    policy_binding: String,
    score_bits: u32,
    last_id: String,
}

fn record_document(
    fallback_id: &str,
    record_class: FullTextRecordClass,
    revision: u64,
    properties: &graph_core::PropertyMap,
    fallback_kind: &str,
) -> Result<FullTextDocument, FullTextSearchError> {
    let id = property_strings(properties.get("opencti.canonical_id"))
        .into_iter()
        .next()
        .unwrap_or_else(|| fallback_id.to_owned());
    let kind = property_strings(properties.get("opencti.entity_type"))
        .into_iter()
        .next()
        .unwrap_or_else(|| fallback_kind.to_owned());
    let access = match properties.get("opencti.access") {
        Some(PropertyValue::Json(value)) => {
            serde_json::from_value(value.clone()).map_err(|error| {
                FullTextSearchError::Backend(format!("invalid canonical access metadata: {error}"))
            })?
        }
        None => AccessMetadata::default(),
        Some(_) => {
            return Err(FullTextSearchError::Backend(
                "canonical opencti.access is not JSON".to_owned(),
            ));
        }
    };
    let fields = properties
        .iter()
        .filter_map(|(key, value)| {
            let field = key.strip_prefix("opencti.field.")?;
            if field == "content" {
                return None;
            }
            let values = property_strings(Some(value));
            (!values.is_empty()).then(|| (field.to_owned(), values))
        })
        .collect();
    Ok(FullTextDocument {
        id,
        record_class,
        kind,
        revision,
        fields,
        access,
    })
}

fn property_strings(value: Option<&PropertyValue>) -> Vec<String> {
    match value {
        Some(PropertyValue::String(value)) => vec![value.clone()],
        Some(PropertyValue::StringList(values)) => values.clone(),
        Some(PropertyValue::Integer(value)) => vec![value.to_string()],
        Some(PropertyValue::IntegerList(values)) => {
            values.iter().map(ToString::to_string).collect()
        }
        Some(PropertyValue::Float(value)) => vec![value.to_string()],
        Some(PropertyValue::FloatList(values)) => values.iter().map(ToString::to_string).collect(),
        Some(PropertyValue::Bool(value)) => vec![value.to_string()],
        Some(PropertyValue::BoolList(values)) => values.iter().map(ToString::to_string).collect(),
        Some(PropertyValue::Json(value)) => json_strings(value),
        Some(PropertyValue::Null) | None => Vec::new(),
    }
}

fn json_strings(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(value) => vec![value.clone()],
        serde_json::Value::Array(values) => values.iter().flat_map(json_strings).collect(),
        serde_json::Value::Object(values) => values.values().flat_map(json_strings).collect(),
        serde_json::Value::Number(value) => vec![value.to_string()],
        serde_json::Value::Bool(value) => vec![value.to_string()],
        serde_json::Value::Null => Vec::new(),
    }
}

fn canonical_documents(
    documents: &[FullTextDocument],
) -> Result<Vec<FullTextDocument>, FullTextSearchError> {
    let mut canonical = documents.to_vec();
    for document in &mut canonical {
        document.id = document.id.trim().to_owned();
        document.kind = normalize_keyword(&document.kind);
        if document.id.is_empty() || document.kind.is_empty() {
            return Err(FullTextSearchError::InvalidQuery(
                "indexed documents require non-empty id and kind".to_owned(),
            ));
        }
        let mut normalized_fields = BTreeMap::new();
        for (field, values) in std::mem::take(&mut document.fields) {
            let field = normalize_field_name(&field)?;
            let mut values = values
                .into_iter()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            values.sort();
            values.dedup();
            if !values.is_empty() {
                normalized_fields.insert(field, values);
            }
        }
        document.fields = normalized_fields;
    }
    canonical.sort_by(|left, right| left.id.cmp(&right.id));
    for pair in canonical.windows(2) {
        if pair[0].id == pair[1].id {
            return Err(FullTextSearchError::InvalidQuery(format!(
                "duplicate canonical search document {}",
                pair[0].id
            )));
        }
    }
    Ok(canonical)
}

fn document_generation(documents: &[FullTextDocument]) -> Result<String, FullTextSearchError> {
    let bytes = serde_json::to_vec(documents).map_err(|error| {
        FullTextSearchError::Backend(format!("failed to canonicalize search generation: {error}"))
    })?;
    Ok(hex_digest(&bytes))
}

fn build_schema(documents: &[FullTextDocument]) -> (Schema, SearchSchemaFields) {
    let mut field_names = BTreeSet::new();
    for document in documents {
        field_names.extend(document.fields.keys().cloned());
    }
    let mut builder = Schema::builder();
    let id = builder.add_text_field("id", STRING | STORED);
    let record_class = builder.add_text_field("record_class", STRING | STORED);
    let kind = builder.add_text_field("kind", STRING | STORED);
    let revision = builder.add_u64_field("revision", STORED);
    let access = builder.add_text_field("access", STORED);
    let mut text_fields = BTreeMap::new();
    let mut exact_fields = BTreeMap::new();
    for field_name in field_names {
        let suffix = field_suffix(&field_name);
        text_fields.insert(
            suffix.clone(),
            builder.add_text_field(&format!("text_{suffix}"), TEXT),
        );
        exact_fields.insert(
            suffix.clone(),
            builder.add_text_field(&format!("exact_{suffix}"), STRING),
        );
    }
    (
        builder.build(),
        SearchSchemaFields {
            id,
            record_class,
            kind,
            revision,
            access,
            text_fields,
            exact_fields,
        },
    )
}

fn tantivy_document(
    document: &FullTextDocument,
    fields: &SearchSchemaFields,
) -> Result<TantivyDocument, FullTextSearchError> {
    let mut indexed = TantivyDocument::default();
    indexed.add_text(fields.id, &document.id);
    indexed.add_text(
        fields.record_class,
        &serde_json::to_string(&document.record_class).map_err(|error| {
            FullTextSearchError::Backend(format!("failed to encode record class: {error}"))
        })?,
    );
    indexed.add_text(fields.kind, &document.kind);
    indexed.add_u64(fields.revision, document.revision);
    indexed.add_text(
        fields.access,
        &serde_json::to_string(&document.access).map_err(|error| {
            FullTextSearchError::Backend(format!("failed to encode access metadata: {error}"))
        })?,
    );
    for (field_name, values) in &document.fields {
        let suffix = field_suffix(field_name);
        let text_field = fields.text_fields.get(&suffix).ok_or_else(|| {
            FullTextSearchError::Backend(format!("missing text schema field for {field_name}"))
        })?;
        let exact_field = fields.exact_fields.get(&suffix).ok_or_else(|| {
            FullTextSearchError::Backend(format!("missing exact schema field for {field_name}"))
        })?;
        for value in values {
            indexed.add_text(*text_field, value);
            indexed.add_text(*exact_field, normalize_keyword(value));
        }
    }
    Ok(indexed)
}

fn select_text_fields(
    schema: &Schema,
    fields: &SearchSchemaFields,
    requested: &[String],
) -> Vec<(String, Field)> {
    if requested.is_empty() {
        return schema
            .fields()
            .filter_map(|(field, entry)| {
                entry
                    .name()
                    .strip_prefix("text_")
                    .map(|suffix| (suffix.to_owned(), field))
            })
            .collect();
    }
    requested
        .iter()
        .filter_map(|field_name| {
            normalize_field_name(field_name)
                .ok()
                .map(|field_name| field_suffix(&field_name))
                .and_then(|suffix| {
                    fields
                        .text_fields
                        .get(&suffix)
                        .copied()
                        .map(|field| (suffix, field))
                })
        })
        .collect()
}

fn compile_query(
    index: &Index,
    request: &FullTextQuery,
    fields: &SearchSchemaFields,
    selected_fields: Vec<(String, Field)>,
) -> Result<Box<dyn Query>, FullTextSearchError> {
    let default_fields = selected_fields
        .iter()
        .map(|(_, field)| *field)
        .collect::<Vec<_>>();
    let mut parser = QueryParser::for_index(index, default_fields.clone());
    parser.set_conjunction_by_default();
    for (suffix, field) in &selected_fields {
        let boost = field_boost(suffix);
        parser.set_field_boost(*field, boost);
        match request.mode {
            FullTextMatchMode::Fuzzy { distance, prefix } => {
                parser.set_field_fuzzy(*field, prefix, distance, true)
            }
            FullTextMatchMode::Prefix => parser.set_field_fuzzy(*field, true, 0, true),
            FullTextMatchMode::Term | FullTextMatchMode::Phrase => {}
        }
    }
    let input = match request.mode {
        FullTextMatchMode::Phrase => format!("\"{}\"", escape_query_text(&request.text)),
        _ => escape_query_text(&request.text),
    };
    let text_query = parser
        .parse_query(&input)
        .map_err(|error| FullTextSearchError::InvalidQuery(error.to_string()))?;
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![(Occur::Must, text_query)];
    if !request.kinds.is_empty() {
        let kind_clauses = request
            .kinds
            .iter()
            .map(|kind| {
                (
                    Occur::Should,
                    Box::new(TermQuery::new(
                        Term::from_field_text(fields.kind, &normalize_keyword(kind)),
                        IndexRecordOption::Basic,
                    )) as Box<dyn Query>,
                )
            })
            .collect();
        clauses.push((Occur::Must, Box::new(BooleanQuery::new(kind_clauses))));
    }
    let mut filters_by_field = BTreeMap::<String, (Field, Vec<String>)>::new();
    for filter in &request.filters {
        let field_name = normalize_field_name(&filter.field)?;
        let suffix = field_suffix(&field_name);
        let Some(field) = fields.exact_fields.get(&suffix) else {
            return Ok(Box::new(BooleanQuery::new(vec![(
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(fields.id, "__corrobore_no_match__"),
                    IndexRecordOption::Basic,
                )),
            )])));
        };
        filters_by_field
            .entry(suffix)
            .or_insert_with(|| (*field, Vec::new()))
            .1
            .push(filter.value.clone());
    }
    for (_, (field, values)) in filters_by_field {
        let value_clauses = values
            .into_iter()
            .map(|value| {
                (
                    Occur::Should,
                    Box::new(TermQuery::new(
                        Term::from_field_text(field, &normalize_keyword(&value)),
                        IndexRecordOption::Basic,
                    )) as Box<dyn Query>,
                )
            })
            .collect();
        clauses.push((Occur::Must, Box::new(BooleanQuery::new(value_clauses))));
    }
    Ok(Box::new(BooleanQuery::new(clauses)))
}

fn validate_query(query: &FullTextQuery) -> Result<(), FullTextSearchError> {
    if query.text.trim().is_empty() {
        return Err(FullTextSearchError::InvalidQuery(
            "search text must not be blank".to_owned(),
        ));
    }
    if query.limit == 0 || query.limit > 1_000 {
        return Err(FullTextSearchError::InvalidQuery(
            "limit must be between 1 and 1000".to_owned(),
        ));
    }
    if let FullTextMatchMode::Fuzzy { distance, .. } = query.mode
        && !(1..=2).contains(&distance)
    {
        return Err(FullTextSearchError::InvalidQuery(
            "fuzzy distance must be 1 or 2".to_owned(),
        ));
    }
    for field in query
        .fields
        .iter()
        .chain(query.filters.iter().map(|filter| &filter.field))
    {
        normalize_field_name(field)?;
    }
    Ok(())
}

fn normalize_field_name(field: &str) -> Result<String, FullTextSearchError> {
    let field = field.trim().to_ascii_lowercase();
    if field.is_empty()
        || !field
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(FullTextSearchError::InvalidQuery(format!(
            "unsupported search field {field:?}"
        )));
    }
    Ok(field)
}

fn field_suffix(field: &str) -> String {
    hex_digest(field.as_bytes())
}

fn field_boost(suffix: &str) -> f32 {
    if suffix == field_suffix("name") {
        3.0
    } else if suffix == field_suffix("aliases") || suffix == field_suffix("x_opencti_aliases") {
        2.0
    } else {
        1.0
    }
}

fn normalize_keyword(value: &str) -> String {
    value.trim().to_lowercase()
}

fn escape_query_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.trim().chars() {
        if matches!(
            character,
            '\\' | '+'
                | '^'
                | '`'
                | ':'
                | '{'
                | '}'
                | '['
                | ']'
                | '('
                | ')'
                | '"'
                | '~'
                | '*'
                | '?'
                | '|'
                | '&'
                | '/'
                | '-'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn stored_text(document: &TantivyDocument, field: Field) -> Result<&str, FullTextSearchError> {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| FullTextSearchError::Backend("stored text field is missing".to_owned()))
}

fn stored_u64(document: &TantivyDocument, field: Field) -> Result<u64, FullTextSearchError> {
    document
        .get_first(field)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| FullTextSearchError::Backend("stored u64 field is missing".to_owned()))
}

fn query_fingerprint(query: &FullTextQuery) -> Result<String, FullTextSearchError> {
    let mut normalized = query.clone();
    normalized.cursor = None;
    normalized.text = normalized.text.trim().to_owned();
    normalized.fields = normalized
        .fields
        .iter()
        .map(|field| normalize_field_name(field))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.fields.sort();
    normalized.fields.dedup();
    normalized.kinds = normalized
        .kinds
        .iter()
        .map(|kind| normalize_keyword(kind))
        .collect();
    normalized.kinds.sort();
    normalized.kinds.dedup();
    normalized.filters.sort_by(|left, right| {
        left.field
            .cmp(&right.field)
            .then_with(|| left.value.cmp(&right.value))
    });
    let bytes = serde_json::to_vec(&normalized).map_err(|error| {
        FullTextSearchError::Backend(format!("failed to fingerprint query: {error}"))
    })?;
    Ok(hex_digest(&bytes))
}

fn policy_binding(
    key: &[u8],
    policy: &opencti_access::OpenCtiAccessPolicy,
) -> Result<String, FullTextSearchError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| FullTextSearchError::Backend("failed to bind access policy".to_owned()))?;
    mac.update(b"corrobore-full-text-policy\0");
    mac.update(policy.fingerprint().as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn encode_cursor(claims: &FullTextCursorClaims, key: &[u8]) -> Result<String, FullTextSearchError> {
    let payload = serde_json::to_vec(claims).map_err(|error| {
        FullTextSearchError::Backend(format!("failed to encode full-text cursor: {error}"))
    })?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .map_err(|_| FullTextSearchError::Backend("failed to sign full-text cursor".to_owned()))?;
    mac.update(&payload);
    Ok(format!(
        "fts1.{}.{}",
        URL_SAFE_NO_PAD.encode(&payload),
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

fn decode_cursor(
    cursor: &str,
    key: &[u8],
    query_fingerprint: &str,
    generation: &str,
    policy_version: &str,
    policy_binding: &str,
) -> Result<FullTextCursorClaims, FullTextSearchError> {
    let mut parts = cursor.split('.');
    if parts.next() != Some("fts1") {
        return Err(FullTextSearchError::IncompatibleCursor);
    }
    let payload = parts
        .next()
        .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
        .ok_or(FullTextSearchError::IncompatibleCursor)?;
    let tag = parts
        .next()
        .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
        .ok_or(FullTextSearchError::IncompatibleCursor)?;
    if parts.next().is_some() {
        return Err(FullTextSearchError::IncompatibleCursor);
    }
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).map_err(|_| FullTextSearchError::IncompatibleCursor)?;
    mac.update(&payload);
    mac.verify_slice(&tag)
        .map_err(|_| FullTextSearchError::IncompatibleCursor)?;
    let claims: FullTextCursorClaims =
        serde_json::from_slice(&payload).map_err(|_| FullTextSearchError::IncompatibleCursor)?;
    if claims.version != CURSOR_VERSION
        || claims.query_fingerprint != query_fingerprint
        || claims.generation != generation
        || claims.policy_version != policy_version
        || claims.policy_binding != policy_binding
    {
        return Err(FullTextSearchError::IncompatibleCursor);
    }
    Ok(claims)
}

fn read_metadata(path: &Path) -> Result<LifecycleMetadata, FullTextSearchError> {
    let bytes = fs::read(path.join(LIFECYCLE_METADATA)).map_err(io_error)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| FullTextSearchError::Io(format!("invalid lifecycle metadata: {error}")))
}

fn ready_metadata(path: &Path, schema_version: &str) -> Option<LifecycleMetadata> {
    read_metadata(path).ok().filter(|metadata| {
        metadata.schema_version == schema_version
            && metadata.readiness == FullTextSearchReadiness::Ready
    })
}

fn write_metadata(path: &Path, metadata: &LifecycleMetadata) -> Result<(), FullTextSearchError> {
    let bytes = serde_json::to_vec_pretty(metadata).map_err(|error| {
        FullTextSearchError::Backend(format!("failed to encode lifecycle metadata: {error}"))
    })?;
    let temporary = path.join(format!("{LIFECYCLE_METADATA}.next"));
    fs::write(&temporary, bytes).map_err(io_error)?;
    fs::rename(temporary, path.join(LIFECYCLE_METADATA)).map_err(io_error)
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn directory_bytes(path: &Path) -> Result<u64, FullTextSearchError> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let metadata = entry.metadata().map_err(io_error)?;
        total = total.saturating_add(if metadata.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            metadata.len()
        });
    }
    Ok(total)
}

fn io_error(error: std::io::Error) -> FullTextSearchError {
    FullTextSearchError::Io(error.to_string())
}

fn backend_error(error: tantivy::TantivyError) -> FullTextSearchError {
    FullTextSearchError::Backend(error.to_string())
}
