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

//! Vector index and hybrid retrieval (issue #326).
//!
//! Nodes carry embeddings as numeric list properties. The index is exact
//! (every vector is compared), kept per commit and moved forward with the
//! changes of the commits in between, behind the [`VectorIndex`] trait so an
//! approximate index can replace it without changing callers. Hybrid search
//! fuses BM25 full text, vector similarity and graph proximity with
//! Reciprocal Rank Fusion.

use std::collections::{BTreeMap, HashMap, HashSet};

use graph_core::{Node, PropertyValue};
use opencti_search::{FullTextMatchMode, FullTextQuery};

use super::CanonicalEngineStore;
use crate::graph_history::{CommitView, GraphCommitId, HistoryRecordKind, MAIN_BRANCH};
use crate::{CanonicalAccessContext, GraphStorageError, GraphStorageResult};

/// Reciprocal Rank Fusion constant from Cormack et al. (2009).
pub const RRF_K: f64 = 60.0;
/// Commits replayed forward before an index is rebuilt from scratch.
const MAX_FORWARD_COMMITS: usize = 1_000;

/// Similarity used to rank vectors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorMetric {
    /// Cosine similarity; higher is closer.
    #[default]
    Cosine,
    /// Negative Euclidean distance; higher is closer.
    L2,
}

/// Index of node embeddings.
pub trait VectorIndex: Send + std::fmt::Debug {
    /// Insert or replace a vector.
    fn upsert(&mut self, id: String, vector: Vec<f32>);
    /// Remove a vector.
    fn remove(&mut self, id: &str);
    /// The `k` closest vectors with their similarity, best first.
    fn search(&self, query: &[f32], k: usize, metric: VectorMetric) -> Vec<(String, f64)>;
    /// Dimensions of the indexed vectors, once one is indexed.
    fn dimensions(&self) -> Option<usize>;
    /// Indexed vectors.
    fn len(&self) -> usize;
    /// Whether the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Copy of the index behind a new box.
    fn boxed_clone(&self) -> Box<dyn VectorIndex>;
}

/// Exact index: compares the query with every vector.
#[derive(Clone, Debug, Default)]
pub struct ExactVectorIndex {
    vectors: BTreeMap<String, Vec<f32>>,
    dimensions: Option<usize>,
}

fn similarity(left: &[f32], right: &[f32], metric: VectorMetric) -> f64 {
    match metric {
        VectorMetric::Cosine => {
            let (mut dot, mut left_norm, mut right_norm) = (0.0_f64, 0.0_f64, 0.0_f64);
            for (a, b) in left.iter().zip(right) {
                dot += f64::from(*a) * f64::from(*b);
                left_norm += f64::from(*a) * f64::from(*a);
                right_norm += f64::from(*b) * f64::from(*b);
            }
            if left_norm == 0.0 || right_norm == 0.0 {
                0.0
            } else {
                dot / (left_norm.sqrt() * right_norm.sqrt())
            }
        }
        VectorMetric::L2 => -left
            .iter()
            .zip(right)
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
            .sum::<f64>()
            .sqrt(),
    }
}

impl VectorIndex for ExactVectorIndex {
    fn upsert(&mut self, id: String, vector: Vec<f32>) {
        if self.dimensions.is_none() {
            self.dimensions = Some(vector.len());
        }
        self.vectors.insert(id, vector);
    }

    fn remove(&mut self, id: &str) {
        self.vectors.remove(id);
    }

    fn search(&self, query: &[f32], k: usize, metric: VectorMetric) -> Vec<(String, f64)> {
        let mut scored: Vec<(String, f64)> = self
            .vectors
            .iter()
            .filter(|(_, vector)| vector.len() == query.len())
            .map(|(id, vector)| (id.clone(), similarity(query, vector, metric)))
            .collect();
        scored.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        scored.truncate(k);
        scored
    }

    fn dimensions(&self) -> Option<usize> {
        self.dimensions
    }

