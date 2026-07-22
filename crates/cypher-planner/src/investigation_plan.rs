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
use std::collections::BTreeSet;

use cypher_parser::{
    Allowance, InvestigationIntent, InvestigationQuery, InvestigationTargetKind, Requirement,
    ReturnProjection,
};
use thiserror::Error;

/// Complete compiled investigation plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationPlan {
    /// Intent-level operations in deterministic execution order.
    pub logical: InvestigationLogicalPlan,
    /// Engine primitive operations in deterministic execution order.
    pub physical: InvestigationPhysicalPlan,
    /// One audit explanation for each physical operation.
    pub explanations: Vec<InvestigationPlanExplanation>,
}

impl InvestigationPlan {
    /// Serializes the complete normalized plan and explanations canonically.
    pub fn to_canonical_string(&self) -> String {
        let logical = self
            .logical
            .stages
            .iter()
            .map(|stage| stage.canonical_name())
            .collect::<Vec<_>>()
            .join(",");
        let physical = self
            .physical
            .stages
            .iter()
            .map(InvestigationPhysicalStage::canonical_string)
            .collect::<Vec<_>>()
            .join(",");
        let explanations = self
            .explanations
            .iter()
            .map(|explanation| {
                format!(
                    "{}:{}:{}:{}",
                    explanation.stage_index,
                    explanation.stage_kind.canonical_name(),
                    explanation.source_contract,
                    explanation.detail
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("logical=[{logical}];physical=[{physical}];explanations=[{explanations}]")
    }
}

/// Ordered intent-level investigation plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationLogicalPlan {
    /// Logical operations selected from the declarative contracts.
    pub stages: Vec<InvestigationLogicalStage>,
}

/// Intent-level planner operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationLogicalStage {
    /// Resolve the investigation target into graph seeds.
    SeedSelection,
    /// Construct a bounded learned working set.
    WorkingSetConstruction,
    /// Apply the requested graph snapshot.
    TemporalFiltering,
    /// Apply source independence and trust requirements.
    EvidenceFiltering,
    /// Install declared resource guards.
    BudgetEnforcement,
    /// Traverse evidence relevant to the investigation intent.
    EvidenceTraversal,
    /// Arbitrate supporting, hypothetical, and contradictory evidence.
    EvidenceArbitration,
    /// Verify evidence completeness after traversal and arbitration.
    CompletenessVerification,
    /// Produce only the response fields requested by `RETURN`.
    ResponseProjection,
}

impl InvestigationLogicalStage {
    fn canonical_name(&self) -> &'static str {
        match self {
            Self::SeedSelection => "seed_selection",
            Self::WorkingSetConstruction => "working_set_construction",
            Self::TemporalFiltering => "temporal_filtering",
            Self::EvidenceFiltering => "evidence_filtering",
            Self::BudgetEnforcement => "budget_enforcement",
            Self::EvidenceTraversal => "evidence_traversal",
            Self::EvidenceArbitration => "evidence_arbitration",
            Self::CompletenessVerification => "completeness_verification",
            Self::ResponseProjection => "response_projection",
        }
    }
}

/// Ordered executable investigation plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationPhysicalPlan {
    /// Physical engine operations.
    pub stages: Vec<InvestigationPhysicalStage>,
}

impl InvestigationPhysicalPlan {
    /// Returns stable kinds for the physical operations in execution order.
    #[must_use]
    pub fn stage_kinds(&self) -> Vec<PhysicalStageKind> {
        self.stages
            .iter()
            .map(InvestigationPhysicalStage::kind)
            .collect()
    }
}

