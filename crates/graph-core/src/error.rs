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
use thiserror::Error;

use crate::claim::{ClaimStatus, ClaimTarget};
use crate::expansion_budget::{ExpansionBudgetExceeded, SupernodeExpansionBlocked};
use crate::ids::HypothesisWorkspaceId;
use crate::loading_profile::UnknownLoadingProfile;
use crate::semantic_seed::SemanticSeedResolutionError;
use crate::{
    ClaimId, EvidenceId, FactId, NodeId, NodeVersionId, ObservationId, RelationshipId,
    RelationshipVersionId, SourceId, WorkingSetId,
};

/// Typed error model for graph-core operations.
///
/// This enum is the single public error boundary for the in-memory graph core.
/// Each variant represents a stable domain failure category that is
/// deterministic, matchable with [`matches!`], and testable without relying on
/// string parsing.
///
/// The in-memory graph core covers identifier validation, property storage,
/// confidence bounds, epistemic claim workflows, and working-set safety
/// contracts such as expansion budget exhaustion, supernode guards, loading
/// profile lookup, and working-set manager lookup. Persistent storage,
/// traversal execution, and domain-specific expansion rules live in separate
/// crates.
///
/// The `NotImplemented` variant acts as a placeholder for APIs whose contract
/// is declared before the implementation is complete. As each operation is
/// implemented, its `NotImplemented` usage is replaced by the typed variants
/// below.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum GraphError {
    /// The requested stable node ID does not exist for an operation where absence
    /// is an error, such as update or tombstone.
    #[error("node not found: {0:?}")]
    NodeNotFound(NodeId),

    /// The requested stable relationship ID does not exist for an operation
    /// where absence is an error, such as update or tombstone.
    #[error("relationship not found: {0:?}")]
    RelationshipNotFound(RelationshipId),

    /// The requested claim ID does not exist for an operation where absence is
    /// an explicit epistemic error.
    #[error("claim not found: {0:?}")]
    ClaimNotFound(ClaimId),

    /// Claim target validation could not resolve the referenced target record.
    #[error("claim target not found: {0:?}")]
    ClaimTargetNotFound(ClaimTarget),

    /// A claim proposition names an entity object that is not a known node.
    #[error("claim proposition entity not found: {0:?}")]
    ClaimPropositionEntityNotFound(NodeId),

    /// The requested evidence ID does not exist for an operation where absence
    /// is an explicit epistemic error.
    #[error("evidence not found: {0:?}")]
    EvidenceNotFound(EvidenceId),

    /// A source reference (for example a parent source) names an identity the
    /// source store has never registered.
    #[error("source not found: {0:?}")]
    SourceNotFound(SourceId),

    /// An observation reference names an identifier the observation store has
    /// never created.
    #[error("observation not found: {0:?}")]
    ObservationNotFound(ObservationId),

    /// A verifier registration is invalid: blank identity, or a different
    /// implementation under an identifier and version that already exist.
    #[error("invalid verifier registration: {0}")]
    InvalidVerifierRegistration(String),

    /// No verifier is registered under this identifier and version.
    #[error("verifier not found: {id} version {version}")]
    VerifierNotFound {
        /// Verifier identifier.
        id: String,
        /// Requested version.
        version: String,
    },

    /// A registered verifier could not complete its external or internal
    /// evaluation and therefore produced no verification record.
    #[error("verifier execution failed for {id} version {version}: {reason}")]
    VerifierExecutionFailed {
        /// Verifier identifier.
        id: String,
        /// Verifier implementation version.
        version: String,
        /// Stable human-readable failure reason.
        reason: String,
    },

    /// A governed record (source, observation, verdict, state transition,
    /// verification record) was re-submitted with different content under an
    /// existing identifier. Governed records are append-only; correction goes
    /// through supersession, never through an in-place update.
    #[error("immutable {kind} record conflict: {id}", kind = kind.as_str())]
    ImmutableRecordConflict {
        /// Record kind.
        kind: ImmutableRecordKind,
        /// Identifier of the existing record.
        id: String,
    },

    /// A claim-link request is invalid according to deterministic guard rules.
    #[error("invalid claim link: {0}")]
    InvalidClaimLink(String),

    /// Contradiction semantics require two distinct claims.
    #[error("self-contradiction is not allowed: {0:?}")]
    SelfContradictionNotAllowed(ClaimId),

    /// Supersession semantics require distinct newer and older claims.
    #[error("self-supersession is not allowed: {0:?}")]
    SelfSupersessionNotAllowed(ClaimId),

    /// Retraction workflows require a non-empty reason for auditability.
    #[error("missing retraction reason")]
    MissingRetractionReason,

    /// Rejection workflows require a non-empty reason for auditability.
    #[error("missing rejection reason")]
    MissingRejectionReason,

    /// Stance lookup failed because the requested stance record is missing.
    #[error("stance not found: {0}")]
    StanceNotFound(String),

    /// Hypothesis workspace lookup failed because the workspace is missing.
    #[error("hypothesis workspace not found: {0:?}")]
    HypothesisWorkspaceNotFound(HypothesisWorkspaceId),

    /// Trust input references a subject that has not been registered.
    #[error("trust subject not found: {0}")]
    TrustSubjectNotFound(String),

    /// Trust input value failed bounded trust-value validation.
    #[error("invalid trust input value: {0}")]
    InvalidTrustInputValue(f64),

    /// Resolution policy lookup failed because the requested policy is missing.
    #[error("resolution policy not found: {0}")]
    ResolutionPolicyNotFound(String),

    /// Resolution policy evaluation failed deterministically.
    #[error("resolution policy evaluation failed: {0}")]
    ResolutionPolicyEvaluationFailed(String),

    /// Claim explanation lookup failed because the target claim has no
    /// explanation entry.
    #[error("claim explanation not found: {0:?}")]
    ClaimExplanationNotFound(ClaimId),

    /// Claim-link explanation lookup failed because the link key is missing.
    #[error("claim link explanation not found: {0}")]
    ClaimLinkExplanationNotFound(String),

    /// Resolution explanation lookup failed because the resolution reference is
    /// missing.
    #[error("resolution explanation not found: {0}")]
    ResolutionExplanationNotFound(String),

    /// Claim target kind is explicitly represented but not supported by current
    /// deterministic validation logic.
    #[error("unsupported claim target kind: {0}")]
    UnsupportedClaimTargetKind(String),

    /// A relationship creation request references a source node that is not
    /// present or is not visible as a current node.
    #[error("source node not found: {0:?}")]
    SourceNodeNotFound(NodeId),

    /// A relationship creation request references a target node that is not
    /// present or is not visible as a current node.
    #[error("target node not found: {0:?}")]
    TargetNodeNotFound(NodeId),

    /// A typed identifier constructor rejected the raw identifier value.
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),

    /// A node label failed graph-core label validation.
    #[error("invalid label: {0}")]
    InvalidLabel(String),

    /// A relationship type failed graph-core relationship-type validation.
    #[error("invalid relationship type: {0}")]
    InvalidRelationshipType(String),

    /// A graph property value failed domain validation.
    #[error("invalid property value: {0}")]
    InvalidPropertyValue(String),

    /// A confidence score failed bounded-confidence validation.
    #[error("invalid confidence: {0}")]
    InvalidConfidence(f64),

    /// A record lifecycle transition is missing native export prerequisites.
    #[error("invalid record status transition: {0}")]
    InvalidRecordStatusTransition(String),

    /// An outcome probability failed finite unit-interval validation.
    #[error("invalid outcome probability: {0}")]
    InvalidOutcomeProbability(f64),

    /// An information-gain request contained an invalid outcome distribution.
    #[error("invalid information-gain input: {0}")]
    InvalidInformationGainInput(String),

    /// A Next Best Evidence score term failed finite unit-interval validation.
    #[error("invalid Next Best Evidence score term: {0}")]
    InvalidNextBestEvidenceScoreTerm(f64),

    /// A Next Best Evidence request contained an invalid candidate set.
    #[error("invalid Next Best Evidence input: {0}")]
    InvalidNextBestEvidenceInput(String),

    /// A calibrated assessment violated its public envelope invariants.
    #[error("invalid calibrated assessment: {0}")]
    InvalidCalibratedAssessment(String),

    /// A stop-condition budget value failed finite unit-interval validation.
    #[error("invalid stop-condition budget: {0}")]
    InvalidStopConditionBudget(f64),

    /// A stop-condition policy configuration violated public invariants.
    #[error("invalid stop-condition policy: {0}")]
    InvalidStopConditionPolicy(String),

    /// A world/branch model request violated immutable-base or lineage invariants.
    #[error("invalid world/branch model: {0}")]
    InvalidWorldBranchModel(String),

    /// A branch-local overlay request violated scope or reference invariants.
    #[error("invalid branch overlay: {0}")]
    InvalidBranchOverlay(String),

    /// A cross-branch score term was not finite or within the unit interval.
    #[error("invalid cross-branch score term: {0}")]
    InvalidCrossBranchScoreTerm(f64),

    /// Cross-branch comparison inputs violated scope or uniqueness invariants.
    #[error("invalid cross-branch comparison: {0}")]
    InvalidCrossBranchComparison(String),

    /// A branch evidence query violated selector, provenance, or input invariants.
    #[error("invalid branch evidence query: {0}")]
    InvalidBranchEvidenceQuery(String),

    /// A branch merge or discard violated audit, validation, or lifecycle rules.
    #[error("invalid branch resolution: {0}")]
    InvalidBranchResolution(String),

    /// A multi-resolution artifact or derivation rule violated model invariants.
    #[error("invalid resolution model: {0}")]
    InvalidResolutionModel(String),

    /// Question-driven resolution selection received unsupported or conflicting input.
    #[error("invalid resolution selection: {0}")]
    InvalidResolutionSelection(String),

    /// An n-ary hyperrelation violated schema, role, temporal, or context invariants.
    #[error("invalid hyperrelation: {0}")]
    InvalidHyperrelation(String),

    /// A mixed binary, resolution, or hyperrelation traversal violated query semantics.
    #[error("invalid mixed traversal: {0}")]
    InvalidMixedTraversal(String),

    /// A pheromone decay factor failed bounded-factor validation.
    #[error("invalid pheromone decay factor: {0}")]
    InvalidPheromoneDecay(f64),

    /// A retrieval-completeness ratio failed bounded-ratio validation.
    #[error("invalid retrieval completeness: {0}")]
    InvalidRetrievalCompleteness(f64),

    /// A bitemporal stamp failed canonical-form or interval validation.
    #[error("invalid bitemporal stamp: {0}")]
    InvalidBitemporalStamp(String),

    /// A bitemporal assertion attempted an overwrite-style update.
    #[error("bitemporal overwrite forbidden for fact: {0:?}")]
    BitemporalOverwriteForbidden(FactId),

    /// A graph tier transition violated the immune movement rules.
    #[error("invalid tier transition: {0}")]
    InvalidTierTransition(String),

    /// A verification probe transition violated the lifecycle rules.
    #[error("invalid probe transition: {0}")]
    InvalidProbeTransition(String),

    /// A claim lifecycle transition is not allowed by the current policy.
    #[error("invalid claim status transition: {from:?} -> {to:?}")]
    InvalidClaimStatusTransition {
        /// From.
        from: ClaimStatus,
        /// To.
        to: ClaimStatus,
    },

    /// Versioned storage detected a broken transition or impossible current
    /// state that callers should not be able to create directly.
    #[error("invalid version state: {0}")]
    InvalidVersionState(String),

    /// The operation cannot be applied because the current record state is
    /// already tombstoned.
    #[error("record already tombstoned: {0}")]
    RecordAlreadyTombstoned(String),

    /// Internal graph state violated an invariant that should hold after public
    /// API validation has succeeded.
    #[error("internal invariant violation: {0}")]
    InternalInvariantViolation(String),

    /// Expansion was stopped because a configured working-set budget limit would
    /// be exceeded.
    #[error("expansion budget exceeded: {0:?}")]
    ExpansionBudgetExceeded(ExpansionBudgetExceeded),

    /// Expansion was stopped because a high-degree node requires additional
    /// narrowing guards before traversal may continue.
    #[error("supernode expansion blocked: {0:?}")]
    SupernodeExpansionBlocked(SupernodeExpansionBlocked),

    /// Loading profile lookup was requested with a name not present in the
    /// built-in profile registry.
    #[error("unknown loading profile: {0:?}")]
    UnknownLoadingProfile(UnknownLoadingProfile),

    /// Working set lookup was requested with an ID that is not registered in the
    /// in-memory working set manager.
    #[error("working set not found: {0:?}")]
    WorkingSetNotFound(WorkingSetId),

    /// Runtime state was required to be open for an operation, but the runtime
    /// has not been opened yet.
    #[error("runtime is not open")]
    RuntimeNotOpen,

    /// Runtime open request failed validation at the runtime boundary.
    #[error("invalid runtime configuration: {0}")]
    InvalidRuntimeConfiguration(String),

    /// Semantic seed resolver failed to produce a safe deterministic seed set.
    #[error("semantic seed resolution failed: {0:?}")]
    SemanticSeedResolutionFailed(SemanticSeedResolutionError),

    /// Export metadata model validation rejected a required field value.
    #[error("invalid export metadata field: {0}")]
    InvalidExportMetadataField(String),

    /// Strict export-mode readiness validation failed.
    #[error("strict export mode rejected export plan: {0}")]
    ExportStrictModeRejected(String),

    /// Temporary placeholder for APIs whose contract is declared before the
    /// implementation is complete.
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
}

