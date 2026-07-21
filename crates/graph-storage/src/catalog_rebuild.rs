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
use std::fs;
use std::path::{Path, PathBuf};

use graph_core::{AdjacencyDirection, LabelSet, RelationshipType};
use serde::Deserialize;

use crate::{
    AdjacencyStorageCatalogEntry, GraphCatalog, GraphRecordVersion, GraphStorageError,
    GraphStorageResult, LabelIndexNodeMetadata, LatestRecordCatalogEntry, PersistedAdjacencyRecord,
    PersistedRecordEnvelope, PersistedRecordId, PersistedRecordKind,
    RelationshipTypeIndexRelationshipMetadata, StorageRef, StorageRoot, StorageSegment,
    check_duplicate_latest_record_conflict, index_appended_node_record,
    index_appended_relationship_record, index_incoming_adjacency_storage_ref, index_node_labels,
    index_outgoing_adjacency_storage_ref, index_relationship_type,
    validate_persisted_record_envelope, validate_storage_ref,
};

/// Options controlling a catalog rebuild from persisted append-only logs.
///
///
/// - Make rebuild behavior explicit at the API boundary before implementing log
///   scanning.
/// - Allow future implementations to rebuild the full catalog, or a focused subset, without
///   changing the public entry point.
/// - Keep rebuild options separate from storage-root opening, graph payload
///   loading, and pager setup.
///
///
/// Future implementations should use these flags to decide which persisted
/// segments are read during rebuild. The default should rebuild all catalog-owned
/// metadata that the catalog rebuild is responsible for: latest node records, latest
/// relationship records, label indexes, relationship-type indexes, and adjacency
/// catalog entries.
///
/// # Errors
///
///
/// Invalid combinations, missing required logs, corrupted records, duplicate latest
/// conflicts, and unsupported persisted formats must return explicit
/// `GraphStorageError` values instead of producing partial silent catalog state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRebuildOptions {
    /// Whether node append logs should be scanned for latest node mappings and
    /// label index reconstruction.
    pub include_node_records: bool,

    /// Whether relationship append logs should be scanned for latest relationship
    /// mappings and relationship-type index reconstruction.
    pub include_relationship_records: bool,

    /// Whether outgoing adjacency logs should be scanned for adjacency catalog
    /// entries.
    pub include_outgoing_adjacency: bool,

    /// Whether incoming adjacency logs should be scanned for adjacency catalog
    /// entries.
    pub include_incoming_adjacency: bool,

    /// Whether rebuild should stop on the first corrupted or conflicting record.
    pub fail_fast: bool,
}

impl Default for CatalogRebuildOptions {
    fn default() -> Self {
        Self {
            // Include node records.
            include_node_records: true,
            // Include relationship records.
            include_relationship_records: true,
            // Include outgoing adjacency.
            include_outgoing_adjacency: true,
            // Include incoming adjacency.
            include_incoming_adjacency: true,
            // Fail fast.
            fail_fast: true,
        }
    }
}

/// One typed record discovered while scanning persisted storage for catalog rebuild.
///
///
/// - Preserve the source segment for each rebuild input without exposing raw bytes.
/// - Keep node, relationship, outgoing adjacency, and incoming adjacency rebuild
///   inputs distinguishable.
/// - Carry only the metadata required to reconstruct catalog state, not full graph
///   payloads beyond lightweight labels, relationship type, persisted envelope, or
///   adjacency records.
///
///
/// Future implementations should produce these values by streaming append logs and decoding
/// record envelopes one unit at a time. Rebuild code should consume them to rebuild
/// catalog metadata deterministically under the provided rebuild options.
///
/// # Errors
///
///
/// Corrupted bytes, unsupported formats, checksum mismatches, unexpected record
/// kinds, and invalid storage references must be rejected before a record is added
/// to a rebuild batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogRebuildRecord {
    /// Persisted node record envelope discovered in the node append log.
    Node {
        /// Envelope.
        envelope: PersistedRecordEnvelope,
        /// Storage ref.
        storage_ref: StorageRef,
        /// Labels.
        labels: LabelSet,
    },

    /// Persisted relationship record envelope discovered in the relationship append
    /// log.
    Relationship {
        /// Envelope.
        envelope: PersistedRecordEnvelope,
        /// Storage ref.
        storage_ref: StorageRef,
        /// Relationship type.
        relationship_type: RelationshipType,
    },

    /// Persisted outgoing adjacency record discovered in the outgoing adjacency log.
    OutgoingAdjacency {
        /// Record.
        record: PersistedAdjacencyRecord,
        /// Storage ref.
        storage_ref: StorageRef,
    },

    /// Persisted incoming adjacency record discovered in the incoming adjacency log.
    IncomingAdjacency {
        /// Record.
        record: PersistedAdjacencyRecord,
        /// Storage ref.
        storage_ref: StorageRef,
    },
}

/// Counts of records inspected during catalog rebuild.
///
///
/// - Provide diagnostics without exposing full rebuilt catalog internals.
/// - Let later acceptance tests assert that rebuild scanned the expected segments.
/// - Keep observability around rebuild separate from graph payload loading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogRebuildRecordCounts {
    /// Node records.
    pub node_records: usize,
    /// Relationship records.
    pub relationship_records: usize,
    /// Outgoing adjacency records.
    pub outgoing_adjacency_records: usize,
    /// Incoming adjacency records.
    pub incoming_adjacency_records: usize,
}

/// Diagnostic report produced by a catalog rebuild.
///
///
/// - Make rebuild output auditable instead of returning only the reconstructed
///   catalog.
/// - Preserve enough counters for later validation of reopen and recovery flows.
/// - Keep warnings explicit so future implementations can decide whether non-fatal records
///   are allowed when `fail_fast` is false.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogRebuildReport {
    /// Records read.
    pub records_read: CatalogRebuildRecordCounts,
    /// Latest node records reconstructed.
    pub latest_node_records_reconstructed: usize,
    /// Latest relationship records reconstructed.
    pub latest_relationship_records_reconstructed: usize,
    /// Label index entries reconstructed.
    pub label_index_entries_reconstructed: usize,
    /// Relationship type index entries reconstructed.
    pub relationship_type_index_entries_reconstructed: usize,
    /// Adjacency catalog entries reconstructed.
    pub adjacency_catalog_entries_reconstructed: usize,
    /// Warnings.
    pub warnings: Vec<String>,
}

/// Successful catalog rebuild output.
///
///
/// - Return the reconstructed catalog together with rebuild diagnostics.
/// - Keep catalog rebuild independent from writing catalog files back to disk.
/// - Allow later reopen flows to decide whether the rebuilt catalog should be used,
///   persisted, compared, or discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRebuildOutcome {
    /// Catalog.
    pub catalog: GraphCatalog,
    /// Report.
    pub report: CatalogRebuildReport,
}

#[derive(Deserialize)]
struct NodeRebuildLogRecord {
    envelope: PersistedRecordEnvelope,
    storage_ref: Option<StorageRef>,
    labels: Option<LabelSet>,
}

#[derive(Deserialize)]
struct RelationshipRebuildLogRecord {
    envelope: PersistedRecordEnvelope,
    storage_ref: Option<StorageRef>,
    relationship_type: Option<RelationshipType>,
}

#[derive(Deserialize)]
struct AdjacencyRebuildLogRecord {
    record: PersistedAdjacencyRecord,
    storage_ref: Option<StorageRef>,
}

/// Rebuild a storage catalog by scanning persisted append-only logs.
pub fn rebuild_catalog_from_append_logs(
    root: &StorageRoot,
    options: CatalogRebuildOptions,
) -> GraphStorageResult<CatalogRebuildOutcome> {
    let mut records = Vec::new();

    if options.include_node_records {
        records.extend(read_node_record_log_for_catalog_rebuild(root)?);
    }
    if options.include_relationship_records {
        records.extend(read_relationship_record_log_for_catalog_rebuild(root)?);
    }
    if options.include_outgoing_adjacency {
        records.extend(read_outgoing_adjacency_log_for_catalog_rebuild(root)?);
    }
    if options.include_incoming_adjacency {
        records.extend(read_incoming_adjacency_log_for_catalog_rebuild(root)?);
    }

    reconstruct_catalog_from_rebuild_records(&records, options)
}

/// Read node append-log records for catalog rebuild.
pub fn read_node_record_log_for_catalog_rebuild(
    root: &StorageRoot,
) -> GraphStorageResult<Vec<CatalogRebuildRecord>> {
    read_json_lines(
        node_record_log_path(root),
        StorageSegment::NodeRecords,
        |line, storage_ref| {
            if let Ok(record) = serde_json::from_slice::<NodeRebuildLogRecord>(line) {
                validate_rebuild_envelope(
                    &record.envelope,
                    record
                        .storage_ref
                        .as_ref()
                        .unwrap_or(&record.envelope.storage_ref),
                    PersistedRecordKind::Node,
                    StorageSegment::NodeRecords,
                )?;
                return Ok(CatalogRebuildRecord::Node {
                    storage_ref: record
                        .storage_ref
                        .unwrap_or_else(|| record.envelope.storage_ref.clone()),
                    envelope: record.envelope,
                    labels: record.labels.unwrap_or_default(),
                });
            }

            let envelope = decode_envelope_line(
                line,
                storage_ref,
                PersistedRecordKind::Node,
                StorageSegment::NodeRecords,
            )?;
            Ok(CatalogRebuildRecord::Node {
                storage_ref: envelope.storage_ref.clone(),
                envelope,
                labels: Vec::new(),
            })
        },
    )
}

