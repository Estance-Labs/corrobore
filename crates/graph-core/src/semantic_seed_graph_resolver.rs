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
//! Graph-backed hybrid semantic seed resolver.
//!
//! First real implementation of the [`SemanticSeedResolver`] contract from
//! Epic 0012. It resolves a natural-language objective into ranked seed node
//! IDs by scoring the current nodes of an in-memory [`Graph`]:
//!
//! - lexical relevance: BM25-style weighting of objective terms against node
//!   labels and string property values, normalized into `[0, 1)`;
//! - graph signals (hybrid mode): relationship degree, node confidence, and
//!   record-status maturity, blended with fixed deterministic weights.
//!
//! Design boundary:
//!
//! - a candidate must match at least one objective term — centrality alone
//!   can never seed a working set;
//! - `Rejected` and `Tombstoned` records never seed;
//! - vector-index retrieval is a non-goal here (per the epic): `Semantic` and
//!   `Vector` modes fall back to hybrid scoring and disclose the fallback in
//!   candidate boundary notes;
//! - the resolver scans the graph at resolve time; persistent lexical or
//!   vector indexes belong to a later storage-backed implementation.

use std::collections::HashMap;

use crate::{
    Graph, GraphError, Node, PropertyValue, RecordStatus, SemanticSeedCandidate,
    SemanticSeedExplanationMetadata, SemanticSeedQueryRequest, SemanticSeedQueryResponse,
    SemanticSeedResolutionError, SemanticSeedResolutionErrorCode, SemanticSeedResolver,
    SemanticSeedRetrievalMode,
};

// BM25 constants (standard Robertson defaults).
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;
// Saturation constant mapping unbounded BM25 sums into [0, 1).
const LEXICAL_SATURATION: f64 = 2.0;

// Hybrid blend weights; they sum to 1 so blended scores stay in [0, 1).
const WEIGHT_LEXICAL: f64 = 0.70;
const WEIGHT_DEGREE: f64 = 0.15;
const WEIGHT_CONFIDENCE: f64 = 0.10;
const WEIGHT_STATUS: f64 = 0.05;

// Confidence assumed when a node carries no confidence assessment.
const NEUTRAL_CONFIDENCE: f64 = 0.5;
// Matched candidates beyond `top_k * multiplier` classify as overbroad.
const OVERBROAD_TOP_K_MULTIPLIER: usize = 10;
// Two scores closer than this are considered an exact ranking tie.
const SCORE_TIE_EPSILON: f64 = 1e-9;

// Objective words carrying no discriminating intent.
const STOPWORDS: &[&str] = &[
    "a", "all", "an", "and", "are", "find", "for", "from", "identify", "in", "is", "list", "of",
    "on", "or", "show", "that", "the", "this", "tied", "to", "what", "which", "with",
];

/// Hybrid lexical and graph-signal seed resolver over an in-memory graph.
#[derive(Debug)]
pub struct GraphSemanticSeedResolver<'g> {
    graph: &'g Graph,
}

impl<'g> GraphSemanticSeedResolver<'g> {
    /// Creates a resolver over `graph`.
    pub fn new(graph: &'g Graph) -> Self {
        Self { graph }
    }
}

