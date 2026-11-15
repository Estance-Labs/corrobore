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

//! Cold tier for graph history (ADR-0023): commits that retention prunes
//! from the hot WAL and JSONL store are archived to Lance datasets, on a
//! local path or an object store, and stay readable at their commit.
//!
//! Layout under the archive URI:
//!
//! - `commits.lance`: one row per archived commit (id, branch, generation,
//!   parents, actor, message, creation time) with the `run_id` and ordinal
//!   of the archive run that wrote it.
//! - `nodes.lance` and `relationships.lance`: one row per record version,
//!   the record serialized as JSON, valid from ordinal `start` (inclusive)
//!   to `end` (exclusive) within its run.
//!
//! Each archive run starts with a keyframe (every record of its first
//! commit) followed by the records that change, so a commit is read from its
//! own run only. The commit rows are written last: they are the visibility
//! point, and record rows of a run that never wrote its commits are ignored.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use futures::TryStreamExt;
use graph_core::{Graph, GraphSequenceFloor, Node, Relationship};
use lance::dataset::builder::DatasetBuilder;
use lance::dataset::{WriteMode, WriteParams};
use lance::deps::arrow_array::{Array, RecordBatch, RecordBatchIterator, StringArray, UInt64Array};
use lance::deps::arrow_schema::{DataType, Field, Schema, SchemaRef};
use lance::io::{ObjectStoreParams, StorageOptionsAccessor};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ColdTierError {
    #[error("cold tier storage failed: {0}")]
    Storage(String),
    #[error("cold tier data is invalid: {0}")]
    Invalid(String),
}

pub type ColdTierResult<T> = Result<T, ColdTierError>;

fn storage(error: impl std::fmt::Display) -> ColdTierError {
    ColdTierError::Storage(error.to_string())
}

/// An archived commit's metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdCommit {
    pub commit_id: String,
    pub branch: String,
    pub generation: u64,
    pub parents: Vec<String>,
    pub actor: Option<String>,
    pub message: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ArchiveReport {
    /// Commits written by this run.
    pub archived: usize,
    /// Commits already in the archive, left as they were.
    pub already_archived: usize,
    pub node_rows: usize,
    pub relationship_rows: usize,
    /// Run identifier, when this call wrote anything.
    pub run_id: Option<String>,
}

/// A Lance archive at one URI (`file:///…`, a local path, or `s3://…`).
#[derive(Clone, Debug)]
pub struct ColdTier {
    uri: String,
    options: HashMap<String, String>,
}

const COMMITS: &str = "commits.lance";
const NODES: &str = "nodes.lance";
const RELATIONSHIPS: &str = "relationships.lance";

