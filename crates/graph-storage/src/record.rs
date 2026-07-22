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
use graph_core::{
    AdjacencyDirection, Node, NodeId, NodeVersionId, Relationship, RelationshipId,
    RelationshipVersionId,
};
use serde::{Deserialize, Serialize};

use crate::{GraphStorageError, GraphStorageResult, RecordFormat, StorageVersion};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Storage segment.
pub enum StorageSegment {
    /// Manifest.
    Manifest,
    /// Catalog.
    Catalog,
    /// Node records.
    NodeRecords,
    /// Relationship records.
    RelationshipRecords,
    /// Outgoing adjacency.
    OutgoingAdjacency,
    /// Incoming adjacency.
    IncomingAdjacency,
    /// Logs.
    Logs,
    /// Snapshots.
    Snapshots,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Record checksum.
pub struct RecordChecksum {
    /// Algorithm.
    pub algorithm: String,
    /// Value.
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Storage ref.
pub struct StorageRef {
    /// Segment.
    pub segment: StorageSegment,
    /// Offset.
    pub offset: u64,
    /// Length.
    pub length: u64,
    /// Checksum.
    pub checksum: Option<RecordChecksum>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Persisted record kind.
pub enum PersistedRecordKind {
    /// Manifest.
    Manifest,
    /// Catalog.
    Catalog,
    /// Node.
    Node,
    /// Relationship.
    Relationship,
    /// Outgoing adjacency.
    OutgoingAdjacency,
    /// Incoming adjacency.
    IncomingAdjacency,
    /// Log.
    Log,
    /// Snapshot.
    Snapshot,
    /// Metadata.
    Metadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Persisted record id.
pub enum PersistedRecordId {
    /// Manifest.
    Manifest,
    /// Catalog.
    Catalog {
        /// Key.
        key: String,
    },
    /// Node.
    Node(NodeId),
    /// Relationship.
    Relationship(RelationshipId),
    /// Adjacency.
    Adjacency {
        /// Owner node id.
        owner_node_id: NodeId,
        /// Direction.
        direction: AdjacencyDirection,
    },
    /// Log.
    Log {
        /// Key.
        key: String,
    },
    /// Snapshot.
    Snapshot {
        /// Key.
        key: String,
    },
    /// Metadata.
    Metadata {
        /// Key.
        key: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Graph record version.
pub enum GraphRecordVersion {
    /// Node.
    Node {
        /// Version id.
        version_id: NodeVersionId,
        /// Version.
        version: u64,
        /// Current.
        current: bool,
        /// Previous version id.
        previous_version_id: Option<NodeVersionId>,
    },
    /// Relationship.
    Relationship {
        /// Version id.
        version_id: RelationshipVersionId,
        /// Version.
        version: u64,
        /// Current.
        current: bool,
        /// Previous version id.
        previous_version_id: Option<RelationshipVersionId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Persisted record envelope.
pub struct PersistedRecordEnvelope {
    /// Record id.
    pub record_id: PersistedRecordId,
    /// Kind.
    pub kind: PersistedRecordKind,
    /// Storage version.
    pub storage_version: StorageVersion,
    /// Record format.
    pub record_format: RecordFormat,
    /// Graph record version.
    pub graph_record_version: Option<GraphRecordVersion>,
    /// Storage ref.
    pub storage_ref: StorageRef,
    /// Record checksum.
    pub record_checksum: Option<RecordChecksum>,
}

/// Validates the storage ref.
pub fn validate_storage_ref(storage_ref: &StorageRef) -> GraphStorageResult<()> {
    if storage_ref.length == 0 {
        return Err(invalid_storage_ref(
            storage_ref,
            "length must be greater than zero",
        ));
    }
    if storage_ref.offset.checked_add(storage_ref.length).is_none() {
        return Err(invalid_storage_ref(
            storage_ref,
            "offset plus length must not overflow",
        ));
    }
    validate_storage_checksum(storage_ref)?;
    Ok(())
}

/// Validates the persisted record envelope.
pub fn validate_persisted_record_envelope(
    envelope: &PersistedRecordEnvelope,
) -> GraphStorageResult<()> {
    validate_storage_version(&envelope.storage_version)?;
    validate_record_format(&envelope.record_format)?;
    validate_storage_ref(&envelope.storage_ref)?;
    validate_record_checksum(&envelope.record_checksum)?;
    validate_kind_id_match(envelope)?;
    validate_kind_segment_match(envelope)?;
    validate_graph_record_version_match(envelope)?;
    Ok(())
}

/// Creates the node record envelope.
pub fn create_node_record_envelope(
    node: &Node,
    storage_ref: StorageRef,
    storage_version: StorageVersion,
    record_format: RecordFormat,
    record_checksum: Option<RecordChecksum>,
) -> GraphStorageResult<PersistedRecordEnvelope> {
    let envelope = PersistedRecordEnvelope {
        record_id: PersistedRecordId::Node(node.id().clone()),
        kind: PersistedRecordKind::Node,
        storage_version,
        record_format,
        graph_record_version: Some(GraphRecordVersion::Node {
            version_id: node.version_id().clone(),
            version: node.version(),
            current: node.is_current(),
            previous_version_id: node.previous_version_id().cloned(),
        }),
        storage_ref,
        record_checksum,
    };
    validate_persisted_record_envelope(&envelope)?;
    Ok(envelope)
}

/// Creates the relationship record envelope.
pub fn create_relationship_record_envelope(
    relationship: &Relationship,
    storage_ref: StorageRef,
    storage_version: StorageVersion,
    record_format: RecordFormat,
    record_checksum: Option<RecordChecksum>,
) -> GraphStorageResult<PersistedRecordEnvelope> {
    let envelope = PersistedRecordEnvelope {
        record_id: PersistedRecordId::Relationship(relationship.id().clone()),
        kind: PersistedRecordKind::Relationship,
        storage_version,
        record_format,
        graph_record_version: Some(GraphRecordVersion::Relationship {
            version_id: relationship.version_id().clone(),
            version: relationship.version(),
            current: relationship.is_current(),
            previous_version_id: relationship.previous_version_id().cloned(),
        }),
        storage_ref,
        record_checksum,
    };
    validate_persisted_record_envelope(&envelope)?;
    Ok(envelope)
}

/// Creates the adjacency record envelope.
pub fn create_adjacency_record_envelope(
    owner_node_id: &NodeId,
    direction: AdjacencyDirection,
    storage_ref: StorageRef,
    storage_version: StorageVersion,
    record_format: RecordFormat,
    record_checksum: Option<RecordChecksum>,
) -> GraphStorageResult<PersistedRecordEnvelope> {
    let kind = match direction {
        AdjacencyDirection::Outgoing => PersistedRecordKind::OutgoingAdjacency,
        AdjacencyDirection::Incoming => PersistedRecordKind::IncomingAdjacency,
    };
    let envelope = PersistedRecordEnvelope {
        record_id: PersistedRecordId::Adjacency {
            owner_node_id: owner_node_id.clone(),
            direction,
        },
        kind,
        storage_version,
        record_format,
        graph_record_version: None,
        storage_ref,
        record_checksum,
    };
    validate_persisted_record_envelope(&envelope)?;
    Ok(envelope)
}

fn invalid_storage_ref(storage_ref: &StorageRef, reason: impl Into<String>) -> GraphStorageError {
    GraphStorageError::InvalidStorageRef {
        storage_ref: storage_ref.clone(),
        reason: reason.into(),
    }
}

fn validate_storage_checksum(storage_ref: &StorageRef) -> GraphStorageResult<()> {
    if let Some(checksum) = &storage_ref.checksum
        && (checksum.algorithm.trim().is_empty() || checksum.value.trim().is_empty())
    {
        return Err(invalid_storage_ref(
            storage_ref,
            "checksum algorithm and value must not be empty",
        ));
    }
    Ok(())
}

fn validate_storage_version(storage_version: &StorageVersion) -> GraphStorageResult<()> {
    match storage_version {
        StorageVersion::V1 => Ok(()),
        StorageVersion::Unsupported(version) => Err(GraphStorageError::UnsupportedStorageVersion {
            version: version.clone(),
        }),
    }
}

fn validate_record_format(record_format: &RecordFormat) -> GraphStorageResult<()> {
    match record_format {
        RecordFormat::JsonLinesV1 => Ok(()),
        RecordFormat::Unsupported(format) => Err(GraphStorageError::UnsupportedRecordFormat {
            format: format.clone(),
        }),
    }
}

fn validate_record_checksum(record_checksum: &Option<RecordChecksum>) -> GraphStorageResult<()> {
    if let Some(checksum) = record_checksum
        && (checksum.algorithm.trim().is_empty() || checksum.value.trim().is_empty())
    {
        return Err(GraphStorageError::InvalidEnvelope {
            reason: "record checksum algorithm and value must not be empty".to_owned(),
        });
    }
    Ok(())
}

fn validate_kind_id_match(envelope: &PersistedRecordEnvelope) -> GraphStorageResult<()> {
    let matches = match (envelope.kind, &envelope.record_id) {
        (PersistedRecordKind::Manifest, PersistedRecordId::Manifest) => true,
        (PersistedRecordKind::Catalog, PersistedRecordId::Catalog { .. }) => true,
        (PersistedRecordKind::Node, PersistedRecordId::Node(_)) => true,
        (PersistedRecordKind::Relationship, PersistedRecordId::Relationship(_)) => true,
        (
            PersistedRecordKind::OutgoingAdjacency,
            PersistedRecordId::Adjacency { direction, .. },
        ) => *direction == AdjacencyDirection::Outgoing,
        (
            PersistedRecordKind::IncomingAdjacency,
            PersistedRecordId::Adjacency { direction, .. },
        ) => *direction == AdjacencyDirection::Incoming,
        (PersistedRecordKind::Log, PersistedRecordId::Log { .. }) => true,
        (PersistedRecordKind::Snapshot, PersistedRecordId::Snapshot { .. }) => true,
        (PersistedRecordKind::Metadata, PersistedRecordId::Metadata { .. }) => true,
        _ => false,
    };

    if matches {
        Ok(())
    } else {
        Err(GraphStorageError::InvalidEnvelope {
            reason: format!(
                "kind/id mismatch: {:?} does not match {:?}",
                envelope.kind, envelope.record_id
            ),
        })
    }
}

fn validate_kind_segment_match(envelope: &PersistedRecordEnvelope) -> GraphStorageResult<()> {
    let expected_segment = expected_segment_for_kind(envelope.kind);
    if envelope.storage_ref.segment == expected_segment {
        return Ok(());
    }

    let reason = if matches!(
        envelope.kind,
        PersistedRecordKind::OutgoingAdjacency | PersistedRecordKind::IncomingAdjacency
    ) {
        format!(
            "adjacency segment mismatch: {:?} requires {:?}, got {:?}",
            envelope.kind, expected_segment, envelope.storage_ref.segment
        )
    } else {
        format!(
            "kind/segment mismatch: {:?} requires {:?}, got {:?}",
            envelope.kind, expected_segment, envelope.storage_ref.segment
        )
    };

    Err(GraphStorageError::InvalidEnvelope { reason })
}

fn expected_segment_for_kind(kind: PersistedRecordKind) -> StorageSegment {
    match kind {
        PersistedRecordKind::Manifest => StorageSegment::Manifest,
        PersistedRecordKind::Catalog => StorageSegment::Catalog,
        PersistedRecordKind::Node => StorageSegment::NodeRecords,
        PersistedRecordKind::Relationship => StorageSegment::RelationshipRecords,
        PersistedRecordKind::OutgoingAdjacency => StorageSegment::OutgoingAdjacency,
        PersistedRecordKind::IncomingAdjacency => StorageSegment::IncomingAdjacency,
        PersistedRecordKind::Log => StorageSegment::Logs,
        PersistedRecordKind::Snapshot => StorageSegment::Snapshots,
        PersistedRecordKind::Metadata => StorageSegment::Catalog,
    }
}

fn validate_graph_record_version_match(
    envelope: &PersistedRecordEnvelope,
) -> GraphStorageResult<()> {
    match (envelope.kind, &envelope.graph_record_version) {
        (PersistedRecordKind::Node, Some(GraphRecordVersion::Node { version, .. })) => {
            validate_graph_version_number(*version)
        }
        (PersistedRecordKind::Node, _) => Err(GraphStorageError::InvalidEnvelope {
            reason: "node envelope requires graph record version metadata".to_owned(),
        }),
        (
            PersistedRecordKind::Relationship,
            Some(GraphRecordVersion::Relationship { version, .. }),
        ) => validate_graph_version_number(*version),
        (PersistedRecordKind::Relationship, _) => Err(GraphStorageError::InvalidEnvelope {
            reason: "relationship envelope requires graph record version metadata".to_owned(),
        }),
        (_, None) => Ok(()),
        (_, Some(_)) => Err(GraphStorageError::InvalidEnvelope {
            reason: "non graph record envelope must not include graph record version metadata"
                .to_owned(),
        }),
    }
}

fn validate_graph_version_number(version: u64) -> GraphStorageResult<()> {
    if version == 0 {
        Err(GraphStorageError::InvalidEnvelope {
            reason: "graph record version must be greater than zero".to_owned(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use graph_core::{Graph, NodeInput, RelationshipInput};

    fn checksum() -> RecordChecksum {
        RecordChecksum {
            // Algorithm.
            algorithm: "sha256".to_owned(),
            // Value.
            value: "6f1ed002ab5595859014ebf0951522d9".to_owned(),
        }
    }

    fn storage_ref(segment: StorageSegment) -> StorageRef {
        StorageRef {
            segment,
            // Offset.
            offset: 128,
            // Length.
            length: 256,
            // Checksum.
            checksum: Some(checksum()),
        }
    }

    fn node_fixture() -> Node {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Campaign", "FIMI"]))
            .unwrap();
        graph.get_node(&node_id).unwrap().unwrap()
    }

    fn relationship_fixture() -> Relationship {
        let mut graph = Graph::new();
        let source = graph.create_node(NodeInput::new(["Actor"])).unwrap();
        let target = graph
            .create_node(NodeInput::new(["Infrastructure"]))
            .unwrap();
        let relationship_id = graph
            .create_relationship(RelationshipInput::new(source, "USES", target).unwrap())
            .unwrap();
        graph.get_relationship(&relationship_id).unwrap().unwrap()
    }

    #[test]
    fn validate_storage_ref_accepts_non_empty_byte_range_with_checksum() {
        assert_eq!(
            validate_storage_ref(&storage_ref(StorageSegment::NodeRecords)),
            Ok(())
        );
    }

    #[test]
    fn validate_storage_ref_rejects_zero_length_units() {
        let reference = StorageRef {
            segment: StorageSegment::RelationshipRecords,
            offset: 16,
            length: 0,
            checksum: None,
        };
        let error = validate_storage_ref(&reference).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::InvalidStorageRef { storage_ref, reason }
        if storage_ref == reference && reason.contains("length")
        ));
    }

    #[test]
    fn validate_storage_ref_rejects_incomplete_checksum_metadata() {
        let reference = StorageRef {
            segment: StorageSegment::Catalog,
            offset: 0,
            length: 128,
            checksum: Some(RecordChecksum {
                algorithm: "".to_owned(),
                value: "abc123".to_owned(),
            }),
        };
        let error = validate_storage_ref(&reference).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::InvalidStorageRef { storage_ref, reason }
        if storage_ref == reference && reason.contains("checksum")
        ));
    }

    #[test]
    fn create_node_record_envelope_preserves_node_identity_and_versions() {
        let node = node_fixture();
        let reference = storage_ref(StorageSegment::NodeRecords);
        let record_checksum = checksum();
        let envelope = create_node_record_envelope(
            &node,
            reference.clone(),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            Some(record_checksum.clone()),
        )
        .unwrap();
        assert_eq!(
            envelope.record_id,
            PersistedRecordId::Node(node.id().clone())
        );
        assert_eq!(envelope.kind, PersistedRecordKind::Node);
        assert_eq!(envelope.storage_ref, reference);
        assert_eq!(envelope.record_checksum, Some(record_checksum));
        assert_eq!(
            envelope.graph_record_version,
            Some(GraphRecordVersion::Node {
                version_id: node.version_id().clone(),
                version: node.version(),
                current: node.is_current(),
                previous_version_id: node.previous_version_id().cloned(),
            })
        );
    }

    #[test]
    fn create_relationship_record_envelope_preserves_relationship_identity_and_versions() {
        let relationship = relationship_fixture();
        let reference = storage_ref(StorageSegment::RelationshipRecords);
        let envelope = create_relationship_record_envelope(
            &relationship,
            reference.clone(),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            Some(checksum()),
        )
        .unwrap();
        assert_eq!(
            envelope.record_id,
            PersistedRecordId::Relationship(relationship.id().clone())
        );
        assert_eq!(envelope.kind, PersistedRecordKind::Relationship);
        assert_eq!(envelope.storage_ref, reference);
        assert_eq!(
            envelope.graph_record_version,
            Some(GraphRecordVersion::Relationship {
                version_id: relationship.version_id().clone(),
                version: relationship.version(),
                current: relationship.is_current(),
                previous_version_id: relationship.previous_version_id().cloned(),
            })
        );
    }

    #[test]
    fn create_adjacency_record_envelope_maps_outgoing_direction_to_outgoing_segment() {
        let owner_node_id = NodeId::new("node--owner").unwrap();
        let reference = storage_ref(StorageSegment::OutgoingAdjacency);
        let envelope = create_adjacency_record_envelope(
            &owner_node_id,
            AdjacencyDirection::Outgoing,
            reference.clone(),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();
        assert_eq!(envelope.kind, PersistedRecordKind::OutgoingAdjacency);
        assert_eq!(envelope.storage_ref, reference);
        assert_eq!(envelope.graph_record_version, None);
    }

    #[test]
    fn create_adjacency_record_envelope_maps_incoming_direction_to_incoming_segment() {
        let owner_node_id = NodeId::new("node--owner").unwrap();
        let reference = storage_ref(StorageSegment::IncomingAdjacency);
        let envelope = create_adjacency_record_envelope(
            &owner_node_id,
            AdjacencyDirection::Incoming,
            reference.clone(),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();
        assert_eq!(envelope.kind, PersistedRecordKind::IncomingAdjacency);
        assert_eq!(envelope.storage_ref, reference);
        assert_eq!(envelope.graph_record_version, None);
    }

    #[test]
    fn validate_persisted_record_envelope_accepts_consistent_node_envelope() {
        let node = node_fixture();
        let envelope = create_node_record_envelope(
            &node,
            storage_ref(StorageSegment::NodeRecords),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            Some(checksum()),
        )
        .unwrap();
        assert_eq!(validate_persisted_record_envelope(&envelope), Ok(()));
    }

    #[test]
    fn validate_persisted_record_envelope_rejects_kind_id_mismatch() {
        let relationship_id = RelationshipId::new("relationship--1").unwrap();
        let envelope = PersistedRecordEnvelope {
            record_id: PersistedRecordId::Relationship(relationship_id),
            kind: PersistedRecordKind::Node,
            storage_version: StorageVersion::V1,
            record_format: RecordFormat::JsonLinesV1,
            graph_record_version: None,
            storage_ref: storage_ref(StorageSegment::NodeRecords),
            record_checksum: Some(checksum()),
        };
        let error = validate_persisted_record_envelope(&envelope).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::InvalidEnvelope { reason }
        if reason.contains("kind") && reason.contains("id")
        ));
    }

    #[test]
    fn validate_persisted_record_envelope_rejects_kind_segment_mismatch() {
        let node_id = NodeId::new("node--1").unwrap();
        let envelope = PersistedRecordEnvelope {
            record_id: PersistedRecordId::Node(node_id),
            kind: PersistedRecordKind::Node,
            storage_version: StorageVersion::V1,
            record_format: RecordFormat::JsonLinesV1,
            graph_record_version: Some(GraphRecordVersion::Node {
                version_id: NodeVersionId::new("node-version--1").unwrap(),
                version: 1,
                current: true,
                previous_version_id: None,
            }),
            storage_ref: storage_ref(StorageSegment::RelationshipRecords),
            record_checksum: Some(checksum()),
        };
        let error = validate_persisted_record_envelope(&envelope).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::InvalidEnvelope { reason } if reason.contains("segment")
        ));
    }

    #[test]
    fn validate_persisted_record_envelope_rejects_node_without_graph_version_metadata() {
        let node_id = NodeId::new("node--1").unwrap();
        let envelope = PersistedRecordEnvelope {
            record_id: PersistedRecordId::Node(node_id),
            kind: PersistedRecordKind::Node,
            storage_version: StorageVersion::V1,
            record_format: RecordFormat::JsonLinesV1,
            graph_record_version: None,
            storage_ref: storage_ref(StorageSegment::NodeRecords),
            record_checksum: Some(checksum()),
        };
        let error = validate_persisted_record_envelope(&envelope).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::InvalidEnvelope { reason }
        if reason.contains("graph") && reason.contains("version")
        ));
    }