impl SemanticSeedResolver for GraphSemanticSeedResolver<'_> {
    fn resolve(
        &self,
        request: &SemanticSeedQueryRequest,
    ) -> Result<SemanticSeedQueryResponse, GraphError> {
        let terms = objective_terms(request.objective());
        if terms.is_empty() {
            return Err(GraphError::SemanticSeedResolutionFailed(
                SemanticSeedResolutionError::new(
                    SemanticSeedResolutionErrorCode::OverbroadObjective,
                    "Objective contains no informative terms after stopword removal.",
                    "Add discriminating entity, campaign, or infrastructure terms to the objective.",
                )
                .with_threshold(request.score_threshold()),
            ));
        }

        let nodes: Vec<Node> = self
            .graph
            .list_nodes()?
            .into_iter()
            .filter(|node| is_seedable(node.status()))
            .collect();

        let documents: Vec<Vec<String>> = nodes.iter().map(node_document).collect();
        let corpus = CorpusStatistics::new(&documents, &terms);
        let degrees = self.relationship_degrees();
        let max_degree = nodes
            .iter()
            .map(|node| degrees.get(node.id().as_str()).copied().unwrap_or(0))
            .max()
            .unwrap_or(0);

        let hybrid_mode = request.retrieval_mode() != SemanticSeedRetrievalMode::FullText;
        let fallback_note = match request.retrieval_mode() {
            SemanticSeedRetrievalMode::Semantic | SemanticSeedRetrievalMode::Vector => {
                Some("vector retrieval is not configured; hybrid lexical fallback applied")
            }
            SemanticSeedRetrievalMode::FullText | SemanticSeedRetrievalMode::Hybrid => None,
        };

        let mut scored: Vec<ScoredSeed> = Vec::new();
        for (node, document) in nodes.iter().zip(documents.iter()) {
            let (lexical_raw, matched_terms) = corpus.bm25(document, &terms);
            if matched_terms.is_empty() {
                continue;
            }

            let lexical = lexical_raw / (lexical_raw + LEXICAL_SATURATION);
            let degree = degree_signal(
                degrees.get(node.id().as_str()).copied().unwrap_or(0),
                max_degree,
            );
            let confidence = NEUTRAL_CONFIDENCE;
            let status_weight = status_signal(node.status());

            let score = if hybrid_mode {
                WEIGHT_LEXICAL * lexical
                    + WEIGHT_DEGREE * degree
                    + WEIGHT_CONFIDENCE * confidence
                    + WEIGHT_STATUS * status_weight
            } else {
                lexical
            };

            if score < request.score_threshold() {
                continue;
            }

            let rationale = if hybrid_mode {
                format!(
                    "matched terms [{}]; lexical {:.4}; degree {:.4}; confidence {:.4}; status {}",
                    matched_terms.join(", "),
                    lexical,
                    degree,
                    confidence,
                    status_label(node.status())
                )
            } else {
                format!(
                    "matched terms [{}]; lexical {:.4}",
                    matched_terms.join(", "),
                    lexical
                )
            };

            let source_refs = node
                .evidence_refs()
                .iter()
                .map(|evidence_id| evidence_id.as_str().to_owned())
                .collect();

            let mut explanation = SemanticSeedExplanationMetadata::new(rationale, source_refs);
            if let Some(note) = fallback_note {
                explanation = explanation.with_boundary_note(note);
            }

            scored.push(ScoredSeed {
                node_id: node.id().clone(),
                score,
                explanation,
            });
        }

        scored.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.node_id.as_str().cmp(right.node_id.as_str()))
        });

        let candidate_count = scored.len();
        if candidate_count == 0 {
            return Err(GraphError::SemanticSeedResolutionFailed(
                SemanticSeedResolutionError::new(
                    SemanticSeedResolutionErrorCode::NoSeed,
                    "No seed candidate matched the objective above the score threshold.",
                    "Broaden objective wording or lower the score threshold.",
                )
                .with_candidate_count(0)
                .with_threshold(request.score_threshold()),
            ));
        }

        if candidate_count > request.top_k() * OVERBROAD_TOP_K_MULTIPLIER {
            return Err(GraphError::SemanticSeedResolutionFailed(
                SemanticSeedResolutionError::new(
                    SemanticSeedResolutionErrorCode::OverbroadObjective,
                    "Objective matched too many candidates to safely seed a bounded working set.",
                    "Add stronger objective qualifiers before loading a working set.",
                )
                .with_candidate_count(candidate_count)
                .with_threshold(request.score_threshold()),
            ));
        }

        if candidate_count > request.top_k() {
            let boundary = scored[request.top_k() - 1].score;
            let first_excluded = scored[request.top_k()].score;
            if (boundary - first_excluded).abs() < SCORE_TIE_EPSILON {
                return Err(GraphError::SemanticSeedResolutionFailed(
                    SemanticSeedResolutionError::new(
                        SemanticSeedResolutionErrorCode::AmbiguousSeed,
                        "Candidates at the top_k boundary have identical scores.",
                        "Increase top_k, refine the objective, or raise the score threshold.",
                    )
                    .with_candidate_count(candidate_count)
                    .with_threshold(request.score_threshold()),
                ));
            }
        }

        scored.truncate(request.top_k());

        let mut candidates = Vec::with_capacity(scored.len());
        for seed in scored {
            candidates.push(SemanticSeedCandidate::new(
                seed.node_id,
                seed.score,
                seed.explanation,
            )?);
        }

        SemanticSeedQueryResponse::new(request.clone(), candidates)
    }
}

