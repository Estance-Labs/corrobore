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
#![warn(missing_docs)]

//! Core in-memory graph primitives for Corrobore.
//!
//! This facade re-exports the stable public API of the graph core. Implementation
//! details live in focused modules so the crate remains easy to navigate as the
//! graph core grows.
//!
//! # Module boundary contract
//!
//! - Identifier logic belongs in `ids`.
//! - Error modeling belongs in `error`.
//! - Property and label storage shapes belong in `properties`.
//! - Confidence validation belongs in `confidence`.
//! - Node models and node input builders belong in `node`.
//! - Relationship models and relationship input builders belong in `relationship`.
//! - Graph storage, lifecycle operations, and traversal APIs belong in `graph`.
//! - The bounded working-set data model and warm adjacency frontier entries
//!   belong in `working_set`.
//! - Working-set loading explanation contracts belong in
//!   `working_set_explanation`.
//! - In-memory working-set lifecycle and record tracking belong in
//!   `working_set_manager`.
//! - Working-set decision telemetry contracts and the manager-owned recorder
//!   belong in `working_set_telemetry`.
//! - Pheromone trace contracts and the telemetry-fed pheromone field belong in
//!   `pheromone_trace`.
//! - The anti-pheromone negative navigation field belongs in `anti_pheromone`.
//! - Contextual bandit context, action, reward, and controller contracts
//!   belong in `bandit_controller`.
//! - The working-set benchmark harness and baseline policy suite belong in
//!   `working_set_benchmark`.
//! - The explicit epistemic node and relation vocabulary belongs in
//!   `epistemic_vocabulary`.
//! - The proof-carrying answer envelope belongs in `proof_carrying_answer`.
//! - Active-investigation assessment output belongs in `calibrated_assessment`.
//! - Budget-aware investigation stop decisions belong in `stop_condition`.
//! - The retrieval-completeness computation belongs in
//!   `retrieval_completeness`.
//! - The bitemporal fact model and as-of query semantics belong in
//!   `bitemporal`.
//! - Full-trajectory provenance capture belongs in `trajectory_provenance`.
//! - The immune graph tier model belongs in `graph_tiers`.
//! - Immune structural validators belong in `structural_validators`.
//! - Immune epistemic validators belong in `epistemic_validators`.
//! - Immune behavioral validators belong in `behavioral_validators`.
//! - The non-destructive immune response belongs in `immune_response`.
//! - Verification probe generation and lifecycle belong in
//!   `verification_probes`.
//! - Budgeted working-set expansion request/result contracts belong in
//!   `working_set_expansion`.
//! - Graph pager contracts and storage references belong in `graph_pager`.
//! - Expansion budget and supernode safety contracts belong in `expansion_budget`.
//! - Loading profile contracts and registry belong in `loading_profile`.
//! - Semantic seed request/response contracts and resolver traits belong in
//!   `semantic_seed`.
//! - Hypothetical world/branch model contracts belong in `world_branch`.
//! - Branch-local hypothetical records belong in `branch_overlay`.
//! - Deterministic comparison of hypothetical branches belongs in
//!   `cross_branch_scoring`.
//! - Counterfactual and discriminating-evidence queries belong in
//!   `branch_evidence_query`.
//! - Audited branch merge/discard decisions belong in `branch_resolution`.
//! - Multi-resolution level model and derivation links belong in
//!   `resolution_model`.
//! - Question-driven resolution selection belongs in
//!   `question_resolution_selection`.
//! - First-class n-ary coordinated-event contracts belong in `hyperrelation`.
//! - Mixed resolution and hyperrelation traversal semantics belong in
//!   `mixed_traversal`.
//! - Internal adjacency indexes are private to the graph core.
//! - Semantic retrieval implementations, prefetch policies, persistent storage
//!   catalogs, domain expansion implementations, and domain-specific rules
//!   (CTI, FIMI, crisis) live outside `graph-core`.
//!
//! # Public API contract
//!
//! Callers should import stable graph-core types through this facade instead of
//! reaching into private modules. Tests should exercise the public API exposed
//! here, not the crate's internal module layout.