    fn len(&self) -> usize {
        self.vectors.len()
    }

    fn boxed_clone(&self) -> Box<dyn VectorIndex> {
        Box::new(self.clone())
    }
}

/// Fuse rankings (best first) with Reciprocal Rank Fusion: each item scores
/// the sum of `1 / (k + rank)` over the rankings it appears in.
pub fn reciprocal_rank_fusion(rankings: &[Vec<String>], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<&str, f64> = HashMap::new();
    for ranking in rankings {
        for (index, id) in ranking.iter().enumerate() {
            *scores.entry(id.as_str()).or_default() += 1.0 / (k + index as f64 + 1.0);
        }
    }
    let mut fused: Vec<(String, f64)> = scores
        .into_iter()
        .map(|(id, score)| (id.to_owned(), score))
        .collect();
    fused.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    fused
}

/// Numeric list property as an embedding.
fn embedding(node: &Node, property: &str) -> Option<Vec<f32>> {
    match node.properties().get(property)? {
        #[allow(clippy::cast_possible_truncation)]
        PropertyValue::FloatList(values) => Some(
            values
                .iter()
                .map(|value| *value as f32)
                .collect::<Vec<f32>>(),
        ),
        #[allow(clippy::cast_precision_loss)]
        PropertyValue::IntegerList(values) => Some(
            values
                .iter()
                .map(|value| *value as f32)
                .collect::<Vec<f32>>(),
        ),
        _ => None,
    }
    .filter(|vector| !vector.is_empty())
}

/// Vector index kept for one property at one commit.
#[derive(Debug)]
pub(crate) struct VectorIndexCache {
    property: String,
    commit: GraphCommitId,
    index: Box<dyn VectorIndex>,
}

impl Clone for VectorIndexCache {
    fn clone(&self) -> Self {
        Self {
            property: self.property.clone(),
            commit: self.commit.clone(),
            index: self.index.boxed_clone(),
        }
    }
}

/// One vector or hybrid hit.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphSearchHit {
    /// Node id.
    pub node_id: String,
    /// Similarity for vector search, fused RRF score for hybrid search.
    pub score: f64,
    /// Node labels at the commit.
    pub labels: Vec<String>,
    /// 1-based rank per signal (`full_text`, `vector`, `graph`).
    pub ranks: BTreeMap<&'static str, usize>,
}

/// A hybrid retrieval request.
#[derive(Clone, Debug, Default)]
pub struct HybridSearchRequest {
    /// Text for BM25 full-text retrieval.
    pub text: Option<String>,
    /// Query embedding for vector retrieval.
    pub vector: Option<Vec<f32>>,
    /// Embedding property (default `embedding`).
    pub property: Option<String>,
    /// Hits to return.
    pub k: usize,
    /// Whether neighbours of the best text and vector hits join the fusion.
    pub graph_expansion: bool,
    /// Caller access for the access-aware full-text index.
    pub access: CanonicalAccessContext,
}

/// Default embedding property.
pub const DEFAULT_EMBEDDING_PROPERTY: &str = "embedding";