    #[test]
    fn validate_persisted_record_envelope_rejects_adjacency_direction_segment_mismatch() {
        let owner_node_id = NodeId::new("node--owner").unwrap();
        let envelope = PersistedRecordEnvelope {
            record_id: PersistedRecordId::Adjacency {
                owner_node_id,
                direction: AdjacencyDirection::Outgoing,
            },
            kind: PersistedRecordKind::OutgoingAdjacency,
            storage_version: StorageVersion::V1,
            record_format: RecordFormat::JsonLinesV1,
            graph_record_version: None,
            storage_ref: storage_ref(StorageSegment::IncomingAdjacency),
            record_checksum: None,
        };
        let error = validate_persisted_record_envelope(&envelope).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::InvalidEnvelope { reason }
        if reason.contains("adjacency") && reason.contains("segment")
        ));
    }

    #[test]
    fn validate_persisted_record_envelope_rejects_unsupported_storage_version() {
        let node = node_fixture();
        let envelope = PersistedRecordEnvelope {
            record_id: PersistedRecordId::Node(node.id().clone()),
            kind: PersistedRecordKind::Node,
            storage_version: StorageVersion::Unsupported("V999".to_owned()),
            record_format: RecordFormat::JsonLinesV1,
            graph_record_version: Some(GraphRecordVersion::Node {
                version_id: node.version_id().clone(),
                version: node.version(),
                current: node.is_current(),
                previous_version_id: node.previous_version_id().cloned(),
            }),
            storage_ref: storage_ref(StorageSegment::NodeRecords),
            record_checksum: Some(checksum()),
        };
        let error = validate_persisted_record_envelope(&envelope).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::UnsupportedStorageVersion { version } if version == "V999"
        ));
    }

    #[test]
    fn validate_persisted_record_envelope_rejects_unsupported_record_format() {
        let node = node_fixture();
        let envelope = PersistedRecordEnvelope {
            record_id: PersistedRecordId::Node(node.id().clone()),
            kind: PersistedRecordKind::Node,
            storage_version: StorageVersion::V1,
            record_format: RecordFormat::Unsupported("BinaryV9".to_owned()),
            graph_record_version: Some(GraphRecordVersion::Node {
                version_id: node.version_id().clone(),
                version: node.version(),
                current: node.is_current(),
                previous_version_id: node.previous_version_id().cloned(),
            }),
            storage_ref: storage_ref(StorageSegment::NodeRecords),
            record_checksum: Some(checksum()),
        };
        let error = validate_persisted_record_envelope(&envelope).unwrap_err();
        assert!(matches!(
        error,
        GraphStorageError::UnsupportedRecordFormat { format } if format == "BinaryV9"
        ));
    }
}
