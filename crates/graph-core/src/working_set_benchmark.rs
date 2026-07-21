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
//! Working-set benchmark harness and baseline policy suite (Epic 0017).
//!
//!
//!
//! - Compare the epic's seven working-set policies — LRU, LFU, static loading
//!   profiles, semantic-only, PageRank/spreading activation, contextual
//!   bandit, and learned pheromone policy — on identical workloads with known
//!   expected evidence subgraphs.
//! - Measure pages loaded, peak resident records, p95 per-step page-in cost,
//!   and multi-hop evidence recall at fixed budgets, all in logical units so
//!   two runs on the same inputs produce byte-identical reports.
//! - Reuse the engine's real primitives: policies drive iterative one-hop
//!   expansions through the working-set manager, metrics derive from recorded
//!   telemetry, and the learned policies exercise the bandit controller and
//!   pheromone fields from the earlier issues.
//! - Do not measure wall-clock time or process memory: determinism is an
//!   acceptance criterion, so latency is the per-step page-in count and
//!   memory is the resident record count.
//!
//! # Policy semantics (deterministic)
//!
//! Each policy selects the next frontier source to expand, one step at a
//! time, until the workload's source-expansion budget is spent or the
//! frontier is empty:
//!
//! - `Lru`: first-in-first-out discovery order (the blind baseline);
//! - `Lfu`: most-rediscovered frontier candidate first, ties by discovery;
//! - `StaticProfile`: FIFO, but candidates admitted through a loading-profile
//!   prioritized relationship type come first;
//! - `SemanticOnly`: highest overlap between candidate labels and the
//!   workload's relevant labels first, ties by discovery;
//! - `SpreadingActivation`: highest activation first, where seeds start at
//!   1.0 and each expansion spreads its activation equally over its admitted
//!   neighbors; ties by node id;
//! - `ContextualBandit`: FIFO frontier order driven through the controller
//!   boundary (`expand_working_set_with_controller`) with the greedy
//!   budget-aware bandit baseline observing derived rewards each step;
//! - `LearnedPheromone`: a FIFO warm-up episode records telemetry into the
//!   pheromone and anti-pheromone fields, then the measured episode ranks
//!   candidates by the navigation-field score of their admitting edge.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    AntiPheromoneField, BanditReward, EvidenceId, ExpansionBudget, ExpansionDirection,
    ExpansionFilters, ExpansionRequest, Graph, GraphError, GraphPager, GraphRecordRef,
    GraphWorkingSetCreateRequest, GraphWorkingSetManager, GreedyExpandController, LoadingProfile,
    NodeId, NodeInput, PheromoneDecay, PheromoneField, PheromoneTaskScope, RelationshipId,
    RelationshipInput, RelationshipType, RequestId, RetrievalOutcome, RetrievalTelemetryRecord,
    TelemetryQueryDescriptor, UtilityContext, WorkingSetController, WorkingSetDecisionEvent,
    WorkingSetId, default_fimi_investigation_profile, expand_working_set_from_graph_adjacency,
    expand_working_set_with_controller, navigation_field_score,
};

/// The seven compared working-set policies, in stable report order.
///
///
/// keep the comparison set identical to the epic's acceptance list so reports
/// are complete by construction.
///
///
/// name each policy; `ALL` fixes the report order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BenchmarkPolicyKind {
    /// First-in-first-out discovery order.
    Lru,

    /// Most-rediscovered candidate first.
    Lfu,

    /// Loading-profile prioritized relationship types first.
    StaticProfile,

    /// Relevant-label overlap first.
    SemanticOnly,

    /// Spreading-activation score first.
    SpreadingActivation,

    /// Controller-driven expansion with the greedy bandit baseline.
    ContextualBandit,

    /// Navigation-field score first, learned from a warm-up episode.
    LearnedPheromone,
}