impl GraphError {
    /// Build a typed node-version-state error for a missing current-version
    /// pointer.
    pub fn missing_current_node_version_pointer(node_id: &NodeId) -> Self {
        Self::InvalidVersionState(format!(
            "missing current node version pointer for {}",
            node_id.as_str()
        ))
    }

    /// Build a typed node-version-state error for a dangling current-version
    /// pointer.
    pub fn missing_current_node_version(node_id: &NodeId, version_id: &NodeVersionId) -> Self {
        Self::InvalidVersionState(format!(
            "current node version {} is missing for {}",
            version_id.as_str(),
            node_id.as_str()
        ))
    }

    /// Build a typed node-version-state error for a missing previous version
    /// during an append-only transition.
    pub fn missing_previous_node_version(
        node_id: &NodeId,
        previous_version_id: &NodeVersionId,
    ) -> Self {
        Self::InvalidVersionState(format!(
            "previous node version {} is missing for {}",
            previous_version_id.as_str(),
            node_id.as_str()
        ))
    }

    /// Build a typed relationship-version-state error for a missing
    /// current-version pointer.
    pub fn missing_current_relationship_version_pointer(relationship_id: &RelationshipId) -> Self {
        Self::InvalidVersionState(format!(
            "missing current relationship version pointer for {}",
            relationship_id.as_str()
        ))
    }