impl GraphSemanticSeedResolver<'_> {
    fn relationship_degrees(&self) -> HashMap<String, usize> {
        let mut degrees: HashMap<String, usize> = HashMap::new();

        if let Ok(relationships) = self.graph.list_relationships() {
            for relationship in relationships {
                *degrees
                    .entry(relationship.source().as_str().to_owned())
                    .or_insert(0) += 1;
                *degrees
                    .entry(relationship.target().as_str().to_owned())
                    .or_insert(0) += 1;
            }
        }

        degrees
    }
}

struct ScoredSeed {
    node_id: crate::NodeId,
    score: f64,
    explanation: SemanticSeedExplanationMetadata,
}

struct CorpusStatistics {
    document_count: usize,
    average_length: f64,
    term_document_frequency: HashMap<String, usize>,
}

impl CorpusStatistics {
    fn new(documents: &[Vec<String>], terms: &[String]) -> Self {
        let document_count = documents.len();
        let total_length: usize = documents.iter().map(Vec::len).sum();
        let average_length = if document_count > 0 {
            total_length as f64 / document_count as f64
        } else {
            0.0
        };

        let mut term_document_frequency = HashMap::new();
        for term in terms {
            let frequency = documents
                .iter()
                .filter(|document| document.iter().any(|token| token == term))
                .count();
            term_document_frequency.insert(term.clone(), frequency);
        }

        Self {
            document_count,
            average_length,
            term_document_frequency,
        }
    }

    /// Returns the BM25 sum and the matched terms for one document.
    fn bm25(&self, document: &[String], terms: &[String]) -> (f64, Vec<String>) {
        let mut score = 0.0;
        let mut matched = Vec::new();

        if document.is_empty() || self.average_length == 0.0 {
            return (score, matched);
        }

        let length_ratio = document.len() as f64 / self.average_length;
        for term in terms {
            let term_frequency = document.iter().filter(|token| *token == term).count() as f64;
            if term_frequency == 0.0 {
                continue;
            }

            let document_frequency =
                self.term_document_frequency.get(term).copied().unwrap_or(0) as f64;
            // Lucene-style non-negative IDF keeps scores positive even for
            // terms present in every document.
            let idf = (1.0
                + (self.document_count as f64 - document_frequency + 0.5)
                    / (document_frequency + 0.5))
                .ln();
            let saturation = (term_frequency * (BM25_K1 + 1.0))
                / (term_frequency + BM25_K1 * (1.0 - BM25_B + BM25_B * length_ratio));

            score += idf * saturation;
            matched.push(term.clone());
        }

        (score, matched)
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.len() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn objective_terms(objective: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for token in tokenize(objective) {
        if STOPWORDS.contains(&token.as_str()) || terms.contains(&token) {
            continue;
        }
        terms.push(token);
    }
    terms
}

fn node_document(node: &Node) -> Vec<String> {
    let mut tokens = Vec::new();

    for label in &node.labels {
        tokens.extend(tokenize(label));
    }

    // Sort property keys so the token order (irrelevant for bag-of-words
    // scoring, but visible in debugging) stays deterministic.
    let mut keys: Vec<&String> = node.properties.keys().collect();
    keys.sort();
    for key in keys {
        match &node.properties[key] {
            PropertyValue::String(value) => tokens.extend(tokenize(value)),
            PropertyValue::StringList(values) => {
                for value in values {
                    tokens.extend(tokenize(value));
                }
            }
            _ => {}
        }
    }

    tokens
}

fn is_seedable(status: RecordStatus) -> bool {
    !matches!(status, RecordStatus::Rejected | RecordStatus::Tombstoned)
}

fn degree_signal(degree: usize, max_degree: usize) -> f64 {
    if max_degree == 0 {
        return 0.0;
    }

    ((1 + degree) as f64).ln() / ((1 + max_degree) as f64).ln()
}

fn status_signal(status: RecordStatus) -> f64 {
    match status {
        RecordStatus::Validated | RecordStatus::Exportable | RecordStatus::Exported => 1.0,
        RecordStatus::NeedsReview => 0.7,
        RecordStatus::NeedsEvidence => 0.6,
        RecordStatus::Candidate => 0.5,
        RecordStatus::Rejected | RecordStatus::Tombstoned => 0.0,
    }
}

fn status_label(status: RecordStatus) -> &'static str {
    match status {
        RecordStatus::Candidate => "candidate",
        RecordStatus::NeedsEvidence => "needs_evidence",
        RecordStatus::NeedsReview => "needs_review",
        RecordStatus::Validated => "validated",
        RecordStatus::Rejected => "rejected",
        RecordStatus::Exportable => "exportable",
        RecordStatus::Exported => "exported",
        RecordStatus::Tombstoned => "tombstoned",
    }
}