/// Read relationship append-log records for catalog rebuild.
pub fn read_relationship_record_log_for_catalog_rebuild(
    root: &StorageRoot,
) -> GraphStorageResult<Vec<CatalogRebuildRecord>> {
    read_json_lines(
        relationship_record_log_path(root),
        StorageSegment::RelationshipRecords,
        |line, storage_ref| {
            let record =
                serde_json::from_slice::<RelationshipRebuildLogRecord>(line).map_err(|error| {
                    corrupted_record(
                        StorageSegment::RelationshipRecords,
                        Some(storage_ref.clone()),
                        format!(
 "relationship rebuild log requires envelope, storage_ref, and relationship_type metadata: {error}"
 ),
                    )
                })?;
            let relationship_type = record.relationship_type.ok_or_else(|| {
                GraphStorageError::CatalogRebuildFailed {
 stage: "read_relationship_record_log_for_catalog_rebuild",
 reason: "relationship rebuild records require lightweight relationship_type metadata"
 .to_owned(),
 }
            })?;
            let record_storage_ref = record
                .storage_ref
                .unwrap_or_else(|| record.envelope.storage_ref.clone());
            validate_rebuild_envelope(
                &record.envelope,
                &record_storage_ref,
                PersistedRecordKind::Relationship,
                StorageSegment::RelationshipRecords,
            )?;
            Ok(CatalogRebuildRecord::Relationship {
                envelope: record.envelope,
                storage_ref: record_storage_ref,
                relationship_type,
            })
        },
    )
}

/// Read outgoing adjacency records for catalog rebuild.
pub fn read_outgoing_adjacency_log_for_catalog_rebuild(
    root: &StorageRoot,
) -> GraphStorageResult<Vec<CatalogRebuildRecord>> {
    read_adjacency_log_for_catalog_rebuild(
        outgoing_adjacency_log_path(root),
        StorageSegment::OutgoingAdjacency,
        AdjacencyDirection::Outgoing,
    )
}

/// Read incoming adjacency records for catalog rebuild.
pub fn read_incoming_adjacency_log_for_catalog_rebuild(
    root: &StorageRoot,
) -> GraphStorageResult<Vec<CatalogRebuildRecord>> {
    read_adjacency_log_for_catalog_rebuild(
        incoming_adjacency_log_path(root),
        StorageSegment::IncomingAdjacency,
        AdjacencyDirection::Incoming,
    )
}

/// Reconstruct a fresh catalog from already-decoded rebuild records.
pub fn reconstruct_catalog_from_rebuild_records(
    records: &[CatalogRebuildRecord],
    options: CatalogRebuildOptions,
) -> GraphStorageResult<CatalogRebuildOutcome> {
    detect_corrupted_catalog_rebuild_records(records)?;
    detect_duplicate_latest_record_conflicts_for_rebuild(records)?;

    let mut catalog = GraphCatalog::default();

    if options.include_node_records {
        reconstruct_latest_node_records_from_rebuild_records(&mut catalog, records)?;
        reconstruct_label_indexes_from_rebuild_records(&mut catalog, records)?;
    }
    if options.include_relationship_records {
        reconstruct_latest_relationship_records_from_rebuild_records(&mut catalog, records)?;
        reconstruct_relationship_type_indexes_from_rebuild_records(&mut catalog, records)?;
    }
    if options.include_outgoing_adjacency || options.include_incoming_adjacency {
        reconstruct_adjacency_catalog_entries_from_rebuild_records(&mut catalog, records)?;
    }

    let report = CatalogRebuildReport {
        records_read: count_rebuild_records(records),
        latest_node_records_reconstructed: catalog.latest_node_records.len(),
        latest_relationship_records_reconstructed: catalog.latest_relationship_records.len(),
        label_index_entries_reconstructed: catalog
            .metadata_indexes
            .labels
            .values()
            .map(|entry| entry.nodes.len())
            .sum(),
        relationship_type_index_entries_reconstructed: catalog
            .metadata_indexes
            .relationship_types
            .values()
            .map(|entry| entry.relationships.len())
            .sum(),
        adjacency_catalog_entries_reconstructed: count_adjacency_records(records),
        warnings: Vec::new(),
    };

    Ok(CatalogRebuildOutcome { catalog, report })
}

/// Reconstruct latest node catalog entries from rebuild records.
pub fn reconstruct_latest_node_records_from_rebuild_records(
    catalog: &mut GraphCatalog,
    records: &[CatalogRebuildRecord],
) -> GraphStorageResult<()> {
    for record in records {
        if let CatalogRebuildRecord::Node {
            envelope,
            storage_ref,
            ..
        } = record
            && record_is_current(envelope)?
        {
            index_appended_node_record(catalog, envelope, storage_ref.clone())?;
        }
    }
    Ok(())
}

/// Reconstruct latest relationship catalog entries from rebuild records.
pub fn reconstruct_latest_relationship_records_from_rebuild_records(
    catalog: &mut GraphCatalog,
    records: &[CatalogRebuildRecord],
) -> GraphStorageResult<()> {
    for record in records {
        if let CatalogRebuildRecord::Relationship {
            envelope,
            storage_ref,
            ..
        } = record
            && record_is_current(envelope)?
        {
            index_appended_relationship_record(catalog, envelope, storage_ref.clone())?;
        }
    }
    Ok(())
}

/// Reconstruct label indexes from rebuilt node records.
pub fn reconstruct_label_indexes_from_rebuild_records(
    catalog: &mut GraphCatalog,
    records: &[CatalogRebuildRecord],
) -> GraphStorageResult<()> {
    for record in records {
        if let CatalogRebuildRecord::Node {
            envelope,
            storage_ref,
            labels,
        } = record
        {
            if !record_is_current(envelope)? {
                continue;
            }
            let PersistedRecordId::Node(node_id) = &envelope.record_id else {
                return Err(corrupted_record(
                    StorageSegment::NodeRecords,
                    Some(storage_ref.clone()),
                    "node rebuild record must carry a node record id",
                ));
            };
            index_node_labels(
                catalog,
                labels,
                LabelIndexNodeMetadata {
                    // Node id.
                    node_id: node_id.clone(),
                    // Latest storage ref.
                    latest_storage_ref: Some(storage_ref.clone()),
                    // Graph record version.
                    graph_record_version: envelope.graph_record_version.clone(),
                },
            )?;
        }
    }
    Ok(())
}

/// Reconstruct relationship-type indexes from rebuilt relationship records.
pub fn reconstruct_relationship_type_indexes_from_rebuild_records(
    catalog: &mut GraphCatalog,
    records: &[CatalogRebuildRecord],
) -> GraphStorageResult<()> {
    for record in records {
        if let CatalogRebuildRecord::Relationship {
            envelope,
            storage_ref,
            relationship_type,
        } = record
        {
            if !record_is_current(envelope)? {
                continue;
            }
            let PersistedRecordId::Relationship(relationship_id) = &envelope.record_id else {
                return Err(corrupted_record(
                    StorageSegment::RelationshipRecords,
                    Some(storage_ref.clone()),
                    "relationship rebuild record must carry a relationship record id",
                ));
            };
            index_relationship_type(
                catalog,
                relationship_type,
                RelationshipTypeIndexRelationshipMetadata {
                    // Relationship id.
                    relationship_id: relationship_id.clone(),
                    // Latest storage ref.
                    latest_storage_ref: Some(storage_ref.clone()),
                    // Graph record version.
                    graph_record_version: envelope.graph_record_version.clone(),
                },
            )?;
        }
    }
    Ok(())
}

/// Reconstruct adjacency catalog entries from persisted adjacency records.
pub fn reconstruct_adjacency_catalog_entries_from_rebuild_records(
    catalog: &mut GraphCatalog,
    records: &[CatalogRebuildRecord],
) -> GraphStorageResult<()> {
    for record in records {
        match record {
            CatalogRebuildRecord::OutgoingAdjacency {
                record,
                storage_ref,
            } => index_adjacency_rebuild_record(
                catalog,
                record,
                storage_ref,
                AdjacencyDirection::Outgoing,
                StorageSegment::OutgoingAdjacency,
            )?,
            CatalogRebuildRecord::IncomingAdjacency {
                record,
                storage_ref,
            } => index_adjacency_rebuild_record(
                catalog,
                record,
                storage_ref,
                AdjacencyDirection::Incoming,
                StorageSegment::IncomingAdjacency,
            )?,
            _ => {}
        }
    }
    Ok(())
}