mod actionability;
mod adjacency;
mod advanced_loading_policy;
mod anti_pheromone;
mod bandit_controller;
mod behavioral_validators;
mod bitemporal;
mod branch_evidence_query;
mod branch_overlay;
mod branch_resolution;
mod calibrated_assessment;
mod claim;
mod cluster_aggregation;
pub use actionability::{ActionabilityAssessment, ActionabilityBlocker, ActionabilityPolicy};
mod reconciliation;
pub use ids::ReconciliationRecordId;
pub use reconciliation::{
    ReconciliationCitation, ReconciliationDecider, ReconciliationEvidence, ReconciliationFeature,
    ReconciliationInput, ReconciliationOutcome, ReconciliationRecord, ReconciliationSimilarity,
    ReconciliationSimilarityKind, ReconciliationStore,
};
mod entity_mention;
pub use entity_mention::{
    EntityMention, EntityMentionInput, EntityMentionStore, MentionFeatures, MentionOffsets,
    MentionRelationDirection, MentionRelationFeature,
};
pub use ids::EntityMentionId;
mod candidate_constraints;
pub use candidate_constraints::{
    CandidateConstraint, CandidateFailure, CandidateRepair, CandidateRule, CandidateValidation,
    CandidateValueType,
};
mod candidate_ingestion;
pub use candidate_ingestion::{
    CandidateInput, CandidatePromotion, CandidatePromotionInput, CandidateStore,
};
mod confidence;
mod confidence_dimensions;
mod cross_branch_scoring;
mod deterministic_verifiers;
mod epistemic_stores;
mod epistemic_validators;
mod epistemic_vocabulary;
mod error;
mod evidence;
mod execution_status;
mod expansion_budget;
mod export_metadata;
mod export_plan;
mod graph;
mod graph_pager;
mod graph_tiers;
mod hyperrelation;
mod ids;
mod immune_response;
mod independence;
mod information_gain;
mod loading_profile;
mod mixed_traversal;
mod next_best_evidence;
mod node;
mod observation;
mod pheromone_trace;
mod proof_carrying_answer;
mod properties;
mod question_resolution_selection;
mod relationship;
mod resolution_model;
mod retrieval_completeness;
mod runtime;
mod semantic_seed;
mod semantic_seed_graph_resolver;
mod snapshot;
mod source;
mod source_authority;
mod status;
mod stop_condition;
mod structural_validators;
mod temporal;
mod trajectory_provenance;
mod transaction;
mod traversal_cost;
mod validation;
mod verdict;
mod verdict_explanation;
pub use verdict_explanation::{
    ExplainedCluster, ExplainedMember, UncertaintyKind, VerdictExplanation,
};
mod verification_probes;
mod verifier;
mod working_set;
mod working_set_benchmark;
mod working_set_expansion;
mod working_set_explanation;
mod working_set_manager;
mod working_set_telemetry;
mod world_branch;

