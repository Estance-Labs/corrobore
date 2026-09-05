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
use super::*;

/// Explanation entry category for epistemic operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EpistemicExplanationKind {
    /// Claim state change.
    ClaimStateChange,
    /// Support link.
    SupportLink,
    /// Refutation link.
    RefutationLink,
    /// Contradiction link.
    ContradictionLink,
    /// Supersession link.
    SupersessionLink,
    /// Retraction.
    Retraction,
    /// Rejection.
    Rejection,
    /// Stance update.
    StanceUpdate,
    /// Resolution output.
    ResolutionOutput,
    /// A context link was attached (Epic 0029).
    ContextLink,
    /// A duplicate link was attached (Epic 0029).
    DuplicateLink,
    /// A derivation link was attached (Epic 0029).
    DerivationLink,
    /// A dependency link was attached (Epic 0029).
    DependencyLink,
}

/// Explanation metadata for epistemic operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpistemicExplanation {
    pub(crate) target_ref: String,
    pub(crate) kind: EpistemicExplanationKind,
    pub(crate) consumed_inputs: Vec<String>,
    pub(crate) actor_ref: Option<String>,
    pub(crate) session_ref: Option<String>,
    pub(crate) workspace_ref: Option<String>,
    pub(crate) reason_ref: Option<String>,
}

impl EpistemicExplanation {
    /// Creates a new instance.
    pub fn new(target_ref: String, kind: EpistemicExplanationKind) -> Self {
        Self {
            target_ref,
            kind,
            // Consumed inputs.
            consumed_inputs: Vec::new(),
            // Actor ref.
            actor_ref: None,
            // Session ref.
            session_ref: None,
            // Workspace ref.
            workspace_ref: None,
            // Reason ref.
            reason_ref: None,
        }
    }

    /// Sets the consumed input.
    pub fn with_consumed_input(mut self, input: String) -> Self {
        self.consumed_inputs.push(input);
        self
    }

    /// Sets the actor ref.
    pub fn with_actor_ref(mut self, actor_ref: Option<String>) -> Self {
        self.actor_ref = actor_ref;
        self
    }

    /// Sets the session ref.
    pub fn with_session_ref(mut self, session_ref: Option<String>) -> Self {
        self.session_ref = session_ref;
        self
    }

    /// Sets the workspace ref.
    pub fn with_workspace_ref(mut self, workspace_ref: Option<String>) -> Self {
        self.workspace_ref = workspace_ref;
        self
    }

    /// Sets the reason ref.
    pub fn with_reason_ref(mut self, reason_ref: Option<String>) -> Self {
        self.reason_ref = reason_ref;
        self
    }

    /// Target ref.
    pub fn target_ref(&self) -> &str {
        self.target_ref.as_str()
    }

    /// Kind.
    pub fn kind(&self) -> EpistemicExplanationKind {
        self.kind
    }

    /// Consumed inputs.
    pub fn consumed_inputs(&self) -> &[String] {
        self.consumed_inputs.as_slice()
    }

    /// Actor ref.
    pub fn actor_ref(&self) -> Option<&str> {
        self.actor_ref.as_deref()
    }

    /// Session ref.
    pub fn session_ref(&self) -> Option<&str> {
        self.session_ref.as_deref()
    }

    /// Workspace ref.
    pub fn workspace_ref(&self) -> Option<&str> {
        self.workspace_ref.as_deref()
    }

    /// Reason ref.
    pub fn reason_ref(&self) -> Option<&str> {
        self.reason_ref.as_deref()
    }
}

/// Trust signal representation used by deterministic resolution policies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolutionTrustInput {
    pub(crate) subject_ref: String,
    pub(crate) value: f64,
}

impl ResolutionTrustInput {
    /// Creates a new instance.
    pub fn new(subject_ref: String, value: f64) -> Self {
        Self { subject_ref, value }
    }

    /// Subject ref.
    pub fn subject_ref(&self) -> &str {
        self.subject_ref.as_str()
    }

