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

//! Approximate nearest-neighbour index (issue #342).
//!
//! A Hierarchical Navigable Small World graph (Malkov and Yashunin, 2018):
//! every vector is a node linked to close neighbours on layer 0 and, with a
//! geometrically decreasing probability, on the layers above it. A query
//! descends greedily from the top layer and then runs a beam search of width
//! `ef_search` on layer 0, so it compares a few hundred vectors instead of all
//! of them.
//!
//! The graph is built for one [`VectorMetric`]; a query with another metric is
//! answered by an exact scan of the same vectors. The layer of a vector is
//! drawn from a hash of its record id and the configured seed, and every tie
//! is broken by insertion slot, so the same operations always build the same
//! graph and return the same hits. Removals leave tombstones that still route
//! searches but are never returned; the graph is rebuilt from the live vectors
//! once tombstones outnumber them.

use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BinaryHeap, HashMap, HashSet},
    fmt,
};

use super::{VectorIndex, VectorIndexKind, VectorMetric, ranked, similarity};

/// Highest layer a vector can be drawn on.
const MAX_LAYER: usize = 16;
/// Tombstones tolerated before a rebuild is considered at all.
const MIN_REBUILD_TOMBSTONES: usize = 64;
/// Graphs up to this many slots track visited nodes in a bitset per query.
const BITSET_VISIT_LIMIT: usize = 1 << 20;

/// Construction and search parameters of an [`HnswVectorIndex`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HnswParams {
    /// Links kept per node on the upper layers; layer 0 keeps twice as many.
    pub m: usize,
    /// Beam width while inserting; larger builds a better graph, slower.
    pub ef_construction: usize,
    /// Beam width while searching, raised to `k` when smaller; larger trades
    /// latency for recall.
    pub ef_search: usize,
    /// Seed mixed with each record id to draw the layer of its vector.
    pub seed: u64,
}

impl Default for HnswParams {
    fn default() -> Self {
        Self {
            m: 16,
            ef_construction: 128,
            ef_search: 128,
            seed: 0x0342_0342_0342_0342,
        }
    }
}

/// HNSW index over node embeddings, built for one metric.
#[derive(Clone)]
pub struct HnswVectorIndex {
    metric: VectorMetric,
    params: HnswParams,
    dimensions: Option<usize>,
    /// Record id per slot, live or tombstoned.
    ids: Vec<String>,
    /// Vector per slot.
    vectors: Vec<Vec<f32>>,
    /// `1 / ‖v‖` per slot for cosine; 0 for a zero vector.
    inverse_norms: Vec<f32>,
    /// Neighbour slots per slot and layer.
    links: Vec<Vec<Vec<u32>>>,
    /// Tombstone flag per slot.
    deleted: Vec<bool>,
    /// Live slot of each record id.
    slots: HashMap<String, u32>,
    /// Vectors whose dimensions differ from the graph's; only an exact scan
    /// with a query of their size can return them.
    strays: BTreeMap<String, Vec<f32>>,
    /// Entry slot on the top layer.
    entry: Option<u32>,
    top_layer: usize,
    tombstones: usize,
    /// Visit marks reused across insertions.
    marks: Vec<u32>,
    epoch: u32,
}

impl fmt::Debug for HnswVectorIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HnswVectorIndex")
            .field("metric", &self.metric)
            .field("params", &self.params)
            .field("dimensions", &self.dimensions)
            .field("live", &self.slots.len())
            .field("strays", &self.strays.len())
            .field("tombstones", &self.tombstones)
            .field("top_layer", &self.top_layer)
            .finish_non_exhaustive()
    }
}

/// A slot and its distance to the current target; ordered by distance, then
/// slot, so heaps and sorts never depend on anything but the data.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    distance: f32,
    slot: u32,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .total_cmp(&other.distance)
            .then_with(|| self.slot.cmp(&other.slot))
    }
}

/// Visited-slot tracking for one layer search.
trait Visit {
    /// Marks `slot` and reports whether it was not marked yet.
    fn first_visit(&mut self, slot: u32) -> bool;
}

/// Epoch-stamped marks reused across insertions.
struct Stamps<'a> {
    marks: &'a mut [u32],
    epoch: u32,
}