impl BenchmarkPolicyKind {
    /// The complete, stable comparison set of the benchmark.
    ///
    ///
    /// fix the report order so reports diff cleanly across runs.
    pub const ALL: [BenchmarkPolicyKind; 7] = [
        BenchmarkPolicyKind::Lru,
        BenchmarkPolicyKind::Lfu,
        BenchmarkPolicyKind::StaticProfile,
        BenchmarkPolicyKind::SemanticOnly,
        BenchmarkPolicyKind::SpreadingActivation,
        BenchmarkPolicyKind::ContextualBandit,
        BenchmarkPolicyKind::LearnedPheromone,
    ];
}

/// One benchmark workload with a known expected evidence subgraph.
///
///
/// make the comparison inputs explicit and reusable: the same graph, seeds,
/// budgets, and ground truth drive every policy.
///
///
/// carry the graph, the retrieval entry points, the expected multi-hop
/// evidence edges, the semantic ground-truth labels, and the fixed budgets.
pub struct WorkingSetBenchmarkWorkload {
    /// Stable workload name reported alongside the metrics.
    pub name: String,

    /// Graph under benchmark; policies only read it through the pager seam.
    pub graph: Graph,

    /// Retrieval entry points.
    pub seed_node_ids: Vec<NodeId>,

    /// Expected multi-hop evidence edges the policies should recover.
    pub expected_evidence_relationship_ids: Vec<RelationshipId>,

    /// Labels marking the relevant chain for the semantic-only policy.
    pub relevant_labels: Vec<String>,

    /// Task family scoping the pheromone fields.
    pub task_label: String,

    /// Loading profile used by the static-profile policy and the engine.
    pub loading_profile: LoadingProfile,

    /// Fixed number of source expansions each policy may spend.
    pub max_source_expansions: u64,

    /// Fixed per-step engine budget.
    pub expansion_budget: ExpansionBudget,
}

/// Build the deterministic FIMI multi-hop benchmark workload.
///
///
/// provide the reference scenario of the epic's comparison: a campaign whose
/// adjacency lists dead-end distractors before the relevant
/// narrative -> claim -> evidence chain, under a source budget too tight for
/// blind FIFO exploration.
///
///
/// generate the same graph, seeds, expected edges, and budgets on every call.
///
/// # Errors
///
/// none expected because the generator only builds valid inputs.
pub fn fimi_multi_hop_benchmark_workload() -> WorkingSetBenchmarkWorkload {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("benchmark campaign node should be created");

    // Dead-end distractors are created first so blind FIFO exploration
    // discovers and expands them before the relevant chain.
    for _ in 0..3 {
        let distractor = graph
            .create_node(NodeInput::new(["Post"]))
            .expect("benchmark distractor node should be created");
        graph
            .create_relationship(
                RelationshipInput::new(campaign.clone(), "MENTIONS", distractor)
                    .expect("benchmark distractor input should be valid"),
            )
            .expect("benchmark distractor relationship should be created");
    }

    let narrative = graph
        .create_node(NodeInput::new(["Narrative"]))
        .expect("benchmark narrative node should be created");
    let claim = graph
        .create_node(NodeInput::new(["Claim"]))
        .expect("benchmark claim node should be created");
    let evidence = graph
        .create_node(NodeInput::new(["Evidence"]))
        .expect("benchmark evidence node should be created");
    let promotes = graph
        .create_relationship(
            RelationshipInput::new(campaign.clone(), "PROMOTES", narrative.clone())
                .expect("benchmark promotes input should be valid"),
        )
        .expect("benchmark promotes relationship should be created");
    let makes_claim = graph
        .create_relationship(
            RelationshipInput::new(narrative, "MAKES_CLAIM", claim.clone())
                .expect("benchmark makes-claim input should be valid"),
        )
        .expect("benchmark makes-claim relationship should be created");
    let supported_by = graph
        .create_relationship(
            RelationshipInput::new(claim, "SUPPORTED_BY", evidence)
                .expect("benchmark supported-by input should be valid"),
        )
        .expect("benchmark supported-by relationship should be created");

    WorkingSetBenchmarkWorkload {
        name: "fimi-multi-hop".to_owned(),
        graph,
        seed_node_ids: vec![campaign],
        expected_evidence_relationship_ids: vec![promotes, makes_claim, supported_by],
        relevant_labels: vec![
            "Narrative".to_owned(),
            "Claim".to_owned(),
            "Evidence".to_owned(),
        ],
        task_label: "fimi_investigation".to_owned(),
        loading_profile: default_fimi_investigation_profile(),
        max_source_expansions: 4,
        expansion_budget: ExpansionBudget {
            max_loaded_node_count: 64,
            max_loaded_relationship_count: 64,
            max_hot_node_count: 64,
            max_hot_relationship_count: 64,
            max_warm_adjacency_entry_count: 64,
            max_hop_count: 4,
            max_supernode_expansion_count: 16,
            max_payload_byte_count: 1_048_576,
            max_execution_time_ms: 1_000,
        },
    }
}