pub use advanced_loading_policy::{
    EvictionDecision, EvictionDecisionReason, EvictionPolicy, EvictionPolicyKind,
    EvictionProtectionRule, PrefetchDecision, PrefetchDecisionKind, PrefetchMetrics,
    WorkingSetObservabilityMetrics,
};
pub use anti_pheromone::{
    AntiPheromoneField, AntiPheromoneSignal, AntiPheromoneVector, navigation_field_score,
};
pub use behavioral_validators::{
    BehavioralBounds, BehavioralValidationInputs, validate_graph_behavior,
};
pub use bitemporal::{BitemporalFactState, BitemporalFactStore, BitemporalStamp};
pub use branch_evidence_query::{
    BranchEvidenceObservation, BranchObservationAssessment, BranchObservationEffect,
    BranchSelector, CounterfactualExpectedFactsResult, DiscriminatingEvidenceResult,
    DiscriminatingObservation, SmallestDisprovingEvidenceResult, SourceRemovalBranchImpact,
    SourceRemovalImpactResult, query_counterfactual_expected_facts,
    query_discriminating_observations, query_smallest_disproving_evidence,
    query_source_removal_impact,
};
pub use branch_overlay::{
    BranchContradiction, BranchContradictionId, BranchDerivedRelation, BranchDerivedRelationId,
    BranchExpectedEvidence, BranchOverlay, BranchOverlayReference, BranchPrediction,
    BranchPredictionId, ExpectedEvidenceMarkerId, OverlayHypothesis, OverlayHypothesisId,
};
pub use branch_resolution::{
    BranchResolutionAuditMetadata, BranchResolutionDecision, BranchResolutionDecisionId,
    BranchResolutionKind, BranchResolutionLedger, BranchValidationAuditMetadata,
    CanonicalPromotionRecord,
};
pub use calibrated_assessment::CalibratedAssessment;