/// Detect duplicate latest-record conflicts during catalog rebuild.
pub fn detect_duplicate_latest_record_conflicts_for_rebuild(
    records: &[CatalogRebuildRecord],
) -> GraphStorageResult<()> {
    let mut latest: HashMap<PersistedRecordId, LatestRecordCatalogEntry> = HashMap::new();

    for record in records {
        let Some(entry) = latest_entry_candidate(record)? else {
            continue;
        };
        if !latest_entry_is_current(&entry)? {
            continue;
        }
        if let Some(existing) = latest.get(&entry.record_id).cloned() {
            check_duplicate_latest_record_conflict(&entry.record_id, &existing, &entry)?;
        }
        latest.insert(entry.record_id.clone(), entry);
    }

    Ok(())
}

/// Detect corrupted records before accepting a rebuilt catalog.
pub fn detect_corrupted_catalog_rebuild_records(
    records: &[CatalogRebuildRecord],
) -> GraphStorageResult<()> {
    for record in records {
        match record {
            CatalogRebuildRecord::Node {
                envelope,
                storage_ref,
                labels,
            } => {
                validate_rebuild_envelope(
                    envelope,
                    storage_ref,
                    PersistedRecordKind::Node,
                    StorageSegment::NodeRecords,
                )?;
                for label in labels {
                    if label.trim().is_empty() {
                        return Err(corrupted_record(
                            StorageSegment::NodeRecords,
                            Some(storage_ref.clone()),
                            "node rebuild label must not be empty",
                        ));
                    }
                }
            }
            CatalogRebuildRecord::Relationship {
                envelope,
                storage_ref,
                ..
            } => validate_rebuild_envelope(
                envelope,
                storage_ref,
                PersistedRecordKind::Relationship,
                StorageSegment::RelationshipRecords,
            )?,
            CatalogRebuildRecord::OutgoingAdjacency {
                record,
                storage_ref,
            } => validate_adjacency_rebuild_record(
                record,
                storage_ref,
                AdjacencyDirection::Outgoing,
                StorageSegment::OutgoingAdjacency,
            )?,
            CatalogRebuildRecord::IncomingAdjacency {
                record,
                storage_ref,
            } => validate_adjacency_rebuild_record(
                record,
                storage_ref,
                AdjacencyDirection::Incoming,
                StorageSegment::IncomingAdjacency,
            )?,
        }
    }
    Ok(())
}

/// Convert an adjacency direction into the rebuild segment expected for catalog
/// diagnostics.
pub fn catalog_rebuild_adjacency_direction_label(
    direction: AdjacencyDirection,
) -> GraphStorageResult<&'static str> {
    match direction {
        AdjacencyDirection::Outgoing => Ok("outgoing"),
        AdjacencyDirection::Incoming => Ok("incoming"),
    }
}

fn read_adjacency_log_for_catalog_rebuild(
    path: PathBuf,
    segment: StorageSegment,
    direction: AdjacencyDirection,
) -> GraphStorageResult<Vec<CatalogRebuildRecord>> {
    read_json_lines(path, segment.clone(), |line, storage_ref| {
        let parsed = serde_json::from_slice::<AdjacencyRebuildLogRecord>(line);
        let (record, record_storage_ref) = match parsed {
            Ok(parsed) => {
                let record_storage_ref = parsed.storage_ref.unwrap_or_else(|| {
                    parsed
                        .record
                        .storage_ref
                        .clone()
                        .unwrap_or_else(|| storage_ref.clone())
                });
                (parsed.record, record_storage_ref)
            }
            Err(_) => {
                let record: PersistedAdjacencyRecord =
                    serde_json::from_slice(line).map_err(|error| {
                        corrupted_record(
                            segment.clone(),
                            Some(storage_ref.clone()),
                            format!("failed to decode adjacency rebuild record: {error}"),
                        )
                    })?;
                let record_storage_ref = record
                    .storage_ref
                    .clone()
                    .unwrap_or_else(|| storage_ref.clone());
                (record, record_storage_ref)
            }
        };
        validate_adjacency_rebuild_record(
            &record,
            &record_storage_ref,
            direction,
            segment.clone(),
        )?;
        match direction {
            AdjacencyDirection::Outgoing => Ok(CatalogRebuildRecord::OutgoingAdjacency {
                record,
                storage_ref: record_storage_ref,
            }),
            AdjacencyDirection::Incoming => Ok(CatalogRebuildRecord::IncomingAdjacency {
                record,
                storage_ref: record_storage_ref,
            }),
        }
    })
}

fn read_json_lines<F>(
    path: PathBuf,
    segment: StorageSegment,
    mut decode: F,
) -> GraphStorageResult<Vec<CatalogRebuildRecord>>
where
    F: FnMut(&[u8], &StorageRef) -> GraphStorageResult<CatalogRebuildRecord>,
{
    let bytes = fs::read(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GraphStorageError::CatalogRebuildSourceMissing {
                segment: segment.clone(),
                path: path.clone(),
            }
        } else {
            GraphStorageError::IoOperationFailed {
                operation: "read_catalog_rebuild_log",
                path: Some(path.clone()),
                message: error.to_string(),
            }
        }
    })?;

    let mut records = Vec::new();
    let mut offset = 0_u64;
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let length = line.len() as u64;
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            offset += length;
            continue;
        }
        let storage_ref = StorageRef {
            segment: segment.clone(),
            offset,
            length,
            checksum: None,
        };
        records.push(decode(line, &storage_ref)?);
        offset += length;
    }

    Ok(records)
}

fn decode_envelope_line(
    line: &[u8],
    storage_ref: &StorageRef,
    expected_kind: PersistedRecordKind,
    expected_segment: StorageSegment,
) -> GraphStorageResult<PersistedRecordEnvelope> {
    let envelope: PersistedRecordEnvelope = serde_json::from_slice(line).map_err(|error| {
        corrupted_record(
            expected_segment.clone(),
            Some(storage_ref.clone()),
            format!("failed to decode persisted envelope: {error}"),
        )
    })?;
    validate_rebuild_envelope(
        &envelope,
        &envelope.storage_ref,
        expected_kind,
        expected_segment,
    )?;
    Ok(envelope)
}

fn validate_rebuild_envelope(
    envelope: &PersistedRecordEnvelope,
    storage_ref: &StorageRef,
    expected_kind: PersistedRecordKind,
    expected_segment: StorageSegment,
) -> GraphStorageResult<()> {
    validate_persisted_record_envelope(envelope).map_err(|error| {
        corrupted_record(
            expected_segment.clone(),
            Some(storage_ref.clone()),
            format!("invalid persisted envelope during rebuild: {error}"),
        )
    })?;
    validate_storage_ref_segment(storage_ref, expected_segment.clone())?;
    if envelope.kind != expected_kind {
        return Err(GraphStorageError::UnexpectedRecordKind {
            expected: expected_kind,
            actual: envelope.kind,
        });
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
                "catalog rebuild record requires {:?}, got {:?}",
                expected_segment, storage_ref.segment
            ),
        });
    }
    Ok(())
}

fn validate_adjacency_rebuild_record(
    record: &PersistedAdjacencyRecord,
    storage_ref: &StorageRef,
    expected_direction: AdjacencyDirection,
    expected_segment: StorageSegment,
) -> GraphStorageResult<()> {
    validate_storage_ref_segment(storage_ref, expected_segment.clone()).map_err(|error| {
        corrupted_record(
            expected_segment.clone(),
            Some(storage_ref.clone()),
            format!("invalid adjacency storage reference during rebuild: {error}"),
        )
    })?;

    if record.direction != expected_direction {
        return Err(corrupted_record(
            expected_segment.clone(),
            Some(storage_ref.clone()),
            format!(
                "adjacency direction mismatch during rebuild: expected {:?}, got {:?}",
                expected_direction, record.direction
            ),
        ));
    }

    if let Some(record_ref) = &record.storage_ref {
        validate_storage_ref_segment(record_ref, expected_segment.clone()).map_err(|error| {
            corrupted_record(
                expected_segment.clone(),
                Some(record_ref.clone()),
                format!("invalid embedded adjacency storage reference during rebuild: {error}"),
            )
        })?;
    }

    for entry in &record.entries {
        if entry.direction != expected_direction {
            return Err(corrupted_record(
                expected_segment.clone(),
                Some(storage_ref.clone()),
                format!(
                    "adjacency entry direction mismatch during rebuild: expected {:?}, got {:?}",
                    expected_direction, entry.direction
                ),
            ));
        }
        let owner_matches = match expected_direction {
            AdjacencyDirection::Outgoing => entry.source_node_id == record.owner_node_id,
            AdjacencyDirection::Incoming => entry.target_node_id == record.owner_node_id,
        };
        if !owner_matches {
            return Err(corrupted_record(
                expected_segment.clone(),
                Some(storage_ref.clone()),
                "adjacency entry endpoints do not match owner node during rebuild",
            ));
        }
    }

    Ok(())
}