    /// Value.
    pub fn value(&self) -> f64 {
        self.value
    }
}

/// Deterministic policy evaluation context for a single claim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EpistemicResolutionContext {
    pub(crate) claim: Claim,
    pub(crate) links: Vec<ClaimLink>,
    pub(crate) stances: Vec<AgentStance>,
    pub(crate) trust_inputs: Vec<ResolutionTrustInput>,
    pub(crate) temporal: TemporalMetadata,
    pub(crate) policy_metadata: Vec<(String, String)>,
}

impl EpistemicResolutionContext {
    /// Creates a new instance.
    pub fn new(claim: Claim) -> Self {
        Self {
            claim,
            // Links.
            links: Vec::new(),
            // Stances.
            stances: Vec::new(),
            // Trust inputs.
            trust_inputs: Vec::new(),
            // Temporal.
            temporal: TemporalMetadata::default(),
            // Policy metadata.
            policy_metadata: Vec::new(),
        }
    }

    /// Sets the link.
    pub fn with_link(mut self, link: ClaimLink) -> Self {
        self.links.push(link);
        self
    }

    /// Sets the stance.
    pub fn with_stance(mut self, stance: AgentStance) -> Self {
        self.stances.push(stance);
        self
    }

    /// Sets the trust input.
    pub fn with_trust_input(mut self, trust_input: ResolutionTrustInput) -> Self {
        self.trust_inputs.push(trust_input);
        self
    }

    /// Sets the temporal.
    pub fn with_temporal(mut self, temporal: TemporalMetadata) -> Self {
        self.temporal = temporal;
        self
    }

    /// Sets the policy metadata.
    pub fn with_policy_metadata(mut self, key: String, value: String) -> Self {
        self.policy_metadata.push((key, value));
        self
    }

    /// Claim.
    pub fn claim(&self) -> &Claim {
        &self.claim
    }

    /// Links.
    pub fn links(&self) -> &[ClaimLink] {
        self.links.as_slice()
    }

    /// Stances.
    pub fn stances(&self) -> &[AgentStance] {
        self.stances.as_slice()
    }

    /// Trust inputs.
    pub fn trust_inputs(&self) -> &[ResolutionTrustInput] {
        self.trust_inputs.as_slice()
    }

    /// Temporal.
    pub fn temporal(&self) -> &TemporalMetadata {
        &self.temporal
    }

    /// Policy metadata.
    pub fn policy_metadata(&self) -> &[(String, String)] {
        self.policy_metadata.as_slice()
    }
}

/// Deterministic resolution output for a claim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EpistemicResolution {
    pub(crate) recommended_status: ClaimStatus,
    pub(crate) confidence: Option<f64>,
    pub(crate) explanation: String,
    pub(crate) consumed_input_refs: Vec<String>,
}

impl EpistemicResolution {
    /// Creates a new instance.
    pub fn new(
        recommended_status: ClaimStatus,
        confidence: Option<f64>,
        explanation: String,
        consumed_input_refs: Vec<String>,
    ) -> Self {
        Self {
            recommended_status,
            confidence,
            explanation,
            consumed_input_refs,
        }
    }

    /// Recommended status.
    pub fn recommended_status(&self) -> ClaimStatus {
        self.recommended_status
    }

    /// Confidence.
    pub fn confidence(&self) -> Option<f64> {
        self.confidence
    }

    /// Explanation.
    pub fn explanation(&self) -> &str {
        self.explanation.as_str()
    }

    /// Consumed input refs.
    pub fn consumed_input_refs(&self) -> &[String] {
        self.consumed_input_refs.as_slice()
    }
}

/// Named deterministic policy variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EpistemicResolutionPolicyKind {
    /// Conservative deterministic.
    ConservativeDeterministic,
}

/// Registration payload for named policy entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpistemicResolutionPolicyRegistration {
    pub(crate) name: String,
    pub(crate) kind: EpistemicResolutionPolicyKind,
    pub(crate) metadata: Vec<(String, String)>,
}