pub use bandit_controller::{
    BanditContext, BanditReward, BanditRewardWeights, FrontierDegree, GreedyExpandController,
    WorkingSetAction, WorkingSetController,
};
pub use claim::{
    AgentStance, AgentStanceInput, AgentStancePatch, BeliefState, Claim, ClaimAnalyticalTarget,
    ClaimArithmeticConstraint, ClaimArithmeticPart, ClaimConfidenceTarget, ClaimDecision,
    ClaimDecisionKind, ClaimEvidenceTargetRef, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimModality, ClaimPolarity, ClaimProposition, ClaimPropositionObject,
    ClaimSourceTargetRef, ClaimStatement, ClaimStatus, ClaimStore, ClaimTarget, ClaimTargetKind,
    ClaimTargetMetadata, ClaimTargetValidationContext, ClaimTemporalTarget, ClaimValidTimeScope,
    EpistemicExplanation, EpistemicExplanationKind, EpistemicResolution,
    EpistemicResolutionContext, EpistemicResolutionPolicy, EpistemicResolutionPolicyKind,
    EpistemicResolutionPolicyRegistration, HypothesisWorkspace, HypothesisWorkspaceInput,
    HypothesisWorkspaceStatus, RegisteredEpistemicResolutionPolicy, ResolutionTrustInput,
    StanceKind, TrustInput, TrustInputInput, TrustInputKind,
};
pub use confidence::Confidence;
pub use confidence_dimensions::{ConfidenceDimension, ConfidenceDimensions};
pub use cross_branch_scoring::{
    CrossBranchRanking, CrossBranchScoreBreakdown, CrossBranchScoreInput, CrossBranchScoreTerm,
    RankedBranchScore, rank_cross_branch_scores,
};
pub use deterministic_verifiers::{
    ARITHMETIC_CONSISTENCY_VERIFIER_ID, ARITHMETIC_CONSISTENCY_VERIFIER_VERSION,
    ArithmeticConsistencyVerifier, CONTENT_HASH_VERIFIER_ID, CONTENT_HASH_VERIFIER_VERSION,
    ContentHashVerifier, GRAPH_CONSISTENCY_VERIFIER_ID, GRAPH_CONSISTENCY_VERIFIER_VERSION,
    GraphConsistencyVerifier, IDENTIFIER_SYNTAX_VERIFIER_ID, IDENTIFIER_SYNTAX_VERIFIER_VERSION,
    IdentifierSyntaxVerifier, SCHEMA_CONSTRAINT_VERIFIER_ID, SCHEMA_CONSTRAINT_VERIFIER_VERSION,
    SchemaConstraintVerifier, TEMPORAL_ORDERING_VERIFIER_ID, TEMPORAL_ORDERING_VERIFIER_VERSION,
    TemporalOrderingVerifier,
};
pub use epistemic_stores::EpistemicStores;
pub use epistemic_validators::{
    CLAIM_LIFECYCLE_WITHOUT_OBSERVATION_PATH_CODE, EpistemicValidationInputs,
    validate_claim_reachability, validate_graph_epistemics,
};
pub use epistemic_vocabulary::{
    EpistemicNodeKind, EpistemicPrimitive, EpistemicRelationKind, classify_epistemic_node,
    epistemic_nodes_of_kind,
};
pub use error::{GraphError, ImmutableRecordKind};
pub use evidence::{
    EvidenceAttachment, EvidenceAttachmentTarget, EvidenceInput, EvidenceLocator, EvidenceRecord,
    EvidenceRecordStore, EvidenceSourceType,
};
pub use execution_status::{
    ExecutionContinuation, ExecutionStatusCode, PageInAwareExecutionStatus,
};
pub use expansion_budget::{
    ExpansionBudget, ExpansionBudgetExceeded, ExpansionBudgetUsage, ExpansionLimit,
    ExpansionSafetyErrorCode, SupernodeExpansionBlocked, SupernodePolicy,
};
pub use export_metadata::{ExportMetadata, ExportMode, ExportProfile, ValidationReportRef};
pub use export_plan::{
    DeterministicExportPlan, ExportPlanOptions, ExportRecord, ExportRecordKind,
    build_deterministic_export_plan, build_deterministic_export_plan_with_options,
    node_eligible_for_export_profile,
};
pub use graph::{Graph, GraphPersistenceSnapshot, GraphSequenceFloor};
pub use graph_pager::{
    AdjacencyDirection, GraphPager, GraphPagerError, GraphPagerResult, GraphRecordMetadata,
    GraphRecordRef, PageIdentity, PageIdentityKind, PageInRequest, PageInResult, PageInStatus,
    PagedAdjacency, PagedAdjacencyEntry, PagedNode, PagedRelationship, StorageRef,
};
pub use graph_tiers::{
    GraphTier, GraphTierRegistry, TierRecordRef, TierTransition, TierTransitionReason,
};
pub use hyperrelation::{
    CoordinatedEventHyperrelation, HyperrelationBinaryProjection, HyperrelationId,
    HyperrelationParticipant, HyperrelationParticipantRole, HyperrelationProjectionType,
    HyperrelationSchema, HyperrelationTimeWindow,
};
pub use ids::{
    ActorId, CandidateId, ClaimId, ClaimVersionId, EvidenceId, ExtractionRunId, FactId,
    HypothesisWorkspaceId, NodeId, NodeVersionId, ObservationId, RelationshipId,
    RelationshipVersionId, RequestId, RuntimeId, SessionId, SnapshotId, SourceId, SourceVersionId,
    StateTransitionId, TransactionId, ValidationErrorId, VerdictId, VerificationRecordId,
    WorkspaceId,
};
pub use immune_response::{ImmuneResponder, ImmuneResponse, ImmuneResponseAction};
pub use independence::{
    DependencyReason, DependencySignal, IndependenceCluster, NearDuplicateArtifact,
    SourceDependencySignals, SourceIndependence,
};
pub use information_gain::{
    CandidateEvidenceOutcome, InformationGainEstimate, InformationGainInput, OutcomeProbability,
    estimate_information_gain,
};
pub use loading_profile::{
    LoadingProfile, LoadingProfileErrorCode, LoadingProfileKind, UnknownLoadingProfile,
    default_crisis_investigation_profile, default_cti_investigation_profile,
    default_fimi_investigation_profile, default_generic_loading_profile, lookup_loading_profile,
};
pub use mixed_traversal::{
    HyperrelationExpansionExplanation, HyperrelationExpansionRequest, HyperrelationExpansionResult,
    MixedTraversalEndpoint, MixedTraversalExplanation, MixedTraversalOperator,
    MixedTraversalResult, MixedTraversalScore, MixedTraversalStep, execute_mixed_traversal,
    query_hyperrelation_expansion,
};
pub use next_best_evidence::{
    InvestigationAction, NextBestEvidenceCandidateInput, NextBestEvidenceConstraints,
    NextBestEvidenceIneligibilityReason, NextBestEvidenceProposalScope, NextBestEvidenceRanking,
    NextBestEvidenceScoreBreakdown, NextBestEvidenceScoreTerm, RankedNextBestEvidenceCandidate,
    rank_next_best_evidence,
};
pub use node::{Node, NodeInput, NodePatch};
pub use observation::{Observation, ObservationInput, ObservationModality, ObservationStore};
pub use pheromone_trace::{
    EdgeUtility, PheromoneDecay, PheromoneField, PheromoneTaskScope, UtilityContext,
    edge_utility_score,
};
pub use proof_carrying_answer::{
    AnswerStatement, EvidenceSubgraph, ProofCarryingAnswer, RetrievalCompleteness,
    SourceProvenanceRef, UnresolvedUnknown,
};
pub use properties::{LabelSet, PropertyMap, PropertyValue};
pub use question_resolution_selection::{
    IntentResolutionMapping, QuestionIntent, ResolutionSelection, ResolutionSelectionReason,
    ResolutionSelectionRequest, ResolutionSelectionTrace, select_question_resolution,
};
pub use relationship::{Relationship, RelationshipInput, RelationshipPatch, RelationshipType};
pub use resolution_model::{
    DerivationLink, DerivationLinkId, MultiResolutionModel, ResolutionArtifact,
    ResolutionArtifactId, ResolutionLevel, ResolutionLevelMetadata, ResolutionRecordRef,
};
pub use retrieval_completeness::{
    CompletenessReduction, CompletenessReductionKind, RetrievalCompletenessReport,
    compute_retrieval_completeness,
};
pub use runtime::{
    GraphRuntime, GraphStoreRef, PagerBackedRuntime, PagerBackedRuntimeQuery,
    PagerBackedRuntimeResult, RuntimeOpenRequest, RuntimePolicyRef, SessionRegistryRef,
    WorkspaceRegistryRef,
};
pub use semantic_seed::{
    SemanticBoundaryPolicy, SemanticDomainProfile, SemanticSeedCandidate,
    SemanticSeedExplanationMetadata, SemanticSeedQueryRequest, SemanticSeedQueryResponse,
    SemanticSeedResolutionError, SemanticSeedResolutionErrorCode, SemanticSeedResolver,
    SemanticSeedRetrievalMode, SourceVisibilityScope,
};
pub use semantic_seed_graph_resolver::GraphSemanticSeedResolver;
pub use snapshot::{Snapshot, SnapshotCreateRequest, SnapshotManager};
pub use source::{
    SOURCE_CONTENT_DRIFT_CODE, Source, SourceContentDrift, SourceInput, SourceRegistration,
    SourceRegistrationOutcome, SourceStore,
};
pub use status::RecordStatus;
pub use stop_condition::{
    InvestigationStopCondition, InvestigationStopConditionDecision, InvestigationStopReason,
    InvestigationStopThresholds, StopConditionBudget, evaluate_investigation_stop_condition,
};
pub use structural_validators::validate_graph_structure;
pub use temporal::{TemporalMetadata, TemporalTimestamp};
pub use trajectory_provenance::{
    RetrievalTrajectory, SurfacingStep, TrajectoryProvenance, TrajectoryStep,
    capture_trajectory_provenance,
};
pub use transaction::TransactionMetadata;
pub use traversal_cost::{
    TraversalBudgetDecision, TraversalCostBreakdown, TraversalCostEstimate, TraversalCostRejection,
};
pub use validation::{
    RuleId, ValidationErrorRecord, ValidationErrorSeverity, ValidationErrorStatus,
    ValidationRuleContext, ValidationRuleRegistry, ValidationTarget,
};
pub use verdict::{
    CLAIM_UNREACHABLE_EVIDENCE_CODE, DimensionMigrationFinding, HypothesisSet, RankedHypothesis,
    ReachabilityGap, ResolutionInputs, ResolutionOutcome, StateTransition, TransitionTrigger,
    VERIFICATION_AUTHORITY_DISAGREEMENT_CODE, Verdict, VerdictAsOf, VerdictState, VerdictStore,
    VerificationCoverage, VerificationCoverageClass, VerificationCoverageEntry,
    VerificationCoverageTarget, VerificationDisagreement, VerificationInputs, VerificationRecord,
    VerificationRecordStore, VerificationResult, lifecycle_token, project_verdict_state,
    resolve_claim_verdict,
};
pub use verification_probes::{
    ProbeAnswer, ProbeKind, ProbeRegistry, ProbeStatus, ProbeTransition, VerificationProbe,
};
pub use verifier::{
    SchemaConstraintEvaluation, SchemaConstraintProvider, SchemaConstraintTarget,
    VerificationContext, VerificationOutcome, VerificationRequest, Verifier, VerifierCostClass,
    VerifierRegistry, VerifierSpec,
};
pub use working_set::{
    GraphWorkingSet, GraphWorkingSetStats, LoadingState, WarmAdjacencyEntry,
    WarmAdjacencyEntryInput, WarmAdjacencyRelevanceScore, WorkingSetId,
};
pub use working_set_benchmark::{
    BenchmarkPolicyKind, PolicyBenchmarkMetrics, WorkingSetBenchmarkReport,
    WorkingSetBenchmarkWorkload, fimi_multi_hop_benchmark_workload,
    render_working_set_benchmark_markdown, run_working_set_benchmark,
};
pub use working_set_expansion::{
    ExpansionDirection, ExpansionFilters, ExpansionGuards, ExpansionRequest, ExpansionResult,
    ExpansionResultStatus, build_supernode_block_explanation, check_supernode_expansion_guards,
    expand_working_set_from_graph_adjacency, expand_working_set_with_controller,
    observed_degree_from_adjacency, record_supernode_blocked_expansion,
};
pub use working_set_explanation::{
    BudgetCounterExplanation, ExpansionFixHint, FixHintScope, HotNodeExplanation,
    HotNodeLoadReason, HotRelationshipExplanation, HotRelationshipLoadReason, SeedNodeExplanation,
    SeedSourceKind, SeedSourceMetadata, SkippedExpansionExplanation, SkippedExpansionReason,
    SupernodeBlockExplanation, SupernodeBlockReason, SupernodeGuard, WarmAdjacencyExplanation,
    WarmAdjacencyReason, WorkingSetExplanation,
};
pub use working_set_manager::{
    GraphWorkingSetCreateRequest, GraphWorkingSetManager, WorkingSetEvictionOutcome,
    WorkingSetHotBudget,
};
pub use working_set_telemetry::{
    RetrievalOutcome, RetrievalTelemetryRecord, TelemetryQueryDescriptor, WorkingSetDecisionEvent,
    WorkingSetTelemetryEvent, WorkingSetTelemetryRecorder,
};
pub use world_branch::{
    BranchCreationInput, BranchDescriptor, BranchId, BranchStatus, HypothesisWorldDescriptor,
    HypothesisWorldModel, WorldId,
};

pub use source_authority::{
    AuthorityTrustRule, ResolvedSourceAuthority, SourceAuthority, SourceAuthorityPolicy,
    SourceAuthorityResolution,
};

pub use cluster_aggregation::{
    CLUSTER_AGGREGATION_POLICY_VERSION, ClusterAggregation, ClusterContribution, ClusterWeight,
    DEFAULT_VERDICT_POLICY_VERSION, WITHIN_CLUSTER_INCREMENT_CAP,
};

pub use verdict::resolve_current_claim_verdict;