/// Metrics of one policy over one workload, in logical units.
///
///
/// report the epic's four comparison metrics per policy without wall-clock or
/// process-memory sampling, keeping reports reproducible.
///
///
/// carry pages loaded (page-in count), peak resident records (hot plus warm),
/// the p95 per-step page-in cost, and the evidence recall against the
/// workload's expected subgraph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyBenchmarkMetrics {
    /// Policy the row describes.
    pub policy: BenchmarkPolicyKind,

    /// Total page-ins recorded across the measured episode.
    pub pages_loaded: u64,

    /// Peak resident record count (hot and warm) across the episode.
    pub peak_resident_records: u64,

    /// 95th percentile of per-step page-in counts.
    pub p95_step_page_in_count: u64,

    /// Expected evidence edges recovered, as a ratio in [0, 1].
    pub evidence_recall: f64,

    /// Dead-end frontier expansions spent during the measured episode.
    pub dead_end_expansions: u64,
}

/// Deterministic report of one benchmark run.
///
///
/// give the epic's acceptance suite and the reproducibility report one stable
/// artifact: equal inputs must yield equal reports.
///
///
/// carry the workload name and one metrics row per policy in `ALL` order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkingSetBenchmarkReport {
    /// Name of the workload the run measured.
    pub workload_name: String,

    /// One metrics row per compared policy, in `BenchmarkPolicyKind::ALL` order.
    pub policy_metrics: Vec<PolicyBenchmarkMetrics>,
}

