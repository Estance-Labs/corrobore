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
use std::fs;
use std::path::{Path, PathBuf};

use graph_core::{NodeId, RelationshipId, RelationshipType};
use serde::{Deserialize, Serialize};

use crate::{
    GraphCatalog, GraphCatalogIndexes, GraphStorageError, GraphStorageResult,
    HistoricalRecordCatalogEntry, LabelIndexCatalogEntry, LatestRecordCatalogEntry,
    RelationshipTypeIndexCatalogEntry, StorageRoot,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedGraphCatalogMetadata {
    latest_node_records: Vec<(NodeId, LatestRecordCatalogEntry)>,
    latest_relationship_records: Vec<(RelationshipId, LatestRecordCatalogEntry)>,
    historical_records: Vec<HistoricalRecordCatalogEntry>,
    label_indexes: Vec<(String, LabelIndexCatalogEntry)>,
    relationship_type_indexes: Vec<(RelationshipType, RelationshipTypeIndexCatalogEntry)>,
    #[serde(default)]
    identifier_indexes: Vec<(String, Vec<crate::LabelIndexNodeMetadata>)>,
    #[serde(default)]
    property_indexes: Vec<(String, HashMap<String, Vec<crate::LabelIndexNodeMetadata>>)>,
    #[serde(default)]
    temporal_indexes: Vec<(String, HashMap<String, Vec<crate::LabelIndexNodeMetadata>>)>,
}

/// Persist derived catalog metadata for fast startup.
pub fn persist_graph_catalog_metadata(
    root: &StorageRoot,
    catalog: &GraphCatalog,
) -> GraphStorageResult<()> {
    if !root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: root.path().to_path_buf(),
        });
    }

    let path = catalog_metadata_file_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| GraphStorageError::OperationFailed {
            operation: "persist_graph_catalog_metadata",
            message: "catalog metadata path has no parent directory".to_owned(),
        })?;
    fs::create_dir_all(parent).map_err(|error| GraphStorageError::IoOperationFailed {
        operation: "persist_graph_catalog_metadata",
        path: Some(parent.to_path_buf()),
        message: error.to_string(),
    })?;

    let persisted = PersistedGraphCatalogMetadata::from_catalog(catalog);
    let bytes =
        serde_json::to_vec(&persisted).map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_graph_catalog_metadata",
            message: format!("failed to encode catalog metadata: {error}"),
        })?;
    fs::write(&path, bytes).map_err(|error| GraphStorageError::IoOperationFailed {
        operation: "persist_graph_catalog_metadata",
        path: Some(path.clone()),
        message: error.to_string(),
    })?;
    Ok(())
}

/// Read derived catalog metadata if it exists.
pub fn read_persisted_graph_catalog_metadata(
    root: &StorageRoot,
) -> GraphStorageResult<Option<GraphCatalog>> {
    if !root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: root.path().to_path_buf(),
        });
    }

    let path = catalog_metadata_file_path(root);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| GraphStorageError::IoOperationFailed {
        operation: "read_persisted_graph_catalog_metadata",
        path: Some(path.clone()),
        message: error.to_string(),
    })?;
    let persisted =
        serde_json::from_slice::<PersistedGraphCatalogMetadata>(&bytes).map_err(|error| {
            GraphStorageError::OperationFailed {
                operation: "read_persisted_graph_catalog_metadata",
                message: format!(
                    "failed to decode catalog metadata from {}: {error}",
                    path_display(&path)
                ),
            }
        })?;
    let catalog = persisted.into_catalog()?;
    Ok(Some(catalog))
}

fn catalog_metadata_file_path(root: &StorageRoot) -> PathBuf {
    root.path().join("catalog").join("catalog_metadata.json")
}

fn path_display(path: &Path) -> String {
    path.display().to_string()
}

impl PersistedGraphCatalogMetadata {
    fn from_catalog(catalog: &GraphCatalog) -> Self {
        Self {
            latest_node_records: catalog
                .latest_node_records
                .iter()
                .map(|(node_id, entry)| (node_id.clone(), entry.clone()))
                .collect(),
            latest_relationship_records: catalog
                .latest_relationship_records
                .iter()
                .map(|(relationship_id, entry)| (relationship_id.clone(), entry.clone()))
                .collect(),
            historical_records: catalog.historical_records.clone(),
            label_indexes: catalog
                .metadata_indexes
                .labels
                .iter()
                .map(|(label, entry)| (label.clone(), entry.clone()))
                .collect(),
            relationship_type_indexes: catalog
                .metadata_indexes
                .relationship_types
                .iter()
                .map(|(relationship_type, entry)| (relationship_type.clone(), entry.clone()))
                .collect(),
            identifier_indexes: catalog
                .metadata_indexes
                .identifiers
                .iter()
                .map(|(identifier, entries)| (identifier.clone(), entries.clone()))
                .collect(),
            property_indexes: catalog
                .metadata_indexes
                .properties
                .iter()
                .map(|(field, values)| (field.clone(), values.clone()))
                .collect(),
            temporal_indexes: catalog
                .metadata_indexes
                .temporal
                .iter()
                .map(|(field, values)| (field.clone(), values.clone()))
                .collect(),
        }
    }

    fn into_catalog(self) -> GraphStorageResult<GraphCatalog> {
        Ok(GraphCatalog {
            latest_node_records: collect_unique_pairs(
                self.latest_node_records,
                "read_persisted_graph_catalog_metadata",
                "latest node records",
            )?,
            latest_relationship_records: collect_unique_pairs(
                self.latest_relationship_records,
                "read_persisted_graph_catalog_metadata",
                "latest relationship records",
            )?,
            historical_records: self.historical_records,
            metadata_indexes: GraphCatalogIndexes {
                labels: collect_unique_pairs(
                    self.label_indexes,
                    "read_persisted_graph_catalog_metadata",
                    "label indexes",
                )?,
                relationship_types: collect_unique_pairs(
                    self.relationship_type_indexes,
                    "read_persisted_graph_catalog_metadata",
                    "relationship type indexes",
                )?,
                identifiers: collect_unique_pairs(
                    self.identifier_indexes,
                    "read_persisted_graph_catalog_metadata",
                    "identifier indexes",
                )?,
                properties: collect_unique_pairs(
                    self.property_indexes,
                    "read_persisted_graph_catalog_metadata",
                    "property indexes",
                )?,
                temporal: collect_unique_pairs(
                    self.temporal_indexes,
                    "read_persisted_graph_catalog_metadata",
                    "temporal indexes",
                )?,
            },
        })
    }
}

fn collect_unique_pairs<K, V>(
    entries: Vec<(K, V)>,
    operation: &'static str,
    field: &'static str,
) -> GraphStorageResult<HashMap<K, V>>
where
    K: Eq + std::hash::Hash,
{
    let mut map = HashMap::new();
    for (key, value) in entries {
        if map.insert(key, value).is_some() {
            return Err(GraphStorageError::OperationFailed {
                operation,
                message: format!("duplicate key found while decoding {field}"),
            });
        }
    }
    Ok(map)
}