impl Visit for Stamps<'_> {
    fn first_visit(&mut self, slot: u32) -> bool {
        let mark = &mut self.marks[slot as usize];
        if *mark == self.epoch {
            false
        } else {
            *mark = self.epoch;
            true
        }
    }
}

/// Per-query visit tracking: a bitset for graphs of moderate size, a hash set
/// beyond that so a query never clears megabytes of marks.
enum QueryVisits {
    Bits(Vec<u64>),
    Set(HashSet<u32>),
}

impl QueryVisits {
    fn new(slots: usize) -> Self {
        if slots <= BITSET_VISIT_LIMIT {
            Self::Bits(vec![0; slots.div_ceil(64)])
        } else {
            Self::Set(HashSet::new())
        }
    }
}

impl Visit for QueryVisits {
    fn first_visit(&mut self, slot: u32) -> bool {
        match self {
            Self::Bits(bits) => {
                let (word, bit) = (slot as usize / 64, 1_u64 << (slot % 64));
                let fresh = bits[word] & bit == 0;
                bits[word] |= bit;
                fresh
            }
            Self::Set(set) => set.insert(slot),
        }
    }
}

/// Sum of `term(a, b)` over paired lanes, accumulated in eight independent
/// lanes and combined in a fixed order: the result is reproducible and the
/// loop vectorises without reassociating floats.
#[inline]
fn lane_sum(left: &[f32], right: &[f32], term: impl Fn(f32, f32) -> f32) -> f32 {
    let mut lanes = [0.0_f32; 8];
    let mut left_chunks = left.chunks_exact(8);
    let mut right_chunks = right.chunks_exact(8);
    for (a, b) in (&mut left_chunks).zip(&mut right_chunks) {
        lanes[0] += term(a[0], b[0]);
        lanes[1] += term(a[1], b[1]);
        lanes[2] += term(a[2], b[2]);
        lanes[3] += term(a[3], b[3]);
        lanes[4] += term(a[4], b[4]);
        lanes[5] += term(a[5], b[5]);
        lanes[6] += term(a[6], b[6]);
        lanes[7] += term(a[7], b[7]);
    }
    let mut sum = ((lanes[0] + lanes[1]) + (lanes[2] + lanes[3]))
        + ((lanes[4] + lanes[5]) + (lanes[6] + lanes[7]));
    for (a, b) in left_chunks.remainder().iter().zip(right_chunks.remainder()) {
        sum += term(*a, *b);
    }
    sum
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    lane_sum(left, right, |a, b| a * b)
}

fn squared_l2(left: &[f32], right: &[f32]) -> f32 {
    lane_sum(left, right, |a, b| (a - b) * (a - b))
}

fn inverse_norm(vector: &[f32]) -> f32 {
    let norm = dot(vector, vector).sqrt();
    if norm > 0.0 && norm.is_finite() {
        1.0 / norm
    } else {
        0.0
    }
}

/// Next epoch of epoch-stamped marks, clearing them when the counter wraps.
fn advance(marks: &mut [u32], epoch: &mut u32) -> u32 {
    *epoch = epoch.wrapping_add(1);
    if *epoch == 0 {
        marks.fill(0);
        *epoch = 1;
    }
    *epoch
}

/// FNV-1a over the id bytes, finished with the SplitMix64 mixer.
fn id_hash(seed: u64, id: &str) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64 ^ seed;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    hash ^ (hash >> 31)
}

impl HnswVectorIndex {
    /// Empty index whose graph serves `metric`. `m` is raised to 2,
    /// `ef_construction` to `m` and `ef_search` to 1 when smaller.
    #[must_use]
    pub fn new(metric: VectorMetric, params: HnswParams) -> Self {
        let m = params.m.max(2);
        Self {
            metric,
            params: HnswParams {
                m,
                ef_construction: params.ef_construction.max(m),
                ef_search: params.ef_search.max(1),
                seed: params.seed,
            },
            dimensions: None,
            ids: Vec::new(),
            vectors: Vec::new(),
            inverse_norms: Vec::new(),
            links: Vec::new(),
            deleted: Vec::new(),
            slots: HashMap::new(),
            strays: BTreeMap::new(),
            entry: None,
            top_layer: 0,
            tombstones: 0,
            marks: Vec::new(),
            epoch: 0,
        }
    }