impl ColdTier {
    /// `options` are Lance storage options, such as `aws_endpoint`,
    /// `aws_region` or `aws_allow_http`; credentials also come from the
    /// standard AWS environment.
    pub fn new(uri: impl Into<String>, options: HashMap<String, String>) -> Self {
        Self {
            uri: uri.into().trim_end_matches('/').to_owned(),
            options,
        }
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    fn table(&self, name: &str) -> String {
        format!("{}/{name}", self.uri)
    }

    async fn open(&self, name: &str) -> ColdTierResult<Option<lance::Dataset>> {
        match DatasetBuilder::from_uri(self.table(name))
            .with_storage_options(self.options.clone())
            .load()
            .await
        {
            Ok(dataset) => Ok(Some(dataset)),
            Err(lance::Error::DatasetNotFound { .. }) => Ok(None),
            Err(error) => Err(storage(error)),
        }
    }

    async fn append(&self, name: &str, batch: RecordBatch) -> ColdTierResult<()> {
        if batch.num_rows() == 0 {
            return Ok(());
        }
        let exists = self.open(name).await?.is_some();
        let schema = batch.schema();
        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        let params = WriteParams {
            mode: if exists {
                WriteMode::Append
            } else {
                WriteMode::Create
            },
            store_params: Some(ObjectStoreParams {
                storage_options_accessor: Some(Arc::new(
                    StorageOptionsAccessor::with_static_options(self.options.clone()),
                )),
                ..ObjectStoreParams::default()
            }),
            ..WriteParams::default()
        };
        lance::Dataset::write(reader, self.table(name).as_str(), Some(params))
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn scan(&self, name: &str, filter: Option<&str>) -> ColdTierResult<Vec<RecordBatch>> {
        let Some(dataset) = self.open(name).await? else {
            return Ok(Vec::new());
        };
        let mut scanner = dataset.scan();
        if let Some(filter) = filter {
            scanner.filter(filter).map_err(storage)?;
        }
        scanner
            .try_into_stream()
            .await
            .map_err(storage)?
            .try_collect()
            .await
            .map_err(storage)
    }

    /// Every archived commit with the run and ordinal that hold it.
    async fn commit_rows(
        &self,
        filter: Option<&str>,
    ) -> ColdTierResult<Vec<(ColdCommit, String, u64)>> {
        let mut rows = Vec::new();
        for batch in self.scan(COMMITS, filter).await? {
            let json = strings(&batch, "commit_json")?;
            let runs = strings(&batch, "run_id")?;
            let ordinals = numbers(&batch, "ordinal")?;
            for index in 0..batch.num_rows() {
                let commit: ColdCommit = serde_json::from_str(json.value(index))
                    .map_err(|error| ColdTierError::Invalid(error.to_string()))?;
                rows.push((commit, runs.value(index).to_owned(), ordinals.value(index)));
            }
        }
        Ok(rows)
    }

    /// Archived commits, oldest first.
    pub async fn commits(&self) -> ColdTierResult<Vec<ColdCommit>> {
        let mut rows = self.commit_rows(None).await?;
        rows.sort_by(|a, b| {
            (a.0.generation, &a.0.created_at, &a.0.commit_id).cmp(&(
                b.0.generation,
                &b.0.created_at,
                &b.0.commit_id,
            ))
        });
        let mut seen = HashSet::new();
        Ok(rows
            .into_iter()
            .filter(|(commit, _, _)| seen.insert(commit.commit_id.clone()))
            .map(|(commit, _, _)| commit)
            .collect())
    }

    /// One archived commit's metadata.
    pub async fn commit(&self, commit_id: &str) -> ColdTierResult<Option<ColdCommit>> {
        let filter = format!("commit_id = {}", literal(commit_id)?);
        Ok(self
            .commit_rows(Some(&filter))
            .await?
            .into_iter()
            .next()
            .map(|(commit, _, _)| commit))
    }

    /// The full graph at an archived commit, or `None` when it is not archived.
    pub async fn graph_at(&self, commit_id: &str) -> ColdTierResult<Option<Graph>> {
        let filter = format!("commit_id = {}", literal(commit_id)?);
        let Some((_, run, ordinal)) = self.commit_rows(Some(&filter)).await?.into_iter().next()
        else {
            return Ok(None);
        };
        let rows = format!(
            "run_id = {} AND valid_from <= {ordinal} AND valid_to > {ordinal}",
            literal(&run)?
        );
        let mut nodes: Vec<Node> = Vec::new();
        for batch in self.scan(NODES, Some(&rows)).await? {
            let json = strings(&batch, "record_json")?;
            for index in 0..batch.num_rows() {
                nodes.push(
                    serde_json::from_str(json.value(index))
                        .map_err(|error| ColdTierError::Invalid(error.to_string()))?,
                );
            }
        }
        let mut relationships: Vec<Relationship> = Vec::new();
        for batch in self.scan(RELATIONSHIPS, Some(&rows)).await? {
            let json = strings(&batch, "record_json")?;
            for index in 0..batch.num_rows() {
                relationships.push(
                    serde_json::from_str(json.value(index))
                        .map_err(|error| ColdTierError::Invalid(error.to_string()))?,
                );
            }
        }
        let floor = sequence_floor(&nodes, &relationships);
        Graph::from_current_records(nodes, relationships, floor)
            .map(Some)
            .map_err(|error| ColdTierError::Invalid(error.to_string()))
    }

    /// Archive `commits` (oldest first) as one run, loading each commit's
    /// full view with `load` one at a time, so memory holds one view and
    /// the run's open record versions. Commits already in the archive are
    /// skipped, so a failed or repeated cleanup can archive again.
    pub async fn archive<F>(
        &self,
        commits: Vec<ColdCommit>,
        mut load: F,
    ) -> ColdTierResult<ArchiveReport>
    where
        F: FnMut(&ColdCommit) -> Result<Graph, String>,
    {
        let known: HashSet<String> = self
            .commit_rows(None)
            .await?
            .into_iter()
            .map(|(commit, _, _)| commit.commit_id)
            .collect();
        let mut report = ArchiveReport::default();
        let commits: Vec<ColdCommit> = commits
            .into_iter()
            .filter(|commit| {
                let fresh = !known.contains(&commit.commit_id);
                if !fresh {
                    report.already_archived += 1;
                }
                fresh
            })
            .collect();
        if commits.is_empty() {
            return Ok(report);
        }
        let run = uuid::Uuid::new_v4().simple().to_string();
        let mut nodes = Versions::default();
        let mut relationships = Versions::default();
        let mut commit_rows = Vec::new();
        for (ordinal, commit) in commits.iter().enumerate() {
            let ordinal = ordinal as u64;
            let graph = load(commit).map_err(ColdTierError::Storage)?;
            let current_nodes = graph
                .list_nodes()
                .map_err(|error| ColdTierError::Invalid(error.to_string()))?
                .into_iter()
                .map(|node| Ok((node.id().as_str().to_owned(), serialize(&node)?)))
                .collect::<ColdTierResult<BTreeMap<_, _>>>()?;
            let current_relationships = graph
                .list_relationships()
                .map_err(|error| ColdTierError::Invalid(error.to_string()))?
                .into_iter()
                .map(|relationship| {
                    Ok((
                        relationship.id().as_str().to_owned(),
                        serialize(&relationship)?,
                    ))
                })
                .collect::<ColdTierResult<BTreeMap<_, _>>>()?;
            nodes.advance(ordinal, current_nodes);
            relationships.advance(ordinal, current_relationships);
            commit_rows.push((commit.commit_id.clone(), serialize(commit)?, ordinal));
        }
        let end = commits.len() as u64;
        let node_rows = nodes.finish(end);
        let relationship_rows = relationships.finish(end);
        report.node_rows = node_rows.len();
        report.relationship_rows = relationship_rows.len();
        self.append(NODES, version_batch(&run, node_rows)?).await?;
        self.append(RELATIONSHIPS, version_batch(&run, relationship_rows)?)
            .await?;
        // Visibility point: the run's records are readable once its commits are.
        self.append(COMMITS, commit_batch(&run, commit_rows)?)
            .await?;
        report.archived = commits.len();
        report.run_id = Some(run);
        Ok(report)
    }
}

/// Record versions of one run: each id's open version and the closed ones.
#[derive(Default)]
struct Versions {
    open: BTreeMap<String, (String, u64)>,
    closed: Vec<(String, String, u64, u64)>,
}

impl Versions {
    fn advance(&mut self, ordinal: u64, current: BTreeMap<String, String>) {
        let ids: Vec<String> = self.open.keys().cloned().collect();
        for id in ids {
            let changed = current.get(&id) != self.open.get(&id).map(|(json, _)| json);
            if changed && let Some((json, start)) = self.open.remove(&id) {
                self.closed.push((id, json, start, ordinal));
            }
        }
        for (id, json) in current {
            self.open.entry(id).or_insert((json, ordinal));
        }
    }

    fn finish(mut self, end: u64) -> Vec<(String, String, u64, u64)> {
        for (id, (json, start)) in std::mem::take(&mut self.open) {
            self.closed.push((id, json, start, end));
        }
        self.closed
    }
}

fn serialize<T: Serialize>(value: &T) -> ColdTierResult<String> {
    serde_json::to_string(value).map_err(|error| ColdTierError::Invalid(error.to_string()))
}

/// A SQL string literal for a Lance filter; ids never contain quotes, but a
/// quote is refused rather than escaped.
fn literal(value: &str) -> ColdTierResult<String> {
    if value.contains('\'') || value.contains('\\') {
        return Err(ColdTierError::Invalid(format!(
            "{value:?} is not a valid id"
        )));
    }
    Ok(format!("'{value}'"))
}

fn version_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("id", DataType::Utf8, false),
        Field::new("record_json", DataType::Utf8, false),
        Field::new("valid_from", DataType::UInt64, false),
        Field::new("valid_to", DataType::UInt64, false),
    ]))
}