fn index_adjacency_rebuild_record(
    catalog: &mut GraphCatalog,
    record: &PersistedAdjacencyRecord,
    storage_ref: &StorageRef,
    expected_direction: AdjacencyDirection,
    expected_segment: StorageSegment,
) -> GraphStorageResult<()> {
    validate_adjacency_rebuild_record(record, storage_ref, expected_direction, expected_segment)?;
    let entry = AdjacencyStorageCatalogEntry {
        owner_node_id: record.owner_node_id.clone(),
        direction: expected_direction,
        storage_ref: storage_ref.clone(),
        relationship_count: record.entries.len(),
        relationship_types: relationship_types_for_entries(&record.entries),
    };
    match expected_direction {
        AdjacencyDirection::Outgoing => index_outgoing_adjacency_storage_ref(catalog, entry),
        AdjacencyDirection::Incoming => index_incoming_adjacency_storage_ref(catalog, entry),
    }
}

fn relationship_types_for_entries(
    entries: &[crate::PersistedAdjacencyEntry],
) -> Vec<RelationshipType> {
    let mut seen = HashSet::new();
    let mut relationship_types = Vec::new();
    for entry in entries {
        if seen.insert(entry.relationship_type.clone()) {
            relationship_types.push(entry.relationship_type.clone());
        }
    }
    relationship_types
}

fn latest_entry_candidate(
    record: &CatalogRebuildRecord,
) -> GraphStorageResult<Option<LatestRecordCatalogEntry>> {
    match record {
        CatalogRebuildRecord::Node {
            envelope,
            storage_ref,
            ..
        } => Ok(Some(LatestRecordCatalogEntry {
            record_id: envelope.record_id.clone(),
            kind: PersistedRecordKind::Node,
            graph_record_version: envelope.graph_record_version.clone(),
            storage_ref: storage_ref.clone(),
        })),
        CatalogRebuildRecord::Relationship {
            envelope,
            storage_ref,
            ..
        } => Ok(Some(LatestRecordCatalogEntry {
            record_id: envelope.record_id.clone(),
            kind: PersistedRecordKind::Relationship,
            graph_record_version: envelope.graph_record_version.clone(),
            storage_ref: storage_ref.clone(),
        })),
        _ => Ok(None),
    }
}

fn latest_entry_is_current(entry: &LatestRecordCatalogEntry) -> GraphStorageResult<bool> {
    match &entry.graph_record_version {
        Some(GraphRecordVersion::Node { current, .. })
        | Some(GraphRecordVersion::Relationship { current, .. }) => Ok(*current),
        None => Err(GraphStorageError::InvalidEnvelope {
            reason: "latest rebuild candidate requires graph record version metadata".to_owned(),
        }),
    }
}

fn record_is_current(envelope: &PersistedRecordEnvelope) -> GraphStorageResult<bool> {
    match &envelope.graph_record_version {
        Some(GraphRecordVersion::Node { current, .. })
        | Some(GraphRecordVersion::Relationship { current, .. }) => Ok(*current),
        None => Err(GraphStorageError::InvalidEnvelope {
            reason: "rebuild record requires graph record version metadata".to_owned(),
        }),
    }
}

fn count_rebuild_records(records: &[CatalogRebuildRecord]) -> CatalogRebuildRecordCounts {
    let mut counts = CatalogRebuildRecordCounts::default();
    for record in records {
        match record {
            CatalogRebuildRecord::Node { .. } => counts.node_records += 1,
            CatalogRebuildRecord::Relationship { .. } => counts.relationship_records += 1,
            CatalogRebuildRecord::OutgoingAdjacency { .. } => {
                counts.outgoing_adjacency_records += 1
            }
            CatalogRebuildRecord::IncomingAdjacency { .. } => {
                counts.incoming_adjacency_records += 1
            }
        }
    }
    counts
}

fn count_adjacency_records(records: &[CatalogRebuildRecord]) -> usize {
    records
        .iter()
        .filter(|record| {
            matches!(
                record,
                CatalogRebuildRecord::OutgoingAdjacency { .. }
                    | CatalogRebuildRecord::IncomingAdjacency { .. }
            )
        })
        .count()
}

fn node_record_log_path(root: &StorageRoot) -> PathBuf {
    root.path().join("nodes").join("node_records.log")
}

fn relationship_record_log_path(root: &StorageRoot) -> PathBuf {
    root.path()
        .join("relationships")
        .join("relationship_records.log")
}

fn outgoing_adjacency_log_path(root: &StorageRoot) -> PathBuf {
    root.path().join("adjacency").join("outgoing_adjacency.log")
}

fn incoming_adjacency_log_path(root: &StorageRoot) -> PathBuf {
    root.path().join("adjacency").join("incoming_adjacency.log")
}

fn corrupted_record(
    segment: StorageSegment,
    storage_ref: Option<StorageRef>,
    reason: impl Into<String>,
) -> GraphStorageError {
    GraphStorageError::CatalogRebuildCorruptedRecord {
        segment,
        storage_ref: storage_ref.map(Box::new),
        reason: reason.into(),
    }
}

