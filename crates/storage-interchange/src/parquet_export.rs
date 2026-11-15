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

//! Apache Parquet export of one commit (issue #327).
//!
//! One file per label (`nodes/<label>.parquet`) and per relationship type
//! (`relationships/<type>.parquet`), plus a `manifest.json`. Every file keeps
//! the native fields and the full properties as JSON, and adds a typed
//! `prop_<key>` column for every property whose values share one scalar type
//! at that commit, so DuckDB, Polars or Spark read the graph without Corrobore.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::builder::{
    BooleanBuilder, Float64Builder, Int64Builder, ListBuilder, StringBuilder,
};
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use graph_core::{Node, PropertyMap, PropertyValue, Relationship};
use graph_storage::{CanonicalEngineStore, GraphCommitId, HistoryRecordKind, SchemaProperty};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::Serialize;

use crate::InterchangeError;

const PAGE: usize = 1_000;

/// One written Parquet file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParquetFile {
    /// Path relative to the export directory.
    pub path: String,
    /// `node` or `relationship`.
    pub kind: &'static str,
    /// Label or relationship type.
    pub name: String,
    /// Rows written.
    pub rows: usize,
    /// Typed property columns.
    pub typed_columns: Vec<String>,
}

/// Export diagnostics, also written as `manifest.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParquetExportReport {
    /// Commit exported.
    pub commit_id: String,
    /// Files written.
    pub files: Vec<ParquetFile>,
    /// Rows written across files.
    pub total_rows: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScalarType {
    Integer,
    Float,
    Text,
    Boolean,
}

fn scalar_type(property: &SchemaProperty) -> Option<ScalarType> {
    let types: Vec<&str> = property
        .value_types
        .iter()
        .map(String::as_str)
        .filter(|value| *value != "null")
        .collect();
    match types.as_slice() {
        ["integer"] => Some(ScalarType::Integer),
        ["float"] | ["float", "integer"] | ["integer", "float"] => Some(ScalarType::Float),
        ["string"] => Some(ScalarType::Text),
        ["boolean"] => Some(ScalarType::Boolean),
        _ => None,
    }
}

/// File-name-safe form of a label or type.
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if stem.is_empty() {
        "_".to_owned()
    } else {
        stem
    }
}

fn property_json(value: &PropertyValue) -> serde_json::Value {
    match value {
        PropertyValue::Null => serde_json::Value::Null,
        PropertyValue::Bool(value) => serde_json::Value::Bool(*value),
        PropertyValue::Integer(value) => serde_json::Value::from(*value),
        PropertyValue::Float(value) => serde_json::Value::from(*value),
        PropertyValue::String(value) => serde_json::Value::String(value.clone()),
        PropertyValue::StringList(values) => serde_json::Value::from(values.clone()),
        PropertyValue::IntegerList(values) => serde_json::Value::from(values.clone()),
        PropertyValue::FloatList(values) => serde_json::Value::from(values.clone()),
        PropertyValue::BoolList(values) => serde_json::Value::from(values.clone()),
        PropertyValue::Json(value) => value.clone(),
    }
}

fn properties_json(properties: &PropertyMap) -> String {
    let ordered: BTreeMap<&String, serde_json::Value> = properties
        .iter()
        .map(|(key, value)| (key, property_json(value)))
        .collect();
    serde_json::to_string(&ordered).unwrap_or_else(|_| "{}".to_owned())
}

/// Rows of one file: native columns, labels, properties.
struct Row<'a> {
    id: &'a str,
    version_id: &'a str,
    status: String,
    labels: Vec<String>,
    endpoints: Option<(&'a str, &'a str)>,
    first_seen: Option<&'a str>,
    last_seen: Option<&'a str>,
    properties: &'a PropertyMap,
}

fn status_name(status: graph_core::RecordStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{status:?}"))
}

fn node_row(node: &Node) -> Row<'_> {
    Row {
        id: node.id().as_str(),
        version_id: node.version_id().as_str(),
        status: status_name(node.status()),
        labels: node.labels().to_vec(),
        endpoints: None,
        first_seen: node.first_seen(),
        last_seen: node.last_seen(),
        properties: node.properties(),
    }
}