/// Executable engine primitive and its normalized contract values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvestigationPhysicalStage {
    /// Select graph seeds from the typed intent and target.
    SeedSelection {
        /// Requested investigation operation.
        intent: InvestigationIntent,
        /// Typed seed category.
        target_kind: InvestigationTargetKind,
        /// Stable seed identifier.
        target_identifier: String,
    },
    /// Construct the learned working set around selected seeds.
    WorkingSetConstruction,
    /// Filter graph state to the declared temporal snapshot.
    TemporalFilter {
        /// Validated date or timestamp.
        at_time: String,
    },
    /// Require a minimum count of independent sources.
    IndependentSourceFilter {
        /// Inclusive minimum source count.
        minimum: u32,
    },
    /// Require a minimum normalized source-reliability score.
    SourceReliabilityFilter {
        /// Inclusive threshold in parts per million.
        minimum_parts_per_million: u32,
    },
    /// Enforce declared physical resource limits.
    BudgetGuard {
        /// Optional maximum memory in bytes.
        memory_bytes: Option<u64>,
        /// Optional maximum latency in milliseconds.
        latency_millis: Option<u64>,
        /// Optional maximum external retrieval count.
        external_retrievals: Option<u32>,
    },
    /// Traverse evidence according to the investigation operation.
    EvidenceTraversal {
        /// Requested investigation operation.
        intent: InvestigationIntent,
    },
    /// Configure evidence arbitration allowances.
    EvidenceArbitration {
        /// Explicit hypothesis allowance, if declared.
        allow_hypotheses: Option<bool>,
        /// Explicit contradictory-evidence allowance, if declared.
        allow_contradictory_evidence: Option<bool>,
    },
    /// Verify retrieval completeness against an optional hard threshold.
    CompletenessVerification {
        /// Inclusive threshold in parts per million, if declared.
        minimum_parts_per_million: Option<u32>,
    },
    /// Project the requested response fields.
    ResponseProjection {
        /// Deterministically ordered projections.
        projections: Vec<ReturnProjection>,
    },
}

impl InvestigationPhysicalStage {
    fn kind(&self) -> PhysicalStageKind {
        match self {
            Self::SeedSelection { .. } => PhysicalStageKind::SeedSelection,
            Self::WorkingSetConstruction => PhysicalStageKind::WorkingSetConstruction,
            Self::TemporalFilter { .. } => PhysicalStageKind::TemporalFilter,
            Self::IndependentSourceFilter { .. } => PhysicalStageKind::IndependentSourceFilter,
            Self::SourceReliabilityFilter { .. } => PhysicalStageKind::SourceReliabilityFilter,
            Self::BudgetGuard { .. } => PhysicalStageKind::BudgetGuard,
            Self::EvidenceTraversal { .. } => PhysicalStageKind::EvidenceTraversal,
            Self::EvidenceArbitration { .. } => PhysicalStageKind::EvidenceArbitration,
            Self::CompletenessVerification { .. } => PhysicalStageKind::CompletenessVerification,
            Self::ResponseProjection { .. } => PhysicalStageKind::ResponseProjection,
        }
    }

    fn canonical_string(&self) -> String {
        match self {
            Self::SeedSelection {
                intent,
                target_kind,
                target_identifier,
            } => format!(
                "seed_selection(intent={},target={}:{target_identifier})",
                intent_name(*intent),
                target_kind_name(*target_kind)
            ),
            Self::WorkingSetConstruction => "working_set_construction".to_owned(),
            Self::TemporalFilter { at_time } => {
                format!("temporal_filter(at_time={at_time})")
            }
            Self::IndependentSourceFilter { minimum } => {
                format!("independent_source_filter(minimum={minimum})")
            }
            Self::SourceReliabilityFilter {
                minimum_parts_per_million,
            } => format!("source_reliability_filter(minimum_ppm={minimum_parts_per_million})"),
            Self::BudgetGuard {
                memory_bytes,
                latency_millis,
                external_retrievals,
            } => format!(
                "budget_guard(memory_bytes={},latency_millis={},external_retrievals={})",
                optional_number(*memory_bytes),
                optional_number(*latency_millis),
                optional_number(*external_retrievals)
            ),
            Self::EvidenceTraversal { intent } => {
                format!("evidence_traversal(intent={})", intent_name(*intent))
            }
            Self::EvidenceArbitration {
                allow_hypotheses,
                allow_contradictory_evidence,
            } => format!(
                "evidence_arbitration(hypotheses={},contradictory_evidence={})",
                optional_bool(*allow_hypotheses),
                optional_bool(*allow_contradictory_evidence)
            ),
            Self::CompletenessVerification {
                minimum_parts_per_million,
            } => format!(
                "completeness_verification(minimum_ppm={})",
                optional_number(*minimum_parts_per_million)
            ),
            Self::ResponseProjection { projections } => format!(
                "response_projection({})",
                projections
                    .iter()
                    .map(|projection| projection_name(*projection))
                    .collect::<Vec<_>>()
                    .join("|")
            ),
        }
    }
}