/// Run the seven-policy benchmark over one workload.
///
///
/// execute the epic's comparison: each policy explores the same graph from
/// the same seeds under the same budgets, and its decisions are measured
/// through the recorded telemetry.
///
///
/// run every policy in `ALL` order over fresh working sets, derive the
/// metrics from the per-step retrieval records, and assemble the report.
///
/// # Errors
///
/// propagate typed engine errors; policy selection itself cannot fail.
pub fn run_working_set_benchmark(
    workload: &WorkingSetBenchmarkWorkload,
) -> Result<WorkingSetBenchmarkReport, GraphError> {
    let mut policy_metrics = Vec::new();

    for policy in BenchmarkPolicyKind::ALL {
        let episode = match policy {
            BenchmarkPolicyKind::Lru => {
                run_episode(workload, "lru", &mut FifoSelector, ControllerMode::Plain)?
            }
            BenchmarkPolicyKind::Lfu => {
                run_episode(workload, "lfu", &mut LfuSelector, ControllerMode::Plain)?
            }
            BenchmarkPolicyKind::StaticProfile => run_episode(
                workload,
                "static-profile",
                &mut StaticProfileSelector {
                    prioritized: workload
                        .loading_profile
                        .prioritized_relationship_types
                        .clone(),
                },
                ControllerMode::Plain,
            )?,
            BenchmarkPolicyKind::SemanticOnly => run_episode(
                workload,
                "semantic-only",
                &mut SemanticSelector {
                    relevant_labels: workload.relevant_labels.clone(),
                },
                ControllerMode::Plain,
            )?,
            BenchmarkPolicyKind::SpreadingActivation => run_episode(
                workload,
                "spreading-activation",
                &mut ActivationSelector,
                ControllerMode::Plain,
            )?,
            BenchmarkPolicyKind::ContextualBandit => run_episode(
                workload,
                "contextual-bandit",
                &mut FifoSelector,
                ControllerMode::Bandit(GreedyExpandController::new()),
            )?,
            BenchmarkPolicyKind::LearnedPheromone => {
                let warm_up = run_episode(
                    workload,
                    "pheromone-warm-up",
                    &mut FifoSelector,
                    ControllerMode::Plain,
                )?;
                let decay =
                    PheromoneDecay::new(0.9).expect("benchmark pheromone decay should be valid");
                let mut positive = PheromoneField::new(decay);
                let mut negative = AntiPheromoneField::new(decay);
                for record in &warm_up.retrieval_records {
                    positive.apply_retrieval_record(record);
                    negative.apply_retrieval_record(record);
                }
                run_episode(
                    workload,
                    "learned-pheromone",
                    &mut PheromoneSelector {
                        positive,
                        negative,
                        scope: PheromoneTaskScope::task(&workload.task_label),
                        relevant_labels: workload.relevant_labels.clone(),
                    },
                    ControllerMode::Plain,
                )?
            }
        };

        policy_metrics.push(PolicyBenchmarkMetrics {
            policy,
            pages_loaded: episode.pages_loaded,
            peak_resident_records: episode.peak_resident_records,
            p95_step_page_in_count: p95(&episode.step_page_in_counts),
            evidence_recall: episode.evidence_recall,
            dead_end_expansions: episode.dead_end_expansions,
        });
    }

    Ok(WorkingSetBenchmarkReport {
        workload_name: workload.name.clone(),
        policy_metrics,
    })
}

/// Render one benchmark report as deterministic Markdown.
///
///
/// give the epic's reproducibility report a rendering that is regenerable
/// from code: equal reports must produce byte-identical Markdown, so the
/// committed document stays verifiable against a rerun.
///
///
/// emit the workload name and one table row per policy covering pages loaded,
/// peak resident records, p95 step page-ins, evidence recall, and dead-end
/// expansions, in report order.
///
/// # Errors
///
/// none expected because rendering is pure formatting.
pub fn render_working_set_benchmark_markdown(report: &WorkingSetBenchmarkReport) -> String {
    let mut rendering = String::new();
    rendering.push_str(&format!(
        "## Working-set benchmark report: {}\n\n",
        report.workload_name
    ));
    rendering.push_str(
        "| Policy | Pages loaded | Peak resident records | P95 step page-ins | Evidence recall | Dead-end expansions |\n",
    );
    rendering.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for metrics in &report.policy_metrics {
        rendering.push_str(&format!(
            "| {:?} | {} | {} | {} | {:.3} | {} |\n",
            metrics.policy,
            metrics.pages_loaded,
            metrics.peak_resident_records,
            metrics.p95_step_page_in_count,
            metrics.evidence_recall,
            metrics.dead_end_expansions,
        ));
    }
    rendering
}

/// One frontier candidate a policy can choose to expand next.
struct FrontierCandidate {
    node_id: NodeId,
    admitting_relationship_id: Option<RelationshipId>,
    admitting_relationship_type: Option<RelationshipType>,
    labels: Vec<String>,
    discovery_index: u64,
    rediscovery_count: u64,
    activation: f64,
}

/// How an episode drives the engine: plain expansion or through the
/// controller boundary with the greedy bandit baseline observing rewards.
enum ControllerMode {
    Plain,
    Bandit(GreedyExpandController),
}

/// Deterministic measurements of one exploration episode.
struct EpisodeOutcome {
    pages_loaded: u64,
    peak_resident_records: u64,
    step_page_in_counts: Vec<u64>,
    evidence_recall: f64,
    dead_end_expansions: u64,
    retrieval_records: Vec<RetrievalTelemetryRecord>,
}

/// Frontier-selection strategy of one policy.
trait FrontierSelector {
    fn select(&mut self, frontier: &[FrontierCandidate]) -> usize;
}

