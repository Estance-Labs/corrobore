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

//! Persisted schema registry and schema at a commit (issue #323).
//!
//! Corrobore graphs are schema-free: labels, relationship types and property
//! keys appear as records use them. The schema of a commit is therefore
//! derived from the records visible there: counts, the JSON value types each
//! property holds, and the label pairs each relationship type connects.
//! Every label, relationship type and property key gets a stable numeric id
//! the first time any commit's schema contains it; ids are persisted in
//! `history/schema_registry.log` and never reused, so a client can follow a
//! type across commits even when it disappears and comes back.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use graph_core::{PropertyMap, PropertyValue};
use serde::{Deserialize, Serialize};

use super::CanonicalEngineStore;
use crate::graph_history::{CommitView, GraphCommitId};
use crate::{GraphStorageError, GraphStorageResult};

const REGISTRY_FILE: &str = "schema_registry.log";
const SCHEMA_CACHE: usize = 4;

/// One property key of a label or relationship type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaProperty {
    /// Stable id of `(owner, key)`.
    pub id: u64,
    /// Property key.
    pub key: String,
    /// JSON value types seen (`string`, `integer`, `float`, `boolean`,
    /// `null`, `string_list`, `integer_list`, `float_list`, `boolean_list`, `json`).
    pub value_types: Vec<String>,
    /// Records of the owner carrying the key.
    pub count: usize,
}

/// One node label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaLabel {
    /// Stable id.
    pub id: u64,
    /// Label name.
    pub name: String,
    /// Live nodes carrying the label.
    pub count: usize,
    /// Property keys of those nodes.
    pub properties: Vec<SchemaProperty>,
}

/// Label pair a relationship type connects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaEndpoint {
    /// A label of the source node (`""` when it has none).
    pub source_label: String,
    /// A label of the target node (`""` when it has none).
    pub target_label: String,
    /// Live relationships between those labels.
    pub count: usize,
}

/// One relationship type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaRelationshipType {
    /// Stable id.
    pub id: u64,
    /// Type name.
    pub name: String,
    /// Live relationships of the type.
    pub count: usize,
    /// Label pairs it connects, most frequent first.
    pub endpoints: Vec<SchemaEndpoint>,
    /// Property keys of those relationships.
    pub properties: Vec<SchemaProperty>,
}

/// Schema of the records visible at one commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphSchema {
    /// Commit the schema describes.
    pub commit_id: GraphCommitId,
    /// Labels by name.
    pub labels: Vec<SchemaLabel>,
    /// Relationship types by name.
    pub relationship_types: Vec<SchemaRelationshipType>,
}

#[derive(Serialize, Deserialize)]
struct RegistryLine {
    key: String,
    id: u64,
}

/// Stable ids of labels, relationship types and property keys.
#[derive(Clone, Debug, Default)]
pub(crate) struct SchemaRegistry {
    ids: HashMap<String, u64>,
    next: u64,
    schemas: HashMap<GraphCommitId, Arc<GraphSchema>>,
    order: VecDeque<GraphCommitId>,
}

impl SchemaRegistry {
    fn path(store: &CanonicalEngineStore) -> PathBuf {
        store.root.path().join("history").join(REGISTRY_FILE)
    }

    fn load(path: &PathBuf) -> GraphStorageResult<Self> {
        let mut registry = Self {
            next: 1,
            ..Self::default()
        };
        let Ok(text) = fs::read_to_string(path) else {
            return Ok(registry);
        };
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            // A torn final line is ignored; its key gets a fresh id.
            let Ok(entry) = serde_json::from_str::<RegistryLine>(line) else {
                continue;
            };
            registry.next = registry.next.max(entry.id + 1);
            registry.ids.insert(entry.key, entry.id);
        }
        Ok(registry)
    }

    fn assign(&mut self, keys: BTreeSet<String>, path: &PathBuf) -> GraphStorageResult<()> {
        let missing: Vec<String> = keys
            .into_iter()
            .filter(|key| !self.ids.contains_key(key))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let mut lines = Vec::new();
        for key in missing {
            let id = self.next;
            self.next += 1;
            let mut line = serde_json::to_vec(&RegistryLine {
                key: key.clone(),
                id,
            })
            .map_err(|error| GraphStorageError::OperationFailed {
                operation: "assign_schema_ids",
                message: error.to_string(),
            })?;
            line.push(b'\n');
            lines.extend(line);
            self.ids.insert(key, id);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error(path, error))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| io_error(path, error))?;
        file.write_all(&lines)
            .and_then(|()| file.sync_data())
            .map_err(|error| io_error(path, error))
    }

    fn id(&self, key: &str) -> u64 {
        self.ids.get(key).copied().unwrap_or_default()
    }

    fn remember(&mut self, schema: Arc<GraphSchema>) {
        if self
            .schemas
            .insert(schema.commit_id.clone(), Arc::clone(&schema))
            .is_none()
        {
            self.order.push_back(schema.commit_id.clone());
        }
        while self.order.len() > SCHEMA_CACHE {
            if let Some(evicted) = self.order.pop_front() {
                self.schemas.remove(&evicted);
            }
        }
    }
}

fn io_error(path: &std::path::Path, error: std::io::Error) -> GraphStorageError {
    GraphStorageError::IoOperationFailed {
        operation: "schema_registry",
        path: Some(path.to_path_buf()),
        message: error.to_string(),
    }
}