/// Stable physical operation category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PhysicalStageKind {
    /// Seed-selection primitive.
    SeedSelection,
    /// Working-set construction primitive.
    WorkingSetConstruction,
    /// Temporal filtering primitive.
    TemporalFilter,
    /// Independent-source filtering primitive.
    IndependentSourceFilter,
    /// Trust filtering primitive.
    SourceReliabilityFilter,
    /// Resource-budget guard.
    BudgetGuard,
    /// Evidence traversal primitive.
    EvidenceTraversal,
    /// Evidence arbitration primitive.
    EvidenceArbitration,
    /// Completeness verification primitive.
    CompletenessVerification,
    /// Response projection primitive.
    ResponseProjection,
}

impl PhysicalStageKind {
    /// Returns the stable audit name for this physical operation.
    #[must_use]
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::SeedSelection => "seed_selection",
            Self::WorkingSetConstruction => "working_set_construction",
            Self::TemporalFilter => "temporal_filter",
            Self::IndependentSourceFilter => "independent_source_filter",
            Self::SourceReliabilityFilter => "source_reliability_filter",
            Self::BudgetGuard => "budget_guard",
            Self::EvidenceTraversal => "evidence_traversal",
            Self::EvidenceArbitration => "evidence_arbitration",
            Self::CompletenessVerification => "completeness_verification",
            Self::ResponseProjection => "response_projection",
        }
    }
}

/// Audit record connecting one physical stage to its source contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationPlanExplanation {
    /// Zero-based operation index.
    pub stage_index: usize,
    /// Stable physical operation kind.
    pub stage_kind: PhysicalStageKind,
    /// Declarative clause or implicit pipeline invariant that selected the stage.
    pub source_contract: String,
    /// Human-readable normalized mapping explanation.
    pub detail: String,
}

/// Engine capability required to compile a physical stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlannerCapability {
    /// Typed seed selection.
    SeedSelection,
    /// Learned working-set construction.
    WorkingSetConstruction,
    /// Temporal snapshot filtering.
    TemporalFiltering,
    /// Evidence independence and trust filtering.
    TrustFiltering,
    /// Resource-budget enforcement.
    BudgetEnforcement,
    /// Intent-driven evidence traversal.
    EvidenceTraversal,
    /// Evidence arbitration.
    EvidenceArbitration,
    /// Retrieval-completeness verification.
    CompletenessVerification,
    /// Typed response projection.
    ResponseProjection,
}

impl PlannerCapability {
    /// Returns the stable audit name for this capability.
    #[must_use]
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::SeedSelection => "seed_selection",
            Self::WorkingSetConstruction => "working_set_construction",
            Self::TemporalFiltering => "temporal_filtering",
            Self::TrustFiltering => "trust_filtering",
            Self::BudgetEnforcement => "budget_enforcement",
            Self::EvidenceTraversal => "evidence_traversal",
            Self::EvidenceArbitration => "evidence_arbitration",
            Self::CompletenessVerification => "completeness_verification",
            Self::ResponseProjection => "response_projection",
        }
    }
}

/// Capabilities available to the investigation-plan compiler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationPlannerCapabilities {
    supported: BTreeSet<PlannerCapability>,
}

impl InvestigationPlannerCapabilities {
    /// Enables every currently supported investigation engine primitive.
    #[must_use]
    pub fn all() -> Self {
        Self {
            supported: [
                PlannerCapability::SeedSelection,
                PlannerCapability::WorkingSetConstruction,
                PlannerCapability::TemporalFiltering,
                PlannerCapability::TrustFiltering,
                PlannerCapability::BudgetEnforcement,
                PlannerCapability::EvidenceTraversal,
                PlannerCapability::EvidenceArbitration,
                PlannerCapability::CompletenessVerification,
                PlannerCapability::ResponseProjection,
            ]
            .into_iter()
            .collect(),
        }
    }

    /// Returns a capability set with one engine primitive disabled.
    #[must_use]
    pub fn without(mut self, capability: PlannerCapability) -> Self {
        self.supported.remove(&capability);
        self
    }

    /// Reports whether a physical primitive is available.
    #[must_use]
    pub fn supports(&self, capability: PlannerCapability) -> bool {
        self.supported.contains(&capability)
    }
}

/// Stable investigation planning failure category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationPlanErrorCode {
    /// A required engine primitive is unavailable.
    UnsupportedCapability,
}

/// Actionable investigation-plan compilation failure.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct InvestigationPlanError {
    /// Stable machine-readable error category.
    pub code: InvestigationPlanErrorCode,
    /// Missing capability, when capability validation failed.
    pub capability: Option<PlannerCapability>,
    /// Human-readable failure description.
    pub message: String,
    /// Guidance for making the plan executable.
    pub suggestion: Option<String>,
}