/// FIFO discovery order: the blind baseline shared by LRU and the bandit run.
struct FifoSelector;

impl FrontierSelector for FifoSelector {
    fn select(&mut self, frontier: &[FrontierCandidate]) -> usize {
        min_by_key_index(frontier, |candidate| candidate.discovery_index)
    }
}

/// Most-rediscovered candidate first; ties fall back to discovery order.
struct LfuSelector;

impl FrontierSelector for LfuSelector {
    fn select(&mut self, frontier: &[FrontierCandidate]) -> usize {
        min_by_key_index(frontier, |candidate| {
            (
                u64::MAX - candidate.rediscovery_count,
                candidate.discovery_index,
            )
        })
    }
}

/// Profile-prioritized relationship types first, then discovery order.
struct StaticProfileSelector {
    prioritized: Vec<RelationshipType>,
}

impl FrontierSelector for StaticProfileSelector {
    fn select(&mut self, frontier: &[FrontierCandidate]) -> usize {
        min_by_key_index(frontier, |candidate| {
            let prioritized = match &candidate.admitting_relationship_type {
                Some(relationship_type) => self.prioritized.contains(relationship_type),
                None => true,
            };
            (u64::from(!prioritized), candidate.discovery_index)
        })
    }
}

/// Highest relevant-label overlap first, then discovery order.
struct SemanticSelector {
    relevant_labels: Vec<String>,
}

impl FrontierSelector for SemanticSelector {
    fn select(&mut self, frontier: &[FrontierCandidate]) -> usize {
        min_by_key_index(frontier, |candidate| {
            let overlap = label_overlap(&candidate.labels, &self.relevant_labels);
            (u64::MAX - overlap, candidate.discovery_index)
        })
    }
}

/// Highest spreading activation first; ties by node id for determinism.
struct ActivationSelector;

impl FrontierSelector for ActivationSelector {
    fn select(&mut self, frontier: &[FrontierCandidate]) -> usize {
        let mut best = 0;
        for index in 1..frontier.len() {
            let candidate = &frontier[index];
            let leader = &frontier[best];
            if candidate.activation > leader.activation
                || (candidate.activation == leader.activation
                    && candidate.node_id.as_str() < leader.node_id.as_str())
            {
                best = index;
            }
        }
        best
    }
}

/// Highest navigation-field score first, learned from the warm-up episode.
struct PheromoneSelector {
    positive: PheromoneField,
    negative: AntiPheromoneField,
    scope: PheromoneTaskScope,
    relevant_labels: Vec<String>,
}

impl FrontierSelector for PheromoneSelector {
    fn select(&mut self, frontier: &[FrontierCandidate]) -> usize {
        let mut best = 0;
        let mut best_score = f64::NEG_INFINITY;
        for (index, candidate) in frontier.iter().enumerate() {
            let score = match &candidate.admitting_relationship_id {
                // Seeds carry no admitting edge and are always expanded first.
                None => f64::INFINITY,
                Some(relationship_id) => {
                    let utility = self
                        .positive
                        .edge_utility(relationship_id, &self.scope)
                        .unwrap_or_default();
                    let anti = self
                        .negative
                        .edge_anti_pheromone(relationship_id, &self.scope)
                        .unwrap_or_default();
                    let context = UtilityContext {
                        semantic_relevance: label_overlap(&candidate.labels, &self.relevant_labels)
                            as f64,
                        temporal_relevance: 0.0,
                    };
                    navigation_field_score(&utility, &anti, &context)
                }
            };
            if score > best_score {
                best_score = score;
                best = index;
            }
        }
        best
    }
}

fn min_by_key_index<K: Ord>(
    frontier: &[FrontierCandidate],
    mut key: impl FnMut(&FrontierCandidate) -> K,
) -> usize {
    let mut best = 0;
    for index in 1..frontier.len() {
        if key(&frontier[index]) < key(&frontier[best]) {
            best = index;
        }
    }
    best
}