#[allow(dead_code)]
fn path_display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{NodeId, NodeVersionId, RelationshipId, RelationshipVersionId};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{
        AdjacencyStorageLookupMode, CatalogIndexLookupMode, GraphId, PersistedAdjacencyEntry,
        RecordChecksum, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
        create_storage_root, resolve_incoming_adjacency_storage_ref,
        resolve_latest_node_storage_ref, resolve_latest_relationship_storage_ref,
        resolve_node_ids_by_label, resolve_outgoing_adjacency_storage_ref,
        resolve_relationship_ids_by_type,
    };

    fn unique_temp_path(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "intelligence_graph_engine_catalog_rebuild_unit_{test_name}_{}_{}",
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
                value: "graph--catalog-rebuild-unit".to_owned(),
            },
            // Created at.
            created_at: StorageTimestamp {
                // Value.
                value: "2026-07-07T00:00:00Z".to_owned(),
            },
            // Updated at.
            updated_at: StorageTimestamp {
                // Value.
                value: "2026-07-07T00:00:00Z".to_owned(),
            },
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
        }
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

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node id should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship id should be valid")
    }

    fn relationship_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
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
        id: &NodeId,
        version: GraphRecordVersion,
        reference: StorageRef,
    ) -> PersistedRecordEnvelope {
        PersistedRecordEnvelope {
            // Record id.
            record_id: PersistedRecordId::Node(id.clone()),
            // Kind.
            kind: PersistedRecordKind::Node,
            // Storage version.
            storage_version: StorageVersion::V1,
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
            // Graph record version.
            graph_record_version: Some(version),
            // Storage ref.
            storage_ref: reference,
            // Record checksum.
            record_checksum: Some(checksum("node-record")),
        }
    }

    fn relationship_envelope(
        id: &RelationshipId,
        version: GraphRecordVersion,
        reference: StorageRef,
    ) -> PersistedRecordEnvelope {
        PersistedRecordEnvelope {
            // Record id.
            record_id: PersistedRecordId::Relationship(id.clone()),
            // Kind.
            kind: PersistedRecordKind::Relationship,
            // Storage version.
            storage_version: StorageVersion::V1,
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
            // Graph record version.
            graph_record_version: Some(version),
            // Storage ref.
            storage_ref: reference,
            // Record checksum.
            record_checksum: Some(checksum("relationship-record")),
        }
    }

    fn node_rebuild_record(
        id: &NodeId,
        reference: StorageRef,
        labels: &[&str],
    ) -> CatalogRebuildRecord {
        CatalogRebuildRecord::Node {
            envelope: node_envelope(
                id,
                node_version("node-version--1", 1, true, None),
                reference.clone(),
            ),
            storage_ref: reference,
            labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        }
    }

    fn relationship_rebuild_record(
        id: &RelationshipId,
        reference: StorageRef,
        rel_type: RelationshipType,
    ) -> CatalogRebuildRecord {
        CatalogRebuildRecord::Relationship {
            envelope: relationship_envelope(
                id,
                relationship_version("relationship-version--1", 1, true, None),
                reference.clone(),
            ),
            storage_ref: reference,
            relationship_type: rel_type,
        }
    }

    fn latest_node_entry(id: &NodeId, reference: StorageRef) -> LatestRecordCatalogEntry {
        LatestRecordCatalogEntry {
            // Record id.
            record_id: PersistedRecordId::Node(id.clone()),
            // Kind.
            kind: PersistedRecordKind::Node,
            // Graph record version.
            graph_record_version: Some(node_version("node-version--1", 1, true, None)),
            // Storage ref.
            storage_ref: reference,
        }
    }

    fn adjacency_entry(
        relationship_id: &RelationshipId,
        source_node_id: &NodeId,
        target_node_id: &NodeId,
        rel_type: RelationshipType,
        direction: AdjacencyDirection,
    ) -> PersistedAdjacencyEntry {
        PersistedAdjacencyEntry {
            // Relationship id.
            relationship_id: relationship_id.clone(),
            // Source node id.
            source_node_id: source_node_id.clone(),
            // Target node id.
            target_node_id: target_node_id.clone(),
            // Relationship type.
            relationship_type: rel_type,
            direction,
            // Relationship storage ref.
            relationship_storage_ref: Some(storage_ref(StorageSegment::RelationshipRecords, 600)),
            // Source node storage ref.
            source_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 700)),
            // Target node storage ref.
            target_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 800)),
        }
    }

    fn adjacency_record(
        owner_node_id: &NodeId,
        direction: AdjacencyDirection,
        entries: Vec<PersistedAdjacencyEntry>,
        reference: StorageRef,
    ) -> PersistedAdjacencyRecord {
        PersistedAdjacencyRecord {
            // Owner node id.
            owner_node_id: owner_node_id.clone(),
            direction,
            entries,
            // Storage ref.
            storage_ref: Some(reference),
        }
    }

    //
    // Specify the default rebuild profile. A default rebuild must cover every
    // catalog-owned segment needed for recovery while failing fast on
    // corruption or conflicts.
    //
    // Given no caller-provided rebuild options,
    // when the default options are constructed,
    // then node records, relationship records, outgoing adjacency, incoming
    // adjacency, and fail-fast behavior should all be enabled.
    #[test]
    fn catalog_rebuild_options_default_scans_all_recoverable_catalog_sources() {
        let options = CatalogRebuildOptions::default();

        assert!(options.include_node_records);
        assert!(options.include_relationship_records);
        assert!(options.include_outgoing_adjacency);
        assert!(options.include_incoming_adjacency);
        assert!(options.fail_fast);
    }

    //
    // Specify the stable diagnostic labels used when rebuild errors or scan reports
    // need to refer to adjacency direction without exposing filesystem paths.
    //
    // Given outgoing and incoming adjacency directions,
    // when each direction is converted into a rebuild diagnostic label,
    // then the labels should be deterministic and human-readable.
    #[test]
    fn catalog_rebuild_adjacency_direction_label_maps_directions() {
        assert_eq!(
            catalog_rebuild_adjacency_direction_label(AdjacencyDirection::Outgoing),
            Ok("outgoing")
        );
        assert_eq!(
            catalog_rebuild_adjacency_direction_label(AdjacencyDirection::Incoming),
            Ok("incoming")
        );
    }

    //
    // Specify latest node lookup reconstruction from decoded node rebuild records.
    // This protects reopen behavior where catalog files are missing but append logs
    // still contain recoverable node metadata.
    //
    // Given a decoded current node rebuild record with a stable `NodeId` and
    // storage reference,
    // when latest node records are reconstructed into an empty catalog,
    // then resolving that `NodeId` should return the persisted storage reference.
    #[test]
    fn reconstruct_latest_node_records_restores_node_id_lookup() {
        let mut catalog = GraphCatalog::default();
        let id = node_id("node--campaign-1");
        let reference = storage_ref(StorageSegment::NodeRecords, 100);
        let records = vec![node_rebuild_record(&id, reference.clone(), &["Campaign"])];

        reconstruct_latest_node_records_from_rebuild_records(&mut catalog, &records)
            .expect("latest node records should be rebuilt from node append logs");

        assert_eq!(
            resolve_latest_node_storage_ref(&catalog, &id),
            Ok(reference)
        );
    }

    //
    // Specify latest relationship lookup reconstruction from decoded relationship
    // rebuild records. This protects reopen behavior where relationship catalog
    // entries are recovered from append-only relationship logs.
    //
    // Given a decoded current relationship rebuild record with a stable
    // `RelationshipId` and storage reference,
    // when latest relationship records are reconstructed into an empty catalog,
    // then resolving that `RelationshipId` should return the persisted storage
    // reference.
    #[test]
    fn reconstruct_latest_relationship_records_restores_relationship_id_lookup() {
        let mut catalog = GraphCatalog::default();
        let id = relationship_id("relationship--promotes-1");
        let reference = storage_ref(StorageSegment::RelationshipRecords, 200);
        let records = vec![relationship_rebuild_record(
            &id,
            reference.clone(),
            relationship_type("PROMOTES"),
        )];

        reconstruct_latest_relationship_records_from_rebuild_records(&mut catalog, &records)
            .expect("latest relationship records should be rebuilt from relationship append logs");

        assert_eq!(
            resolve_latest_relationship_storage_ref(&catalog, &id),
            Ok(reference)
        );
    }

    //
    // Specify that full catalog reconstruction restores latest lookups, label
    // indexes, relationship-type indexes, and diagnostic counters from decoded
    // rebuild records without requiring callers to load the full graph.
    //
    // Given decoded current node and relationship rebuild records carrying
    // lightweight label/type metadata,
    // when the catalog is reconstructed from those records,
    // then latest lookups, label lookups, relationship-type lookups, and report
    // counters should all reflect the recovered metadata.
    #[test]
    fn reconstruct_catalog_from_records_restores_latest_and_metadata_indexes() {
        let node_id = node_id("node--campaign-1");
        let relationship_id = relationship_id("relationship--promotes-1");
        let node_ref = storage_ref(StorageSegment::NodeRecords, 300);
        let relationship_ref = storage_ref(StorageSegment::RelationshipRecords, 400);
        let rel_type = relationship_type("PROMOTES");
        let records = vec![
            node_rebuild_record(&node_id, node_ref.clone(), &["Campaign", "FIMI"]),
            relationship_rebuild_record(
                &relationship_id,
                relationship_ref.clone(),
                rel_type.clone(),
            ),
        ];

        let outcome =
            reconstruct_catalog_from_rebuild_records(&records, CatalogRebuildOptions::default())
                .expect("catalog should be reconstructed from decoded rebuild records");

        assert_eq!(
            resolve_latest_node_storage_ref(&outcome.catalog, &node_id),
            Ok(node_ref)
        );
        assert_eq!(
            resolve_latest_relationship_storage_ref(&outcome.catalog, &relationship_id),
            Ok(relationship_ref)
        );
        assert_eq!(
            resolve_node_ids_by_label(&outcome.catalog, "Campaign", CatalogIndexLookupMode::Strict,),
            Ok(vec![node_id.clone()])
        );
        assert_eq!(
            resolve_relationship_ids_by_type(
                &outcome.catalog,
                &rel_type,
                CatalogIndexLookupMode::Strict,
            ),
            Ok(vec![relationship_id.clone()])
        );
        assert_eq!(outcome.report.records_read.node_records, 1);
        assert_eq!(outcome.report.records_read.relationship_records, 1);
        assert_eq!(outcome.report.latest_node_records_reconstructed, 1);
        assert_eq!(outcome.report.latest_relationship_records_reconstructed, 1);
        assert_eq!(outcome.report.label_index_entries_reconstructed, 2);
        assert_eq!(
            outcome.report.relationship_type_index_entries_reconstructed,
            1
        );
    }

    //
    // Specify adjacency catalog reconstruction from decoded outgoing and incoming
    // adjacency records. This ensures rebuild can restore warm-frontier metadata
    // without loading full node or relationship payloads.
    //
    // Given known owner nodes and decoded outgoing/incoming adjacency rebuild
    // records,
    // when adjacency catalog entries are reconstructed,
    // then outgoing and incoming adjacency storage references should resolve by
    // owner node ID and direction.
    #[test]
    fn reconstruct_adjacency_catalog_entries_restores_directional_adjacency_refs() {
        let source = node_id("node--source-1");
        let target = node_id("node--target-1");
        let relationship = relationship_id("relationship--amplifies-1");
        let rel_type = relationship_type("AMPLIFIES");
        let outgoing_ref = storage_ref(StorageSegment::OutgoingAdjacency, 500);
        let incoming_ref = storage_ref(StorageSegment::IncomingAdjacency, 600);
        let outgoing_entry = adjacency_entry(
            &relationship,
            &source,
            &target,
            rel_type.clone(),
            AdjacencyDirection::Outgoing,
        );
        let incoming_entry = adjacency_entry(
            &relationship,
            &source,
            &target,
            rel_type,
            AdjacencyDirection::Incoming,
        );
        let mut catalog = GraphCatalog::default();
        catalog.latest_node_records.insert(
            source.clone(),
            latest_node_entry(&source, storage_ref(StorageSegment::NodeRecords, 700)),
        );
        catalog.latest_node_records.insert(
            target.clone(),
            latest_node_entry(&target, storage_ref(StorageSegment::NodeRecords, 800)),
        );
        let records = vec![
            CatalogRebuildRecord::OutgoingAdjacency {
                record: adjacency_record(
                    &source,
                    AdjacencyDirection::Outgoing,
                    vec![outgoing_entry],
                    outgoing_ref.clone(),
                ),
                storage_ref: outgoing_ref.clone(),
            },
            CatalogRebuildRecord::IncomingAdjacency {
                record: adjacency_record(
                    &target,
                    AdjacencyDirection::Incoming,
                    vec![incoming_entry],
                    incoming_ref.clone(),
                ),
                storage_ref: incoming_ref.clone(),
            },
        ];

        reconstruct_adjacency_catalog_entries_from_rebuild_records(&mut catalog, &records)
            .expect("adjacency catalog entries should be rebuilt from adjacency records");

        assert_eq!(
            resolve_outgoing_adjacency_storage_ref(
                &catalog,
                &source,
                AdjacencyStorageLookupMode::Strict,
            ),
            Ok(Some(outgoing_ref))
        );
        assert_eq!(
            resolve_incoming_adjacency_storage_ref(
                &catalog,
                &target,
                AdjacencyStorageLookupMode::Strict,
            ),
            Ok(Some(incoming_ref))
        );
    }

    //
    // Specify duplicate latest-record conflict detection for rebuild. Rebuild must
    // not silently choose between two current records that both claim to be latest
    // for the same stable graph ID.
    //
    // Given two current node rebuild records with the same `NodeId` and different
    // storage references but no normal successor chain,
    // when duplicate latest conflicts are detected,
    // then the rebuild should fail with an explicit duplicate-latest conflict.
    #[test]
    fn detect_duplicate_latest_record_conflicts_reports_conflicting_current_records() {
        let id = node_id("node--duplicate-current");
        let first_ref = storage_ref(StorageSegment::NodeRecords, 900);
        let second_ref = storage_ref(StorageSegment::NodeRecords, 1000);
        let records = vec![
            CatalogRebuildRecord::Node {
                envelope: node_envelope(
                    &id,
                    node_version("node-version--1", 1, true, None),
                    first_ref.clone(),
                ),
                storage_ref: first_ref.clone(),
                labels: vec!["Campaign".to_owned()],
            },
            CatalogRebuildRecord::Node {
                envelope: node_envelope(
                    &id,
                    node_version("node-version--conflict", 1, true, None),
                    second_ref.clone(),
                ),
                storage_ref: second_ref.clone(),
                labels: vec!["Campaign".to_owned()],
            },
        ];

        let error = detect_duplicate_latest_record_conflicts_for_rebuild(&records)
            .expect_err("conflicting latest records should fail rebuild validation");

        assert!(matches!(
        error,
        GraphStorageError::DuplicateLatestRecordConflict {
        record_id,
        existing_ref,
        conflicting_ref,
        } if record_id == PersistedRecordId::Node(id)
        && existing_ref.as_ref() == &first_ref
        && conflicting_ref.as_ref() == &second_ref
        ));
    }

    //
    // Specify corrupted adjacency detection before accepting a rebuilt catalog. A
    // record discovered in an outgoing adjacency source must not carry incoming
    // adjacency metadata.
    //
    // Given an outgoing rebuild record whose persisted adjacency payload declares
    // incoming direction,
    // when corrupted rebuild records are detected,
    // then rebuild validation should fail with an explicit corrupted-record error.
    #[test]
    fn detect_corrupted_catalog_rebuild_records_rejects_direction_mismatch() {
        let owner = node_id("node--owner-1");
        let other = node_id("node--other-1");
        let relationship = relationship_id("relationship--bad-direction-1");
        let reference = storage_ref(StorageSegment::OutgoingAdjacency, 1100);
        let entry = adjacency_entry(
            &relationship,
            &other,
            &owner,
            relationship_type("AMPLIFIES"),
            AdjacencyDirection::Incoming,
        );
        let records = vec![CatalogRebuildRecord::OutgoingAdjacency {
            record: adjacency_record(
                &owner,
                AdjacencyDirection::Incoming,
                vec![entry],
                reference.clone(),
            ),
            storage_ref: reference.clone(),
        }];

        let error = detect_corrupted_catalog_rebuild_records(&records)
            .expect_err("direction mismatch should be reported as corrupted rebuild input");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildCorruptedRecord {
        segment: StorageSegment::OutgoingAdjacency,
        storage_ref: Some(actual_ref),
        ..
        } if actual_ref.as_ref() == &reference
        ));
    }

    #[test]
    fn read_node_record_log_reports_missing_source_when_log_file_absent() {
        let path = unique_temp_path("missing_node_log");
        fs::create_dir_all(&path).expect("storage root directory should be created");
        let root = StorageRoot { path: path.clone() };

        let error = read_node_record_log_for_catalog_rebuild(&root)
            .expect_err("missing node record log should surface typed source missing error");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
        if segment == StorageSegment::NodeRecords
        ));
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_node_record_log_decodes_envelope_lines_and_skips_blank_lines() {
        let path = unique_temp_path("node_log_envelope_fallback");
        let root = create_storage_root(path.clone(), manifest())
            .expect("storage root should be created with manifest");

        let node = node_id("node--fallback-1");
        let reference = storage_ref(StorageSegment::NodeRecords, 42);
        let envelope = node_envelope(
            &node,
            node_version("node-version--fallback-1", 1, true, None),
            reference.clone(),
        );

        let log_path = root.path().join("nodes").join("node_records.log");
        fs::create_dir_all(log_path.parent().expect("node log should have a parent"))
            .expect("nodes directory should exist");
        let mut bytes = serde_json::to_vec(&envelope).expect("envelope should serialize");
        bytes.extend_from_slice(b"\n \n");
        fs::write(&log_path, bytes).expect("node log should be written");

        let records = read_node_record_log_for_catalog_rebuild(&root)
            .expect("envelope lines should be decoded for rebuild");

        assert_eq!(records.len(), 1);
        assert!(matches!(
        &records[0],
        CatalogRebuildRecord::Node {
        storage_ref,
        labels,
        ..
        } if storage_ref == &reference && labels.is_empty()
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_relationship_record_log_reports_missing_source_when_log_file_absent() {
        let path = unique_temp_path("missing_relationship_log");
        fs::create_dir_all(&path).expect("storage root directory should be created");
        let root = StorageRoot { path: path.clone() };

        let error = read_relationship_record_log_for_catalog_rebuild(&root)
            .expect_err("missing relationship record log should surface source missing error");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
        if segment == StorageSegment::RelationshipRecords
        ));
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_relationship_record_log_requires_relationship_type_metadata() {
        let path = unique_temp_path("relationship_log_missing_type");
        let root = create_storage_root(path.clone(), manifest())
            .expect("storage root should be created with manifest");
        let relationship = relationship_id("relationship--missing-type");
        let reference = storage_ref(StorageSegment::RelationshipRecords, 222);
        let envelope = relationship_envelope(
            &relationship,
            relationship_version("relationship-version--missing-type", 1, true, None),
            reference.clone(),
        );

        let log_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        fs::create_dir_all(
            log_path
                .parent()
                .expect("relationship log should have a parent"),
        )
        .expect("relationship directory should exist");
        let line = serde_json::json!({
        "envelope": envelope,
        "storage_ref": reference,
        });
        fs::write(
            &log_path,
            format!(
                "{}\n",
                serde_json::to_string(&line).expect("line should serialize")
            ),
        )
        .expect("relationship log fixture should be written");

        let error = read_relationship_record_log_for_catalog_rebuild(&root)
            .expect_err("relationship rebuild logs require relationship_type metadata");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildFailed { stage, .. }
        if stage == "read_relationship_record_log_for_catalog_rebuild"
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_outgoing_adjacency_log_reports_missing_source_when_log_file_absent() {
        let path = unique_temp_path("missing_outgoing_adjacency_log");
        fs::create_dir_all(&path).expect("storage root directory should be created");
        let root = StorageRoot { path: path.clone() };

        let error = read_outgoing_adjacency_log_for_catalog_rebuild(&root)
            .expect_err("missing outgoing adjacency log should surface source missing error");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
        if segment == StorageSegment::OutgoingAdjacency
        ));
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_incoming_adjacency_log_reports_missing_source_when_log_file_absent() {
        let path = unique_temp_path("missing_incoming_adjacency_log");
        fs::create_dir_all(&path).expect("storage root directory should be created");
        let root = StorageRoot { path: path.clone() };

        let error = read_incoming_adjacency_log_for_catalog_rebuild(&root)
            .expect_err("missing incoming adjacency log should surface source missing error");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
        if segment == StorageSegment::IncomingAdjacency
        ));
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_node_record_log_reports_io_error_when_log_path_is_directory() {
        let path = unique_temp_path("node_log_directory_io_error");
        let root = create_storage_root(path.clone(), manifest())
            .expect("storage root should be created with manifest");
        let log_path = root.path().join("nodes").join("node_records.log");
        fs::create_dir_all(&log_path)
            .expect("creating directory at node log path should succeed for fixture");

        let error = read_node_record_log_for_catalog_rebuild(&root)
            .expect_err("directory log path should report io operation failure");

        assert!(matches!(
        error,
        GraphStorageError::IoOperationFailed {
        operation,
        path: Some(actual_path),
        ..
        } if operation == "read_catalog_rebuild_log" && actual_path == log_path
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_node_record_log_rejects_relationship_envelope_in_fallback_decode_path() {
        let path = unique_temp_path("node_log_wrong_envelope_kind");
        let root = create_storage_root(path.clone(), manifest())
            .expect("storage root should be created with manifest");

        let relationship = relationship_id("relationship--wrong-kind-fallback");
        let relationship_ref = storage_ref(StorageSegment::NodeRecords, 555);
        let envelope = relationship_envelope(
            &relationship,
            relationship_version("relationship-version--wrong-kind", 1, true, None),
            relationship_ref,
        );

        let log_path = root.path().join("nodes").join("node_records.log");
        fs::create_dir_all(log_path.parent().expect("node log should have parent"))
            .expect("nodes directory should exist");
        fs::write(
            &log_path,
            serde_json::to_vec(&envelope).expect("envelope should serialize"),
        )
        .expect("node log fixture should be written");

        let error = read_node_record_log_for_catalog_rebuild(&root)
            .expect_err("relationship envelope in node log should be rejected");

        assert!(matches!(
            error,
            GraphStorageError::CatalogRebuildCorruptedRecord {
                segment: StorageSegment::NodeRecords,
                ..
            }
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn read_incoming_adjacency_log_rejects_embedded_storage_ref_segment_mismatch() {
        let path = unique_temp_path("incoming_adjacency_embedded_ref_segment_mismatch");
        let root = create_storage_root(path.clone(), manifest())
            .expect("storage root should be created with manifest");

        let source = node_id("node--incoming-source");
        let owner = node_id("node--incoming-owner");
        let relationship = relationship_id("relationship--incoming-mismatch");
        let log_storage_ref = storage_ref(StorageSegment::IncomingAdjacency, 777);
        let entry = adjacency_entry(
            &relationship,
            &source,
            &owner,
            relationship_type("LINKS"),
            AdjacencyDirection::Incoming,
        );
        let record = adjacency_record(
            &owner,
            AdjacencyDirection::Incoming,
            vec![entry],
            storage_ref(StorageSegment::OutgoingAdjacency, 778),
        );
        let line = serde_json::json!({
        "record": record,
        "storage_ref": log_storage_ref,
        });

        let log_path = root.path().join("adjacency").join("incoming_adjacency.log");
        fs::create_dir_all(log_path.parent().expect("incoming log should have parent"))
            .expect("adjacency directory should exist");
        fs::write(
            &log_path,
            format!(
                "{}\n",
                serde_json::to_string(&line).expect("line should serialize")
            ),
        )
        .expect("incoming adjacency fixture should be written");

        let error = read_incoming_adjacency_log_for_catalog_rebuild(&root)
            .expect_err("embedded storage ref segment mismatch should be rejected");

        assert!(matches!(
            error,
            GraphStorageError::CatalogRebuildCorruptedRecord {
                segment: StorageSegment::IncomingAdjacency,
                ..
            }
        ));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn reconstruct_catalog_from_rebuild_records_honors_segment_include_flags() {
        let node = node_id("node--included-by-flag");
        let relationship = relationship_id("relationship--included-by-flag");
        let node_ref = storage_ref(StorageSegment::NodeRecords, 333);
        let relationship_ref = storage_ref(StorageSegment::RelationshipRecords, 444);
        let rel_type = relationship_type("ASSOCIATED_WITH");
        let records = vec![
            node_rebuild_record(&node, node_ref, &["Campaign"]),
            relationship_rebuild_record(&relationship, relationship_ref.clone(), rel_type.clone()),
        ];
        let options = CatalogRebuildOptions {
            include_node_records: false,
            include_relationship_records: true,
            include_outgoing_adjacency: false,
            include_incoming_adjacency: false,
            fail_fast: true,
        };

        let outcome = reconstruct_catalog_from_rebuild_records(&records, options)
            .expect("rebuild should succeed while filtering node reconstruction by options");

        assert!(matches!(
        resolve_latest_node_storage_ref(&outcome.catalog, &node),
        Err(GraphStorageError::MissingNodeCatalogEntry { node_id }) if node_id == node
        ));
        assert_eq!(
            resolve_latest_relationship_storage_ref(&outcome.catalog, &relationship),
            Ok(relationship_ref)
        );
        assert_eq!(
            resolve_relationship_ids_by_type(
                &outcome.catalog,
                &rel_type,
                CatalogIndexLookupMode::Strict,
            ),
            Ok(vec![relationship])
        );
        assert_eq!(outcome.report.latest_node_records_reconstructed, 0);
        assert_eq!(outcome.report.latest_relationship_records_reconstructed, 1);
    }

    #[test]
    fn detect_corrupted_catalog_rebuild_records_rejects_adjacency_owner_mismatch() {
        let owner = node_id("node--owner-mismatch");
        let source = node_id("node--source-other");
        let target = node_id("node--target-other");
        let relationship = relationship_id("relationship--owner-mismatch");
        let reference = storage_ref(StorageSegment::OutgoingAdjacency, 1200);
        let entry = adjacency_entry(
            &relationship,
            &source,
            &target,
            relationship_type("LINKS"),
            AdjacencyDirection::Outgoing,
        );

        let records = vec![CatalogRebuildRecord::OutgoingAdjacency {
            record: adjacency_record(
                &owner,
                AdjacencyDirection::Outgoing,
                vec![entry],
                reference.clone(),
            ),
            storage_ref: reference.clone(),
        }];

        let error = detect_corrupted_catalog_rebuild_records(&records)
            .expect_err("owner mismatch should be surfaced as a corrupted adjacency record");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildCorruptedRecord {
        segment: StorageSegment::OutgoingAdjacency,
        storage_ref: Some(actual_ref),
        ..
        } if actual_ref.as_ref() == &reference
        ));
    }

    #[test]
    fn relationship_types_for_entries_deduplicates_while_preserving_first_seen_order() {
        let source = node_id("node--types-source");
        let target = node_id("node--types-target");
        let entries = vec![
            adjacency_entry(
                &relationship_id("relationship--types-1"),
                &source,
                &target,
                relationship_type("AMPLIFIES"),
                AdjacencyDirection::Outgoing,
            ),
            adjacency_entry(
                &relationship_id("relationship--types-2"),
                &source,
                &target,
                relationship_type("AMPLIFIES"),
                AdjacencyDirection::Outgoing,
            ),
            adjacency_entry(
                &relationship_id("relationship--types-3"),
                &source,
                &target,
                relationship_type("SUPPORTS"),
                AdjacencyDirection::Outgoing,
            ),
        ];

        let types = relationship_types_for_entries(&entries);

        assert_eq!(
            types,
            vec![
                relationship_type("AMPLIFIES"),
                relationship_type("SUPPORTS")
            ]
        );
    }

    #[test]
    fn latest_entry_candidate_supports_node_relationship_and_ignores_adjacency() {
        let node = node_id("node--latest-candidate");
        let relationship = relationship_id("relationship--latest-candidate");
        let node_ref = storage_ref(StorageSegment::NodeRecords, 1301);
        let relationship_ref = storage_ref(StorageSegment::RelationshipRecords, 1302);

        let node_record = node_rebuild_record(&node, node_ref.clone(), &["Campaign"]);
        let node_candidate = latest_entry_candidate(&node_record)
            .expect("node latest candidate should be derivable")
            .expect("node candidate should be present");
        assert_eq!(node_candidate.record_id, PersistedRecordId::Node(node));
        assert_eq!(node_candidate.storage_ref, node_ref);

        let relationship_record = relationship_rebuild_record(
            &relationship,
            relationship_ref.clone(),
            relationship_type("LINKS"),
        );
        let relationship_candidate = latest_entry_candidate(&relationship_record)
            .expect("relationship latest candidate should be derivable")
            .expect("relationship candidate should be present");
        assert_eq!(
            relationship_candidate.record_id,
            PersistedRecordId::Relationship(relationship)
        );
        assert_eq!(relationship_candidate.storage_ref, relationship_ref);

        let adjacency = CatalogRebuildRecord::OutgoingAdjacency {
            record: adjacency_record(
                &node_id("node--owner-latest-candidate"),
                AdjacencyDirection::Outgoing,
                Vec::new(),
                storage_ref(StorageSegment::OutgoingAdjacency, 1303),
            ),
            storage_ref: storage_ref(StorageSegment::OutgoingAdjacency, 1303),
        };
        assert!(
            latest_entry_candidate(&adjacency)
                .expect("adjacency candidate lookup should not fail")
                .is_none()
        );
    }

    #[test]
    fn latest_entry_and_record_current_flags_validate_graph_version_metadata() {
        let node = node_id("node--current-flags");
        let entry = LatestRecordCatalogEntry {
            record_id: PersistedRecordId::Node(node.clone()),
            kind: PersistedRecordKind::Node,
            graph_record_version: Some(node_version("node-version--current-flags", 1, false, None)),
            storage_ref: storage_ref(StorageSegment::NodeRecords, 1304),
        };

        assert!(!latest_entry_is_current(&entry).expect("entry current flag should be readable"));

        let mut missing_entry = entry.clone();
        missing_entry.graph_record_version = None;
        assert!(matches!(
        latest_entry_is_current(&missing_entry),
        Err(GraphStorageError::InvalidEnvelope { reason })
        if reason.contains("latest rebuild candidate requires graph record version metadata")
        ));

        let envelope = node_envelope(
            &node,
            node_version("node-version--record-flags", 1, false, None),
            storage_ref(StorageSegment::NodeRecords, 1305),
        );
        assert!(!record_is_current(&envelope).expect("record current flag should be readable"));

        let mut missing_version_envelope = envelope.clone();
        missing_version_envelope.graph_record_version = None;
        assert!(matches!(
        record_is_current(&missing_version_envelope),
        Err(GraphStorageError::InvalidEnvelope { reason })
        if reason.contains("rebuild record requires graph record version metadata")
        ));
    }

    #[test]
    fn count_helpers_report_expected_node_relationship_and_adjacency_totals() {
        let node = node_id("node--count-helpers");
        let relationship = relationship_id("relationship--count-helpers");
        let records = vec![
            node_rebuild_record(
                &node,
                storage_ref(StorageSegment::NodeRecords, 1306),
                &["Campaign"],
            ),
            relationship_rebuild_record(
                &relationship,
                storage_ref(StorageSegment::RelationshipRecords, 1307),
                relationship_type("LINKS"),
            ),
            CatalogRebuildRecord::OutgoingAdjacency {
                record: adjacency_record(
                    &node,
                    AdjacencyDirection::Outgoing,
                    Vec::new(),
                    storage_ref(StorageSegment::OutgoingAdjacency, 1308),
                ),
                storage_ref: storage_ref(StorageSegment::OutgoingAdjacency, 1308),
            },
            CatalogRebuildRecord::IncomingAdjacency {
                record: adjacency_record(
                    &node,
                    AdjacencyDirection::Incoming,
                    Vec::new(),
                    storage_ref(StorageSegment::IncomingAdjacency, 1309),
                ),
                storage_ref: storage_ref(StorageSegment::IncomingAdjacency, 1309),
            },
        ];

        let counts = count_rebuild_records(&records);
        assert_eq!(counts.node_records, 1);
        assert_eq!(counts.relationship_records, 1);
        assert_eq!(counts.outgoing_adjacency_records, 1);
        assert_eq!(counts.incoming_adjacency_records, 1);
        assert_eq!(count_adjacency_records(&records), 2);
    }

    #[test]
    fn decode_envelope_line_reports_corrupted_record_for_invalid_json() {
        let reference = storage_ref(StorageSegment::NodeRecords, 1400);

        let error = decode_envelope_line(
            b"{not-valid-json}\n",
            &reference,
            PersistedRecordKind::Node,
            StorageSegment::NodeRecords,
        )
        .expect_err("invalid envelope line should return a corrupted-record error");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildCorruptedRecord { reason, .. }
        if reason.contains("failed to decode persisted envelope")
        ));
    }

    #[test]
    fn index_rebuild_rejects_wrong_record_ids_and_skips_non_current_versions() {
        let mut catalog = GraphCatalog::default();

        let non_current_node = CatalogRebuildRecord::Node {
            envelope: node_envelope(
                &node_id("node--non-current"),
                node_version(
                    "node-version--non-current",
                    2,
                    false,
                    Some("node-version--1"),
                ),
                storage_ref(StorageSegment::NodeRecords, 1401),
            ),
            storage_ref: storage_ref(StorageSegment::NodeRecords, 1401),
            labels: vec!["Campaign".to_owned()],
        };
        reconstruct_label_indexes_from_rebuild_records(&mut catalog, &[non_current_node])
            .expect("non-current node records should be ignored for label index rebuild");
        assert!(catalog.metadata_indexes.labels.is_empty());

        let wrong_node_id = CatalogRebuildRecord::Node {
            envelope: PersistedRecordEnvelope {
                record_id: PersistedRecordId::Relationship(relationship_id(
                    "relationship--wrong-in-node",
                )),
                kind: PersistedRecordKind::Node,
                storage_version: StorageVersion::V1,
                record_format: RecordFormat::JsonLinesV1,
                graph_record_version: Some(node_version("node-version--wrong", 1, true, None)),
                storage_ref: storage_ref(StorageSegment::NodeRecords, 1402),
                record_checksum: Some(checksum("node-wrong-id")),
            },
            storage_ref: storage_ref(StorageSegment::NodeRecords, 1402),
            labels: vec!["Campaign".to_owned()],
        };
        let node_error =
            reconstruct_label_indexes_from_rebuild_records(&mut catalog, &[wrong_node_id])
                .expect_err("node rebuild record carrying a relationship ID should be rejected");
        assert!(matches!(
        node_error,
        GraphStorageError::CatalogRebuildCorruptedRecord { reason, .. }
        if reason.contains("node rebuild record must carry a node record id")
        ));

        let non_current_relationship = CatalogRebuildRecord::Relationship {
            envelope: relationship_envelope(
                &relationship_id("relationship--non-current"),
                relationship_version(
                    "relationship-version--non-current",
                    2,
                    false,
                    Some("relationship-version--1"),
                ),
                storage_ref(StorageSegment::RelationshipRecords, 1403),
            ),
            storage_ref: storage_ref(StorageSegment::RelationshipRecords, 1403),
            relationship_type: relationship_type("RELATES_TO"),
        };
        reconstruct_relationship_type_indexes_from_rebuild_records(
            &mut catalog,
            &[non_current_relationship],
        )
        .expect("non-current relationship records should be ignored for type index rebuild");
        assert!(catalog.metadata_indexes.relationship_types.is_empty());

        let wrong_relationship_id = CatalogRebuildRecord::Relationship {
            envelope: PersistedRecordEnvelope {
                record_id: PersistedRecordId::Node(node_id("node--wrong-in-relationship")),
                kind: PersistedRecordKind::Relationship,
                storage_version: StorageVersion::V1,
                record_format: RecordFormat::JsonLinesV1,
                graph_record_version: Some(relationship_version(
                    "relationship-version--wrong",
                    1,
                    true,
                    None,
                )),
                storage_ref: storage_ref(StorageSegment::RelationshipRecords, 1404),
                record_checksum: Some(checksum("relationship-wrong-id")),
            },
            storage_ref: storage_ref(StorageSegment::RelationshipRecords, 1404),
            relationship_type: relationship_type("RELATES_TO"),
        };
        let relationship_error = reconstruct_relationship_type_indexes_from_rebuild_records(
            &mut catalog,
            &[wrong_relationship_id],
        )
        .expect_err("relationship rebuild record carrying a node ID should be rejected");
        assert!(matches!(
        relationship_error,
        GraphStorageError::CatalogRebuildCorruptedRecord { reason, .. }
        if reason.contains("relationship rebuild record must carry a relationship record id")
        ));
    }

    #[test]
    fn outgoing_adjacency_rebuild_reports_decode_failures_for_malformed_lines() {
        let root_path = unique_temp_path("malformed_outgoing_log");

        let root = create_storage_root(&root_path, manifest())
            .expect("storage root should be created for rebuild test");
        let outgoing_path = root.path().join("adjacency").join("outgoing_adjacency.log");
        fs::create_dir_all(
            outgoing_path
                .parent()
                .expect("outgoing adjacency path should have a parent"),
        )
        .expect("outgoing adjacency directory should be created");
        fs::write(&outgoing_path, b"{broken-json}\n")
            .expect("malformed outgoing log fixture should be written");

        let error = read_outgoing_adjacency_log_for_catalog_rebuild(&root)
            .expect_err("malformed outgoing adjacency line should be rejected");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildCorruptedRecord { reason, .. }
        if reason.contains("failed to decode adjacency rebuild record")
        ));

        fs::remove_dir_all(&root_path).expect("temporary root directory should be removed");
    }

    #[test]
    fn path_helpers_and_corrupted_record_mapper_return_deterministic_values() {
        let root = StorageRoot {
            path: PathBuf::from("/tmp/catalog-rebuild-path-helpers"),
        };

        assert!(node_record_log_path(&root).ends_with("nodes/node_records.log"));
        assert!(
            relationship_record_log_path(&root).ends_with("relationships/relationship_records.log")
        );
        assert!(outgoing_adjacency_log_path(&root).ends_with("adjacency/outgoing_adjacency.log"));
        assert!(incoming_adjacency_log_path(&root).ends_with("adjacency/incoming_adjacency.log"));
        assert_eq!(
            path_display(Path::new("catalog/rebuild")),
            "catalog/rebuild".to_owned()
        );

        let storage_ref = storage_ref(StorageSegment::NodeRecords, 1310);
        let mapped = corrupted_record(
            StorageSegment::NodeRecords,
            Some(storage_ref.clone()),
            "fixture corrupted",
        );
        assert!(matches!(
        mapped,
        GraphStorageError::CatalogRebuildCorruptedRecord {
        segment: StorageSegment::NodeRecords,
        storage_ref: Some(actual_ref),
        reason,
        } if actual_ref.as_ref() == &storage_ref && reason == "fixture corrupted"
        ));
    }
}