/// JSON type name of a property value.
pub fn property_value_type(value: &PropertyValue) -> &'static str {
    match value {
        PropertyValue::Null => "null",
        PropertyValue::Bool(_) => "boolean",
        PropertyValue::Integer(_) => "integer",
        PropertyValue::Float(_) => "float",
        PropertyValue::String(_) => "string",
        PropertyValue::StringList(_) => "string_list",
        PropertyValue::IntegerList(_) => "integer_list",
        PropertyValue::FloatList(_) => "float_list",
        PropertyValue::BoolList(_) => "boolean_list",
        PropertyValue::Json(_) => "json",
    }
}

#[derive(Default)]
struct OwnerStats {
    count: usize,
    properties: BTreeMap<String, (BTreeSet<&'static str>, usize)>,
    endpoints: BTreeMap<(String, String), usize>,
}

impl OwnerStats {
    fn observe(&mut self, properties: &PropertyMap) {
        self.count += 1;
        for (key, value) in properties {
            let entry = self.properties.entry(key.clone()).or_default();
            entry.0.insert(property_value_type(value));
            entry.1 += 1;
        }
    }
}

impl CanonicalEngineStore {
    /// Schema of the records visible at a commit, with stable ids.
    pub fn graph_schema(
        &mut self,
        commit_id: &GraphCommitId,
    ) -> GraphStorageResult<Arc<GraphSchema>> {
        let path = SchemaRegistry::path(self);
        if self.schema_registry.is_none() {
            self.schema_registry = Some(SchemaRegistry::load(&path)?);
        }
        if let Some(schema) = self
            .schema_registry
            .as_ref()
            .and_then(|registry| registry.schemas.get(commit_id))
        {
            return Ok(Arc::clone(schema));
        }
        let view = self.commit_view(commit_id)?;
        let (labels, types) = self.collect_schema_stats(&view)?;
        let mut keys = BTreeSet::new();
        for (name, stats) in &labels {
            keys.insert(format!("label:{name}"));
            for key in stats.properties.keys() {
                keys.insert(format!("label:{name}:{key}"));
            }
        }
        for (name, stats) in &types {
            keys.insert(format!("type:{name}"));
            for key in stats.properties.keys() {
                keys.insert(format!("type:{name}:{key}"));
            }
        }
        let registry =
            self.schema_registry
                .as_mut()
                .ok_or_else(|| GraphStorageError::OperationFailed {
                    operation: "graph_schema",
                    message: "schema registry not loaded".to_owned(),
                })?;
        registry.assign(keys, &path)?;
        let properties = |prefix: &str, stats: &OwnerStats| -> Vec<SchemaProperty> {
            stats
                .properties
                .iter()
                .map(|(key, (types, count))| SchemaProperty {
                    id: registry.id(&format!("{prefix}:{key}")),
                    key: key.clone(),
                    value_types: types.iter().map(|value| (*value).to_owned()).collect(),
                    count: *count,
                })
                .collect()
        };
        let schema = Arc::new(GraphSchema {
            commit_id: commit_id.clone(),
            labels: labels
                .iter()
                .map(|(name, stats)| SchemaLabel {
                    id: registry.id(&format!("label:{name}")),
                    name: name.clone(),
                    count: stats.count,
                    properties: properties(&format!("label:{name}"), stats),
                })
                .collect(),
            relationship_types: types
                .iter()
                .map(|(name, stats)| {
                    let mut endpoints: Vec<SchemaEndpoint> = stats
                        .endpoints
                        .iter()
                        .map(|((source_label, target_label), count)| SchemaEndpoint {
                            source_label: source_label.clone(),
                            target_label: target_label.clone(),
                            count: *count,
                        })
                        .collect();
                    endpoints.sort_by_key(|endpoint| std::cmp::Reverse(endpoint.count));
                    SchemaRelationshipType {
                        id: registry.id(&format!("type:{name}")),
                        name: name.clone(),
                        count: stats.count,
                        endpoints,
                        properties: properties(&format!("type:{name}"), stats),
                    }
                })
                .collect(),
        });
        registry.remember(Arc::clone(&schema));
        Ok(schema)
    }

    fn collect_schema_stats(
        &self,
        view: &CommitView,
    ) -> GraphStorageResult<(BTreeMap<String, OwnerStats>, BTreeMap<String, OwnerStats>)> {
        let mut labels: BTreeMap<String, OwnerStats> = BTreeMap::new();
        for entry in view.nodes.values().filter(|entry| !entry.deleted) {
            let node = self.decode_node(entry)?;
            for label in node.labels() {
                labels
                    .entry(label.clone())
                    .or_default()
                    .observe(node.properties());
            }
        }
        let mut types: BTreeMap<String, OwnerStats> = BTreeMap::new();
        for entry in view.relationships.values().filter(|entry| !entry.deleted) {
            let relationship = self.decode_relationship(entry)?;
            let stats = types
                .entry(relationship.rel_type().as_str().to_owned())
                .or_default();
            stats.observe(relationship.properties());
            let endpoint_labels = |id: Option<&String>| -> Vec<String> {
                id.and_then(|id| view.nodes.get(id))
                    .map(|node| node.labels.clone())
                    .filter(|labels| !labels.is_empty())
                    .unwrap_or_else(|| vec![String::new()])
            };
            for source in endpoint_labels(entry.source.as_ref()) {
                for target in endpoint_labels(entry.target.as_ref()) {
                    *stats.endpoints.entry((source.clone(), target)).or_default() += 1;
                }
            }
        }
        Ok((labels, types))
    }
}