fn label_overlap(labels: &[String], relevant: &[String]) -> u64 {
    labels
        .iter()
        .filter(|label| relevant.contains(label))
        .count() as u64
}

fn p95(step_counts: &[u64]) -> u64 {
    if step_counts.is_empty() {
        return 0;
    }
    let mut sorted = step_counts.to_vec();
    sorted.sort_unstable();
    let rank = ((sorted.len() as f64) * 0.95).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn run_episode(
    workload: &WorkingSetBenchmarkWorkload,
    slug: &str,
    selector: &mut dyn FrontierSelector,
    mut controller_mode: ControllerMode,
) -> Result<EpisodeOutcome, GraphError> {
    let working_set_id = WorkingSetId::new(format!("working-set--bench-{slug}"))?;
    let mut manager = GraphWorkingSetManager::new();
    manager.create_working_set(GraphWorkingSetCreateRequest::new(working_set_id.clone()))?;

    let mut frontier: Vec<FrontierCandidate> = Vec::new();
    let mut known: HashSet<NodeId> = HashSet::new();
    let mut expanded: HashSet<NodeId> = HashSet::new();
    let mut discovery_counter: u64 = 0;
    let mut found_expected: HashSet<RelationshipId> = HashSet::new();
    let mut pages_loaded: u64 = 0;
    let mut peak_resident_records: u64 = 0;
    let mut step_page_in_counts: Vec<u64> = Vec::new();

    for seed_node_id in &workload.seed_node_ids {
        frontier.push(FrontierCandidate {
            node_id: seed_node_id.clone(),
            admitting_relationship_id: None,
            admitting_relationship_type: None,
            labels: node_labels(&workload.graph, seed_node_id)?,
            discovery_index: discovery_counter,
            rediscovery_count: 0,
            activation: 1.0,
        });
        known.insert(seed_node_id.clone());
        discovery_counter += 1;
    }

    let mut step: u64 = 0;
    while step < workload.max_source_expansions && !frontier.is_empty() {
        let source = frontier.remove(selector.select(&frontier));
        expanded.insert(source.node_id.clone());

        let retrieval_id = RequestId::new(format!("request--bench-{slug}-{step}"))?;
        manager.begin_retrieval_telemetry(
            &working_set_id,
            retrieval_id.clone(),
            TelemetryQueryDescriptor {
                query_text: Some(workload.name.clone()),
                profile_kind: Some(workload.loading_profile.kind),
                task_label: Some(workload.task_label.clone()),
            },
        )?;

        let request = ExpansionRequest::new(
            working_set_id.clone(),
            vec![source.node_id.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            workload.loading_profile.clone(),
            workload.expansion_budget.clone(),
        );
        let result = match &mut controller_mode {
            ControllerMode::Plain => {
                expand_working_set_from_graph_adjacency(&mut manager, &workload.graph, request)?
            }
            ControllerMode::Bandit(controller) => expand_working_set_with_controller(
                &mut manager,
                &workload.graph,
                request,
                controller,
            )?,
        };

        // Track newly recovered expected edges and refresh the frontier from
        // the hot relationships this step admitted.
        let mut newly_found: u64 = 0;
        let mut admitted: Vec<(RelationshipId, RelationshipType, NodeId)> = Vec::new();
        for hot in result.explanation().hot_relationships() {
            if hot.source_node_id == source.node_id {
                admitted.push((
                    hot.relationship_id.clone(),
                    hot.relationship_type.clone(),
                    hot.target_node_id.clone(),
                ));
            }
            if workload
                .expected_evidence_relationship_ids
                .contains(&hot.relationship_id)
                && found_expected.insert(hot.relationship_id.clone())
            {
                newly_found += 1;
            }
        }

        let mut evidence_record_ids = Vec::new();
        for index in 0..newly_found {
            evidence_record_ids.push(EvidenceId::new(format!(
                "evidence--bench-{slug}-{step}-{index}"
            ))?);
        }
        manager.complete_retrieval_telemetry(
            &working_set_id,
            &retrieval_id,
            RetrievalOutcome {
                evidence_record_ids,
                answer_quality: None,
                memory_cost_bytes: 0,
                latency_ms: 0,
            },
        )?;

        let spread = if admitted.is_empty() {
            0.0
        } else {
            source.activation / admitted.len() as f64
        };
        for (relationship_id, relationship_type, target_node_id) in admitted {
            if expanded.contains(&target_node_id) {
                continue;
            }
            if known.contains(&target_node_id) {
                if let Some(candidate) = frontier
                    .iter_mut()
                    .find(|candidate| candidate.node_id == target_node_id)
                {
                    candidate.rediscovery_count += 1;
                    candidate.activation += spread;
                }
                continue;
            }
            frontier.push(FrontierCandidate {
                node_id: target_node_id.clone(),
                admitting_relationship_id: Some(relationship_id),
                admitting_relationship_type: Some(relationship_type),
                labels: node_labels(&workload.graph, &target_node_id)?,
                discovery_index: discovery_counter,
                rediscovery_count: 0,
                activation: spread,
            });
            known.insert(target_node_id);
            discovery_counter += 1;
        }

        let records = manager.telemetry(&working_set_id)?.retrieval_records();
        let step_record = records.last().ok_or_else(|| {
            GraphError::InternalInvariantViolation(
                "benchmark step should produce a retrieval record".to_owned(),
            )
        })?;
        let step_pages = step_record
            .events
            .iter()
            .filter(|event| matches!(event.decision, WorkingSetDecisionEvent::PageIn { .. }))
            .count() as u64;
        pages_loaded += step_pages;
        step_page_in_counts.push(step_pages);

        if let ControllerMode::Bandit(controller) = &mut controller_mode {
            let reward = BanditReward::from_retrieval_record(step_record);
            let context = crate::BanditContext::from_expansion_request(
                &ExpansionRequest::new(
                    working_set_id.clone(),
                    vec![source.node_id.clone()],
                    ExpansionDirection::Outgoing,
                    ExpansionFilters::empty(),
                    1,
                    workload.loading_profile.clone(),
                    workload.expansion_budget.clone(),
                ),
                TelemetryQueryDescriptor {
                    query_text: Some(workload.name.clone()),
                    profile_kind: Some(workload.loading_profile.kind),
                    task_label: Some(workload.task_label.clone()),
                },
            );
            controller.observe_reward(&context, crate::WorkingSetAction::Expand, &reward);
        }

        let stats = manager.stats(&working_set_id)?;
        let resident = stats.hot_node_count()
            + stats.hot_relationship_count()
            + stats.warm_relationship_count();
        peak_resident_records = peak_resident_records.max(resident);

        step += 1;
    }

    let expected_total = workload.expected_evidence_relationship_ids.len();
    let evidence_recall = if expected_total == 0 {
        1.0
    } else {
        found_expected.len() as f64 / expected_total as f64
    };

    let retrieval_records = manager.telemetry(&working_set_id)?.retrieval_records();
    // A dead-end expansion is a spent step whose retrieval observed at least
    // one dead-end frontier: the budget went to a source that admitted nothing.
    let dead_end_expansions = retrieval_records
        .iter()
        .filter(|record| {
            record
                .events
                .iter()
                .any(|event| matches!(event.decision, WorkingSetDecisionEvent::DeadEnd { .. }))
        })
        .count() as u64;

    Ok(EpisodeOutcome {
        pages_loaded,
        peak_resident_records,
        step_page_in_counts,
        evidence_recall,
        dead_end_expansions,
        retrieval_records,
    })
}

fn node_labels(graph: &Graph, node_id: &NodeId) -> Result<Vec<String>, GraphError> {
    let metadata = graph
        .load_indexed_metadata(&GraphRecordRef::Node(node_id.clone()))
        .map_err(|error| {
            GraphError::InternalInvariantViolation(format!("benchmark metadata load: {error}"))
        })?;
    Ok(metadata.labels)
}