    /// Build a typed relationship-version-state error for a dangling
    /// current-version pointer.
    pub fn missing_current_relationship_version(
        relationship_id: &RelationshipId,
        version_id: &RelationshipVersionId,
    ) -> Self {
        Self::InvalidVersionState(format!(
            "current relationship version {} is missing for {}",
            version_id.as_str(),
            relationship_id.as_str()
        ))
    }

    /// Build a typed relationship-version-state error for a missing previous
    /// version during an append-only transition.
    pub fn missing_previous_relationship_version(
        relationship_id: &RelationshipId,
        previous_version_id: &RelationshipVersionId,
    ) -> Self {
        Self::InvalidVersionState(format!(
            "previous relationship version {} is missing for {}",
            previous_version_id.as_str(),
            relationship_id.as_str()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node ID should be valid")
    }

    fn node_version_id(value: &str) -> NodeVersionId {
        NodeVersionId::new(value).expect("test node version ID should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship ID should be valid")
    }

    fn relationship_version_id(value: &str) -> RelationshipVersionId {
        RelationshipVersionId::new(value).expect("test relationship version ID should be valid")
    }

    //
    // Verify that graph error variants remain directly matchable without relying
    // on display strings. This is the core unit-level value of the typed error
    // model introduced for graph-core.
    //
    // Given representative `GraphError` values,
    // when callers use pattern matching,
    // then each branch should be identifiable by its variant and payload.
    #[test]
    fn graph_error_variants_are_matchable_by_branch() {
        let missing_node_id = node_id("node--missing");
        let missing_relationship_id = relationship_id("relationship--missing");

        assert!(matches!(
        GraphError::NodeNotFound(missing_node_id.clone()),
        GraphError::NodeNotFound(id) if id == missing_node_id
        ));
        assert!(matches!(
        GraphError::RelationshipNotFound(missing_relationship_id.clone()),
        GraphError::RelationshipNotFound(id) if id == missing_relationship_id
        ));
        assert!(matches!(
        GraphError::InvalidIdentifier("NodeId".to_owned()),
        GraphError::InvalidIdentifier(kind) if kind == "NodeId"
        ));
        assert!(matches!(
        GraphError::InvalidConfidence(1.01),
        GraphError::InvalidConfidence(value) if value == 1.01
        ));
    }

    //
    // Verify that graph error values can be cloned and compared in focused unit
    // tests. Primitive and graph tests should be able to assert exact typed error
    // outcomes without string parsing.
    //
    // Given a representative `GraphError`,
    // when it is cloned and compared,
    // then the cloned value should remain equal to the original branch.
    #[test]
    fn graph_error_values_are_cloneable_and_comparable() {
        let error = GraphError::InvalidLabel("ThreatActor".to_owned());
        let cloned = error.clone();

        assert_eq!(cloned, error);
    }

    //
    // Verify that node version-state helper constructors produce typed
    // `InvalidVersionState` errors. These helpers are the stable unit boundary
    // for internal node-version invariant failures.
    //
    // Given node and node-version identifiers,
    // when node version-state helper constructors are called,
    // then they should return `GraphError::InvalidVersionState` with useful context.
    #[test]
    fn node_version_state_helpers_return_invalid_version_state() {
        let id = node_id("node--1");
        let version_id = node_version_id("node-version--1");

        for error in [
            GraphError::missing_current_node_version_pointer(&id),
            GraphError::missing_current_node_version(&id, &version_id),
            GraphError::missing_previous_node_version(&id, &version_id),
        ] {
            assert!(matches!(
            error,
            GraphError::InvalidVersionState(message)
            if message.contains(id.as_str())
            ));
        }
    }

    //
    // Verify that relationship version-state helper constructors produce typed
    // `InvalidVersionState` errors. These helpers are the stable unit boundary
    // for internal relationship-version invariant failures.
    //
    // Given relationship and relationship-version identifiers,
    // when relationship version-state helper constructors are called,
    // then they should return `GraphError::InvalidVersionState` with useful context.
    #[test]
    fn relationship_version_state_helpers_return_invalid_version_state() {
        let id = relationship_id("relationship--1");
        let version_id = relationship_version_id("relationship-version--1");

        for error in [
            GraphError::missing_current_relationship_version_pointer(&id),
            GraphError::missing_current_relationship_version(&id, &version_id),
            GraphError::missing_previous_relationship_version(&id, &version_id),
        ] {
            assert!(matches!(
            error,
            GraphError::InvalidVersionState(message)
            if message.contains(id.as_str())
            ));
        }
    }

    //
    // Verify that display output exists only as a human-readable diagnostic layer.
    // Unit tests should document the message shape but still prefer variant
    // matching for control flow assertions.
    //
    // Given a representative `GraphError`,
    // when it is formatted with `to_string`,
    // then the output should contain the branch-specific diagnostic text.
    #[test]
    fn graph_error_display_text_contains_branch_context() {
        let error = GraphError::InvalidRelationshipType("".to_owned());
        let display = error.to_string();

        assert!(display.contains("invalid relationship type"));
    }
}

/// Kinds of append-only governed records protected by
/// [`GraphError::ImmutableRecordConflict`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ImmutableRecordKind {
    /// An observation-bound entity mention.
    EntityMention,
    /// `Source` version.
    Source,
    /// `Observation`.
    Observation,
    /// `Verdict`.
    Verdict,
    /// `StateTransition`.
    StateTransition,
    /// `VerificationRecord`.
    VerificationRecord,
}

impl ImmutableRecordKind {
    /// Canonical lowercase token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EntityMention => "entity_mention",
            Self::Source => "source",
            Self::Observation => "observation",
            Self::Verdict => "verdict",
            Self::StateTransition => "state_transition",
            Self::VerificationRecord => "verification_record",
        }
    }
}