    /// Metric the graph is built for.
    #[must_use]
    pub fn metric(&self) -> VectorMetric {
        self.metric
    }

    /// Effective parameters.
    #[must_use]
    pub fn params(&self) -> HnswParams {
        self.params
    }

    /// Removed or replaced vectors still present in the graph.
    #[must_use]
    pub fn tombstones(&self) -> usize {
        self.tombstones
    }

    /// Layer of a record, drawn from `-ln(U) / ln(m)` with `U` uniform in
    /// (0, 1] derived from the id hash, so it is the same on every build.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn layer_for(&self, id: &str) -> usize {
        let hash = id_hash(self.params.seed, id);
        let uniform = ((hash >> 11) as f64 + 1.0) / (1_u64 << 53) as f64;
        let scale = 1.0 / (self.params.m as f64).ln();
        ((-uniform.ln() * scale).floor() as usize).min(MAX_LAYER)
    }

    fn max_links(&self, layer: usize) -> usize {
        if layer == 0 {
            self.params.m * 2
        } else {
            self.params.m
        }
    }

    /// Graph distance from a prepared query (vector and inverse norm) to a
    /// slot: `1 - cos` or the squared Euclidean distance.
    fn distance(&self, query: &[f32], query_inverse_norm: f32, slot: u32) -> f32 {
        let index = slot as usize;
        match self.metric {
            VectorMetric::Cosine => {
                let scale = query_inverse_norm * self.inverse_norms[index];
                if scale == 0.0 {
                    1.0
                } else {
                    1.0 - dot(query, &self.vectors[index]) * scale
                }
            }
            VectorMetric::L2 => squared_l2(query, &self.vectors[index]),
        }
    }

    fn slot_distance(&self, left: u32, right: u32) -> f32 {
        self.distance(
            &self.vectors[left as usize],
            self.inverse_norms[left as usize],
            right,
        )
    }

    /// Beam search of width `ef` on one layer from `entries`; the closest
    /// candidates found, nearest first. Tombstones are kept: they route.
    fn search_layer(
        &self,
        query: &[f32],
        query_inverse_norm: f32,
        entries: &[u32],
        ef: usize,
        layer: usize,
        visits: &mut impl Visit,
    ) -> Vec<Candidate> {
        let mut frontier: BinaryHeap<Reverse<Candidate>> = BinaryHeap::new();
        let mut found: BinaryHeap<Candidate> = BinaryHeap::new();
        for &slot in entries {
            if visits.first_visit(slot) {
                let candidate = Candidate {
                    distance: self.distance(query, query_inverse_norm, slot),
                    slot,
                };
                frontier.push(Reverse(candidate));
                found.push(candidate);
            }
        }
        while found.len() > ef {
            found.pop();
        }
        while let Some(Reverse(current)) = frontier.pop() {
            if found.len() >= ef && found.peek().is_some_and(|worst| current > *worst) {
                break;
            }
            let Some(neighbours) = self.links[current.slot as usize].get(layer) else {
                continue;
            };
            for &neighbour in neighbours {
                if !visits.first_visit(neighbour) {
                    continue;
                }
                let candidate = Candidate {
                    distance: self.distance(query, query_inverse_norm, neighbour),
                    slot: neighbour,
                };
                if found.len() < ef || found.peek().is_some_and(|worst| candidate < *worst) {
                    frontier.push(Reverse(candidate));
                    found.push(candidate);
                    if found.len() > ef {
                        found.pop();
                    }
                }
            }
        }
        found.into_sorted_vec()
    }

    /// Neighbour selection heuristic (Malkov and Yashunin, algorithm 4): keep
    /// a candidate when it is closer to the target than to every neighbour
    /// already kept, which spreads links across directions; then fill the
    /// remaining places with the closest pruned candidates.
    fn select_neighbours(&self, candidates: &[Candidate], limit: usize) -> Vec<u32> {
        let mut kept: Vec<Candidate> = Vec::with_capacity(limit);
        let mut pruned: Vec<Candidate> = Vec::new();
        for candidate in candidates {
            if kept.len() >= limit {
                break;
            }
            if kept
                .iter()
                .all(|other| self.slot_distance(candidate.slot, other.slot) > candidate.distance)
            {
                kept.push(*candidate);
            } else {
                pruned.push(*candidate);
            }
        }
        for candidate in pruned {
            if kept.len() >= limit {
                break;
            }
            kept.push(candidate);
        }
        kept.into_iter().map(|candidate| candidate.slot).collect()
    }

    /// Insert a vector of the graph's dimensions under a new slot.
    #[allow(clippy::cast_possible_truncation)]
    fn insert(&mut self, id: String, vector: Vec<f32>) {
        let slot = self.ids.len() as u32;
        let layer = self.layer_for(&id);
        self.inverse_norms.push(inverse_norm(&vector));
        self.vectors.push(vector);
        self.links.push(vec![Vec::new(); layer + 1]);
        self.deleted.push(false);
        self.slots.insert(id.clone(), slot);
        self.ids.push(id);
        self.marks.push(0);

        let Some(entry) = self.entry else {
            self.entry = Some(slot);
            self.top_layer = layer;
            return;
        };

        // Search phase: read-only over the graph, with the marks taken out.
        let mut marks = std::mem::take(&mut self.marks);
        let mut epoch = self.epoch;
        let mut chosen: Vec<(usize, Vec<u32>)> = Vec::new();
        {
            let query = &self.vectors[slot as usize];
            let query_inverse_norm = self.inverse_norms[slot as usize];
            let mut entries = vec![entry];
            for current in (layer + 1..=self.top_layer).rev() {
                let stamp = advance(&mut marks, &mut epoch);
                let nearest = self.search_layer(
                    query,
                    query_inverse_norm,
                    &entries,
                    1,
                    current,
                    &mut Stamps {
                        marks: &mut marks,
                        epoch: stamp,
                    },
                );
                if let Some(best) = nearest.first() {
                    entries = vec![best.slot];
                }
            }
            for current in (0..=layer.min(self.top_layer)).rev() {
                let stamp = advance(&mut marks, &mut epoch);
                let found = self.search_layer(
                    query,
                    query_inverse_norm,
                    &entries,
                    self.params.ef_construction,
                    current,
                    &mut Stamps {
                        marks: &mut marks,
                        epoch: stamp,
                    },
                );
                chosen.push((
                    current,
                    self.select_neighbours(&found, self.max_links(current)),
                ));
                entries = found.iter().map(|candidate| candidate.slot).collect();
            }
        }
        self.marks = marks;
        self.epoch = epoch;

        // Link phase: connect both ways and shrink overfull neighbours.
        for (current, neighbours) in chosen {
            let limit = self.max_links(current);
            for &neighbour in &neighbours {
                self.links[neighbour as usize][current].push(slot);
                if self.links[neighbour as usize][current].len() > limit {
                    self.shrink(neighbour, current, limit);
                }
            }
            self.links[slot as usize][current] = neighbours;
        }
        if layer > self.top_layer {
            self.entry = Some(slot);
            self.top_layer = layer;
        }
    }

    /// Re-select the links of `slot` on `layer` down to `limit`.
    fn shrink(&mut self, slot: u32, layer: usize, limit: usize) {
        let mut candidates: Vec<Candidate> = self.links[slot as usize][layer]
            .iter()
            .map(|&neighbour| Candidate {
                distance: self.slot_distance(slot, neighbour),
                slot: neighbour,
            })
            .collect();
        candidates.sort_unstable();
        self.links[slot as usize][layer] = self.select_neighbours(&candidates, limit);
    }

    /// Tombstone the live slot of `id`, if any.
    fn tombstone(&mut self, id: &str) -> bool {
        match self.slots.remove(id) {
            Some(slot) => {
                self.deleted[slot as usize] = true;
                self.tombstones += 1;
                true
            }
            None => false,
        }
    }

    /// Rebuild from the live vectors, in id order, once tombstones outnumber
    /// them (and are numerous enough to be worth it, or nothing is live).
    fn rebuild_if_dominated(&mut self) {
        let live = self.slots.len();
        if self.tombstones == 0
            || self.tombstones <= live
            || (self.tombstones < MIN_REBUILD_TOMBSTONES && live > 0)
        {
            return;
        }
        let mut survivors: Vec<(String, u32)> = self
            .slots
            .iter()
            .map(|(id, slot)| (id.clone(), *slot))
            .collect();
        survivors.sort_unstable();
        let mut vectors = std::mem::take(&mut self.vectors);
        let survivors: Vec<(String, Vec<f32>)> = survivors
            .into_iter()
            .map(|(id, slot)| (id, std::mem::take(&mut vectors[slot as usize])))
            .collect();
        self.ids.clear();
        self.inverse_norms.clear();
        self.links.clear();
        self.deleted.clear();
        self.slots.clear();
        self.marks.clear();
        self.entry = None;
        self.top_layer = 0;
        self.tombstones = 0;
        for (id, vector) in survivors {
            self.insert(id, vector);
        }
    }

    /// Exact scan over live and stray vectors, ranked like the exact index.
    fn scan(&self, query: &[f32], k: usize, metric: VectorMetric) -> Vec<(String, f64)> {
        let live = self
            .slots
            .iter()
            .map(|(id, slot)| (id, &self.vectors[*slot as usize]));
        let scored = live
            .chain(self.strays.iter())
            .filter(|(_, vector)| vector.len() == query.len())
            .map(|(id, vector)| (id.clone(), similarity(query, vector, metric)))
            .collect();
        ranked(scored, k)
    }
}