impl CanonicalEngineStore {
    /// The `k` nodes whose `property` embedding is closest to `query` at a commit.
    pub fn vector_search(
        &mut self,
        commit_id: &GraphCommitId,
        property: &str,
        query: &[f32],
        k: usize,
        metric: VectorMetric,
        label: Option<&str>,
    ) -> GraphStorageResult<Vec<GraphSearchHit>> {
        if query.is_empty() || query.iter().any(|value| !value.is_finite()) {
            return Err(invalid_vector("the query vector must hold finite numbers"));
        }
        let view = self.commit_view(commit_id)?;
        self.ensure_vector_index(commit_id, property, &view)?;
        let cache = self
            .vector_cache
            .as_ref()
            .ok_or_else(|| invalid_vector("vector index unavailable"))?;
        if let Some(dimensions) = cache.index.dimensions()
            && dimensions != query.len()
        {
            return Err(invalid_vector(&format!(
                "the query has {} dimensions but {property} embeddings have {dimensions}",
                query.len()
            )));
        }
        let candidates = if label.is_some() {
            cache.index.len()
        } else {
            k
        };
        Ok(cache
            .index
            .search(query, candidates.max(k), metric)
            .into_iter()
            .filter_map(|(id, score)| {
                let labels = view.nodes.get(&id)?.labels.clone();
                if label.is_some_and(|label| !labels.iter().any(|value| value == label)) {
                    return None;
                }
                Some(GraphSearchHit {
                    node_id: id,
                    score,
                    labels,
                    ranks: BTreeMap::new(),
                })
            })
            .take(k)
            .enumerate()
            .map(|(index, mut hit)| {
                hit.ranks.insert("vector", index + 1);
                hit
            })
            .collect())
    }