fn relationship_row(relationship: &Relationship) -> Row<'_> {
    Row {
        id: relationship.id().as_str(),
        version_id: relationship.version_id().as_str(),
        status: status_name(relationship.status()),
        labels: vec![relationship.rel_type().as_str().to_owned()],
        endpoints: Some((
            relationship.source().as_str(),
            relationship.target().as_str(),
        )),
        first_seen: relationship.first_seen(),
        last_seen: relationship.last_seen(),
        properties: relationship.properties(),
    }
}

fn write_file(
    path: &Path,
    rows: &[Row<'_>],
    typed: &[(String, ScalarType)],
    relationships: bool,
) -> Result<(), InterchangeError> {
    let mut fields = vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("version_id", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
    ];
    let mut columns: Vec<ArrayRef> = Vec::new();
    let text = |values: Vec<Option<&str>>| -> ArrayRef {
        let mut builder = StringBuilder::new();
        for value in values {
            builder.append_option(value);
        }
        Arc::new(builder.finish())
    };
    columns.push(text(rows.iter().map(|row| Some(row.id)).collect()));
    columns.push(text(rows.iter().map(|row| Some(row.version_id)).collect()));
    columns.push(text(
        rows.iter().map(|row| Some(row.status.as_str())).collect(),
    ));
    if relationships {
        fields.push(Field::new("type", DataType::Utf8, false));
        fields.push(Field::new("source", DataType::Utf8, false));
        fields.push(Field::new("target", DataType::Utf8, false));
        columns.push(text(
            rows.iter()
                .map(|row| row.labels.first().map(String::as_str))
                .collect(),
        ));
        columns.push(text(
            rows.iter()
                .map(|row| row.endpoints.map(|(source, _)| source))
                .collect(),
        ));
        columns.push(text(
            rows.iter()
                .map(|row| row.endpoints.map(|(_, target)| target))
                .collect(),
        ));
    } else {
        let mut labels = ListBuilder::new(StringBuilder::new());
        for row in rows {
            for label in &row.labels {
                labels.values().append_value(label);
            }
            labels.append(true);
        }
        let list = labels.finish();
        fields.push(Field::new("labels", list.data_type().clone(), false));
        columns.push(Arc::new(list));
    }
    fields.push(Field::new("first_seen", DataType::Utf8, true));
    fields.push(Field::new("last_seen", DataType::Utf8, true));
    fields.push(Field::new("properties", DataType::Utf8, false));
    columns.push(text(rows.iter().map(|row| row.first_seen).collect()));
    columns.push(text(rows.iter().map(|row| row.last_seen).collect()));
    let json: Vec<String> = rows
        .iter()
        .map(|row| properties_json(row.properties))
        .collect();
    columns.push(text(
        json.iter().map(|value| Some(value.as_str())).collect(),
    ));
    for (key, kind) in typed {
        let name = format!("prop_{key}");
        let values = rows.iter().map(|row| row.properties.get(key));
        let (data_type, column): (DataType, ArrayRef) = match kind {
            ScalarType::Integer => {
                let mut builder = Int64Builder::new();
                for value in values {
                    builder.append_option(match value {
                        Some(PropertyValue::Integer(value)) => Some(*value),
                        _ => None,
                    });
                }
                (DataType::Int64, Arc::new(builder.finish()))
            }
            ScalarType::Float => {
                let mut builder = Float64Builder::new();
                for value in values {
                    #[allow(clippy::cast_precision_loss)]
                    builder.append_option(match value {
                        Some(PropertyValue::Float(value)) => Some(*value),
                        Some(PropertyValue::Integer(value)) => Some(*value as f64),
                        _ => None,
                    });
                }
                (DataType::Float64, Arc::new(builder.finish()))
            }
            ScalarType::Text => {
                let mut builder = StringBuilder::new();
                for value in values {
                    builder.append_option(match value {
                        Some(PropertyValue::String(value)) => Some(value.as_str()),
                        _ => None,
                    });
                }
                (DataType::Utf8, Arc::new(builder.finish()))
            }
            ScalarType::Boolean => {
                let mut builder = BooleanBuilder::new();
                for value in values {
                    builder.append_option(match value {
                        Some(PropertyValue::Bool(value)) => Some(*value),
                        _ => None,
                    });
                }
                (DataType::Boolean, Arc::new(builder.finish()))
            }
        };
        fields.push(Field::new(name, data_type, true));
        columns.push(column);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns)
        .map_err(|error| InterchangeError::Export(error.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| InterchangeError::Export(error.to_string()))?;
    }
    let file = File::create(path).map_err(|error| InterchangeError::Export(error.to_string()))?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties))
        .map_err(|error| InterchangeError::Export(error.to_string()))?;
    writer
        .write(&batch)
        .map_err(|error| InterchangeError::Export(error.to_string()))?;
    writer
        .close()
        .map_err(|error| InterchangeError::Export(error.to_string()))?;
    Ok(())
}