impl VectorIndex for HnswVectorIndex {
    fn upsert(&mut self, id: String, vector: Vec<f32>) {
        if self.dimensions.is_none() {
            self.dimensions = Some(vector.len());
        }
        if let Some(&slot) = self.slots.get(&id) {
            if self.vectors[slot as usize] == vector {
                return;
            }
            self.tombstone(&id);
        }
        self.strays.remove(&id);
        if Some(vector.len()) == self.dimensions {
            self.insert(id, vector);
        } else {
            self.strays.insert(id, vector);
        }
        self.rebuild_if_dominated();
    }

    fn remove(&mut self, id: &str) {
        if self.strays.remove(id).is_none() && self.tombstone(id) {
            self.rebuild_if_dominated();
        }
    }

    fn search(&self, query: &[f32], k: usize, metric: VectorMetric) -> Vec<(String, f64)> {
        if k == 0 {
            return Vec::new();
        }
        let Some(entry) = self.entry else {
            return self.scan(query, k, metric);
        };
        if metric != self.metric || Some(query.len()) != self.dimensions || k >= self.len() {
            return self.scan(query, k, metric);
        }
        let query_inverse_norm = inverse_norm(query);
        let mut visits = QueryVisits::new(self.ids.len());
        let mut entries = vec![entry];
        for layer in (1..=self.top_layer).rev() {
            let nearest =
                self.search_layer(query, query_inverse_norm, &entries, 1, layer, &mut visits);
            if let Some(best) = nearest.first() {
                entries = vec![best.slot];
            }
            visits = QueryVisits::new(self.ids.len());
        }
        // Widen the beam by the tombstones it may meet, up to twice its size.
        let width = self.params.ef_search.max(k);
        let ef = width + self.tombstones.min(width);
        let scored = self
            .search_layer(query, query_inverse_norm, &entries, ef, 0, &mut visits)
            .into_iter()
            .filter(|candidate| !self.deleted[candidate.slot as usize])
            .map(|candidate| {
                let index = candidate.slot as usize;
                (
                    self.ids[index].clone(),
                    similarity(query, &self.vectors[index], metric),
                )
            })
            .collect();
        ranked(scored, k)
    }

    fn dimensions(&self) -> Option<usize> {
        self.dimensions
    }

    fn len(&self) -> usize {
        self.slots.len() + self.strays.len()
    }

    fn kind(&self) -> VectorIndexKind {
        VectorIndexKind::Hnsw
    }

    fn built_for(&self) -> Option<VectorMetric> {
        Some(self.metric)
    }

    fn boxed_clone(&self) -> Box<dyn VectorIndex> {
        Box::new(self.clone())
    }
}
