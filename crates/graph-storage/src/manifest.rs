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
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Storage compatibility version for a persisted graph store.
///
///
/// - Make storage compatibility explicit in the manifest.
/// - Give open/reopen code a stable branch point before it reads catalog or record
///   data from disk.
/// - Keep storage versioning separate from graph record versioning and schema
///   evolution.
///
///
/// `open_storage_root` and `validate_storage_manifest` reject manifests whose
/// storage version is not supported by the current binary with
/// `GraphStorageError::UnsupportedStorageVersion`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageVersion {
    /// First local embedded storage contract for .
    V1,

    /// Version value read from a manifest that this crate does not support.
    Unsupported(String),
}

impl Serialize for StorageVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::V1 => serializer.serialize_str("V1"),
            Self::Unsupported(value) => serializer.serialize_str(value),
        }
    }
}

impl<'de> Deserialize<'de> for StorageVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        match value.as_str() {
            "V1" => Ok(Self::V1),
            _ => Ok(Self::Unsupported(value)),
        }
    }
}

/// Stable logical identity for a graph store.
///
///
/// - Identify the graph represented by a storage root independently of its
///   filesystem path.
/// - Let future snapshots, audit logs, catalog rebuilds, and export/import flows
///   refer to the same graph even when the root is moved.
/// - Avoid reusing node, relationship, workspace, or session identifiers for a
///   storage-level graph identity.
///
///
/// Manifest validation rejects empty or whitespace-only graph IDs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GraphId {
    /// Raw graph identity value stored in the manifest.
    pub value: String,
}

/// Durable record encoding format used by a graph store.
///
///
/// - Keep record-format compatibility explicit and separate from storage root
///   versioning.
/// - Allow the first implementation to choose a deterministic testable format
///   without making every future format look like JSON.
/// - Give later record-codec work a public manifest field to validate before
///   reading append-only logs.
///
///
/// `validate_storage_manifest` rejects unsupported record formats with a
/// deterministic manifest-related error before any record payloads are decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordFormat {
    /// Deterministic JSON Lines record format reserved for the first file-backed MVP.
    JsonLinesV1,

    /// Record format value read from a manifest that this crate does not support.
    Unsupported(String),
}

impl Serialize for RecordFormat {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::JsonLinesV1 => serializer.serialize_str("JsonLinesV1"),
            Self::Unsupported(value) => serializer.serialize_str(value),
        }
    }
}

impl<'de> Deserialize<'de> for RecordFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        match value.as_str() {
            "JsonLinesV1" => Ok(Self::JsonLinesV1),
            _ => Ok(Self::Unsupported(value)),
        }
    }
}

/// Timestamp recorded in the storage manifest.
///
///
/// - Reserve created/updated metadata without introducing a time dependency yet.
/// - Keep phase 1 focused on the manifest contract rather than timestamp parsing.
/// - Let phase 2 tests decide the exact accepted timestamp format.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StorageTimestamp {
    /// Raw timestamp value stored in the manifest.
    pub value: String,
}

/// Manifest describing a local persistent graph storage root.
///
///
/// - Represent the minimum compatibility metadata needed to create and reopen a
///   graph store.
/// - Keep this metadata independent from node payloads, relationship payloads,
///   adjacency pages, catalog indexes, and audit logs.
/// - Let open/reopen code validate compatibility before loading the graph or
///   building a working set.
///
///
/// `create_storage_root` writes this manifest when a new root is created,
/// `read_storage_manifest` reads it back from the storage root, and
/// `validate_storage_manifest` rejects missing, corrupted, unsupported, or
/// internally inconsistent manifest metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageManifest {
    /// Storage compatibility version for the root.
    pub storage_version: StorageVersion,

    /// Stable identity of the graph stored under this root.
    pub graph_id: GraphId,

    /// Creation timestamp recorded by the storage layer.
    pub created_at: StorageTimestamp,

    /// Last manifest update timestamp recorded by the storage layer.
    pub updated_at: StorageTimestamp,

    /// Durable record encoding format expected under this root.
    pub record_format: RecordFormat,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_version_serializes_supported_and_unsupported_values() {
        let supported = serde_json::to_string(&StorageVersion::V1)
            .expect("supported storage version should serialize");
        assert_eq!(supported, "\"V1\"");

        let unsupported = serde_json::to_string(&StorageVersion::Unsupported("V999".to_owned()))
            .expect("unsupported storage version should serialize transparently");
        assert_eq!(unsupported, "\"V999\"");
    }

    #[test]
    fn storage_version_deserializes_supported_and_unsupported_values() {
        let supported: StorageVersion =
            serde_json::from_str("\"V1\"").expect("supported version should deserialize");
        assert_eq!(supported, StorageVersion::V1);

        let unsupported: StorageVersion = serde_json::from_str("\"VX\"")
            .expect("unsupported version should deserialize into explicit enum variant");
        assert_eq!(unsupported, StorageVersion::Unsupported("VX".to_owned()));
    }

    #[test]
    fn record_format_serializes_supported_and_unsupported_values() {
        let supported = serde_json::to_string(&RecordFormat::JsonLinesV1)
            .expect("supported record format should serialize");
        assert_eq!(supported, "\"JsonLinesV1\"");

        let unsupported = serde_json::to_string(&RecordFormat::Unsupported("BinaryV2".to_owned()))
            .expect("unsupported record format should serialize transparently");
        assert_eq!(unsupported, "\"BinaryV2\"");
    }

    #[test]
    fn record_format_deserializes_supported_and_unsupported_values() {
        let supported: RecordFormat =
            serde_json::from_str("\"JsonLinesV1\"").expect("supported format should deserialize");
        assert_eq!(supported, RecordFormat::JsonLinesV1);

        let unsupported: RecordFormat = serde_json::from_str("\"MsgPackV1\"")
            .expect("unsupported format should deserialize into explicit enum variant");
        assert_eq!(
            unsupported,
            RecordFormat::Unsupported("MsgPackV1".to_owned())
        );
    }

    #[test]
    fn storage_manifest_round_trips_all_fields() {
        let manifest = StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--manifest-roundtrip".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-06T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-06T01:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        };

        let encoded = serde_json::to_string(&manifest).expect("manifest should serialize");
        let decoded: StorageManifest =
            serde_json::from_str(&encoded).expect("manifest should deserialize");

        assert_eq!(decoded, manifest);
    }
}