/// Export every live record of a commit to Parquet files under `destination`,
/// which must not exist yet.
pub fn export_commit_parquet(
    store: &mut CanonicalEngineStore,
    commit_id: &GraphCommitId,
    destination: &Path,
) -> Result<ParquetExportReport, InterchangeError> {
    if destination.exists() {
        return Err(InterchangeError::Export(format!(
            "destination {} already exists",
            destination.display()
        )));
    }
    let schema = store.graph_schema(commit_id)?;
    let mut files = Vec::new();
    for (kind, owners) in [
        (
            HistoryRecordKind::Node,
            schema
                .labels
                .iter()
                .map(|label| (&label.name, &label.properties))
                .collect::<Vec<_>>(),
        ),
        (
            HistoryRecordKind::Relationship,
            schema
                .relationship_types
                .iter()
                .map(|kind| (&kind.name, &kind.properties))
                .collect::<Vec<_>>(),
        ),
    ] {
        for (name, properties) in owners {
            let typed: Vec<(String, ScalarType)> = properties
                .iter()
                .filter_map(|property| {
                    scalar_type(property).map(|kind| (property.key.clone(), kind))
                })
                .collect();
            let mut nodes = Vec::new();
            let mut relationships = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let page = store.list_commit_records(
                    commit_id,
                    kind,
                    Some(name),
                    cursor.as_deref(),
                    PAGE,
                    false,
                )?;
                nodes.extend(page.nodes);
                relationships.extend(page.relationships);
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            let (directory, rows): (&str, Vec<Row<'_>>) = match kind {
                HistoryRecordKind::Node => ("nodes", nodes.iter().map(node_row).collect()),
                HistoryRecordKind::Relationship => (
                    "relationships",
                    relationships.iter().map(relationship_row).collect(),
                ),
            };
            let relative = format!("{directory}/{}.parquet", file_stem(name));
            write_file(
                &destination.join(&relative),
                &rows,
                &typed,
                kind == HistoryRecordKind::Relationship,
            )?;
            files.push(ParquetFile {
                path: relative,
                kind: match kind {
                    HistoryRecordKind::Node => "node",
                    HistoryRecordKind::Relationship => "relationship",
                },
                name: name.clone(),
                rows: rows.len(),
                typed_columns: typed.iter().map(|(key, _)| format!("prop_{key}")).collect(),
            });
        }
    }
    fs::create_dir_all(destination).map_err(|error| InterchangeError::Export(error.to_string()))?;
    let report = ParquetExportReport {
        commit_id: commit_id.to_string(),
        total_rows: files.iter().map(|file| file.rows).sum(),
        files,
    };
    let manifest = serde_json::to_vec_pretty(&report)
        .map_err(|error| InterchangeError::Export(error.to_string()))?;
    let manifest_path: PathBuf = destination.join("manifest.json");
    fs::write(&manifest_path, manifest)
        .map_err(|error| InterchangeError::Export(error.to_string()))?;
    Ok(report)
}