/// Compiles a normalized investigation AST into logical and physical plans.
///
/// The implementation will validate capabilities before emitting operations,
/// map every declared contract to one physical primitive, preserve mandatory
/// pipeline stages, and emit one deterministic explanation per operation.
pub fn compile_investigation_plan(
    query: &InvestigationQuery,
    capabilities: &InvestigationPlannerCapabilities,
) -> Result<InvestigationPlan, InvestigationPlanError> {
    validate_capabilities(query, capabilities)?;

    let (minimum_independent_sources, minimum_source_reliability, minimum_completeness) =
        requirement_values(query);
    let (allow_hypotheses, allow_contradictory_evidence) = allowance_values(query);

    let mut logical_stages = vec![
        InvestigationLogicalStage::SeedSelection,
        InvestigationLogicalStage::WorkingSetConstruction,
    ];
    let mut physical_stages = vec![
        InvestigationPhysicalStage::SeedSelection {
            intent: query.intent,
            target_kind: query.target.kind,
            target_identifier: query.target.identifier.clone(),
        },
        InvestigationPhysicalStage::WorkingSetConstruction,
    ];

    if let Some(at_time) = &query.at_time {
        logical_stages.push(InvestigationLogicalStage::TemporalFiltering);
        physical_stages.push(InvestigationPhysicalStage::TemporalFilter {
            at_time: at_time.as_str().to_owned(),
        });
    }

    if minimum_independent_sources.is_some() || minimum_source_reliability.is_some() {
        logical_stages.push(InvestigationLogicalStage::EvidenceFiltering);
    }
    if let Some(minimum) = minimum_independent_sources {
        physical_stages.push(InvestigationPhysicalStage::IndependentSourceFilter { minimum });
    }
    if let Some(minimum_parts_per_million) = minimum_source_reliability {
        physical_stages.push(InvestigationPhysicalStage::SourceReliabilityFilter {
            minimum_parts_per_million,
        });
    }

    if let Some(budget) = query.budget {
        logical_stages.push(InvestigationLogicalStage::BudgetEnforcement);
        physical_stages.push(InvestigationPhysicalStage::BudgetGuard {
            memory_bytes: budget.memory_bytes,
            latency_millis: budget.latency_millis,
            external_retrievals: budget.external_retrievals,
        });
    }

    logical_stages.extend([
        InvestigationLogicalStage::EvidenceTraversal,
        InvestigationLogicalStage::EvidenceArbitration,
        InvestigationLogicalStage::CompletenessVerification,
        InvestigationLogicalStage::ResponseProjection,
    ]);
    physical_stages.extend([
        InvestigationPhysicalStage::EvidenceTraversal {
            intent: query.intent,
        },
        InvestigationPhysicalStage::EvidenceArbitration {
            allow_hypotheses,
            allow_contradictory_evidence,
        },
        InvestigationPhysicalStage::CompletenessVerification {
            minimum_parts_per_million: minimum_completeness,
        },
        InvestigationPhysicalStage::ResponseProjection {
            projections: query.returns.clone(),
        },
    ]);

    let explanations = physical_stages
        .iter()
        .enumerate()
        .map(|(stage_index, stage)| explain_stage(stage_index, stage))
        .collect();

    Ok(InvestigationPlan {
        logical: InvestigationLogicalPlan {
            stages: logical_stages,
        },
        physical: InvestigationPhysicalPlan {
            stages: physical_stages,
        },
        explanations,
    })
}

fn validate_capabilities(
    query: &InvestigationQuery,
    capabilities: &InvestigationPlannerCapabilities,
) -> Result<(), InvestigationPlanError> {
    let has_trust_filter = query.requirements.iter().any(|requirement| {
        matches!(
            requirement,
            Requirement::IndependentSourcesAtLeast(_) | Requirement::SourceReliabilityAtLeast(_)
        )
    });
    let required = [
        (true, PlannerCapability::SeedSelection),
        (true, PlannerCapability::WorkingSetConstruction),
        (
            query.at_time.is_some(),
            PlannerCapability::TemporalFiltering,
        ),
        (has_trust_filter, PlannerCapability::TrustFiltering),
        (query.budget.is_some(), PlannerCapability::BudgetEnforcement),
        (true, PlannerCapability::EvidenceTraversal),
        (true, PlannerCapability::EvidenceArbitration),
        (true, PlannerCapability::CompletenessVerification),
        (true, PlannerCapability::ResponseProjection),
    ];

    if let Some((_, capability)) = required
        .into_iter()
        .find(|(needed, capability)| *needed && !capabilities.supports(*capability))
    {
        return Err(InvestigationPlanError {
            code: InvestigationPlanErrorCode::UnsupportedCapability,
            capability: Some(capability),
            message: format!(
                "engine capability `{}` is required by the investigation plan",
                capability.canonical_name()
            ),
            suggestion: Some(
                "enable the required engine primitive before compiling this investigation"
                    .to_owned(),
            ),
        });
    }
    Ok(())
}