    /// Fuse BM25 full text, vector similarity and graph proximity with RRF.
    ///
    /// Full text reads the published index, which follows the `main` head, so
    /// it joins only when `commit_id` is that head.
    pub fn hybrid_search(
        &mut self,
        commit_id: &GraphCommitId,
        request: &HybridSearchRequest,
    ) -> GraphStorageResult<Vec<GraphSearchHit>> {
        let k = request.k.clamp(1, 1_000);
        let depth = (k * 3).max(30);
        let view = self.commit_view(commit_id)?;
        let mut rankings: Vec<(&'static str, Vec<String>)> = Vec::new();
        if let Some(text) = request
            .text
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            && &self.graph_head(MAIN_BRANCH)? == commit_id
        {
            let page = self
                .search_full_text(
                    &FullTextQuery {
                        text: text.to_owned(),
                        mode: FullTextMatchMode::Term,
                        fields: Vec::new(),
                        kinds: Vec::new(),
                        filters: Vec::new(),
                        limit: u32::try_from(depth).unwrap_or(u32::MAX),
                        cursor: None,
                    },
                    &request.access,
                )
                .map_err(|error| invalid_vector(&error.to_string()))?;
            let ids = page
                .hits
                .iter()
                .filter_map(|hit| self.node_id_for_document(&view, &hit.id))
                .collect();
            rankings.push(("full_text", ids));
        }
        if let Some(vector) = &request.vector {
            let property = request
                .property
                .as_deref()
                .unwrap_or(DEFAULT_EMBEDDING_PROPERTY)
                .to_owned();
            let ids = self
                .vector_search(
                    commit_id,
                    &property,
                    vector,
                    depth,
                    VectorMetric::Cosine,
                    None,
                )?
                .into_iter()
                .map(|hit| hit.node_id)
                .collect();
            rankings.push(("vector", ids));
        }
        if rankings.is_empty() {
            return Err(invalid_vector(
                "hybrid search needs text, a vector, or both",
            ));
        }
        if request.graph_expansion {
            let seeds: Vec<String> = reciprocal_rank_fusion(
                &rankings
                    .iter()
                    .map(|(_, ids)| ids.clone())
                    .collect::<Vec<_>>(),
                RRF_K,
            )
            .into_iter()
            .take(10)
            .map(|(id, _)| id)
            .collect();
            rankings.push(("graph", neighbours_by_links(&view, &seeds)));
        }
        let fused = reciprocal_rank_fusion(
            &rankings
                .iter()
                .map(|(_, ids)| ids.clone())
                .collect::<Vec<_>>(),
            RRF_K,
        );
        Ok(fused
            .into_iter()
            .filter_map(|(id, score)| {
                let labels = view
                    .nodes
                    .get(&id)
                    .filter(|entry| !entry.deleted)?
                    .labels
                    .clone();
                let ranks = rankings
                    .iter()
                    .filter_map(|(signal, ids)| {
                        ids.iter()
                            .position(|item| item == &id)
                            .map(|rank| (*signal, rank + 1))
                    })
                    .collect();
                Some(GraphSearchHit {
                    node_id: id,
                    score,
                    labels,
                    ranks,
                })
            })
            .take(k)
            .collect())
    }

    fn node_id_for_document(&self, view: &CommitView, document_id: &str) -> Option<String> {
        if view.nodes.contains_key(document_id) {
            return Some(document_id.to_owned());
        }
        crate::resolve_identifier_index_entries(&self.state.catalog, document_id)
            .into_iter()
            .map(|entry| entry.node_id.as_str().to_owned())
            .find(|id| view.nodes.contains_key(id))
    }

    fn ensure_vector_index(
        &mut self,
        commit_id: &GraphCommitId,
        property: &str,
        view: &CommitView,
    ) -> GraphStorageResult<()> {
        if let Some(cache) = &self.vector_cache
            && cache.property == property
        {
            if &cache.commit == commit_id {
                return Ok(());
            }
            // Move forward along the first-parent chain when the cached commit
            // is a recent ancestor; otherwise rebuild.
            let mut chain = Vec::new();
            let mut current = Some(self.history.commit(commit_id)?);
            let mut reached = false;
            while let Some(commit) = current.take() {
                if commit.commit_id == cache.commit {
                    reached = true;
                    break;
                }
                if chain.len() > MAX_FORWARD_COMMITS {
                    break;
                }
                current = commit
                    .parents
                    .first()
                    .and_then(|parent| self.history.commit(parent).ok());
                chain.push(commit);
            }
            if reached {
                let mut changed: HashSet<String> = HashSet::new();
                for commit in &chain {
                    for change in &commit.changes {
                        if change.kind == HistoryRecordKind::Node {
                            changed.insert(change.record_id.clone());
                        }
                    }
                }
                let mut updates = Vec::with_capacity(changed.len());
                for id in changed {
                    let vector = match view.nodes.get(&id).filter(|entry| !entry.deleted) {
                        Some(entry) => embedding(&self.decode_node(entry)?, property),
                        None => None,
                    };
                    updates.push((id, vector));
                }
                if let Some(cache) = self.vector_cache.as_mut() {
                    for (id, vector) in updates {
                        match vector {
                            Some(vector) => cache.index.upsert(id, vector),
                            None => cache.index.remove(&id),
                        }
                    }
                    cache.commit = commit_id.clone();
                }
                return Ok(());
            }
        }
        let mut index: Box<dyn VectorIndex> = Box::new(ExactVectorIndex::default());
        for (id, entry) in view.nodes.iter().filter(|(_, entry)| !entry.deleted) {
            if let Some(vector) = embedding(&self.decode_node(entry)?, property) {
                index.upsert(id.clone(), vector);
            }
        }
        self.vector_cache = Some(VectorIndexCache {
            property: property.to_owned(),
            commit: commit_id.clone(),
            index,
        });
        Ok(())
    }
}

/// Neighbours of `seeds`, ranked by how many seeds they connect to.
fn neighbours_by_links(view: &CommitView, seeds: &[String]) -> Vec<String> {
    let seed_set: HashSet<&str> = seeds.iter().map(String::as_str).collect();
    let mut links: BTreeMap<String, usize> = BTreeMap::new();
    for entry in view.relationships.values().filter(|entry| !entry.deleted) {
        let (Some(source), Some(target)) = (&entry.source, &entry.target) else {
            continue;
        };
        if seed_set.contains(source.as_str()) && !seed_set.contains(target.as_str()) {
            *links.entry(target.clone()).or_default() += 1;
        }
        if seed_set.contains(target.as_str()) && !seed_set.contains(source.as_str()) {
            *links.entry(source.clone()).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = links.into_iter().collect();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked.into_iter().map(|(id, _)| id).collect()
}

fn invalid_vector(message: &str) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "graph_search",
        message: message.to_owned(),
    }
}