fn version_batch(run: &str, rows: Vec<(String, String, u64, u64)>) -> ColdTierResult<RecordBatch> {
    let runs: StringArray = rows.iter().map(|_| Some(run)).collect();
    let ids: StringArray = rows.iter().map(|row| Some(row.0.as_str())).collect();
    let json: StringArray = rows.iter().map(|row| Some(row.1.as_str())).collect();
    let starts: UInt64Array = rows.iter().map(|row| Some(row.2)).collect();
    let ends: UInt64Array = rows.iter().map(|row| Some(row.3)).collect();
    RecordBatch::try_new(
        version_schema(),
        vec![
            Arc::new(runs),
            Arc::new(ids),
            Arc::new(json),
            Arc::new(starts),
            Arc::new(ends),
        ],
    )
    .map_err(|error| ColdTierError::Invalid(error.to_string()))
}

fn commit_batch(run: &str, rows: Vec<(String, String, u64)>) -> ColdTierResult<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("commit_id", DataType::Utf8, false),
        Field::new("commit_json", DataType::Utf8, false),
        Field::new("ordinal", DataType::UInt64, false),
    ]));
    let runs: StringArray = rows.iter().map(|_| Some(run)).collect();
    let ids: StringArray = rows.iter().map(|row| Some(row.0.as_str())).collect();
    let json: StringArray = rows.iter().map(|row| Some(row.1.as_str())).collect();
    let ordinals: UInt64Array = rows.iter().map(|row| Some(row.2)).collect();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(runs),
            Arc::new(ids),
            Arc::new(json),
            Arc::new(ordinals),
        ],
    )
    .map_err(|error| ColdTierError::Invalid(error.to_string()))
}