fn requirement_values(query: &InvestigationQuery) -> (Option<u32>, Option<u32>, Option<u32>) {
    let mut independent_sources = None;
    let mut source_reliability = None;
    let mut completeness = None;
    for requirement in &query.requirements {
        match requirement {
            Requirement::IndependentSourcesAtLeast(value) => independent_sources = Some(*value),
            Requirement::SourceReliabilityAtLeast(value) => {
                source_reliability = Some(value.parts_per_million());
            }
            Requirement::EvidenceCompletenessAtLeast(value) => {
                completeness = Some(value.parts_per_million());
            }
        }
    }
    (independent_sources, source_reliability, completeness)
}

fn allowance_values(query: &InvestigationQuery) -> (Option<bool>, Option<bool>) {
    let mut hypotheses = None;
    let mut contradictory_evidence = None;
    for allowance in &query.allowances {
        match allowance {
            Allowance::Hypotheses(value) => hypotheses = Some(*value),
            Allowance::ContradictoryEvidence(value) => {
                contradictory_evidence = Some(*value);
            }
        }
    }
    (hypotheses, contradictory_evidence)
}

fn explain_stage(
    stage_index: usize,
    stage: &InvestigationPhysicalStage,
) -> InvestigationPlanExplanation {
    let source_contract = match stage {
        InvestigationPhysicalStage::SeedSelection { .. }
        | InvestigationPhysicalStage::EvidenceTraversal { .. } => "INVESTIGATE",
        InvestigationPhysicalStage::WorkingSetConstruction => "PIPELINE",
        InvestigationPhysicalStage::TemporalFilter { .. } => "AT TIME",
        InvestigationPhysicalStage::IndependentSourceFilter { .. }
        | InvestigationPhysicalStage::SourceReliabilityFilter { .. } => "REQUIRE",
        InvestigationPhysicalStage::BudgetGuard { .. } => "BUDGET",
        InvestigationPhysicalStage::EvidenceArbitration {
            allow_hypotheses,
            allow_contradictory_evidence,
        } if allow_hypotheses.is_some() || allow_contradictory_evidence.is_some() => "ALLOW",
        InvestigationPhysicalStage::EvidenceArbitration { .. } => "PIPELINE",
        InvestigationPhysicalStage::CompletenessVerification {
            minimum_parts_per_million: Some(_),
        } => "REQUIRE",
        InvestigationPhysicalStage::CompletenessVerification { .. } => "PIPELINE",
        InvestigationPhysicalStage::ResponseProjection { .. } => "RETURN",
    };
    InvestigationPlanExplanation {
        stage_index,
        stage_kind: stage.kind(),
        source_contract: source_contract.to_owned(),
        detail: stage.canonical_string(),
    }
}

fn intent_name(intent: InvestigationIntent) -> &'static str {
    match intent {
        InvestigationIntent::Attribution => "attribution",
    }
}

fn target_kind_name(kind: InvestigationTargetKind) -> &'static str {
    match kind {
        InvestigationTargetKind::Campaign => "Campaign",
        InvestigationTargetKind::Actor => "Actor",
        InvestigationTargetKind::Narrative => "Narrative",
        InvestigationTargetKind::Indicator => "Indicator",
    }
}

fn projection_name(projection: ReturnProjection) -> &'static str {
    match projection {
        ReturnProjection::Assessment => "assessment",
        ReturnProjection::ProofGraph => "proof_graph",
        ReturnProjection::CounterEvidence => "counter_evidence",
        ReturnProjection::Unknowns => "unknowns",
        ReturnProjection::NextBestEvidence => "next_best_evidence",
    }
}

fn optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unspecified",
    }
}

fn optional_number<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "unspecified".to_owned(), |number| number.to_string())
}