impl EpistemicResolutionPolicyRegistration {
    /// Creates a new instance.
    pub fn new(name: String, kind: EpistemicResolutionPolicyKind) -> Self {
        Self {
            name,
            kind,
            // Metadata.
            metadata: Vec::new(),
        }
    }

    /// Sets the metadata.
    pub fn with_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.push((key, value));
        self
    }
}

/// Stored named policy entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredEpistemicResolutionPolicy {
    pub(crate) name: String,
    pub(crate) kind: EpistemicResolutionPolicyKind,
    pub(crate) metadata: Vec<(String, String)>,
}

impl RegisteredEpistemicResolutionPolicy {
    /// Name.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Kind.
    pub fn kind(&self) -> EpistemicResolutionPolicyKind {
        self.kind
    }

    /// Metadata.
    pub fn metadata(&self) -> &[(String, String)] {
        self.metadata.as_slice()
    }
}

/// Deterministic policy boundary for resolving a claim from explicit inputs.
pub trait EpistemicResolutionPolicy {
    /// Evaluates the resolution policy against the given context.
    fn evaluate(
        &self,
        context: &EpistemicResolutionContext,
    ) -> Result<EpistemicResolution, GraphError>;
}

pub(crate) struct ConservativeDeterministicPolicy;

impl EpistemicResolutionPolicy for ConservativeDeterministicPolicy {
    fn evaluate(
        &self,
        context: &EpistemicResolutionContext,
    ) -> Result<EpistemicResolution, GraphError> {
        let mut support_score = 0_u64;
        let mut refute_score = 0_u64;
        let mut consumed_input_refs = Vec::new();

        consumed_input_refs.push(format!("claim:{}", context.claim().id().as_str()));

        for link in context.links() {
            match link.kind() {
                ClaimLinkKind::Supports => support_score += 1,
                ClaimLinkKind::Refutes | ClaimLinkKind::Contradicts => refute_score += 1,
                // Supersession and the Epic 0029 structural kinds carry no
                // support or refutation weight in this conservative policy.
                ClaimLinkKind::Supersedes
                | ClaimLinkKind::ContextFor
                | ClaimLinkKind::Duplicates
                | ClaimLinkKind::DerivedFrom
                | ClaimLinkKind::DependsOn => {}
            }

            consumed_input_refs.push(format!(
                "link:{}:{}",
                link.kind() as u8,
                link.target_claim_id().as_str()
            ));
        }

        for stance in context.stances() {
            match stance.stance() {
                StanceKind::Supports | StanceKind::Accepts => support_score += 1,
                StanceKind::Refutes | StanceKind::Rejects | StanceKind::Disputes => {
                    refute_score += 1
                }
                StanceKind::WithholdsJudgment => {}
            }

            consumed_input_refs.push(format!("stance:{}", stance.stance_id()));
        }

        for trust_input in context.trust_inputs() {
            if trust_input.value() >= 0.6 {
                support_score += 1;
            } else {
                refute_score += 1;
            }

            consumed_input_refs.push(format!("trust:{}", trust_input.subject_ref()));
        }

        let total_score = support_score + refute_score;
        let confidence = if total_score == 0 {
            None
        } else {
            Some((support_score.max(refute_score) as f64) / (total_score as f64))
        };

        let recommended_status = if support_score > refute_score && support_score > 0 {
            ClaimStatus::Supported
        } else if refute_score > support_score && refute_score > 0 {
            ClaimStatus::Disputed
        } else {
            ClaimStatus::Unresolved
        };

        let explanation = format!(
            "conservative deterministic resolution: support_score={}, refute_score={}, status={:?}",
            support_score, refute_score, recommended_status
        );

        Ok(EpistemicResolution::new(
            recommended_status,
            confidence,
            explanation,
            consumed_input_refs,
        ))
    }
}