fn strings<'a>(batch: &'a RecordBatch, column: &str) -> ColdTierResult<&'a StringArray> {
    batch
        .column_by_name(column)
        .and_then(|array| array.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| ColdTierError::Invalid(format!("column {column} is missing")))
}

fn numbers<'a>(batch: &'a RecordBatch, column: &str) -> ColdTierResult<&'a UInt64Array> {
    batch
        .column_by_name(column)
        .and_then(|array| array.as_any().downcast_ref::<UInt64Array>())
        .ok_or_else(|| ColdTierError::Invalid(format!("column {column} is missing")))
}

/// Highest sequences in use, from the numeric suffix of record and version
/// ids, so a graph read from the archive allocates nothing already taken.
fn sequence_floor(nodes: &[Node], relationships: &[Relationship]) -> GraphSequenceFloor {
    let suffix = |id: &str| {
        id.rsplit(['-', '_'])
            .next()
            .and_then(|tail| tail.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let mut floor = GraphSequenceFloor::default();
    for node in nodes {
        floor.node = floor.node.max(suffix(node.id().as_str()));
        floor.node_version = floor.node_version.max(suffix(node.version_id().as_str()));
    }
    for relationship in relationships {
        floor.relationship = floor.relationship.max(suffix(relationship.id().as_str()));
        floor.relationship_version = floor
            .relationship_version
            .max(suffix(relationship.version_id().as_str()));
    }
    floor
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::Versions;
    use std::collections::BTreeMap;

    fn state(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(id, json)| ((*id).to_owned(), (*json).to_owned()))
            .collect()
    }

    #[test]
    fn versions_close_on_change_and_removal() {
        let mut versions = Versions::default();
        versions.advance(0, state(&[("a", "1"), ("b", "1")]));
        versions.advance(1, state(&[("a", "2"), ("b", "1")]));
        versions.advance(2, state(&[("a", "2")]));
        let mut rows = versions.finish(3);
        rows.sort();
        assert_eq!(
            rows,
            [
                ("a".to_owned(), "1".to_owned(), 0, 1),
                ("a".to_owned(), "2".to_owned(), 1, 3),
                ("b".to_owned(), "1".to_owned(), 0, 2),
            ]
        );
    }
}
